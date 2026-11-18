//! Tauri commands: the only surface the frontend talks to.

use crate::{
    db::{self, Db},
    media, scan, trash, volumes,
};
use serde::Serialize;
use tauri::{AppHandle, State};

#[derive(Serialize)]
pub struct AppStatus {
    db_path: String,
    thumbs_dir: String,
    ffmpeg: Option<String>,
    ffprobe: Option<String>,
    ffmpeg_error: Option<String>,
}

#[tauri::command]
pub fn app_status(db: State<'_, Db>) -> AppStatus {
    let (ffmpeg, ffprobe, ffmpeg_error) = match media::detect() {
        Ok(t) => (
            Some(t.ffmpeg.display().to_string()),
            Some(t.ffprobe.display().to_string()),
            None,
        ),
        Err(e) => (None, None, Some(e)),
    };
    AppStatus {
        db_path: db.db_path.display().to_string(),
        thumbs_dir: db.thumbs_dir.display().to_string(),
        ffmpeg,
        ffprobe,
        ffmpeg_error,
    }
}

/// Resolve the drive behind `path`, upsert it, and start a background scan of
/// that folder. Returns the volume id at once.
#[tauri::command(async)]
pub fn add_root(app: AppHandle, db: State<'_, Db>, path: String) -> Result<i64, String> {
    let root = std::fs::canonicalize(&path).map_err(|e| format!("cannot open {path}: {e}"))?;
    if !root.is_dir() {
        return Err(format!("{} is not a folder", root.display()));
    }
    let info = volumes::info_for_mount(&volumes::mount_point_of(&root))?;
    let root_rel = scan::rel_path(&info.mount_path, &root)
        .ok_or_else(|| format!("{} is not inside {}", root.display(), info.mount_path.display()))?;
    let volume_id = {
        let conn = db.conn.lock().unwrap();
        let id = volumes::upsert(&conn, &info)?;
        db::add_root(&conn, id, &root_rel)?;
        id
    };
    scan::start(app, volume_id, info.mount_path, vec![root])?;
    Ok(volume_id)
}

