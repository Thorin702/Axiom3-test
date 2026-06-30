mod workbench;

use workbench::AppState;

/// Returns the open-core verification engine version.
/// Smoke test that the shell -> axiom-core boundary is wired.
#[tauri::command]
fn axiom_core_version() -> String {
    axiom_core::version().to_string()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Hydrate the process env from any keychain-stored BYO provider keys so the fleet
    // uses them without re-entry (env still wins, so `KEY=… pnpm tauri dev` overrides).
    workbench::load_provider_keys_into_env();

    let mut builder = tauri::Builder::default();

    // Single-instance must be registered first (desktop only): a second launch
    // forwards to the running process instead of racing first-run DB-key creation.
    #[cfg(desktop)]
    {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            // A second launch surfaces the running window instead of doing nothing.
            use tauri::Manager;
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.unminimize();
                let _ = w.set_focus();
            }
        }));
    }

    builder
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            axiom_core_version,
            workbench::open_project,
            workbench::project_status,
            workbench::append_event,
            workbench::verify_chain,
            workbench::check_draft,
            workbench::cancel_check,
            workbench::run_workflow,
            workbench::cancel_run,
            workbench::import_document,
            workbench::vector_self_test,
            workbench::export_provenance,
            workbench::system_check,
            workbench::provider_status,
            workbench::set_provider_key,
            workbench::delete_provider_key,
            workbench::close_project,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
