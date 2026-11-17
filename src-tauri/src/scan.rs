//! Background folder scan. Strictly read-only on media: it only lists
//! directories and reads file metadata (size, mtime).

use crate::{db::Db, media};
use rusqlite::{params, Connection};
use serde::Serialize;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::UNIX_EPOCH;
use tauri::{AppHandle, Emitter, Manager};
use walkdir::WalkDir;

const VIDEO_EXTS: [&str; 8] = ["mp4", "mov", "m4v", "mxf", "mkv", "mts", "m2ts", "avi"];
/// ffprobe/ffmpeg jobs running at once.
const WORKERS: usize = 2;
const IMAGE_EXTS: [&str; 4] = ["jpg", "jpeg", "png", "heic"];

/// Volumes with a scan in flight, so the same drive isn't scanned twice at once.
#[derive(Default)]
pub struct ScanState {
    active: Mutex<HashSet<i64>>,
}

#[derive(Clone, Serialize)]
pub struct ScanProgress {
    pub volume_id: i64,
    pub found: usize,
    pub processed: usize,
    /// Files needing metadata + thumbnail in this scan.
    pub total: usize,
    pub done: bool,
    pub error: Option<String>,
}

#[derive(Debug, PartialEq)]
pub struct FoundFile {
    pub rel_path: String,
    pub file_name: String,
    pub kind: &'static str,
    pub size_bytes: i64,
    pub mtime: i64,
}

pub fn kind_for(file_name: &str) -> Option<&'static str> {
    let ext = Path::new(file_name).extension()?.to_str()?.to_ascii_lowercase();
    if VIDEO_EXTS.contains(&ext.as_str()) {
        Some("video")
    } else if IMAGE_EXTS.contains(&ext.as_str()) {
        Some("image")
    } else {
        None
    }
}

/// Path of `path` inside the drive mounted at `mount`, with '/' separators.
/// `None` if it isn't under the mount or isn't valid UTF-8.
pub fn rel_path(mount: &Path, path: &Path) -> Option<String> {
    let rel = path.strip_prefix(mount).ok()?;
    let parts: Option<Vec<&str>> = rel.components().map(|c| c.as_os_str().to_str()).collect();
    Some(parts?.join("/"))
}

/// Walk `root` (a folder on the drive mounted at `mount`) and return every media file.
/// Hidden files and folders (`.Spotlight-V100`, `.Trashes`, `.fseventsd`, `._*`, …)
/// are skipped; symlinks are not followed.
pub fn walk(mount: &Path, root: &Path, mut on_found: impl FnMut(usize)) -> Vec<FoundFile> {
    let mut found = Vec::new();
    let walker = WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| e.depth() == 0 || !e.file_name().to_string_lossy().starts_with('.'));

    // Unreadable folders are skipped rather than failing the whole scan.
    for entry in walker.flatten() {
        if !entry.file_type().is_file() {
            continue;
        }
        let Some(file_name) = entry.file_name().to_str() else { continue };
        let Some(kind) = kind_for(file_name) else { continue };
        let Some(rel_path) = rel_path(mount, entry.path()) else { continue };
        let Ok(meta) = entry.metadata() else { continue };
        let mtime = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        found.push(FoundFile {
            rel_path,
            file_name: file_name.to_owned(),
            kind,
            size_bytes: meta.len() as i64,
            mtime,
        });
        if found.len() % 200 == 0 {
            on_found(found.len());
        }
    }
    found
}

/// Upsert the walk result on (volume_id, rel_path) and flag rows under `root_rel`
/// that weren't seen as missing. Display names, ratings, tags and album
/// membership are never touched. A changed file (size or mtime) loses its
/// thumbnail so it gets re-probed.
pub fn apply(
    conn: &mut Connection,
    volume_id: i64,
    root_rel: &str,
    found: &[FoundFile],
) -> Result<(), String> {
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    if root_rel.is_empty() {
        tx.execute("UPDATE assets SET missing = 1 WHERE volume_id = ?1", [volume_id])
    } else {
        let prefix = format!("{root_rel}/");
        tx.execute(
            "UPDATE assets SET missing = 1
             WHERE volume_id = ?1 AND substr(rel_path, 1, length(?2)) = ?2",
            params![volume_id, prefix],
        )
    }
    .map_err(|e| e.to_string())?;
    {
        let mut stmt = tx
            .prepare(
                "INSERT INTO assets (volume_id, rel_path, file_name, kind, size_bytes, mtime, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)
                 ON CONFLICT(volume_id, rel_path) DO UPDATE SET
                   missing    = 0,
                   file_name  = excluded.file_name,
                   kind       = excluded.kind,
                   thumb_path = CASE WHEN assets.size_bytes <> excluded.size_bytes
                                       OR assets.mtime <> excluded.mtime
                                     THEN NULL ELSE assets.thumb_path END,
                   size_bytes = excluded.size_bytes,
                   mtime      = excluded.mtime",
            )
            .map_err(|e| e.to_string())?;
        for f in found {
            stmt.execute(params![
                volume_id,
                f.rel_path,
                f.file_name,
                f.kind,
                f.size_bytes,
                f.mtime
            ])
            .map_err(|e| e.to_string())?;
        }
    }
    tx.commit().map_err(|e| e.to_string())
}

