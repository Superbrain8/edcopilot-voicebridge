// A release build has no console window.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod capture;
mod catalog;
mod commands;
mod dto;
mod events;
mod model_download;
mod paths;
mod secrets;
mod state;
mod tray;
mod updater;

use std::sync::Arc;

use serde::Serialize;
use tauri::Manager;

use crate::secrets::CredentialManagerStore;
use crate::state::{AppState, RuntimeStarter};

/// The data that the window shows about the program.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AppInfo {
    pub version: String,
    /// True when the program has the local engine. The value comes from a
    /// call into the linked sherpa-onnx library.
    pub engine_local: bool,
}

#[tauri::command]
fn app_info() -> AppInfo {
    let engine_local =
        edcopilot_voicebridge_stt::local_engine_version().is_some_and(|v| !v.is_empty());
    AppInfo {
        version: env!("CARGO_PKG_VERSION").to_string(),
        engine_local,
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // A development build never uses the data folder of the user. See
    // `paths::startup_plan`.
    let plan = paths::startup_plan(
        &args,
        std::env::var_os("APPDATA"),
        std::env::var_os(paths::DEV_DATA_DIR_VARIABLE),
        cfg!(debug_assertions),
    );
    let first_start_dir = plan.first_start_dir.clone();
    let state = AppState::new(
        plan.config_path,
        Arc::new(CredentialManagerStore),
        Arc::new(RuntimeStarter),
    )
    .with_data_dir(plan.data_dir);
    state::lock(&state.startup).problem = plan.problem;
    tauri::Builder::default()
        // The first plugin: a second start of the program shows the window
        // of the first program and ends. The second process ends in the
        // setup of this plugin, before the setup below. Thus it makes no
        // file, starts no session, and its arguments are ignored. Its
        // `--config` never changes the configuration path of the running
        // program: `AppState` gets that path one time, at the start.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            tray::show_main(app);
        }))
        .manage(state)
        // The folder dialogs of the Setup screen. The capability file permits
        // only the open dialog. The Rust side also shows the note of the
        // notification area with it.
        .plugin(tauri_plugin_dialog::init())
        // The update check and install. Only the Rust side uses it: the
        // capability file gives the page no updater permission.
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(move |app| {
            if let Some(data) = &first_start_dir {
                // A release build has no console: the window shows each
                // error and each note of this step.
                let notes = match app.path().resource_dir() {
                    Ok(resources) => paths::prepare_data(data, &resources),
                    Err(error) => paths::StartupNotes {
                        first_start_error: Some(format!(
                            "the program folder is not known, so the program could not make its data folder {}: {error}",
                            data.display()
                        )),
                        ..Default::default()
                    },
                };
                let state = app.state::<AppState>();
                let mut startup = state::lock(&state.startup);
                startup.first_start_error = notes.first_start_error;
                startup.notes = notes.notes;
                startup.update_problems = notes.update_problems;
            }
            if let Err(error) = tray::create(app.handle()) {
                eprintln!("the tray icon could not start: {error}");
            }
            // The window comes after the first start, so that its first
            // read finds the configuration file.
            let config = app
                .config()
                .app
                .windows
                .first()
                .cloned()
                .ok_or("tauri.conf.json has no window")?;
            tauri::WebviewWindowBuilder::from_config(app.handle(), &config)?.build()?;
            // The first automatic update check comes after the start. It
            // does not stop the start.
            commands::ipc::start_update_timer(app.handle().clone());
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                // Without a tray icon, the close button ends the program.
                if window.label() == tray::MAIN_WINDOW && tray::is_ready(window.app_handle()) {
                    api.prevent_close();
                    tray::hide_to_tray(window);
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            app_info,
            commands::ipc::get_config,
            commands::ipc::save_config,
            commands::ipc::check,
            commands::ipc::start_session,
            commands::ipc::stop_session,
            commands::ipc::session_status,
            commands::ipc::list_devices,
            commands::ipc::capture_binding,
            commands::ipc::cancel_capture,
            commands::ipc::set_api_key,
            commands::ipc::clear_api_key,
            commands::ipc::model_info,
            commands::ipc::download_model,
            commands::ipc::cancel_model_download,
            commands::ipc::list_commands,
            commands::ipc::list_left_out,
            commands::ipc::test_phrase,
            commands::ipc::read_table,
            commands::ipc::check_table,
            commands::ipc::save_table,
            commands::ipc::set_diagnostics,
            commands::ipc::open_folder,
            commands::ipc::startup_notes,
            commands::ipc::take_tray_start,
            commands::ipc::update_status,
            commands::ipc::check_for_updates,
            commands::ipc::install_update,
            commands::ipc::cancel_update,
            commands::ipc::hide_update,
        ])
        .build(tauri::generate_context!())
        .expect("the Tauri program could not start")
        .run(on_run_event);
}

/// Stops the session before the program ends. "Quit" in the tray menu
/// stops the session first and then ends the program. Tauri then sends
/// `ExitRequested` and `Exit`. Both callbacks run on the main thread before
/// the process ends. The first call that finds a session stops it, a later
/// call does nothing. Without a tray icon, the close of the last window
/// also ends the program here.
fn on_run_event(app: &tauri::AppHandle, event: tauri::RunEvent) {
    if matches!(
        event,
        tauri::RunEvent::ExitRequested { .. } | tauri::RunEvent::Exit
    ) {
        commands::shutdown(&app.state::<AppState>());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_app_info_reports_the_local_engine() {
        let info = app_info();
        assert!(info.engine_local);
        assert_eq!(info.version, env!("CARGO_PKG_VERSION"));
    }

    /// A speech clip of the private folder `VOICEBRIDGE_STT_FIXTURES`
    /// (default `private/stt_fixtures` in the workspace; a relative path
    /// starts at the workspace folder). The repository has no recordings.
    /// `None` (with a note on stderr): the clip is absent.
    fn speech_clip(name: &str) -> Option<std::path::PathBuf> {
        let folder = std::env::var_os("VOICEBRIDGE_STT_FIXTURES")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::path::PathBuf::from("private/stt_fixtures"));
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join(folder)
            .join(name);
        if path.is_file() {
            Some(path)
        } else {
            eprintln!("skipped: no speech clip at {}", path.display());
            None
        }
    }

    // This test loads the real model with the link settings of this program.
    // It needs the Parakeet model files: set VOICEBRIDGE_PARAKEET_DIR to the
    // model directory. It needs the speech clip of `speech_clip`.
    #[test]
    #[ignore]
    fn the_local_engine_transcribes_in_this_program() {
        use edcopilot_voicebridge_stt::{wav, ParakeetEngine, SttEngine};
        use std::path::Path;

        let Some(clip) = speech_clip("en_set_course_home.wav") else {
            return;
        };
        let dir = std::env::var("VOICEBRIDGE_PARAKEET_DIR").expect("VOICEBRIDGE_PARAKEET_DIR");
        let mut engine = ParakeetEngine::new(Path::new(&dir), 4, None).unwrap();
        let text = engine
            .transcribe(&wav::read_file(&clip).unwrap())
            .unwrap()
            .text;
        assert!(text.to_lowercase().contains("course"), "{text:?}");
    }
}
