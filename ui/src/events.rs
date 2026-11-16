//! `WindowSink`: sends the events of the runtime to the window.

use std::sync::{Arc, Mutex};

use voicebridge::runtime::{EventSink, RuntimeEvent};

use crate::dto::{EventDto, ModelEventDto, StartInfoDto, StopReasonDto, UpdateStatusDto};
use crate::state::{lock, Status};

/// The name of the Tauri event for each session event. The payload is an
/// `EventDto`.
pub const SESSION_EVENT: &str = "session://event";

/// The name of the Tauri event for each step of the model download. The
/// payload is a `ModelEventDto`.
pub const MODEL_EVENT: &str = "model://event";

/// The name of the Tauri event for each change of the update state. The
/// payload is an `UpdateStatusDto`.
pub const UPDATE_EVENT: &str = "update://event";

/// Sends the update state to the web part.
pub trait UpdateEmit: Send + Sync {
    fn emit_update(&self, status: &UpdateStatusDto);
}

impl UpdateEmit for TauriEmit {
    fn emit_update(&self, status: &UpdateStatusDto) {
        use tauri::Emitter;
        // A window that is closed does not stop the update.
        let _ = self.0.emit(UPDATE_EVENT, status);
    }
}

/// Sends one event to the web part.
pub trait Emit: Send + Sync + 'static {
    fn emit(&self, event: &EventDto);
}

/// Sends one step of the model download to the web part.
pub trait ModelEmit: Send + Sync {
    fn emit_model(&self, event: &ModelEventDto);
}

/// Sends the events to all windows of the program.
pub struct TauriEmit(pub tauri::AppHandle);

impl Emit for TauriEmit {
    fn emit(&self, event: &EventDto) {
        use tauri::Emitter;
        // A window that is closed does not stop the session.
        let _ = self.0.emit(SESSION_EVENT, event);
        // The sink changed the status before this call. The tooltip of the
        // tray follows it. This call does not wait for the main thread.
        if matches!(event, EventDto::Started { .. } | EventDto::Stopped { .. }) {
            crate::tray::refresh_later(&self.0);
        }
    }
}

impl ModelEmit for TauriEmit {
    fn emit_model(&self, event: &ModelEventDto) {
        use tauri::Emitter;
        // A window that is closed does not stop the download.
        let _ = self.0.emit(MODEL_EVENT, event);
    }
}

/// Keeps the status of its session up to date and sends each event on.
/// It only stores small values and sends. It never stops or drops the
/// runtime.
pub struct WindowSink {
    emit: Arc<dyn Emit>,
    status: Arc<Mutex<Status>>,
    generation: u64,
}

impl WindowSink {
    pub fn new(emit: Arc<dyn Emit>, status: Arc<Mutex<Status>>, generation: u64) -> WindowSink {
        WindowSink {
            emit,
            status,
            generation,
        }
    }

    fn update(&self, event: &RuntimeEvent) {
        let mut status = lock(&self.status);
        if status.generation != self.generation {
            return;
        }
        match event {
            RuntimeEvent::Started(info) => {
                status.running = true;
                status.dry_run = info.dry_run;
                status.start_info = Some(StartInfoDto::from(info.as_ref()));
                status.stop_reason = None;
            }
            RuntimeEvent::Stopped { reason, .. } => {
                status.running = false;
                status.stop_reason = Some(StopReasonDto::from(reason));
            }
            _ => {}
        }
    }
}

impl EventSink for WindowSink {
    fn event(&self, event: RuntimeEvent) {
        self.update(&event);
        self.emit.emit(&EventDto::from(&event));
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use voicebridge::config::LoadReport;
    use voicebridge::runtime::{Mode, StartInfo, StopReason};

    /// Keeps the JSON of each event.
    #[derive(Default)]
    pub struct FakeEmit(pub Mutex<Vec<serde_json::Value>>);

    impl Emit for FakeEmit {
        fn emit(&self, event: &EventDto) {
            lock(&self.0).push(serde_json::to_value(event).unwrap());
        }
    }

    impl ModelEmit for FakeEmit {
        fn emit_model(&self, event: &ModelEventDto) {
            lock(&self.0).push(serde_json::to_value(event).unwrap());
        }
    }

    impl UpdateEmit for FakeEmit {
        fn emit_update(&self, status: &UpdateStatusDto) {
            lock(&self.0).push(serde_json::to_value(status).unwrap());
        }
    }

    impl FakeEmit {
        /// The `phase` of each update event.
        pub fn phases(&self) -> Vec<String> {
            lock(&self.0)
                .iter()
                .map(|e| e["phase"].as_str().unwrap().to_string())
                .collect()
        }
    }

    impl FakeEmit {
        pub fn types(&self) -> Vec<String> {
            lock(&self.0)
                .iter()
                .map(|e| e["type"].as_str().unwrap().to_string())
                .collect()
        }
    }

    pub fn start_info(dry_run: bool) -> StartInfo {
        StartInfo {
            report: LoadReport::default(),
            engine: "fake".to_string(),
            mode: Mode::PushToTalk,
            mode_line: "push to talk, key \"F13\"".to_string(),
            input_device: None,
            threads: Some(1),
            dry_run,
            hotwords_line: "hotwords: off".to_string(),
            diagnostics_lines: Vec::new(),
        }
    }

    #[test]
    fn test_runtime_events_become_window_events() {
        let emit = Arc::new(FakeEmit::default());
        let status = Arc::new(Mutex::new(Status::default()));
        let sink = WindowSink::new(emit.clone(), status.clone(), 0);
        sink.event(RuntimeEvent::Warning("warning: x".into()));
        sink.event(RuntimeEvent::Started(Box::new(start_info(true))));
        assert!(lock(&status).running);
        assert!(lock(&status).dry_run);
        sink.event(RuntimeEvent::Talk(true));
        sink.event(RuntimeEvent::Talk(false));
        sink.event(RuntimeEvent::Stopped {
            reason: StopReason::EngineError("panic".into()),
            glitches: 0,
        });
        assert_eq!(
            emit.types(),
            ["warning", "started", "talk", "talk", "stopped"]
        );
        assert_eq!(lock(&emit.0)[0]["text"], "x");
        let status = lock(&status).dto(false);
        assert!(!status.running);
        assert_eq!(status.stop_reason.unwrap().kind, "engine_error");
        // The last start lines stay for the status screen.
        assert_eq!(status.start_info.unwrap().engine, "fake");
    }

    #[test]
    fn test_old_session_does_not_change_the_status() {
        let emit = Arc::new(FakeEmit::default());
        let status = Arc::new(Mutex::new(Status {
            generation: 2,
            running: true,
            ..Status::default()
        }));
        let old = WindowSink::new(emit.clone(), status.clone(), 1);
        old.event(RuntimeEvent::Stopped {
            reason: StopReason::Requested,
            glitches: 0,
        });
        assert!(lock(&status).running);
        // The window still gets the event.
        assert_eq!(emit.types(), ["stopped"]);
    }
}