/// Start a scan of `root` on a background thread and return immediately.
/// Progress arrives as `scan_progress` events.
pub fn start(
    app: AppHandle,
    volume_id: i64,
    mount: PathBuf,
    roots: Vec<PathBuf>,
) -> Result<(), String> {
    // With no roots the walk is skipped and only outstanding metadata/thumbnails are finished.
    let roots: Vec<(PathBuf, String)> = roots
        .into_iter()
        .map(|root| {
            let rel = rel_path(&mount, &root)
                .ok_or_else(|| format!("{} is not inside {}", root.display(), mount.display()))?;
            Ok((root, rel))
        })
        .collect::<Result<_, String>>()?;
    if !app.state::<ScanState>().active.lock().unwrap().insert(volume_id) {
        return Err("A scan of this drive is already running.".into());
    }

    std::thread::spawn(move || {
        let emit = |found: usize, processed: usize, total: usize, done: bool, error: Option<String>| {
            let _ = app.emit(
                "scan_progress",
                ScanProgress {
                    volume_id,
                    found,
                    processed,
                    total,
                    done,
                    error,
                },
            );
        };
        emit(0, 0, 0, false, None);
        let mut found = 0;
        let mut result = Ok(());
        for (root, root_rel) in &roots {
            let base = found;
            let files = walk(&mount, root, |n| emit(base + n, 0, 0, false, None));
            found += files.len();
            emit(found, 0, 0, false, None);
            let db = app.state::<Db>();
            let mut conn = db.conn.lock().unwrap();
            result = apply(&mut conn, volume_id, root_rel, &files);
            if result.is_err() {
                break;
            }
        }
        let result = result.and_then(|()| process_pending(&app, volume_id, &mount, found, &emit));

        app.state::<ScanState>().active.lock().unwrap().remove(&volume_id);
        let (processed, total) = *result.as_ref().unwrap_or(&(0, 0));
        emit(found, processed, total, true, result.err());
    });
    Ok(())
}

/// On launch: finish metadata/thumbnails that an earlier run didn't get to
/// (app quit or drive unplugged mid-scan), for every drive that is mounted.
pub fn resume_pending(app: &AppHandle) {
    let mounted = crate::volumes::mounted();
    let pending: Vec<(i64, String)> = {
        let db = app.state::<Db>();
        let conn = db.conn.lock().unwrap();
        if crate::volumes::apply_mounted(&conn, &mounted).is_err() {
            return;
        }
        let Ok(mut stmt) = conn.prepare(
            "SELECT v.id, v.mount_path FROM volumes v
             WHERE v.mount_path IS NOT NULL AND EXISTS
               (SELECT 1 FROM assets a WHERE a.volume_id = v.id AND a.missing = 0 AND a.thumb_path IS NULL)",
        ) else {
            return;
        };
        let Ok(rows) = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?))) else { return };
        rows.flatten().collect()
    };
    for (volume_id, mount) in pending {
        let _ = start(app.clone(), volume_id, PathBuf::from(mount), Vec::new());
    }
}

struct Job {
    id: i64,
    rel_path: String,
    is_video: bool,
}

