//! Drive identity and mount tracking.
//! Files are located by a stable volume ID + relative path, never by absolute path:
//! - macOS: `diskutil` VolumeUUID
//! - Linux: filesystem UUID from /dev/disk/by-uuid (the mount table says what is mounted where)
//! - Windows: the volume GUID behind each drive letter

use rusqlite::{params, Connection};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub use platform::{info_for_mount, mount_point_of, mounted};

#[derive(Debug, Clone)]
pub struct VolumeInfo {
    pub uuid: String,
    pub label: String,
    pub mount_path: PathBuf,
}

#[derive(Debug, Serialize)]
pub struct Volume {
    pub id: i64,
    pub uuid: String,
    pub label: String,
    pub mount_path: Option<String>,
    pub online: bool,
    pub asset_count: i64,
}

pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// A file's full path from its drive's mount point and its catalog path
/// ('/'-separated), joined with the platform's own separator.
pub fn resolve(mount: impl AsRef<Path>, rel: &str) -> PathBuf {
    rel.split('/')
        .filter(|part| !part.is_empty())
        .fold(mount.as_ref().to_path_buf(), |path, part| path.join(part))
}

/// Canonical form of a folder the user picked, without Windows' `\\?\` prefix.
pub fn canonicalize(path: &Path) -> std::io::Result<PathBuf> {
    dunce::canonicalize(path)
}

/// Insert or update a volume by UUID and mark it as mounted. Returns its id.
pub fn upsert(conn: &Connection, info: &VolumeInfo) -> Result<i64, String> {
    conn.query_row(
        "INSERT INTO volumes (uuid, label, mount_path, last_seen_at) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(uuid) DO UPDATE SET
           label = excluded.label, mount_path = excluded.mount_path, last_seen_at = excluded.last_seen_at
         RETURNING id",
        params![
            info.uuid,
            info.label,
            info.mount_path.to_string_lossy(),
            now()
        ],
        |r| r.get(0),
    )
    .map_err(|e| e.to_string())
}

