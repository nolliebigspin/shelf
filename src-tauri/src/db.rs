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
  last_seen_at INTEGER,
  position    INTEGER                 -- your order in the sidebar, NULL = after, by name
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
  trashed_at   INTEGER,               -- set while the clip is in Shelf's Trash
  UNIQUE(volume_id, rel_path)
);

CREATE TABLE IF NOT EXISTS tags        (id INTEGER PRIMARY KEY, name TEXT NOT NULL UNIQUE, position INTEGER);
CREATE TABLE IF NOT EXISTS asset_tags  (asset_id INTEGER, tag_id INTEGER, PRIMARY KEY (asset_id, tag_id));
CREATE TABLE IF NOT EXISTS albums      (id INTEGER PRIMARY KEY, name TEXT NOT NULL, created_at INTEGER,
                                        position INTEGER);
CREATE TABLE IF NOT EXISTS album_items (album_id INTEGER, asset_id INTEGER, position INTEGER,
                                        PRIMARY KEY (album_id, asset_id));

-- Your hand-arranged order for a drive ('volume:<id>') or All clips ('all').
-- Albums keep theirs in album_items.position. Clips without a row come after,
-- in path order.
CREATE TABLE IF NOT EXISTS view_order (
  scope    TEXT NOT NULL,
  asset_id INTEGER NOT NULL,
  position INTEGER NOT NULL,
  PRIMARY KEY (scope, asset_id)
);

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
    migrate(&conn)?;

    Ok(Db {
        conn: Mutex::new(conn),
        db_path,
        thumbs_dir,
    })
}

