mod converter;

use converter::{cancel_convert, check_is_dir, convert_comic, get_mobi_info, get_mobi_page, get_version, list_comics, ConvertCancel, MobiCache};
use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::Mutex;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(MobiCache(Mutex::new(HashMap::new())))
        .manage(ConvertCancel(AtomicBool::new(false)))
        .invoke_handler(tauri::generate_handler![
            check_is_dir,
            get_mobi_info,
            get_mobi_page,
            convert_comic,
            cancel_convert,
            list_comics,
            get_version,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