/// Walk every folder that was added on this drive again.
#[tauri::command(async)]
pub fn rescan(app: AppHandle, db: State<'_, Db>, volume_id: i64) -> Result<(), String> {
    let (mount, roots) = {
        let conn = db.conn.lock().unwrap();
        let mount: Option<String> = conn
            .query_row("SELECT mount_path FROM volumes WHERE id = ?1", [volume_id], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        (mount, db::list_roots(&conn, volume_id)?)
    };
    let mount = std::path::PathBuf::from(mount.ok_or("This drive is offline.")?);
    if roots.is_empty() {
        return Err(
            "This drive was added before the app remembered folders. Use “+ Add folder” on it once more; after that Rescan works."
                .into(),
        );
    }
    let roots = roots.iter().map(|rel| mount.join(rel)).collect();
    scan::start(app, volume_id, mount, roots)
}

/// Match mounted drives to known UUIDs, update mount paths, return all volumes.
#[tauri::command(async)]
pub fn refresh_volumes(db: State<'_, Db>) -> Result<Vec<volumes::Volume>, String> {
    let mounted = volumes::mounted();
    let conn = db.conn.lock().unwrap();
    volumes::apply_mounted(&conn, &mounted)?;
    volumes::list(&conn)
}

/// Catalog rows with online state, full path (while online) and thumbnail path.
#[tauri::command(async)]
pub fn list_assets(db: State<'_, Db>, filter: Option<db::AssetFilter>) -> Result<Vec<db::Asset>, String> {
    db::list_assets(&db.conn.lock().unwrap(), &filter.unwrap_or_default())
}

#[tauri::command(async)]
pub fn update_asset(db: State<'_, Db>, id: i64, patch: db::AssetPatch) -> Result<(), String> {
    db::update_asset(&db.conn.lock().unwrap(), id, &patch)
}

/// `mode` is "add" or "remove".
#[tauri::command(async)]
pub fn set_tags(
    db: State<'_, Db>,
    asset_ids: Vec<i64>,
    names: Vec<String>,
    mode: String,
) -> Result<(), String> {
    let add = match mode.as_str() {
        "add" => true,
        "remove" => false,
        other => return Err(format!("unknown tag mode: {other}")),
    };
    db::set_tags(&mut db.conn.lock().unwrap(), &asset_ids, &names, add)
}

#[tauri::command(async)]
pub fn list_tags(db: State<'_, Db>) -> Result<Vec<db::Tag>, String> {
    db::list_tags(&db.conn.lock().unwrap())
}

#[tauri::command(async)]
pub fn list_albums(db: State<'_, Db>) -> Result<Vec<db::Album>, String> {
    db::list_albums(&db.conn.lock().unwrap())
}

#[tauri::command(async)]
pub fn create_album(db: State<'_, Db>, name: String) -> Result<i64, String> {
    db::create_album(&db.conn.lock().unwrap(), &name, volumes::now())
}

#[tauri::command(async)]
pub fn add_to_album(db: State<'_, Db>, album_id: i64, asset_ids: Vec<i64>) -> Result<(), String> {
    db::add_to_album(&mut db.conn.lock().unwrap(), album_id, &asset_ids)
}

#[tauri::command(async)]
pub fn remove_from_album(db: State<'_, Db>, album_id: i64, asset_ids: Vec<i64>) -> Result<(), String> {
    db::remove_from_album(&mut db.conn.lock().unwrap(), album_id, &asset_ids)
}

/// Show the original file in Finder.
#[tauri::command(async)]
pub fn reveal(db: State<'_, Db>, asset_id: i64) -> Result<(), String> {
    let path = db::asset_path(&db.conn.lock().unwrap(), asset_id)?;
    tauri_plugin_opener::reveal_item_in_dir(path).map_err(|e| e.to_string())
}

/// Open the original file in its default app (QuickTime, Preview, …).
#[tauri::command(async)]
pub fn open_external(db: State<'_, Db>, asset_id: i64) -> Result<(), String> {
    let path = db::asset_path(&db.conn.lock().unwrap(), asset_id)?;
    tauri_plugin_opener::open_path(path, None::<&str>).map_err(|e| e.to_string())
}

/// Move clips to Shelf's Trash (catalog only; the files stay on the drive).
#[tauri::command(async)]
pub fn trash_assets(db: State<'_, Db>, asset_ids: Vec<i64>) -> Result<(), String> {
    db::trash_assets(&mut db.conn.lock().unwrap(), &asset_ids, volumes::now())
}

/// Put clips back from the Trash with their tags, rating and albums.
#[tauri::command(async)]
pub fn restore_assets(db: State<'_, Db>, asset_ids: Vec<i64>) -> Result<(), String> {
    db::restore_assets(&mut db.conn.lock().unwrap(), &asset_ids)
}

#[tauri::command(async)]
pub fn trash_count(db: State<'_, Db>) -> Result<i64, String> {
    db::trash_count(&db.conn.lock().unwrap())
}

/// Permanently delete trashed clips' files from their drives. The frontend
/// asks for confirmation first; clips not in the Trash are never touched.
#[tauri::command(async)]
pub fn delete_from_disk(db: State<'_, Db>, asset_ids: Vec<i64>) -> Result<trash::DeleteReport, String> {
    trash::delete_from_disk(&mut db.conn.lock().unwrap(), &asset_ids)
}

/// Width of the picture under the cursor while dragging clips, in points.
const DRAG_ICON_W: u32 = 120;
/// How see-through it is (0–255), so the drop target stays visible.
const DRAG_ICON_ALPHA: u8 = 150;

/// A small, translucent PNG of the first clip's thumbnail for the drag image,
/// as a data URL. macOS adds the count badge itself when several files are dragged.
#[tauri::command(async)]
pub fn drag_icon(db: State<'_, Db>, asset_ids: Vec<i64>) -> Result<String, String> {
    use base64::Engine;
    use image::{imageops::FilterType, ImageFormat, Rgba};

    let thumb = {
        let conn = db.conn.lock().unwrap();
        asset_ids.iter().find_map(|id| {
            conn.query_row("SELECT thumb_path FROM assets WHERE id = ?1", [id], |r| {
                r.get::<_, Option<String>>(0)
            })
            .ok()
            .flatten()
        })
    }
    .ok_or("no thumbnail to drag")?;
    let src = image::open(&thumb).map_err(|e| e.to_string())?;
    let h = (DRAG_ICON_W * src.height() / src.width().max(1)).clamp(1, DRAG_ICON_W);
    let mut img = src.resize_exact(DRAG_ICON_W, h, FilterType::Triangle).to_rgba8();
    let (w, h) = img.dimensions();
    for (x, y, px) in img.enumerate_pixels_mut() {
        // A thin light edge keeps the outline readable over dark UI.
        if x == 0 || y == 0 || x == w - 1 || y == h - 1 {
            *px = Rgba([255, 255, 255, 200]);
        } else {
            px.0[3] = DRAG_ICON_ALPHA;
        }
    }
    let mut png = std::io::Cursor::new(Vec::new());
    img.write_to(&mut png, ImageFormat::Png).map_err(|e| e.to_string())?;
    Ok(format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(png.into_inner())
    ))
}

/// Rearrange clips by hand in an album, a drive or All clips: `asset_ids` go
/// right before `before_id` (or to the end).
#[tauri::command(async)]
pub fn move_assets(
    db: State<'_, Db>,
    scope: db::OrderScope,
    asset_ids: Vec<i64>,
    before_id: Option<i64>,
) -> Result<(), String> {
    db::move_assets(&mut db.conn.lock().unwrap(), scope, &asset_ids, before_id)
}