/// Bring catalogs created by older versions up to the current SCHEMA.
pub fn migrate(conn: &Connection) -> Result<(), String> {
    for (table, column) in [
        ("assets", "trashed_at"),
        ("albums", "position"),
        ("tags", "position"),
        ("volumes", "position"),
    ] {
        let exists: bool = conn
            .query_row(
                "SELECT count(*) FROM pragma_table_info(?1) WHERE name = ?2",
                [table, column],
                |r| r.get::<_, i64>(0),
            )
            .map_err(|e| e.to_string())?
            > 0;
        if !exists {
            conn.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {column} INTEGER"))
                .map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// The sidebar lists you can sort by hand.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SidebarList {
    Albums,
    Tags,
    Volumes,
}

/// Store a hand-arranged order for albums, tags or drives (`ids` top to bottom).
/// Anything not listed keeps no position and sorts after, by name.
pub fn reorder(conn: &mut Connection, list: SidebarList, ids: &[i64]) -> Result<(), String> {
    let table = match list {
        SidebarList::Albums => "albums",
        SidebarList::Tags => "tags",
        SidebarList::Volumes => "volumes",
    };
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    tx.execute(&format!("UPDATE {table} SET position = NULL"), [])
        .map_err(|e| e.to_string())?;
    {
        let mut stmt = tx
            .prepare(&format!("UPDATE {table} SET position = ?1 WHERE id = ?2"))
            .map_err(|e| e.to_string())?;
        for (i, id) in ids.iter().enumerate() {
            stmt.execute(params![i as i64 + 1, id]).map_err(|e| e.to_string())?;
        }
    }
    tx.commit().map_err(|e| e.to_string())
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
    /// When the clip was moved to Shelf's Trash; None while it isn't there.
    pub trashed_at: Option<i64>,
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
    /// true lists only the Trash; otherwise trashed clips are left out.
    pub trashed: Option<bool>,
    /// "custom" (default), "name", "captured", "added", "size", "duration",
    /// "rating", "kind" or "trashed".
    pub sort: Option<String>,
    pub descending: Option<bool>,
}

/// Which hand-arranged order a view uses.
#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq)]
pub struct OrderScope {
    pub album_id: Option<i64>,
    pub volume_id: Option<i64>,
}

const PATH_ORDER: &str = "v.label COLLATE NOCASE, a.rel_path COLLATE NOCASE";

/// SQL for a clip's place in the scope's custom order (NULL when it has none).
fn position_sql(scope: OrderScope) -> String {
    match scope {
        OrderScope { album_id: Some(id), .. } => format!(
            "(SELECT position FROM album_items ai WHERE ai.album_id = {id} AND ai.asset_id = a.id)"
        ),
        OrderScope { volume_id: Some(v), .. } => format!(
            "(SELECT position FROM view_order o WHERE o.scope = 'volume:{v}' AND o.asset_id = a.id)"
        ),
        _ => "(SELECT position FROM view_order o WHERE o.scope = 'all' AND o.asset_id = a.id)".to_owned(),
    }
}

/// Custom order: arranged clips first, the rest in path order.
fn custom_order_sql(scope: OrderScope, dir: &str) -> String {
    let pos = position_sql(scope);
    format!("{pos} IS NULL, {pos} {dir}, {PATH_ORDER}, a.id")
}

fn order_sql(filter: &AssetFilter) -> String {
    let dir = if filter.descending == Some(true) { "DESC" } else { "ASC" };
    let name = "COALESCE(a.display_name, a.file_name) COLLATE NOCASE";
    let scope = OrderScope { album_id: filter.album_id, volume_id: filter.volume_id };
    let key = match filter.sort.as_deref() {
        Some("name") => format!("{name} {dir}"),
        Some("captured") => format!("a.created_at IS NULL, a.created_at {dir}"),
        Some("added") => format!("a.id {dir}"),
        Some("size") => format!("a.size_bytes {dir}"),
        Some("duration") => format!("a.duration_ms IS NULL, a.duration_ms {dir}"),
        Some("rating") => format!("COALESCE(a.rating, 0) {dir}"),
        Some("kind") => format!("a.kind {dir}, {name}"),
        Some("trashed") => format!("a.trashed_at {dir}"),
        // The Trash has no hand-arranged order; newest first unless asked otherwise.
        _ if filter.trashed == Some(true) => {
            let dir = if filter.descending == Some(false) { "ASC" } else { "DESC" };
            format!("a.trashed_at {dir}")
        }
        _ => return custom_order_sql(scope, dir),
    };
    format!("{key}, {PATH_ORDER}, a.id")
}

fn like_pattern(term: &str) -> String {
    let escaped = term.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_");
    format!("%{escaped}%")
}

pub fn list_assets(conn: &Connection, filter: &AssetFilter) -> Result<Vec<Asset>, String> {
    let mut wheres: Vec<String> = Vec::new();
    let mut args: Vec<Box<dyn ToSql>> = Vec::new();
    wheres.push(if filter.trashed == Some(true) {
        "a.trashed_at IS NOT NULL".to_owned()
    } else {
        "a.trashed_at IS NULL".to_owned()
    });
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
    let where_sql = format!("WHERE {}", wheres.join(" AND "));
    let order_sql = order_sql(filter);
    let sql = format!(
        "SELECT a.id, a.volume_id, a.rel_path, a.file_name, a.kind, a.size_bytes, a.duration_ms,
                a.width, a.height, a.codec, a.created_at, a.thumb_path, a.display_name,
                a.rating, a.missing, v.mount_path, v.label,
                (SELECT group_concat(name, char(31)) FROM
                   (SELECT t.name FROM asset_tags x JOIN tags t ON t.id = x.tag_id
                    WHERE x.asset_id = a.id ORDER BY t.name)),
                a.trashed_at
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
                .map(|m| crate::volumes::resolve(m, &rel_path).to_string_lossy().into_owned());
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
                trashed_at: r.get(18)?,
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
    let path = crate::volumes::resolve(&mount, &rel);
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
            "SELECT t.id, t.name,
                    (SELECT count(*) FROM asset_tags x JOIN assets a ON a.id = x.asset_id
                     WHERE x.tag_id = t.id AND a.trashed_at IS NULL)
             FROM tags t ORDER BY t.position IS NULL, t.position, t.name",
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
                    (SELECT count(*) FROM album_items i JOIN assets a ON a.id = i.asset_id
                     WHERE i.album_id = b.id AND a.trashed_at IS NULL)
             FROM albums b ORDER BY b.position IS NULL, b.position, b.name COLLATE NOCASE",
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

pub fn rename_album(conn: &Connection, album_id: i64, name: &str) -> Result<(), String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("An album needs a name.".into());
    }
    conn.execute("UPDATE albums SET name = ?1 WHERE id = ?2", params![name, album_id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Delete an album. Its clips stay in the catalog (and on the drive).
pub fn delete_album(conn: &mut Connection, album_id: i64) -> Result<(), String> {
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    tx.execute("DELETE FROM album_items WHERE album_id = ?1", [album_id])
        .map_err(|e| e.to_string())?;
    tx.execute("DELETE FROM albums WHERE id = ?1", [album_id])
        .map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())
}

/// Rename a tag on every clip. Renaming onto an existing tag merges the two.
pub fn rename_tag(conn: &mut Connection, old: &str, new: &str) -> Result<(), String> {
    let (old, new) = (normalize_tag(old), normalize_tag(new));
    if new.is_empty() {
        return Err("A tag needs a name.".into());
    }
    if old == new {
        return Ok(());
    }
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    let old_id: Option<i64> = tx
        .query_row("SELECT id FROM tags WHERE name = ?1", [&old], |r| r.get(0))
        .optional()
        .map_err(|e| e.to_string())?;
    let Some(old_id) = old_id else { return Ok(()) };
    let new_id: Option<i64> = tx
        .query_row("SELECT id FROM tags WHERE name = ?1", [&new], |r| r.get(0))
        .optional()
        .map_err(|e| e.to_string())?;
    match new_id {
        None => {
            tx.execute("UPDATE tags SET name = ?1 WHERE id = ?2", params![new, old_id])
                .map_err(|e| e.to_string())?;
        }
        Some(new_id) => {
            tx.execute(
                "INSERT OR IGNORE INTO asset_tags (asset_id, tag_id)
                 SELECT asset_id, ?1 FROM asset_tags WHERE tag_id = ?2",
                params![new_id, old_id],
            )
            .map_err(|e| e.to_string())?;
            tx.execute("DELETE FROM asset_tags WHERE tag_id = ?1", [old_id])
                .map_err(|e| e.to_string())?;
            tx.execute("DELETE FROM tags WHERE id = ?1", [old_id])
                .map_err(|e| e.to_string())?;
        }
    }
    tx.commit().map_err(|e| e.to_string())
}

/// Remove a tag from every clip and forget it.
pub fn delete_tag(conn: &mut Connection, name: &str) -> Result<(), String> {
    let name = normalize_tag(name);
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    tx.execute(
        "DELETE FROM asset_tags WHERE tag_id IN (SELECT id FROM tags WHERE name = ?1)",
        [&name],
    )
    .map_err(|e| e.to_string())?;
    tx.execute("DELETE FROM tags WHERE name = ?1", [&name])
        .map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())
}

/// Move clips to Shelf's Trash. Nothing on the drive changes; tags, rating and
/// album membership are kept so Put Back restores them.
pub fn trash_assets(conn: &mut Connection, asset_ids: &[i64], now: i64) -> Result<(), String> {
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    for id in asset_ids {
        tx.execute(
            "UPDATE assets SET trashed_at = ?1 WHERE id = ?2 AND trashed_at IS NULL",
            params![now, id],
        )
        .map_err(|e| e.to_string())?;
    }
    tx.commit().map_err(|e| e.to_string())
}

pub fn restore_assets(conn: &mut Connection, asset_ids: &[i64]) -> Result<(), String> {
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    for id in asset_ids {
        tx.execute("UPDATE assets SET trashed_at = NULL WHERE id = ?1", [id])
            .map_err(|e| e.to_string())?;
    }
    tx.commit().map_err(|e| e.to_string())
}

pub fn trash_count(conn: &Connection) -> Result<i64, String> {
    conn.query_row("SELECT count(*) FROM assets WHERE trashed_at IS NOT NULL", [], |r| r.get(0))
        .map_err(|e| e.to_string())
}

/// Drop clips from the catalog entirely (rows, tags, album entries). Tags no
/// longer used anywhere go too. Returns the thumbnail files to clean up.
pub fn forget_assets(conn: &mut Connection, asset_ids: &[i64]) -> Result<Vec<PathBuf>, String> {
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    let mut thumbs = Vec::new();
    for id in asset_ids {
        let thumb: Option<Option<String>> = tx
            .query_row("SELECT thumb_path FROM assets WHERE id = ?1", [id], |r| r.get(0))
            .optional()
            .map_err(|e| e.to_string())?;
        thumbs.extend(thumb.flatten().map(PathBuf::from));
        for sql in [
            "DELETE FROM asset_tags WHERE asset_id = ?1",
            "DELETE FROM album_items WHERE asset_id = ?1",
            "DELETE FROM view_order WHERE asset_id = ?1",
            "DELETE FROM assets WHERE id = ?1",
        ] {
            tx.execute(sql, [id]).map_err(|e| e.to_string())?;
        }
    }
    tx.execute("DELETE FROM tags WHERE id NOT IN (SELECT tag_id FROM asset_tags)", [])
        .map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(thumbs)
}

/// Move `asset_ids` in the scope's custom order so they sit, in their current
/// relative order, right before `before_id` (at the end when None). The whole
/// scope is renumbered, including clips hidden by filters or in the Trash.
pub fn move_assets(
    conn: &mut Connection,
    scope: OrderScope,
    asset_ids: &[i64],
    before_id: Option<i64>,
) -> Result<(), String> {
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    let (where_sql, arg) = match scope {
        OrderScope { album_id: Some(id), .. } => (
            "EXISTS (SELECT 1 FROM album_items ai WHERE ai.album_id = ?1 AND ai.asset_id = a.id)",
            Some(id),
        ),
        OrderScope { volume_id: Some(v), .. } => ("a.volume_id = ?1", Some(v)),
        _ => ("1", None),
    };
    let sql = format!(
        "SELECT a.id FROM assets a JOIN volumes v ON v.id = a.volume_id
         WHERE {where_sql} ORDER BY {}",
        custom_order_sql(scope, "ASC")
    );
    let order: Vec<i64> = {
        let mut stmt = tx.prepare(&sql).map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params_from_iter(arg.iter()), |r| r.get(0))
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<_, _>>().map_err(|e| e.to_string())?
    };
    let moving: std::collections::HashSet<i64> = asset_ids.iter().copied().collect();
    let (moved, mut rest): (Vec<i64>, Vec<i64>) = order.into_iter().partition(|id| moving.contains(id));
    let at = before_id
        .and_then(|b| rest.iter().position(|id| *id == b))
        .unwrap_or(rest.len());
    rest.splice(at..at, moved);

    match scope {
        OrderScope { album_id: Some(album), .. } => {
            let mut stmt = tx
                .prepare("UPDATE album_items SET position = ?1 WHERE album_id = ?2 AND asset_id = ?3")
                .map_err(|e| e.to_string())?;
            for (i, id) in rest.iter().enumerate() {
                stmt.execute(params![i as i64 + 1, album, id]).map_err(|e| e.to_string())?;
            }
        }
        _ => {
            let key = match scope.volume_id {
                Some(v) => format!("volume:{v}"),
                None => "all".to_owned(),
            };
            tx.execute("DELETE FROM view_order WHERE scope = ?1", [&key])
                .map_err(|e| e.to_string())?;
            let mut stmt = tx
                .prepare("INSERT INTO view_order (scope, asset_id, position) VALUES (?1, ?2, ?3)")
                .map_err(|e| e.to_string())?;
            for (i, id) in rest.iter().enumerate() {
                stmt.execute(params![key, id, i as i64 + 1]).map_err(|e| e.to_string())?;
            }
        }
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
        assert_eq!(n, 8); // the plan's six + roots + view_order
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
        // Joined with the platform's separator ('\\' on Windows).
        let expected = Path::new("/Volumes/T7").join("a").join("beach_02.mov");
        assert_eq!(all[1].path.as_deref(), Some(expected.to_string_lossy().as_ref()));

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
    fn trash_hides_clips_everywhere_and_put_back_restores_them() {
        let mut conn = catalog();
        let album = create_album(&conn, "Reel", 0).unwrap();
        add_to_album(&mut conn, album, &[3, 1, 2]).unwrap();
        set_tags(&mut conn, &[1, 2], &["drone".into()], true).unwrap();

        trash_assets(&mut conn, &[1], 100).unwrap();
        assert_eq!(ids(&conn, AssetFilter::default()), [2, 3]);
        assert_eq!(ids(&conn, AssetFilter { album_id: Some(album), ..Default::default() }), [3, 2]);
        assert_eq!(ids(&conn, AssetFilter { tag: Some("drone".into()), ..Default::default() }), [2]);
        assert_eq!(ids(&conn, AssetFilter { search: Some("beach".into()), ..Default::default() }), [2]);
        let trash = list_assets(&conn, &AssetFilter { trashed: Some(true), ..Default::default() }).unwrap();
        assert_eq!((trash.len(), trash[0].id, trash[0].trashed_at), (1, 1, Some(100)));
        assert_eq!(trash_count(&conn).unwrap(), 1);
        assert_eq!(list_albums(&conn).unwrap()[0].asset_count, 2);
        assert_eq!(list_tags(&conn).unwrap()[0].asset_count, 1);

        restore_assets(&mut conn, &[1]).unwrap();
        assert_eq!(trash_count(&conn).unwrap(), 0);
        assert_eq!(ids(&conn, AssetFilter { album_id: Some(album), ..Default::default() }), [3, 1, 2]);
        assert_eq!(list_tags(&conn).unwrap()[0].asset_count, 2);
    }

    #[test]
    fn forget_removes_join_rows_and_orphan_tags() {
        let mut conn = catalog();
        let album = create_album(&conn, "Reel", 0).unwrap();
        add_to_album(&mut conn, album, &[1, 2]).unwrap();
        set_tags(&mut conn, &[1], &["solo".into()], true).unwrap();
        set_tags(&mut conn, &[1, 2], &["shared".into()], true).unwrap();
        conn.execute("UPDATE assets SET thumb_path = '/t/1.jpg' WHERE id = 1", []).unwrap();

        assert_eq!(forget_assets(&mut conn, &[1]).unwrap(), [PathBuf::from("/t/1.jpg")]);
        assert_eq!(ids(&conn, AssetFilter::default()), [2, 3]);
        let tags: Vec<String> = list_tags(&conn).unwrap().into_iter().map(|t| t.name).collect();
        assert_eq!(tags, ["shared"]);
        let n: i64 = conn
            .query_row("SELECT count(*) FROM album_items WHERE asset_id = 1", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0);
    }

    #[test]
    fn migrate_adds_trashed_at_to_old_catalogs() {
        let conn = Connection::open_in_memory().unwrap();
        let old = SCHEMA
            .replace("  trashed_at   INTEGER,               -- set while the clip is in Shelf's Trash\n", "")
            .replace(",\n  position    INTEGER                 -- your order in the sidebar, NULL = after, by name", "")
            .replace(", position INTEGER);", ");")
            .replace(",\n                                        position INTEGER);", ");");
        assert!(!old.contains("trashed_at") && !old.contains("position    INTEGER"));
        conn.execute_batch(&old).unwrap();
        migrate(&conn).unwrap();
        migrate(&conn).unwrap();
        conn.execute_batch(
            "SELECT trashed_at FROM assets; SELECT position FROM albums;
             SELECT position FROM tags; SELECT position FROM volumes;",
        )
        .unwrap();
    }

    #[test]
    fn rename_and_delete_albums_and_tags() {
        let mut conn = catalog();
        let album = create_album(&conn, "Reel", 0).unwrap();
        add_to_album(&mut conn, album, &[1, 2]).unwrap();
        rename_album(&conn, album, "  Showreel ").unwrap();
        assert!(rename_album(&conn, album, " ").is_err());
        assert_eq!(list_albums(&conn).unwrap()[0].name, "Showreel");
        delete_album(&mut conn, album).unwrap();
        assert!(list_albums(&conn).unwrap().is_empty());
        assert_eq!(ids(&conn, AssetFilter::default()), [1, 2, 3]); // clips stay
        let n: i64 = conn.query_row("SELECT count(*) FROM album_items", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 0);

        set_tags(&mut conn, &[1, 2], &["drone".into()], true).unwrap();
        set_tags(&mut conn, &[2, 3], &["aerial".into()], true).unwrap();
        rename_tag(&mut conn, "Drone", "Sky Shots").unwrap();
        let tags = |conn: &Connection| -> Vec<(String, i64)> {
            list_tags(conn).unwrap().into_iter().map(|t| (t.name, t.asset_count)).collect()
        };
        assert_eq!(tags(&conn), [("aerial".into(), 2), ("sky shots".into(), 2)]);
        rename_tag(&mut conn, "sky shots", "Aerial").unwrap(); // merges, no duplicates
        assert_eq!(tags(&conn), [("aerial".into(), 3)]);
        assert!(rename_tag(&mut conn, "aerial", "  ").is_err());
        delete_tag(&mut conn, "AERIAL").unwrap();
        assert!(tags(&conn).is_empty());
        assert!(list_assets(&conn, &AssetFilter::default()).unwrap().iter().all(|a| a.tags.is_empty()));
    }

    #[test]
    fn sidebar_lists_keep_a_hand_arranged_order() {
        let mut conn = catalog();
        let a = create_album(&conn, "Alpha", 0).unwrap();
        let b = create_album(&conn, "Beta", 0).unwrap();
        let c = create_album(&conn, "Gamma", 0).unwrap();
        let names = |conn: &Connection| -> Vec<String> { list_albums(conn).unwrap().into_iter().map(|a| a.name).collect() };
        assert_eq!(names(&conn), ["Alpha", "Beta", "Gamma"]);
        reorder(&mut conn, SidebarList::Albums, &[c, a, b]).unwrap();
        assert_eq!(names(&conn), ["Gamma", "Alpha", "Beta"]);
        create_album(&conn, "Delta", 0).unwrap(); // new ones come after, by name
        assert_eq!(names(&conn), ["Gamma", "Alpha", "Beta", "Delta"]);

        set_tags(&mut conn, &[1], &["one".into(), "two".into()], true).unwrap();
        let tags = list_tags(&conn).unwrap();
        reorder(&mut conn, SidebarList::Tags, &[tags[1].id, tags[0].id]).unwrap();
        let names: Vec<String> = list_tags(&conn).unwrap().into_iter().map(|t| t.name).collect();
        assert_eq!(names, ["two", "one"]);

        conn.execute("INSERT INTO volumes (id, uuid, label) VALUES (2, 'u2', 'A-drive')", []).unwrap();
        assert_eq!(crate::volumes::list(&conn).unwrap()[0].label, "A-drive");
        reorder(&mut conn, SidebarList::Volumes, &[1, 2]).unwrap();
        assert_eq!(crate::volumes::list(&conn).unwrap()[0].label, "T7");
    }

    #[test]
    fn sort_options() {
        let conn = catalog();
        conn.execute_batch(
            "UPDATE assets SET size_bytes = 30, created_at = 5, rating = 1, duration_ms = 900 WHERE id = 1;
             UPDATE assets SET size_bytes = 10, created_at = 9, rating = 5, display_name = 'Aardvark' WHERE id = 2;
             UPDATE assets SET size_bytes = 20, created_at = NULL, rating = 3, duration_ms = 100 WHERE id = 3;",
        )
        .unwrap();
        let sorted = |sort: &str, desc: bool| {
            ids(&conn, AssetFilter { sort: Some(sort.into()), descending: Some(desc), ..Default::default() })
        };
        assert_eq!(sorted("name", false), [3, 2, 1]); // "100%_city", "Aardvark", "beach_01"
        assert_eq!(sorted("size", false), [2, 3, 1]);
        assert_eq!(sorted("size", true), [1, 3, 2]);
        assert_eq!(sorted("captured", false), [1, 2, 3]); // unknown dates last
        assert_eq!(sorted("captured", true), [2, 1, 3]);
        assert_eq!(sorted("rating", true), [2, 3, 1]);
        assert_eq!(sorted("duration", false), [3, 1, 2]);
        assert_eq!(sorted("added", true), [3, 2, 1]);
        assert_eq!(sorted("bogus", false), [1, 2, 3]); // unknown keys fall back to custom/path
    }

    #[test]
    fn custom_order_for_drive_all_and_album() {
        let mut conn = catalog();
        let drive = OrderScope { volume_id: Some(1), album_id: None };
        let in_drive = |conn: &Connection| ids(conn, AssetFilter { volume_id: Some(1), ..Default::default() });
        assert_eq!(in_drive(&conn), [1, 2, 3]); // path order until arranged
        move_assets(&mut conn, drive, &[3], Some(1)).unwrap();
        assert_eq!(in_drive(&conn), [3, 1, 2]);
        move_assets(&mut conn, drive, &[3, 1], None).unwrap(); // keeps their relative order
        assert_eq!(in_drive(&conn), [2, 3, 1]);
        assert_eq!(ids(&conn, AssetFilter::default()), [1, 2, 3]); // All clips has its own order
        assert_eq!(in_drive(&conn).len(), 3);

        move_assets(&mut conn, OrderScope::default(), &[2], Some(1)).unwrap();
        assert_eq!(ids(&conn, AssetFilter::default()), [2, 1, 3]);
        // Descending custom order reverses the arranged part.
        assert_eq!(
            ids(&conn, AssetFilter { volume_id: Some(1), descending: Some(true), ..Default::default() }),
            [1, 3, 2]
        );

        let album = create_album(&conn, "Reel", 0).unwrap();
        add_to_album(&mut conn, album, &[1, 2, 3]).unwrap();
        let scope = OrderScope { album_id: Some(album), volume_id: None };
        move_assets(&mut conn, scope, &[3], Some(2)).unwrap();
        assert_eq!(ids(&conn, AssetFilter { album_id: Some(album), ..Default::default() }), [1, 3, 2]);
        // Moving before a clip that is itself moving lands at the end.
        move_assets(&mut conn, scope, &[1], Some(1)).unwrap();
        assert_eq!(ids(&conn, AssetFilter { album_id: Some(album), ..Default::default() }), [3, 2, 1]);
        // New clips join an album at the end.
        conn.execute(
            "INSERT INTO assets (id, volume_id, rel_path, file_name, kind, size_bytes, mtime)
             VALUES (4, 1, 'c/new.mov', 'new.mov', 'video', 1, 1)",
            [],
        )
        .unwrap();
        add_to_album(&mut conn, album, &[4]).unwrap();
        assert_eq!(ids(&conn, AssetFilter { album_id: Some(album), ..Default::default() }), [3, 2, 1, 4]);
        // ...and an unarranged clip comes after the arranged ones in a drive.
        assert_eq!(in_drive(&conn), [2, 3, 1, 4]);
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
