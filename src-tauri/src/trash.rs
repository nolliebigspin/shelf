//! The one place Shelf deletes media: "Delete from Drive" inside the Trash.
//! Everything else in the app only changes the catalog.

use crate::db;
use rusqlite::{Connection, OptionalExtension};
use serde::Serialize;
use std::path::Path;

#[derive(Debug, Default, Serialize, PartialEq)]
pub struct DeleteReport {
    /// Files removed from their drive (or already gone) and dropped from the catalog.
    pub deleted: usize,
    /// Left in the Trash because their drive is unplugged.
    pub skipped_offline: usize,
    /// One line per file that couldn't be deleted.
    pub failed: Vec<String>,
}

/// Permanently delete the files behind `asset_ids` and forget them. Only clips
/// that are already in the Trash are touched; any other id is ignored.
pub fn delete_from_disk(conn: &mut Connection, asset_ids: &[i64]) -> Result<DeleteReport, String> {
    let mut report = DeleteReport::default();
    let mut forget = Vec::new();
    for &id in asset_ids {
        let row: Option<(Option<String>, String)> = conn
            .query_row(
                "SELECT v.mount_path, a.rel_path FROM assets a JOIN volumes v ON v.id = a.volume_id
                 WHERE a.id = ?1 AND a.trashed_at IS NOT NULL",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        let Some((mount, rel)) = row else { continue };
        // A drive unplugged since the last poll still has a mount path; check it's really there.
        let Some(mount) = mount.filter(|m| Path::new(m).is_dir()) else {
            report.skipped_offline += 1;
            continue;
        };
        let path = crate::volumes::resolve(&mount, &rel);
        match std::fs::symlink_metadata(&path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => forget.push(id),
            Err(e) => report.failed.push(format!("{}: {e}", path.display())),
            Ok(meta) if meta.is_dir() => {
                report.failed.push(format!("{}: is a folder, not a file", path.display()))
            }
            Ok(_) => match std::fs::remove_file(&path) {
                Ok(()) => forget.push(id),
                Err(e) => report.failed.push(format!("{}: {e}", path.display())),
            },
        }
    }
    for thumb in db::forget_assets(conn, &forget)? {
        let _ = std::fs::remove_file(thumb);
    }
    report.deleted = forget.len();
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{list_assets, trash_assets, AssetFilter, SCHEMA};

    #[test]
    fn deletes_only_trashed_files_and_forgets_them() {
        let base = std::env::temp_dir().join(format!("shelf-trash-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let drive = base.join("drive");
        std::fs::create_dir_all(&drive).unwrap();
        for f in ["keep.mov", "trash.mov"] {
            std::fs::write(drive.join(f), b"x").unwrap();
        }
        let thumb = base.join("1.jpg");
        std::fs::write(&thumb, b"t").unwrap();

        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(SCHEMA).unwrap();
        conn.execute(
            "INSERT INTO volumes (id, uuid, label, mount_path) VALUES (1, 'u', 'T', ?1), (2, 'o', 'Off', NULL)",
            [drive.to_string_lossy()],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO assets (id, volume_id, rel_path, file_name, kind, size_bytes, mtime, thumb_path) VALUES
               (1, 1, 'trash.mov', 'trash.mov', 'video', 1, 1, ?1),
               (2, 1, 'keep.mov', 'keep.mov', 'video', 1, 1, NULL),
               (3, 1, 'gone.mov', 'gone.mov', 'video', 1, 1, NULL),
               (4, 2, 'offline.mov', 'offline.mov', 'video', 1, 1, NULL)",
            [thumb.to_string_lossy()],
        )
        .unwrap();
        trash_assets(&mut conn, &[1, 3, 4], 10).unwrap();

        // 2 is not in the Trash, so it's ignored even though it's asked for.
        let report = delete_from_disk(&mut conn, &[1, 2, 3, 4]).unwrap();
        assert_eq!(
            report,
            DeleteReport { deleted: 2, skipped_offline: 1, failed: vec![] }
        );
        assert!(!drive.join("trash.mov").exists());
        assert!(drive.join("keep.mov").exists());
        assert!(!thumb.exists());
        let ids = |trashed| -> Vec<i64> {
            list_assets(&conn, &AssetFilter { trashed: Some(trashed), ..Default::default() })
                .unwrap()
                .iter()
                .map(|a| a.id)
                .collect()
        };
        assert_eq!(ids(false), [2]);
        assert_eq!(ids(true), [4]); // offline clip waits in the Trash
        let _ = std::fs::remove_dir_all(&base);
    }
}
