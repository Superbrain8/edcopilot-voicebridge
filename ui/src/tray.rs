//! The icon in the notification area (decision D3).
//!
//! The menu has "Show", "Start...", "Stop", and "Quit".
//!
//! - "Show" and a left click on the icon show the window.
//! - "Start..." shows the window and asks the page to show the screen for a
//!   start: the Status screen, or the Setup screen when `check` has a
//!   problem. The tray does not start a session itself. On the Status screen
//!   the user selects a session or a dry run, and sees each problem.
//! - "Stop" stops the session. It is on only while a session runs or
//!   starts.
//! - "Quit" stops the session, ends a capture and a download, and then
//!   ends the program.
//!
//! The tooltip of the icon shows the state of the session as text.
//!
//! A tray call from a thread that is not the main thread waits for the main
//! thread. Thus each change of the tray from a session thread goes through
//! `AppHandle::run_on_main_thread`, which does not wait. Work that waits,
//! for example a stop, runs on its own thread, never on the main thread.

use std::sync::atomic::{AtomicBool, Ordering};

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, Wry};

use crate::dto::StatusDto;
use crate::state::AppState;

/// The id of the tray icon.
pub const TRAY_ID: &str = "voicebridge";
/// The event that asks the page to show the screen for a start.
pub const START_EVENT: &str = "tray://start";
/// The label of the main window.
pub const MAIN_WINDOW: &str = "main";

const MENU_SHOW: &str = "show";
const MENU_START: &str = "start";
const MENU_STOP: &str = "stop";
const MENU_QUIT: &str = "quit";

/// The text that the window shows one time, before the close button hides
/// it for the first time in a run of the program.
pub const HIDE_NOTE: &str = "VoiceBridge keeps running in the notification area. \
A running session continues.\n\nTo show the window again, click the VoiceBridge icon \
in the notification area. To end the program, right-click the icon and select Quit.";

/// The tray state that Tauri manages.
pub struct Tray {
    /// The "Stop" entry. `refresh` switches it on and off.
    stop: MenuItem<Wry>,
    /// True after the window told the user about the notification area.
    noted: AtomicBool,
}

/// The tooltip text for a status of the session.
pub fn tooltip_text(status: &StatusDto) -> String {
    let state = if status.running && status.dry_run {
        "dry run, nothing is sent"
    } else if status.running {
        "session running"
    } else if status.starting {
        "session starting"
    } else if status
        .stop_reason
        .as_ref()
        .is_some_and(|reason| reason.kind != "requested")
    {
        "stopped after an error"
    } else {
        "stopped"
    };
    format!("VoiceBridge: {state}")
}

/// True when the "Stop" entry is on.
pub fn stop_enabled(status: &StatusDto) -> bool {
    status.running || status.starting
}