/// Reconcile known volumes with what is mounted right now. Unplugged drives get
/// `mount_path = NULL`; that's the normal offline state, not an error.
pub fn apply_mounted(conn: &Connection, mounted: &[VolumeInfo]) -> Result<(), String> {
    let known: Vec<(i64, String)> = {
        let mut stmt = conn
            .prepare("SELECT id, uuid FROM volumes")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<_, _>>().map_err(|e| e.to_string())?
    };
    for (id, uuid) in known {
        match mounted.iter().find(|m| m.uuid == uuid) {
            Some(m) => conn.execute(
                "UPDATE volumes SET mount_path = ?1, label = ?2, last_seen_at = ?3 WHERE id = ?4",
                params![m.mount_path.to_string_lossy(), m.label, now(), id],
            ),
            None => conn.execute("UPDATE volumes SET mount_path = NULL WHERE id = ?1", [id]),
        }
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Drives in your order (drag to sort in the sidebar), new ones alphabetically after.
pub fn list(conn: &Connection) -> Result<Vec<Volume>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT v.id, v.uuid, v.label, v.mount_path,
                    (SELECT count(*) FROM assets a WHERE a.volume_id = v.id AND a.missing = 0
                       AND a.trashed_at IS NULL)
             FROM volumes v ORDER BY v.position IS NULL, v.position, v.label COLLATE NOCASE",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            let mount_path: Option<String> = r.get(3)?;
            Ok(Volume {
                id: r.get(0)?,
                uuid: r.get(1)?,
                label: r.get(2)?,
                online: mount_path.is_some(),
                mount_path,
                asset_count: r.get(4)?,
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<_, _>>().map_err(|e| e.to_string())
}

#[cfg(target_os = "macos")]
mod platform {
    use super::VolumeInfo;
    use std::path::{Component, Path, PathBuf};
    use std::process::Command;

    /// The mount point a (canonical) path lives on: `/Volumes/<name>` for external
    /// drives, `/` for everything on the startup disk.
    pub fn mount_point_of(canonical: &Path) -> PathBuf {
        let mut parts = canonical.components();
        if let (Some(Component::RootDir), Some(Component::Normal(v)), Some(Component::Normal(name))) =
            (parts.next(), parts.next(), parts.next())
            && v == "Volumes"
        {
            return Path::new("/Volumes").join(name);
        }
        PathBuf::from("/")
    }

    /// `diskutil info -plist <mount>` → VolumeUUID, name and mount point.
    pub fn info_for_mount(mount: &Path) -> Result<VolumeInfo, String> {
        let out = Command::new("/usr/sbin/diskutil")
            .arg("info")
            .arg("-plist")
            .arg(mount)
            .output()
            .map_err(|e| format!("cannot run diskutil: {e}"))?;
        let value = plist::Value::from_reader(std::io::Cursor::new(&out.stdout))
            .map_err(|e| format!("cannot parse diskutil output for {}: {e}", mount.display()))?;
        let dict = value
            .as_dictionary()
            .ok_or_else(|| "unexpected diskutil output".to_string())?;
        let get = |key: &str| dict.get(key).and_then(|v| v.as_string()).map(str::to_owned);

        let uuid = get("VolumeUUID").filter(|s| !s.is_empty()).ok_or_else(|| {
            format!(
                "{} has no volume UUID (network shares and some disk formats don't), so it can't be cataloged",
                mount.display()
            )
        })?;
        let label = get("VolumeName")
            .filter(|s| !s.is_empty())
            .or_else(|| mount.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_else(|| "Untitled".into());
        let mount_path = get("MountPoint")
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| mount.to_path_buf());
        Ok(VolumeInfo {
            uuid,
            label,
            mount_path,
        })
    }

    /// Every currently mounted volume: the startup disk plus what's in /Volumes.
    /// Mounts diskutil can't identify are skipped.
    pub fn mounted() -> Vec<VolumeInfo> {
        let mut mounts = vec![PathBuf::from("/")];
        if let Ok(entries) = std::fs::read_dir("/Volumes") {
            for e in entries.flatten() {
                // Skip the "/Volumes/Macintosh HD -> /" symlink; real mounts are directories.
                if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                    mounts.push(e.path());
                }
            }
        }
        mounts.iter().filter_map(|m| info_for_mount(m).ok()).collect()
    }
}

/// /proc/self/mountinfo parsing, kept platform-independent so it is tested everywhere.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
mod mountinfo {
    use std::path::{Path, PathBuf};

    #[derive(Debug, Clone, PartialEq)]
    pub struct Mount {
        pub mount: PathBuf,
        pub fstype: String,
        pub source: String,
    }

    /// Undo the kernel's octal escapes (`\040` is a space) in mountinfo fields.
    pub fn unescape(field: &str) -> String {
        let bytes = field.as_bytes();
        let mut out = Vec::with_capacity(bytes.len());
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'\\'
                && i + 3 < bytes.len()
                && bytes[i + 1..=i + 3].iter().all(|b| (b'0'..=b'7').contains(b))
            {
                let n = (bytes[i + 1] - b'0') * 64 + (bytes[i + 2] - b'0') * 8 + (bytes[i + 3] - b'0');
                out.push(n);
                i += 4;
            } else {
                out.push(bytes[i]);
                i += 1;
            }
        }
        String::from_utf8_lossy(&out).into_owned()
    }

    /// Undo udev's `\x20`-style escapes in /dev/disk/by-label names.
    pub fn unescape_hex(name: &str) -> String {
        let bytes = name.as_bytes();
        let mut out = Vec::with_capacity(bytes.len());
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'\\'
                && bytes.get(i + 1) == Some(&b'x')
                && let Some(hex) = name.get(i + 2..i + 4)
                && let Ok(n) = u8::from_str_radix(hex, 16)
            {
                out.push(n);
                i += 4;
                continue;
            }
            out.push(bytes[i]);
            i += 1;
        }
        String::from_utf8_lossy(&out).into_owned()
    }

    /// `36 35 98:0 /mnt1 /mnt2 rw,noatime master:1 - ext3 /dev/root rw` → mount, fstype, source.
    pub fn parse(text: &str) -> Vec<Mount> {
        text.lines()
            .filter_map(|line| {
                let (left, right) = line.split_once(" - ")?;
                let mount = left.split(' ').nth(4)?;
                let mut right = right.split(' ');
                let fstype = right.next()?;
                let source = right.next()?;
                Some(Mount {
                    mount: PathBuf::from(unescape(mount)),
                    fstype: fstype.to_owned(),
                    source: unescape(source),
                })
            })
            .collect()
    }

    /// Mounts that are real drives a person would catalog: block devices, minus
    /// boot partitions and snap images.
    pub fn is_drive(m: &Mount) -> bool {
        let p = m.mount.to_string_lossy();
        m.source.starts_with("/dev/")
            && !m.source.starts_with("/dev/loop")
            && m.fstype != "squashfs"
            && !(p == "/boot" || p.starts_with("/boot/") || p.starts_with("/snap/") || p.starts_with("/var/snap/"))
    }

    /// The mount a path lives on: the longest mount point that contains it.
    pub fn containing<'a>(mounts: &'a [Mount], path: &Path) -> Option<&'a Mount> {
        mounts
            .iter()
            .filter(|m| path.starts_with(&m.mount))
            .max_by_key(|m| m.mount.components().count())
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use super::mountinfo::{self, Mount};
    use super::VolumeInfo;
    use std::path::{Path, PathBuf};

    fn mounts() -> Vec<Mount> {
        std::fs::read_to_string("/proc/self/mountinfo")
            .map(|t| mountinfo::parse(&t))
            .unwrap_or_default()
    }

    /// Name of the /dev/disk/by-<kind> entry that points at `device`.
    fn disk_link(kind: &str, device: &Path) -> Option<String> {
        let device = std::fs::canonicalize(device).ok()?;
        std::fs::read_dir(format!("/dev/disk/by-{kind}"))
            .ok()?
            .flatten()
            .find(|e| std::fs::canonicalize(e.path()).ok().as_deref() == Some(device.as_path()))
            .map(|e| e.file_name().to_string_lossy().into_owned())
    }

    pub fn mount_point_of(canonical: &Path) -> PathBuf {
        let all = mounts();
        mountinfo::containing(&all, canonical)
            .map(|m| m.mount.clone())
            .unwrap_or_else(|| PathBuf::from("/"))
    }

    pub fn info_for_mount(mount: &Path) -> Result<VolumeInfo, String> {
        let all = mounts();
        let m = all
            .iter()
            .rev() // the last entry for a mount point is the one in effect
            .find(|m| m.mount == mount)
            .ok_or_else(|| format!("{} is not a mount point", mount.display()))?;
        let uuid = if m.source.starts_with("/dev/") {
            disk_link("uuid", Path::new(&m.source))
        } else {
            None
        }
        // Filesystems without a UUID (e.g. zfs datasets) are identified by their source.
        .unwrap_or_else(|| format!("{}:{}", m.fstype, m.source));
        let label = disk_link("label", Path::new(&m.source))
            .map(|l| mountinfo::unescape_hex(&l))
            .or_else(|| {
                (mount != Path::new("/"))
                    .then(|| mount.file_name().map(|n| n.to_string_lossy().into_owned()))
                    .flatten()
            })
            .unwrap_or_else(|| "Computer".into());
        Ok(VolumeInfo {
            uuid,
            label,
            mount_path: mount.to_path_buf(),
        })
    }

    /// Every mounted drive: block-device filesystems such as /, /home and
    /// anything under /media, /run/media or /mnt.
    pub fn mounted() -> Vec<VolumeInfo> {
        let all = mounts();
        let mut seen = std::collections::HashSet::new();
        all.iter()
            .rev()
            .filter(|m| mountinfo::is_drive(m) && seen.insert(m.mount.clone()))
            .filter_map(|m| info_for_mount(&m.mount).ok())
            .collect()
    }
}

