//! Drive identity (diskutil VolumeUUID) and mount tracking.
//! Files are located by volume UUID + relative path, never by absolute path.

use rusqlite::{params, Connection};
use serde::Serialize;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

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

/// The mount point a (canonical) path lives on: `/Volumes/<name>` for external
/// drives, `/` for everything on the startup disk.
pub fn mount_point_of(canonical: &Path) -> PathBuf {
    let mut parts = canonical.components();
    if let (Some(Component::RootDir), Some(Component::Normal(v)), Some(Component::Normal(name))) =
        (parts.next(), parts.next(), parts.next())
    {
        if v == "Volumes" {
            return Path::new("/Volumes").join(name);
        }
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

pub fn list(conn: &Connection) -> Result<Vec<Volume>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT v.id, v.uuid, v.label, v.mount_path,
                    (SELECT count(*) FROM assets a WHERE a.volume_id = v.id AND a.missing = 0)
             FROM volumes v ORDER BY v.label COLLATE NOCASE",
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
