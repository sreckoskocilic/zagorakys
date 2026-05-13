mod converter;

use converter::{convert_comic, get_mobi_info, get_mobi_page, get_version, list_comics};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            get_mobi_info,
            get_mobi_page,
            convert_comic,
            list_comics,
            get_version,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