/// Makes the tray icon and its menu.
pub fn create(app: &AppHandle) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, MENU_SHOW, "Show VoiceBridge", true, None::<&str>)?;
    let start = MenuItem::with_id(app, MENU_START, "Start...", true, None::<&str>)?;
    let stop = MenuItem::with_id(app, MENU_STOP, "Stop", false, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, MENU_QUIT, "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &start, &stop, &separator, &quit])?;
    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .menu(&menu)
        .show_menu_on_left_click(false)
        .tooltip(tooltip_text(&StatusDto::default()))
        .on_menu_event(|app, event| match event.id().as_ref() {
            MENU_SHOW => show_main(app),
            MENU_START => {
                show_main(app);
                // The page decides between Status and Setup. The mark stays
                // until the page takes it, also when the page loads later.
                crate::commands::mark_tray_start(&app.state::<AppState>());
                let _ = app.emit_to(MAIN_WINDOW, START_EVENT, ());
            }
            MENU_STOP => stop_session(app),
            MENU_QUIT => quit_program(app),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    app.manage(Tray {
        stop,
        noted: AtomicBool::new(false),
    });
    Ok(())
}

/// True when the tray icon exists. Without it, the close button ends the
/// program as usual.
pub fn is_ready(app: &AppHandle) -> bool {
    app.try_state::<Tray>().is_some()
}

/// Shows the main window, also when it is minimized, and gives it the
/// focus.
pub fn show_main(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(MAIN_WINDOW) {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

/// Sets the tooltip and the "Stop" entry from the status of the session.
/// Call it on the main thread, or through `refresh_later`.
pub fn refresh(app: &AppHandle) {
    let Ok(status) = crate::commands::session_status(&app.state::<AppState>()) else {
        return;
    };
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let _ = tray.set_tooltip(Some(tooltip_text(&status)));
    }
    if let Some(tray) = app.try_state::<Tray>() {
        let _ = tray.stop.set_enabled(stop_enabled(&status));
    }
}

/// `refresh` from any thread. It does not wait for the main thread.
pub fn refresh_later(app: &AppHandle) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || refresh(&handle));
}

/// Stops the session on its own thread: the stop waits until the
/// utterance in work is complete.
fn stop_session(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || {
        let _ = crate::commands::stop_session(&app.state::<AppState>());
        refresh_later(&app);
    });
}

/// Stops the session cleanly, ends a capture and a download, then ends the
/// program. After the stop request, no command goes to EDCoPilot. The work
/// runs on its own thread, because the stop waits for the session threads,
/// and a session thread can wait for the main thread.
fn quit_program(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || {
        crate::commands::shutdown(&app.state::<AppState>());
        app.exit(0);
    });
}

/// The close button: hides the window, and the program keeps running. The
/// session and a download continue. The first time in a run of the
/// program, a message tells the user where the program is.
pub fn hide_to_tray(window: &tauri::Window) {
    let app = window.app_handle();
    let first = app
        .try_state::<Tray>()
        .is_some_and(|tray| !tray.noted.swap(true, Ordering::SeqCst));
    if !first {
        let _ = window.hide();
        return;
    }
    use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
    let hide = window.clone();
    app.dialog()
        .message(HIDE_NOTE)
        .title("VoiceBridge")
        .kind(MessageDialogKind::Info)
        .buttons(MessageDialogButtons::Ok)
        .parent(window)
        .show(move |_| {
            let _ = hide.hide();
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dto::StopReasonDto;

    fn status(running: bool, starting: bool, dry_run: bool, stop: Option<&str>) -> StatusDto {
        StatusDto {
            running,
            starting,
            dry_run,
            start_info: None,
            stop_reason: stop.map(|kind| StopReasonDto {
                kind: kind.to_string(),
                message: None,
            }),
        }
    }

    #[test]
    fn test_tooltip_shows_the_state_of_the_session_as_text() {
        assert_eq!(
            tooltip_text(&status(false, false, false, None)),
            "VoiceBridge: stopped"
        );
        assert_eq!(
            tooltip_text(&status(false, true, false, None)),
            "VoiceBridge: session starting"
        );
        assert_eq!(
            tooltip_text(&status(true, false, false, None)),
            "VoiceBridge: session running"
        );
        assert_eq!(
            tooltip_text(&status(true, false, true, None)),
            "VoiceBridge: dry run, nothing is sent"
        );
        assert_eq!(
            tooltip_text(&status(false, false, false, Some("requested"))),
            "VoiceBridge: stopped"
        );
        assert_eq!(
            tooltip_text(&status(false, false, true, Some("device_error"))),
            "VoiceBridge: stopped after an error"
        );
    }

    #[test]
    fn test_stop_is_on_only_while_a_session_runs_or_starts() {
        assert!(!stop_enabled(&status(false, false, false, None)));
        assert!(stop_enabled(&status(false, true, false, None)));
        assert!(stop_enabled(&status(true, false, true, None)));
        assert!(!stop_enabled(&status(
            false,
            false,
            false,
            Some("engine_error")
        )));
    }
}
