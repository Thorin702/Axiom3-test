mod auth;
mod deliverables;
mod telemetry;
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
        // GUARD: closing the (only) window destroys it → the process exits → any in-process run is
        // killed mid-flight. While a run/check is live, VETO the close and ask the UI to confirm
        // (it calls `force_quit` on confirm, which arms the bypass). No run → close normally.
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                use tauri::{Emitter, Manager};
                let state = window.state::<AppState>();
                if state.is_busy() && !state.force_quit_armed() {
                    api.prevent_close();
                    let _ = window.emit("close-blocked", ());
                }
            }
        })
        .setup(|app| {
            // Hydrate the process env from any stored BYO provider keys so the fleet uses
            // them without re-entry (env still wins, so `KEY=… pnpm tauri dev` overrides).
            // Done here (not before the builder) so the app data dir — hence the configured
            // secret store — is resolvable.
            // FIRST: apply the persisted telemetry opt-out and arm the crash marker. Must precede
            // anything that could emit — the opt-out has to be in force before the first event, and
            // the marker has to be read before this session overwrites it.
            telemetry::boot(app.handle());
            workbench::load_provider_keys_into_env(app.handle());
            // Register the user's OpenAI-compatible model connections into the runtime dispatch
            // table so runs can reach them by label.
            workbench::register_connections_into_runtime(app.handle());
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
            workbench::recap_run,
            workbench::import_document,
            workbench::vector_self_test,
            workbench::export_provenance,
            workbench::save_report_html,
            workbench::save_png,
            workbench::rewrite_passage,
            workbench::answer_ask,
            workbench::clear_cache,
            workbench::kernel_exec,
            workbench::kernel_reset,
            workbench::kernel_status,
            workbench::kernel_prepare,
            workbench::provision_status,
            workbench::provision_install,
            workbench::system_check,
            workbench::provider_status,
            workbench::set_provider_key,
            workbench::delete_provider_key,
            workbench::list_connections,
            workbench::save_connection,
            workbench::delete_connection,
            workbench::test_connection,
            workbench::close_project,
            deliverables::deliverable_save,
            deliverables::deliverable_list,
            deliverables::deliverable_load,
            deliverables::deliverable_delete,
            deliverables::deliverable_rename,
            auth::account_login,
            auth::account_session,
            auth::account_validate,
            auth::account_logout,
            telemetry::telemetry,
            telemetry::fetch_notice,
            telemetry::set_telemetry,
            workbench::run_active,
            workbench::force_quit,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        // GUARD (Cmd+Q / last-window-close cascade): the same protection at the app-exit level. While a
        // run is live and the user hasn't confirmed via `force_quit`, veto the exit and ask the UI. The
        // `force_quit` command arms `force_quit_armed()` before calling `app.exit(0)`, so the bypass lets
        // the process actually quit.
        .run(|app_handle, event| match event {
            tauri::RunEvent::ExitRequested { api, .. } => {
                use tauri::{Emitter, Manager};
                let state = app_handle.state::<AppState>();
                if state.is_busy() && !state.force_quit_armed() {
                    api.prevent_exit();
                    let _ = app_handle.emit("exit-blocked", ());
                }
            }
            // The app is really going down, through the front door. Clear the run marker so the next
            // launch does NOT report a crash. Deliberately on `Exit` and not `ExitRequested` — the
            // latter can be vetoed above, and a vetoed exit must not clear the marker.
            tauri::RunEvent::Exit => telemetry::mark_clean_exit(app_handle),
            _ => {}
        });
}