#[cfg(target_os = "windows")]
mod platform {
    use super::VolumeInfo;
    use std::path::{Component, Path, PathBuf, Prefix};
    use windows_sys::Win32::Storage::FileSystem::{
        GetDriveTypeW, GetLogicalDrives, GetVolumeInformationW, GetVolumeNameForVolumeMountPointW,
    };

    const DRIVE_REMOTE: u32 = 4;
    const DRIVE_CDROM: u32 = 5;

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn from_wide(buf: &[u16]) -> String {
        let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        String::from_utf16_lossy(&buf[..end])
    }

    /// `D:\Footage\day1` → `D:\`.
    pub fn mount_point_of(canonical: &Path) -> PathBuf {
        match canonical.components().next() {
            Some(Component::Prefix(p)) => match p.kind() {
                Prefix::Disk(letter) | Prefix::VerbatimDisk(letter) => {
                    PathBuf::from(format!("{}:\\", letter as char))
                }
                _ => PathBuf::from(p.as_os_str()),
            },
            _ => PathBuf::from("C:\\"),
        }
    }

    /// The volume GUID (`\\?\Volume{…}\`) and label behind a drive letter.
    pub fn info_for_mount(mount: &Path) -> Result<VolumeInfo, String> {
        let root = wide(&mount.to_string_lossy());
        let kind = unsafe { GetDriveTypeW(root.as_ptr()) };
        if kind == DRIVE_REMOTE {
            return Err(format!(
                "{} is a network drive, which has no stable volume ID, so it can't be cataloged",
                mount.display()
            ));
        }
        let mut guid = [0u16; 64];
        let ok = unsafe {
            GetVolumeNameForVolumeMountPointW(root.as_ptr(), guid.as_mut_ptr(), guid.len() as u32)
        };
        if ok == 0 {
            return Err(format!("{} has no volume ID, so it can't be cataloged", mount.display()));
        }
        let mut name = [0u16; 261];
        let has_label = unsafe {
            GetVolumeInformationW(
                root.as_ptr(),
                name.as_mut_ptr(),
                name.len() as u32,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                0,
            )
        } != 0;
        let letter = mount.to_string_lossy().trim_end_matches('\\').to_owned();
        let label = Some(from_wide(&name))
            .filter(|l| has_label && !l.is_empty())
            .map(|l| format!("{l} ({letter})"))
            .unwrap_or_else(|| format!("Local Disk ({letter})"));
        Ok(VolumeInfo {
            uuid: from_wide(&guid),
            label,
            mount_path: mount.to_path_buf(),
        })
    }

