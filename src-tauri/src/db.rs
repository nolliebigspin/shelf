//! SQLite catalog. Lives in the app data folder, never next to the media.

use rusqlite::{params, params_from_iter, Connection, OptionalExtension, ToSql};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Shared app state: the one catalog connection plus where derived files live.
pub struct Db {
    pub conn: Mutex<Connection>,
    pub db_path: PathBuf,
    pub thumbs_dir: PathBuf,
}

pub const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS volumes (
  id          INTEGER PRIMARY KEY,
  uuid        TEXT NOT NULL UNIQUE,   -- VolumeUUID from diskutil
  label       TEXT NOT NULL,          -- last seen drive name
  mount_path  TEXT,                   -- e.g. /Volumes/T7, NULL when unplugged
  last_seen_at INTEGER
);

CREATE TABLE IF NOT EXISTS assets (
  id           INTEGER PRIMARY KEY,
  volume_id    INTEGER NOT NULL REFERENCES volumes(id),
  rel_path     TEXT NOT NULL,         -- path inside the drive, '/' separators
  file_name    TEXT NOT NULL,
  kind         TEXT NOT NULL,         -- 'video' | 'image'
  size_bytes   INTEGER NOT NULL,
  mtime        INTEGER NOT NULL,
  duration_ms  INTEGER,
  width        INTEGER,
  height       INTEGER,
  codec        TEXT,
  created_at   INTEGER,               -- capture date from metadata, else mtime
  thumb_path   TEXT,                  -- in app data, NULL until generated
  display_name TEXT,                  -- your virtual name
  rating       INTEGER DEFAULT 0,     -- 0-5
  missing      INTEGER DEFAULT 0,     -- 1 = not found on last scan
  UNIQUE(volume_id, rel_path)
);

CREATE TABLE IF NOT EXISTS tags        (id INTEGER PRIMARY KEY, name TEXT NOT NULL UNIQUE);
CREATE TABLE IF NOT EXISTS asset_tags  (asset_id INTEGER, tag_id INTEGER, PRIMARY KEY (asset_id, tag_id));
CREATE TABLE IF NOT EXISTS albums      (id INTEGER PRIMARY KEY, name TEXT NOT NULL, created_at INTEGER);
CREATE TABLE IF NOT EXISTS album_items (album_id INTEGER, asset_id INTEGER, position INTEGER,
                                        PRIMARY KEY (album_id, asset_id));

-- Not in the original plan: the folders you added, so a rescan re-walks only those.
CREATE TABLE IF NOT EXISTS roots (
  id        INTEGER PRIMARY KEY,
  volume_id INTEGER NOT NULL REFERENCES volumes(id),
  rel_path  TEXT NOT NULL,              -- folder inside the drive, '' = whole drive
  UNIQUE(volume_id, rel_path)
);
"#;

/// True if folder `inner` is `outer` or lies inside it ('' is the whole drive).
fn is_within(inner: &str, outer: &str) -> bool {
    outer.is_empty() || inner == outer || inner.starts_with(&format!("{outer}/"))
}

