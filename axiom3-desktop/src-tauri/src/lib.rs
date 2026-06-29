// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

/// Returns the open-core verification engine version.
/// P0.1 smoke test that the shell -> axiom-core boundary is wired.
#[tauri::command]
fn axiom_core_version() -> String {
    axiom_core::version().to_string()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![greet, axiom_core_version])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
