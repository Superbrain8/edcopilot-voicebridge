//! `ConsoleSink`: prints the events of a runtime as the console program
//! printed them before the runtime. Each line goes to the same stream.

use crate::run::{outcome_line, report_line};
use crate::runtime::{EventSink, RuntimeEvent, StartInfo, StopReason};

/// Prints each event on stdout or stderr. After `Stopped` it gives the
/// reason to `on_stopped`.
pub struct ConsoleSink {
    on_stopped: Box<dyn Fn(StopReason) + Send + Sync>,
}

impl ConsoleSink {
    pub fn new(on_stopped: impl Fn(StopReason) + Send + Sync + 'static) -> ConsoleSink {
        ConsoleSink {
            on_stopped: Box::new(on_stopped),
        }
    }
}

impl EventSink for ConsoleSink {
    fn event(&self, event: RuntimeEvent) {
        match event {
            RuntimeEvent::Warning(line) => eprintln!("{line}"),
            RuntimeEvent::Started(info) => {
                for line in start_lines(&info) {
                    println!("{line}");
                }
            }
            // The console shows no line for the talk button.
            RuntimeEvent::Talk(_) => {}
            RuntimeEvent::Utterance(r) => println!(
                "{}",
                outcome_line(&r.outcome, r.transcribe, r.total, r.clip, r.levels, r.marks)
            ),
            RuntimeEvent::Dropped { reason } => println!("utterance dropped: {reason}"),
            RuntimeEvent::VocabularyReloaded { files, names } => {
                println!("vocabulary reloaded: {files} files with {names} names")
            }
            RuntimeEvent::Stopped { reason, glitches } => {
                match &reason {
                    StopReason::Requested => {}
                    StopReason::DeviceError(error) => eprintln!("audio stream error: {error}"),
                    StopReason::EngineError(error) | StopReason::InputError(error) => {
                        eprintln!("session stopped: {error}")
                    }
                }
                if glitches > 0 {
                    println!("audio glitches (buffer overruns and similar): {glitches}");
                }
                (self.on_stopped)(reason);
            }
        }
    }
}

/// The stdout lines of the start, in the order of the console program.
pub fn start_lines(info: &StartInfo) -> Vec<String> {
    let mut lines = vec![report_line(&info.report)];
    if !info.hotwords_line.is_empty() {
        lines.push(info.hotwords_line.clone());
    }
    lines.push(format!("engine: {}", info.engine));
    lines.push(format!("mode: {}", info.mode_line));
    lines.push(format!(
        "input device: {}",
        info.input_device
            .as_deref()
            .unwrap_or("default input device")
    ));
    if let Some(threads) = info.threads {
        lines.push(format!("threads: {threads}"));
    }
    if info.dry_run {
        lines.push("dry run: no request file is written".to_string());
    }
    lines.extend(info.diagnostics_lines.iter().cloned());
    lines.push("listening, Ctrl+C stops".to_string());
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::LoadReport;
    use crate::runtime::Mode;

    #[test]
    fn test_start_lines_have_the_order_of_the_console_program() {
        let mut info = StartInfo {
            report: LoadReport::default(),
            engine: "parakeet".to_string(),
            mode: Mode::PushToTalk,
            mode_line: "push to talk, key \"F13\"".to_string(),
            input_device: None,
            threads: Some(4),
            dry_run: true,
            hotwords_line: "hotwords: off".to_string(),
            diagnostics_lines: vec!["utterance log: u.tsv".to_string()],
        };
        let lines = start_lines(&info);
        assert!(lines[0].starts_with("loaded 0 commands"), "{lines:?}");
        assert_eq!(
            &lines[1..],
            [
                "hotwords: off",
                "engine: parakeet",
                "mode: push to talk, key \"F13\"",
                "input device: default input device",
                "threads: 4",
                "dry run: no request file is written",
                "utterance log: u.tsv",
                "listening, Ctrl+C stops",
            ]
        );
        // The cloud engine: no hotword line and no threads.
        info.hotwords_line = String::new();
        info.threads = None;
        info.dry_run = false;
        info.input_device = Some("USB Mic".to_string());
        let lines = start_lines(&info);
        assert_eq!(
            &lines[1..],
            [
                "engine: parakeet",
                "mode: push to talk, key \"F13\"",
                "input device: USB Mic",
                "utterance log: u.tsv",
                "listening, Ctrl+C stops",
            ]
        );
    }
}