/// Probe and thumbnail every file on the volume that has no thumbnail yet
/// (new, changed, or left over from an interrupted scan), two at a time.
/// Returns (processed, total).
fn process_pending(
    app: &AppHandle,
    volume_id: i64,
    mount: &Path,
    found: usize,
    emit: &(impl Fn(usize, usize, usize, bool, Option<String>) + Sync),
) -> Result<(usize, usize), String> {
    let db = app.state::<Db>();
    let jobs: Vec<Job> = {
        let conn = db.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT id, rel_path, kind FROM assets
                 WHERE volume_id = ?1 AND missing = 0 AND thumb_path IS NULL ORDER BY rel_path",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([volume_id], |r| {
                Ok(Job {
                    id: r.get(0)?,
                    rel_path: r.get(1)?,
                    is_video: r.get::<_, String>(2)? == "video",
                })
            })
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<_, _>>().map_err(|e| e.to_string())?
    };
    let total = jobs.len();
    if total == 0 {
        return Ok((0, 0));
    }
    let tools = media::detect()?;
    emit(found, 0, total, false, None);

    let queue = Mutex::new(jobs.into_iter());
    let processed = AtomicUsize::new(0);
    let unplugged = AtomicBool::new(false);
    std::thread::scope(|scope| {
        for _ in 0..WORKERS {
            scope.spawn(|| loop {
                if unplugged.load(Ordering::Relaxed) {
                    break;
                }
                let Some(job) = queue.lock().unwrap().next() else { break };
                let file = mount.join(&job.rel_path);
                let thumb = db.thumbs_dir.join(format!("{}.jpg", job.id));

                let meta = media::probe(&tools, &file, job.is_video).unwrap_or_default();
                let thumb_ok =
                    media::thumbnail(&tools, &file, &thumb, job.is_video, meta.duration_ms).is_ok();
                if !thumb_ok && !mount.exists() {
                    // Drive was unplugged mid-scan: stop quietly, the rest resumes next scan.
                    unplugged.store(true, Ordering::Relaxed);
                    break;
                }
                let _ = db.conn.lock().unwrap().execute(
                    "UPDATE assets SET duration_ms = ?1, width = ?2, height = ?3, codec = ?4,
                            created_at = COALESCE(?5, mtime), thumb_path = ?6
                     WHERE id = ?7",
                    params![
                        meta.duration_ms,
                        meta.width,
                        meta.height,
                        meta.codec,
                        meta.created_at,
                        thumb_ok.then(|| thumb.to_string_lossy().into_owned()),
                        job.id
                    ],
                );
                let n = processed.fetch_add(1, Ordering::Relaxed) + 1;
                emit(found, n, total, false, None);
            });
        }
    });
    Ok((processed.load(Ordering::Relaxed), total))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_by_extension() {
        assert_eq!(kind_for("A001.MOV"), Some("video"));
        assert_eq!(kind_for("clip.m2ts"), Some("video"));
        assert_eq!(kind_for("still.HEIC"), Some("image"));
        assert_eq!(kind_for("notes.txt"), None);
        assert_eq!(kind_for("mov"), None);
    }

    #[test]
    fn rel_paths_are_relative_to_the_mount() {
        let mount = Path::new("/Volumes/T7");
        assert_eq!(
            rel_path(mount, Path::new("/Volumes/T7/Day 1/a.mov")).as_deref(),
            Some("Day 1/a.mov")
        );
        assert_eq!(rel_path(mount, mount).as_deref(), Some(""));
        assert_eq!(
            rel_path(Path::new("/"), Path::new("/Users/me/a.mov")).as_deref(),
            Some("Users/me/a.mov")
        );
        assert_eq!(rel_path(mount, Path::new("/Volumes/Other/a.mov")), None);
    }

    fn file(rel: &str, size: i64) -> FoundFile {
        FoundFile {
            rel_path: rel.into(),
            file_name: rel.rsplit('/').next().unwrap().into(),
            kind: "video",
            size_bytes: size,
            mtime: 100,
        }
    }

    #[test]
    fn rescans_keep_user_data_and_flag_missing_files() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(crate::db::SCHEMA).unwrap();
        conn.execute("INSERT INTO volumes (id, uuid, label) VALUES (1, 'u', 'T7')", [])
            .unwrap();

        apply(&mut conn, 1, "", &[file("a/1.mov", 10), file("a/2.mov", 10), file("b/3.mov", 10)])
            .unwrap();
        conn.execute(
            "UPDATE assets SET display_name = 'Hero', rating = 5, thumb_path = 't.jpg'",
            [],
        )
        .unwrap();

        // Same files again: no duplicates, nothing lost.
        apply(&mut conn, 1, "", &[file("a/1.mov", 10), file("a/2.mov", 10), file("b/3.mov", 10)])
            .unwrap();
        let (n, kept): (i64, i64) = conn
            .query_row(
                "SELECT count(*), sum(display_name = 'Hero' AND rating = 5 AND thumb_path = 't.jpg')
                 FROM assets",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!((n, kept), (3, 3));

        // Rescan only folder "a": 2.mov is gone, 1.mov changed; "b" is untouched.
        apply(&mut conn, 1, "a", &[file("a/1.mov", 99)]).unwrap();
        let row = |rel: &str| -> (i64, Option<String>, i64) {
            conn.query_row(
                "SELECT missing, thumb_path, rating FROM assets WHERE rel_path = ?1",
                [rel],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap()
        };
        assert_eq!(row("a/1.mov"), (0, None, 5));
        assert_eq!(row("a/2.mov"), (1, Some("t.jpg".into()), 5));
        assert_eq!(row("b/3.mov"), (0, Some("t.jpg".into()), 5));
    }

    #[test]
    fn walk_finds_media_and_skips_hidden_and_other_files() {
        let mount = std::env::temp_dir().join(format!("asset-manager-walk-{}", std::process::id()));
        let root = mount.join("Footage");
        for dir in ["Footage/Day 1", "Footage/.Trashes", "Footage/.Spotlight-V100", "Elsewhere"] {
            std::fs::create_dir_all(mount.join(dir)).unwrap();
        }
        for f in [
            "Footage/a.MOV",
            "Footage/Day 1/b.mp4",
            "Footage/Day 1/c.jpg",
            "Footage/Day 1/._b.mp4",
            "Footage/Day 1/.hidden.mov",
            "Footage/Day 1/notes.txt",
            "Footage/.Trashes/d.mov",
            "Footage/.Spotlight-V100/e.mov",
            "Elsewhere/f.mov",
        ] {
            std::fs::write(mount.join(f), b"x").unwrap();
        }
        let mut rels: Vec<String> = walk(&mount, &root, |_| {}).into_iter().map(|f| f.rel_path).collect();
        rels.sort();
        std::fs::remove_dir_all(&mount).unwrap();
        assert_eq!(rels, ["Footage/Day 1/b.mp4", "Footage/Day 1/c.jpg", "Footage/a.MOV"]);
    }

    /// Step 7 safety check, automated: a full pass (walk, upsert, ffprobe, ffmpeg
    /// thumbnail) leaves every name, size and modified date in the media folder
    /// exactly as it was, and adds nothing to it. Needs Homebrew ffmpeg.
    #[test]
    fn full_scan_leaves_the_media_folder_untouched() {
        use std::time::SystemTime;
        let Ok(tools) = media::detect() else { return };
        let base = std::env::temp_dir().join(format!("asset-manager-safety-{}", std::process::id()));
        let (media_dir, thumbs) = (base.join("drive"), base.join("appdata/thumbs"));
        std::fs::create_dir_all(media_dir.join("Day 1")).unwrap();
        std::fs::create_dir_all(&thumbs).unwrap();
        for (name, args) in [
            ("a.mov", &["-t", "2", "-pix_fmt", "yuv420p"][..]),
            ("Day 1/b.mp4", &["-t", "0.4", "-pix_fmt", "yuv420p"][..]),
            ("Day 1/c.png", &["-frames:v", "1"][..]),
        ] {
            let ok = std::process::Command::new(&tools.ffmpeg)
                .args(["-v", "error", "-y", "-f", "lavfi", "-i", "testsrc=size=320x180:rate=25"])
                .args(args)
                .arg(media_dir.join(name))
                .status()
                .unwrap()
                .success();
            assert!(ok);
        }
        std::fs::write(media_dir.join("Day 1/broken.mov"), b"not a movie").unwrap();
        std::fs::write(media_dir.join("notes.txt"), b"hello").unwrap();

        let snapshot = || -> Vec<(PathBuf, u64, SystemTime)> {
            let mut list: Vec<_> = WalkDir::new(&media_dir)
                .into_iter()
                .map(|e| {
                    let e = e.unwrap();
                    let m = e.metadata().unwrap();
                    (e.path().to_path_buf(), m.len(), m.modified().unwrap())
                })
                .collect();
            list.sort();
            list
        };
        let before = snapshot();

        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(crate::db::SCHEMA).unwrap();
        conn.execute("INSERT INTO volumes (id, uuid, label) VALUES (1, 'u', 'Test')", [])
            .unwrap();
        let files = walk(&media_dir, &media_dir, |_| {});
        assert_eq!(files.len(), 4);
        apply(&mut conn, 1, "", &files).unwrap();
        let mut thumbnails = 0;
        for (i, f) in files.iter().enumerate() {
            let path = media_dir.join(&f.rel_path);
            let is_video = f.kind == "video";
            let meta = media::probe(&tools, &path, is_video).unwrap_or_default();
            let out = thumbs.join(format!("{i}.jpg"));
            if media::thumbnail(&tools, &path, &out, is_video, meta.duration_ms).is_ok() {
                thumbnails += 1;
            }
        }
        assert_eq!(thumbnails, 3); // broken.mov fails without side effects

        let after = snapshot();
        std::fs::remove_dir_all(&base).unwrap();
        assert_eq!(before, after);
    }
}
