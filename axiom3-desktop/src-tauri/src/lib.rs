mod workbench;

use workbench::AppState;

/// Returns the open-core verification engine version.
/// Smoke test that the shell -> axiom-core boundary is wired.
#[tauri::command]
fn axiom_core_version() -> String {
    axiom_core::version().to_string()
}

/// Build the native macOS menu bar (feature-list #42): App / File / Edit / View / Window.
/// Custom items emit a `menu` event the frontend acts on; Edit/Window use predefined items.
#[cfg(target_os = "macos")]
fn setup_menu(app: &mut tauri::App) -> tauri::Result<()> {
    use tauri::menu::{MenuBuilder, MenuItemBuilder, SubmenuBuilder};
    use tauri::Emitter;

    let h = app.handle().clone();
    let new_task = MenuItemBuilder::with_id("new_task", "New Task").accelerator("CmdOrCtrl+N").build(&h)?;
    let new_workflow = MenuItemBuilder::with_id("new_workflow", "New Workflow").accelerator("CmdOrCtrl+Shift+N").build(&h)?;
    let settings = MenuItemBuilder::with_id("open_settings", "Settings…").accelerator("CmdOrCtrl+,").build(&h)?;
    let toggle_rail = MenuItemBuilder::with_id("toggle_rail", "Toggle Activity Rail").accelerator("CmdOrCtrl+/").build(&h)?;

    let app_menu = SubmenuBuilder::new(&h, "Axiom3").about(None).separator().hide().quit().build()?;
    let file_menu = SubmenuBuilder::new(&h, "File").item(&new_task).item(&new_workflow).separator().item(&settings).build()?;
    let edit_menu = SubmenuBuilder::new(&h, "Edit").undo().redo().separator().cut().copy().paste().select_all().build()?;
    let view_menu = SubmenuBuilder::new(&h, "View").item(&toggle_rail).separator().fullscreen().build()?;
    let window_menu = SubmenuBuilder::new(&h, "Window").minimize().separator().close_window().build()?;

    let menu = MenuBuilder::new(&h)
        .items(&[&app_menu, &file_menu, &edit_menu, &view_menu, &window_menu])
        .build()?;
    app.set_menu(menu)?;

    app.on_menu_event(|app, event| {
        let action = match event.id().0.as_str() {
            "new_task" => "new-task",
            "new_workflow" => "new-workflow",
            "open_settings" => "settings",
            "toggle_rail" => "toggle-rail",
            _ => return,
        };
        let _ = app.emit("menu", action);
    });
    Ok(())
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
        // Restore window geometry (size / position / maximized) across launches.
        builder = builder.plugin(tauri_plugin_window_state::Builder::default().build());
    }

    builder
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::default())
        .setup(|app| {
            #[cfg(target_os = "macos")]
            setup_menu(app)?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            axiom_core_version,
            workbench::open_project,
            workbench::project_status,
            workbench::append_event,
            workbench::verify_chain,
            workbench::list_events,
            workbench::check_draft,
            workbench::cancel_check,
            workbench::run_workflow,
            workbench::cancel_run,
            workbench::pause_run,
            workbench::resume_run,
            workbench::coding_run,
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
