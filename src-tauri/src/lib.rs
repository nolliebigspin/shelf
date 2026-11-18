mod commands;
mod db;
mod media;
mod scan;
mod trash;
mod volumes;

use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_drag::init())
        .manage(scan::ScanState::default())
        .manage(commands::AllowedRoots::default())
        .setup(|app| {
            let app_data = app.path().app_data_dir()?;
            let db = db::open(&app_data)?;
            commands::allow_media_roots(app.handle(), &db.conn.lock().unwrap());
            app.manage(db);
            // Pick up thumbnails an earlier run didn't finish, off the main thread.
            let handle = app.handle().clone();
            std::thread::spawn(move || scan::resume_pending(&handle));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::app_status,
            commands::add_root,
            commands::rescan,
            commands::refresh_volumes,
            commands::list_assets,
            commands::update_asset,
            commands::set_tags,
            commands::list_tags,
            commands::list_albums,
            commands::create_album,
            commands::add_to_album,
            commands::remove_from_album,
            commands::reveal,
            commands::open_external,
            commands::trash_assets,
            commands::restore_assets,
            commands::trash_count,
            commands::delete_from_disk,
            commands::drag_icon,
            commands::move_assets
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