/// Remember an added folder. A folder already covered by a recorded one is not
/// stored again, and folders inside the new one are folded into it.
pub fn add_root(conn: &Connection, volume_id: i64, rel_path: &str) -> Result<(), String> {
    let existing = list_roots(conn, volume_id)?;
    if existing.iter().any(|r| is_within(rel_path, r)) {
        return Ok(());
    }
    for inner in existing.iter().filter(|r| is_within(r, rel_path)) {
        conn.execute(
            "DELETE FROM roots WHERE volume_id = ?1 AND rel_path = ?2",
            params![volume_id, inner],
        )
        .map_err(|e| e.to_string())?;
    }
    conn.execute(
        "INSERT INTO roots (volume_id, rel_path) VALUES (?1, ?2)",
        params![volume_id, rel_path],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn list_roots(conn: &Connection, volume_id: i64) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare("SELECT rel_path FROM roots WHERE volume_id = ?1 ORDER BY rel_path")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([volume_id], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<_, _>>().map_err(|e| e.to_string())
}

/// Opens (creating if needed) `catalog.db` and the thumbnail folder inside `app_data`.
pub fn open(app_data: &Path) -> Result<Db, String> {
    let thumbs_dir = app_data.join("thumbs");
    std::fs::create_dir_all(&thumbs_dir)
        .map_err(|e| format!("cannot create {}: {e}", thumbs_dir.display()))?;

    let db_path = app_data.join("catalog.db");
    let conn = Connection::open(&db_path)
        .map_err(|e| format!("cannot open {}: {e}", db_path.display()))?;
    conn.execute_batch("PRAGMA journal_mode = WAL; PRAGMA foreign_keys = ON;")
        .map_err(|e| e.to_string())?;
    conn.execute_batch(SCHEMA).map_err(|e| e.to_string())?;

    Ok(Db {
        conn: Mutex::new(conn),
        db_path,
        thumbs_dir,
    })
}

#[derive(Debug, Serialize)]
pub struct Asset {
    pub id: i64,
    pub volume_id: i64,
    pub rel_path: String,
    pub file_name: String,
    pub kind: String,
    pub size_bytes: i64,
    pub duration_ms: Option<i64>,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub codec: Option<String>,
    pub created_at: Option<i64>,
    pub thumb_path: Option<String>,
    pub display_name: Option<String>,
    pub rating: i64,
    pub missing: bool,
    /// True when the asset's drive is mounted right now.
    pub online: bool,
    /// mount_path + "/" + rel_path; only set while online.
    pub path: Option<String>,
    pub volume_label: String,
    pub tags: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct AssetFilter {
    pub volume_id: Option<i64>,
    pub album_id: Option<i64>,
    /// Matches file name, display name or tag; every word must match.
    pub search: Option<String>,
    pub tag: Option<String>,
    pub min_rating: Option<i64>,
}

fn like_pattern(term: &str) -> String {
    let escaped = term.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_");
    format!("%{escaped}%")
}

pub fn list_assets(conn: &Connection, filter: &AssetFilter) -> Result<Vec<Asset>, String> {
    let mut wheres: Vec<String> = Vec::new();
    let mut args: Vec<Box<dyn ToSql>> = Vec::new();
    let mut push = |clause: &str, arg: Box<dyn ToSql>| {
        args.push(arg);
        wheres.push(clause.replace("?", &format!("?{}", args.len())));
    };
    if let Some(id) = filter.volume_id {
        push("a.volume_id = ?", Box::new(id));
    }
    if let Some(id) = filter.album_id {
        push(
            "EXISTS (SELECT 1 FROM album_items ai WHERE ai.album_id = ? AND ai.asset_id = a.id)",
            Box::new(id),
        );
    }
    if let Some(tag) = filter.tag.as_deref().map(normalize_tag).filter(|t| !t.is_empty()) {
        push(
            "EXISTS (SELECT 1 FROM asset_tags x JOIN tags t ON t.id = x.tag_id
                     WHERE x.asset_id = a.id AND t.name = ?)",
            Box::new(tag),
        );
    }
    if let Some(min) = filter.min_rating.filter(|m| *m > 0) {
        push("COALESCE(a.rating, 0) >= ?", Box::new(min));
    }
    for term in filter.search.as_deref().unwrap_or("").split_whitespace() {
        push(
            "(a.file_name LIKE ? ESCAPE '\\' OR a.display_name LIKE ? ESCAPE '\\' OR EXISTS
               (SELECT 1 FROM asset_tags x JOIN tags t ON t.id = x.tag_id
                WHERE x.asset_id = a.id AND t.name LIKE ? ESCAPE '\\'))",
            Box::new(like_pattern(term)),
        );
    }
    let where_sql = if wheres.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", wheres.join(" AND "))
    };
    // Inside an album, keep the order clips were added in.
    let order_sql = match filter.album_id {
        Some(id) => format!(
            "(SELECT position FROM album_items ai WHERE ai.album_id = {id} AND ai.asset_id = a.id), a.id"
        ),
        None => "v.label COLLATE NOCASE, a.rel_path COLLATE NOCASE".to_owned(),
    };
    let sql = format!(
        "SELECT a.id, a.volume_id, a.rel_path, a.file_name, a.kind, a.size_bytes, a.duration_ms,
                a.width, a.height, a.codec, a.created_at, a.thumb_path, a.display_name,
                a.rating, a.missing, v.mount_path, v.label,
                (SELECT group_concat(name, char(31)) FROM
                   (SELECT t.name FROM asset_tags x JOIN tags t ON t.id = x.tag_id
                    WHERE x.asset_id = a.id ORDER BY t.name))
         FROM assets a JOIN volumes v ON v.id = a.volume_id
         {where_sql} ORDER BY {order_sql}"
    );
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params_from_iter(args.iter()), |r| {
            let rel_path: String = r.get(2)?;
            let mount_path: Option<String> = r.get(15)?;
            let path = mount_path
                .as_ref()
                .map(|m| Path::new(m).join(&rel_path).to_string_lossy().into_owned());
            let tags: Option<String> = r.get(17)?;
            Ok(Asset {
                id: r.get(0)?,
                volume_id: r.get(1)?,
                file_name: r.get(3)?,
                kind: r.get(4)?,
                size_bytes: r.get(5)?,
                duration_ms: r.get(6)?,
                width: r.get(7)?,
                height: r.get(8)?,
                codec: r.get(9)?,
                created_at: r.get(10)?,
                thumb_path: r.get(11)?,
                display_name: r.get(12)?,
                rating: r.get::<_, Option<i64>>(13)?.unwrap_or(0),
                missing: r.get::<_, Option<i64>>(14)?.unwrap_or(0) != 0,
                online: mount_path.is_some(),
                path,
                rel_path,
                volume_label: r.get(16)?,
                tags: tags
                    .map(|t| t.split('\u{1f}').map(str::to_owned).collect())
                    .unwrap_or_default(),
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<_, _>>().map_err(|e| e.to_string())
}

/// Where an asset is right now: mount_path + rel_path. Errors while its drive
/// is unplugged or the file was gone on the last scan.
pub fn asset_path(conn: &Connection, id: i64) -> Result<PathBuf, String> {
    let (mount, rel): (Option<String>, String) = conn
        .query_row(
            "SELECT v.mount_path, a.rel_path FROM assets a JOIN volumes v ON v.id = a.volume_id
             WHERE a.id = ?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|e| e.to_string())?;
    let mount = mount.ok_or("This clip's drive is offline.")?;
    let path = Path::new(&mount).join(rel);
    if !path.exists() {
        return Err(format!("{} is no longer there.", path.display()));
    }
    Ok(path)
}

#[derive(Debug, Default, Deserialize)]
pub struct AssetPatch {
    /// New virtual name; an empty string clears it back to the file name.
    pub display_name: Option<String>,
    /// 0–5; 0 clears the rating.
    pub rating: Option<i64>,
}

pub fn update_asset(conn: &Connection, id: i64, patch: &AssetPatch) -> Result<(), String> {
    if let Some(name) = &patch.display_name {
        let name = name.trim();
        conn.execute(
            "UPDATE assets SET display_name = ?1 WHERE id = ?2",
            params![(!name.is_empty()).then_some(name), id],
        )
        .map_err(|e| e.to_string())?;
    }
    if let Some(rating) = patch.rating {
        conn.execute(
            "UPDATE assets SET rating = ?1 WHERE id = ?2",
            params![rating.clamp(0, 5), id],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Tags are case-insensitive: stored trimmed and lower-cased.
pub fn normalize_tag(name: &str) -> String {
    name.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

/// Add or remove `names` on every asset in `asset_ids`. Adding creates missing
/// tags; removing drops tags that end up unused.
pub fn set_tags(
    conn: &mut Connection,
    asset_ids: &[i64],
    names: &[String],
    add: bool,
) -> Result<(), String> {
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    for name in names.iter().map(|n| normalize_tag(n)).filter(|n| !n.is_empty()) {
        if add {
            tx.execute("INSERT OR IGNORE INTO tags (name) VALUES (?1)", [&name])
                .map_err(|e| e.to_string())?;
        }
        let tag_id: Option<i64> = tx
            .query_row("SELECT id FROM tags WHERE name = ?1", [&name], |r| r.get(0))
            .optional()
            .map_err(|e| e.to_string())?;
        let Some(tag_id) = tag_id else { continue };
        let sql = if add {
            "INSERT OR IGNORE INTO asset_tags (asset_id, tag_id) VALUES (?1, ?2)"
        } else {
            "DELETE FROM asset_tags WHERE asset_id = ?1 AND tag_id = ?2"
        };
        let mut stmt = tx.prepare(sql).map_err(|e| e.to_string())?;
        for asset_id in asset_ids {
            stmt.execute(params![asset_id, tag_id]).map_err(|e| e.to_string())?;
        }
    }
    if !add {
        tx.execute(
            "DELETE FROM tags WHERE id NOT IN (SELECT tag_id FROM asset_tags)",
            [],
        )
        .map_err(|e| e.to_string())?;
    }
    tx.commit().map_err(|e| e.to_string())
}

#[derive(Debug, Serialize)]
pub struct Tag {
    pub id: i64,
    pub name: String,
    pub asset_count: i64,
}

pub fn list_tags(conn: &Connection) -> Result<Vec<Tag>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT t.id, t.name, (SELECT count(*) FROM asset_tags x WHERE x.tag_id = t.id)
             FROM tags t ORDER BY t.name",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            Ok(Tag {
                id: r.get(0)?,
                name: r.get(1)?,
                asset_count: r.get(2)?,
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<_, _>>().map_err(|e| e.to_string())
}

#[derive(Debug, Serialize)]
pub struct Album {
    pub id: i64,
    pub name: String,
    pub created_at: Option<i64>,
    pub asset_count: i64,
}

pub fn list_albums(conn: &Connection) -> Result<Vec<Album>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT b.id, b.name, b.created_at,
                    (SELECT count(*) FROM album_items i WHERE i.album_id = b.id)
             FROM albums b ORDER BY b.name COLLATE NOCASE",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            Ok(Album {
                id: r.get(0)?,
                name: r.get(1)?,
                created_at: r.get(2)?,
                asset_count: r.get(3)?,
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<_, _>>().map_err(|e| e.to_string())
}

pub fn create_album(conn: &Connection, name: &str, now: i64) -> Result<i64, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("An album needs a name.".into());
    }
    conn.execute(
        "INSERT INTO albums (name, created_at) VALUES (?1, ?2)",
        params![name, now],
    )
    .map_err(|e| e.to_string())?;
    Ok(conn.last_insert_rowid())
}

/// Appends clips to the album; ones already in it keep their place.
pub fn add_to_album(conn: &mut Connection, album_id: i64, asset_ids: &[i64]) -> Result<(), String> {
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    let mut position: i64 = tx
        .query_row(
            "SELECT COALESCE(max(position), 0) FROM album_items WHERE album_id = ?1",
            [album_id],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    for asset_id in asset_ids {
        position += 1;
        tx.execute(
            "INSERT OR IGNORE INTO album_items (album_id, asset_id, position) VALUES (?1, ?2, ?3)",
            params![album_id, asset_id, position],
        )
        .map_err(|e| e.to_string())?;
    }
    tx.commit().map_err(|e| e.to_string())
}

pub fn remove_from_album(conn: &mut Connection, album_id: i64, asset_ids: &[i64]) -> Result<(), String> {
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    for asset_id in asset_ids {
        tx.execute(
            "DELETE FROM album_items WHERE album_id = ?1 AND asset_id = ?2",
            params![album_id, asset_id],
        )
        .map_err(|e| e.to_string())?;
    }
    tx.commit().map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_creates_all_tables_and_is_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(SCHEMA).unwrap();
        conn.execute_batch(SCHEMA).unwrap();
        let n: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type = 'table'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 7); // the plan's six + roots
    }

    fn catalog() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(SCHEMA).unwrap();
        conn.execute_batch(
            "INSERT INTO volumes (id, uuid, label, mount_path) VALUES (1, 'u', 'T7', '/Volumes/T7');
             INSERT INTO assets (id, volume_id, rel_path, file_name, kind, size_bytes, mtime) VALUES
               (1, 1, 'a/beach_01.mov', 'beach_01.mov', 'video', 1, 1),
               (2, 1, 'a/beach_02.mov', 'beach_02.mov', 'video', 1, 1),
               (3, 1, 'b/100%_city.mp4', '100%_city.mp4', 'video', 1, 1);",
        )
        .unwrap();
        conn
    }

    fn ids(conn: &Connection, filter: AssetFilter) -> Vec<i64> {
        list_assets(conn, &filter).unwrap().iter().map(|a| a.id).collect()
    }

    #[test]
    fn names_ratings_tags_and_search() {
        let mut conn = catalog();
        update_asset(&conn, 2, &AssetPatch { display_name: Some(" Sunset hero ".into()), rating: Some(9) }).unwrap();
        set_tags(&mut conn, &[1, 2], &["  Drone ".into(), "B-Roll".into()], true).unwrap();
        set_tags(&mut conn, &[1], &["drone".into()], true).unwrap(); // no duplicate

        let all = list_assets(&conn, &AssetFilter::default()).unwrap();
        assert_eq!(all[1].display_name.as_deref(), Some("Sunset hero"));
        assert_eq!(all[1].rating, 5);
        assert_eq!(all[1].tags, ["b-roll", "drone"]);
        assert_eq!(all[1].path.as_deref(), Some("/Volumes/T7/a/beach_02.mov"));

        let search = |q: &str| ids(&conn, AssetFilter { search: Some(q.into()), ..Default::default() });
        assert_eq!(search("BEACH"), [1, 2]); // file name
        assert_eq!(search("sunset"), [2]); // display name
        assert_eq!(search("drone"), [1, 2]); // tag
        assert_eq!(search("beach hero"), [2]); // every word must match
        assert_eq!(search("100%"), [3]); // % is literal
        assert_eq!(search("_0"), [1, 2]); // _ is literal
        assert_eq!(ids(&conn, AssetFilter { tag: Some("Drone".into()), ..Default::default() }), [1, 2]);
        assert_eq!(ids(&conn, AssetFilter { min_rating: Some(4), ..Default::default() }), [2]);

        set_tags(&mut conn, &[1, 2], &["drone".into()], false).unwrap();
        let tags = list_tags(&conn).unwrap();
        assert_eq!(tags.len(), 1);
        assert_eq!((tags[0].name.as_str(), tags[0].asset_count), ("b-roll", 2));

        update_asset(&conn, 2, &AssetPatch { display_name: Some("".into()), rating: None }).unwrap();
        let a = &list_assets(&conn, &AssetFilter::default()).unwrap()[1];
        assert_eq!((a.display_name.clone(), a.rating), (None, 5));
    }

    #[test]
    fn albums_keep_order_and_membership() {
        let mut conn = catalog();
        let album = create_album(&conn, " Reel ", 0).unwrap();
        assert!(create_album(&conn, "  ", 0).is_err());
        add_to_album(&mut conn, album, &[3, 1]).unwrap();
        add_to_album(&mut conn, album, &[1, 2]).unwrap();
        let in_album = || ids(&conn, AssetFilter { album_id: Some(album), ..Default::default() });
        assert_eq!(in_album(), [3, 1, 2]);
        let albums = list_albums(&conn).unwrap();
        assert_eq!((albums[0].name.as_str(), albums[0].asset_count), ("Reel", 3));
        remove_from_album(&mut conn, album, &[1]).unwrap();
        assert_eq!(ids(&conn, AssetFilter { album_id: Some(album), ..Default::default() }), [3, 2]);
        assert_eq!(ids(&conn, AssetFilter::default()).len(), 3); // clips stay in the catalog
    }

    #[test]
    fn roots_are_deduplicated_by_containment() {
        let conn = catalog();
        add_root(&conn, 1, "Media/Raw").unwrap();
        add_root(&conn, 1, "Media/Raw").unwrap();
        add_root(&conn, 1, "Media/Raw/2022").unwrap(); // already covered
        add_root(&conn, 1, "Media/Rawhide").unwrap(); // sibling, not inside Media/Raw
        assert_eq!(list_roots(&conn, 1).unwrap(), ["Media/Raw", "Media/Rawhide"]);
        add_root(&conn, 1, "Media").unwrap(); // swallows both
        assert_eq!(list_roots(&conn, 1).unwrap(), ["Media"]);
        add_root(&conn, 1, "").unwrap(); // whole drive
        add_root(&conn, 1, "Other").unwrap();
        assert_eq!(list_roots(&conn, 1).unwrap(), [""]);
    }
}