    /// Every drive letter with a readable volume (network drives, optical drives
    /// and empty card readers are skipped).
    pub fn mounted() -> Vec<VolumeInfo> {
        let mask = unsafe { GetLogicalDrives() };
        (0..26u8)
            .filter(|i| mask & (1 << i) != 0)
            .map(|i| PathBuf::from(format!("{}:\\", (b'A' + i) as char)))
            .filter(|root| unsafe { GetDriveTypeW(wide(&root.to_string_lossy()).as_ptr()) } != DRIVE_CDROM)
            .filter_map(|root| info_for_mount(&root).ok())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_joins_catalog_paths_with_the_platform_separator() {
        let p = resolve("base", "a/b/clip.mov");
        assert_eq!(p, Path::new("base").join("a").join("b").join("clip.mov"));
        assert_eq!(resolve("base", ""), Path::new("base"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn mount_point_for_external_and_internal_paths() {
        assert_eq!(
            mount_point_of(Path::new("/Volumes/T7/Footage/day1")),
            Path::new("/Volumes/T7")
        );
        assert_eq!(mount_point_of(Path::new("/Volumes/T7")), Path::new("/Volumes/T7"));
        assert_eq!(mount_point_of(Path::new("/Users/me/Movies")), Path::new("/"));
        assert_eq!(mount_point_of(Path::new("/Volumes")), Path::new("/"));
    }

    #[test]
    fn linux_mountinfo_is_parsed_and_filtered() {
        let text = "\
22 1 259:2 / / rw,relatime shared:1 - ext4 /dev/nvme0n1p2 rw
23 22 259:1 / /boot/efi rw shared:2 - vfat /dev/nvme0n1p1 rw
24 22 0:5 / /dev rw shared:3 - devtmpfs udev rw
25 22 7:3 / /snap/core/123 ro shared:4 - squashfs /dev/loop3 ro
26 22 8:17 / /media/me/My\\040SSD rw,nosuid shared:5 - exfat /dev/sdb1 rw
27 22 0:30 / /run/user/1000 rw shared:6 - tmpfs tmpfs rw";
        let mounts = mountinfo::parse(text);
        assert_eq!(mounts.len(), 6);
        assert_eq!(mounts[4].mount, Path::new("/media/me/My SSD"));
        assert_eq!((mounts[4].fstype.as_str(), mounts[4].source.as_str()), ("exfat", "/dev/sdb1"));
        let drives: Vec<_> = mounts.iter().filter(|m| mountinfo::is_drive(m)).map(|m| m.mount.clone()).collect();
        assert_eq!(drives, [PathBuf::from("/"), PathBuf::from("/media/me/My SSD")]);
        let on = |p: &str| mountinfo::containing(&mounts, Path::new(p)).map(|m| m.mount.clone());
        assert_eq!(on("/media/me/My SSD/Footage/a.mov"), Some(PathBuf::from("/media/me/My SSD")));
        assert_eq!(on("/home/me/Videos"), Some(PathBuf::from("/")));
        assert_eq!(mountinfo::unescape_hex("My\\x20SSD"), "My SSD");
    }
}
