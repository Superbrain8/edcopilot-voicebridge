//! Live probes of the `voicebridge` program on the real EDCoPilot profile.
//!
//! The live test is ignored. It reads the EDCoPilot folder from
//! `VOICEBRIDGE_EDCOPILOT_DIR` and writes nothing there. It runs the built
//! program with `text`, `commands` and `check` (never `--send`), with a
//! temporary configuration and temporary `LOCALAPPDATA` and `APPDATA`
//! folders. The configuration sets the request file to a path in the
//! temporary folder, thus a run cannot write the request file of
//! EDCoPilot. The repository holds no EDCoPilot text. Thus the probe data
//! is in the local file of the core probes (`VOICEBRIDGE_LIVE_PROBES`,
//! default `private/live_probes.tsv`), in its `@app` sections.
//!
//! ```text
//! VOICEBRIDGE_EDCOPILOT_DIR=C:\EDCoPilot cargo test -p voicebridge \
//!     --test live_probes -- --ignored --nocapture
//! ```
//!
//! # The format of an `@app` section
//!
//! The fields of a line are separated by a tab. An empty line and a line
//! that starts with `#` are not read. `@app <name>` starts a section, and
//! `@section` ends it (the core reader reads those sections).
//!
//! `<command> <argument> <expected>` is a probe:
//! - `<command>` is `text` (the argument is the phrase), `commands` (the
//!   argument gives the filter words), or `check` (the argument is `-`).
//! - `<expected>` is a text that the output must have. With `!` at the
//!   start, the output must not have the text.
//!
//! Each run must end with exit code 0 (see `expected_success`). Thus a
//! probe with `!` cannot pass because the program failed.
//!
//! Without the file at the default path, the test writes a note to stderr
//! and passes. These errors fail the test: an explicit path that cannot be
//! read, a byte order mark, an unknown command, an empty field, an `@app`
//! section without a probe, a `@vocab` line in an `@app` section, and a
//! line that starts with `@` and is not `@app`, `@section` or `@vocab`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

/// The text of the probe file and its path, with the rules of the core
/// reader. `None` when the default file is absent.
fn probe_file() -> Option<(PathBuf, String)> {
    let explicit = std::env::var_os("VOICEBRIDGE_LIVE_PROBES").map(PathBuf::from);
    let path = workspace().join(
        explicit
            .clone()
            .unwrap_or_else(|| PathBuf::from("private/live_probes.tsv")),
    );
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if explicit.is_none() && e.kind() == std::io::ErrorKind::NotFound => {
            eprintln!(
                "note: no probe file at {}, the live probes are skipped",
                path.display()
            );
            return None;
        }
        Err(e) => panic!("cannot read the probe file {}: {e}", path.display()),
    };
    assert!(
        !text.starts_with('\u{feff}'),
        "the probe file {} starts with a byte order mark",
        path.display()
    );
    Some((path, text))
}

/// A temporary configuration for the real folder: the shipped word tables
/// and alias files, and a dummy key variable. Each path that
/// the program can write is in `dir`: the request file is
/// `dir/request.txt`, and the utterance log and the audio capture are off.
fn write_config(dir: &Path, ed: &Path) -> PathBuf {
    let core = workspace().join("core").canonicalize().unwrap();
    let text = format!(
        "[edcopilot]\ndir = '{}'\nrequest_file = '{}'\n\
         [data]\naliases_dir = '{}'\nsound_alikes = '{}'\nsynonyms = '{}'\nfillers = '{}'\n\
         [stt]\nengine = 'openai_compatible'\n\
         [stt.openai_compatible]\napi_key_env = 'VB_LIVE_PROBE_KEY'\n\
         [diagnostics]\nlog_file = ''\nsave_audio_dir = ''\n",
        ed.display(),
        request_file(dir).display(),
        core.join("aliases").display(),
        core.join("data/sound_alikes.toml").display(),
        core.join("data/synonyms.toml").display(),
        core.join("data/fillers.toml").display(),
    );
    let file = dir.join("voicebridge.toml");
    std::fs::write(&file, text).unwrap();
    file
}

/// The request file of the temporary configuration.
fn request_file(dir: &Path) -> PathBuf {
    dir.join("request.txt")
}

/// The arguments of the program for one probe.
fn arguments(command: &str, argument: &str) -> Result<Vec<String>, String> {
    let args = match command {
        "text" => vec!["text".to_string(), argument.to_string()],
        "commands" => std::iter::once("commands".to_string())
            .chain(argument.split_whitespace().map(str::to_string))
            .collect(),
        "check" => vec!["check".to_string()],
        _ => return Err(format!("unknown command {command:?}")),
    };
    // The probes never send a command to EDCoPilot.
    if args.iter().any(|a| a.contains("--send")) {
        return Err("a probe may not send".to_string());
    }
    Ok(args)
}

/// True when a run of the command must end with exit code 0. Each probe
/// command must:
/// - `text`: a dry run gives exit code 0 for a match, a rejected match and
///   no match. Exit code 1 is a problem of the configuration or the load.
/// - `commands`: exit code 0 also when no command agrees with the filter.
/// - `check`: exit code 0 only when the configuration is valid. The
///   temporary configuration is valid, thus a problem is a failure.
fn expected_success(command: &str) -> bool {
    matches!(command, "text" | "commands" | "check")
}

/// One probe of an `@app` section.
#[derive(Debug, Clone, PartialEq)]
struct AppProbe {
    /// The line number (from 1) and the text of the line.
    line: usize,
    raw: String,
    args: Vec<String>,
    /// The output must have the text (`true`) or must not have it.
    want: bool,
    expected: String,
    success: bool,
}

/// The result of the reader: the probes, the number of `@app` sections,
/// and the errors of the file.
#[derive(Debug, Default)]
struct AppProbes {
    probes: Vec<AppProbe>,
    sections: usize,
    failures: Vec<String>,
}

/// Reads the `@app` sections of the probe file text.
fn parse_app_probes(text: &str) -> AppProbes {
    let mut out = AppProbes::default();
    // The open `@app` section: its line number, its name, and its probes.
    let mut open: Option<(usize, String, usize)> = None;
    let close = |open: &mut Option<(usize, String, usize)>, failures: &mut Vec<String>| {
        if let Some((line, name, 0)) = open.take() {
            failures.push(format!(
                "line {line}: the app section {name:?} has no probe"
            ));
        }
    };
    for (n, line) in text.lines().enumerate() {
        let line = line.trim_end_matches('\r');
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        let fields: Vec<&str> = line.split('\t').collect();
        let at = |i: usize| fields.get(i).map(|f| f.trim()).unwrap_or("");
        let fail = |reason: String| format!("line {}: {line:?}: {reason}", n + 1);
        match at(0) {
            "@app" => {
                close(&mut open, &mut out.failures);
                out.sections += 1;
                open = Some((n + 1, at(1).to_string(), 0));
            }
            "@section" => close(&mut open, &mut out.failures),
            "@vocab" if open.is_some() => out
                .failures
                .push(fail("a vocabulary in an app section".to_string())),
            "@vocab" => {}
            other if other.starts_with('@') => out
                .failures
                .push(fail(format!("unknown directive {other:?}"))),
            _ if open.is_none() => {}
            command => {
                if let Some((_, _, probes)) = open.as_mut() {
                    *probes += 1;
                }
                let (argument, expected) = (at(1), at(2));
                if argument.is_empty() || expected.is_empty() || expected == "!" {
                    out.failures.push(fail("an empty field".to_string()));
                    continue;
                }
                let args = match arguments(command, argument) {
                    Ok(args) => args,
                    Err(e) => {
                        out.failures.push(fail(e));
                        continue;
                    }
                };
                let (want, expected) = match expected.strip_prefix('!') {
                    Some(text) => (false, text),
                    None => (true, expected),
                };
                out.probes.push(AppProbe {
                    line: n + 1,
                    raw: line.to_string(),
                    args,
                    want,
                    expected: expected.to_string(),
                    success: expected_success(command),
                });
            }
        }
    }
    close(&mut open, &mut out.failures);
    out
}

/// The output and the exit code of one run of the program.
#[derive(Debug, Clone)]
struct Run {
    stdout: String,
    stderr: String,
    code: Option<i32>,
}

/// The failure of `probe` on `run`, or `None` when the probe passes. Fix
/// round 10: the probe reads stdout only. The program writes the load
/// warnings to stderr, and a warning can hold the text of a probe. A
/// failure shows the first lines of stdout and of stderr separately.
fn evaluate(probe: &AppProbe, run: &Run) -> Option<String> {
    let first = |text: &str| text.lines().take(3).map(str::to_string).collect::<Vec<_>>();
    let shown = format!(
        "stdout: {:?}, stderr: {:?}",
        first(&run.stdout),
        first(&run.stderr)
    );
    if (run.code == Some(0)) != probe.success {
        return Some(format!("exit code {:?}, {shown}", run.code));
    }
    if run.stdout.contains(&probe.expected) != probe.want {
        return Some(shown);
    }
    None
}

/// The probes of the `@app` sections of the private file, through the
/// program.
#[test]
#[ignore = "needs the EDCoPilot installation (VOICEBRIDGE_EDCOPILOT_DIR) and the private probe file"]
fn test_live_app_probes_from_the_private_file() {
    let Some((path, text)) = probe_file() else {
        return;
    };
    let ed = PathBuf::from(
        std::env::var_os("VOICEBRIDGE_EDCOPILOT_DIR")
            .expect("set VOICEBRIDGE_EDCOPILOT_DIR to the EDCoPilot folder"),
    );
    let dir = tempfile::tempdir().unwrap();
    let local = dir.path().join("local");
    let roaming = dir.path().join("roaming");
    std::fs::create_dir_all(&local).unwrap();
    std::fs::create_dir_all(&roaming).unwrap();
    let config = write_config(dir.path(), &ed);
    let temp_request = request_file(dir.path());
    let real_request = ed.join("EDCoPilot.request.txt");
    let request_before = std::fs::metadata(&real_request)
        .and_then(|m| m.modified())
        .ok();
    let parsed = parse_app_probes(&text);
    let mut failures = parsed.failures.clone();
    // The output and the exit code of each run, for probes with the same
    // arguments.
    let mut outputs: HashMap<Vec<String>, Run> = HashMap::new();
    for probe in &parsed.probes {
        let run = outputs.entry(probe.args.clone()).or_insert_with(|| {
            let out = Command::new(env!("CARGO_BIN_EXE_voicebridge"))
                .arg("--config")
                .arg(&config)
                .args(&probe.args)
                .env("VB_LIVE_PROBE_KEY", "not-a-key")
                .env("LOCALAPPDATA", &local)
                .env("APPDATA", &roaming)
                .env("XDG_RUNTIME_DIR", &local)
                .output()
                .unwrap();
            // A dry run never writes the request file.
            assert!(
                !temp_request.exists(),
                "the run {:?} wrote the request file",
                probe.args
            );
            Run {
                stdout: String::from_utf8_lossy(&out.stdout).to_string(),
                stderr: String::from_utf8_lossy(&out.stderr).to_string(),
                code: out.status.code(),
            }
        });
        if let Some(reason) = evaluate(probe, run) {
            failures.push(format!("line {}: {:?}: {reason}", probe.line, probe.raw));
        }
    }
    println!(
        "live app probes: {} probes in {} sections from {}, {} failures",
        parsed.probes.len(),
        parsed.sections,
        path.display(),
        failures.len()
    );
    for f in &failures {
        println!("  FAIL {f}");
    }
    // The runs never wrote a request file.
    assert!(!temp_request.exists(), "a run wrote the request file");
    let request_after = std::fs::metadata(&real_request)
        .and_then(|m| m.modified())
        .ok();
    assert_eq!(
        request_before, request_after,
        "the EDCoPilot request file changed"
    );
    assert!(failures.is_empty(), "{} probes failed", failures.len());
    assert!(!parsed.probes.is_empty(), "the probe file has no app probe");
}

#[test]
fn test_reader_takes_the_probes_of_the_app_sections_only() {
    let text = "@section\tharbour\tnone\n\
                @vocab\tPort1\tBrightwater\n\
                ring the bell\tBellRing\n\
                @app\tbells\n\
                text\tring the bell\tBellRing\n\
                check\t-\t!problem\n\
                @section\tdocks\tnone\n\
                open the gate\tGateOpen\n";
    let parsed = parse_app_probes(text);
    assert!(parsed.failures.is_empty(), "{:?}", parsed.failures);
    assert_eq!(parsed.sections, 1);
    let lines: Vec<usize> = parsed.probes.iter().map(|p| p.line).collect();
    assert_eq!(lines, [5, 6]);
    assert_eq!(parsed.probes[0].args, ["text", "ring the bell"]);
    assert!(parsed.probes[0].want);
    assert!(!parsed.probes[1].want);
    assert_eq!(parsed.probes[1].expected, "problem");
}

#[test]
fn test_reader_reports_an_app_section_without_a_probe() {
    // An empty section before a core section, and at the end of the file.
    for text in [
        "@app\tempty\n@section\tdocks\tnone\nopen the gate\tGateOpen\n",
        "@app\tfull\ntext\tring the bell\tBellRing\n@app\tempty\n",
    ] {
        let parsed = parse_app_probes(text);
        assert_eq!(parsed.failures.len(), 1, "{text:?}: {:?}", parsed.failures);
        assert!(parsed.failures[0].contains("\"empty\" has no probe"));
    }
}

#[test]
fn test_reader_accepts_a_vocabulary_only_outside_an_app_section() {
    let core = parse_app_probes("@section\tdocks\tnone\n@vocab\tPort1\tBrightwater\n");
    assert!(core.failures.is_empty(), "{:?}", core.failures);
    let app = parse_app_probes("@app\tbells\n@vocab\tPort1\tBrightwater\ntext\tring\tBell\n");
    assert_eq!(app.failures.len(), 1, "{:?}", app.failures);
    assert!(app.failures[0].contains("a vocabulary in an app section"));
}

#[test]
fn test_reader_reports_a_bad_line() {
    for (text, reason) in [
        (
            "@app\tbells\n@sectoin\tdocks\tnone\ntext\tring\tBell\n",
            "unknown directive",
        ),
        (
            "@app\tbells\nspeak\tring the bell\tBell\n",
            "unknown command",
        ),
        ("@app\tbells\ntext\tring the bell\t!\n", "an empty field"),
        ("@app\tbells\ntext\t\tBell\n", "an empty field"),
        ("@app\tbells\ntext\tring --send\tBell\n", "may not send"),
    ] {
        let parsed = parse_app_probes(text);
        assert!(
            parsed.failures.iter().any(|f| f.contains(reason)),
            "{text:?}: {:?}",
            parsed.failures
        );
    }
}

/// Fix round 10: a probe reads stdout only, and a failure shows stderr
/// apart from stdout.
#[test]
fn test_a_probe_reads_stdout_only_and_shows_stderr_apart() {
    let parsed = parse_app_probes(
        "@app\tbells\n\
         text\tring the bell\tBellRing\n\
         text\tring the bell\t!lantern\n",
    );
    let (has, has_not) = (&parsed.probes[0], &parsed.probes[1]);
    let run = |stdout: &str, stderr: &str, code: i32| Run {
        stdout: stdout.to_string(),
        stderr: stderr.to_string(),
        code: Some(code),
    };
    // A warning on stderr with the text of a probe changes nothing.
    let warned = run(
        "match: BellRing\n",
        "warning: the lantern table has an odd line\n",
        0,
    );
    assert_eq!(evaluate(has, &warned), None);
    assert_eq!(evaluate(has_not, &warned), None);
    // The text only on stderr does not pass a probe that wants it.
    let only_stderr = run("no match\n", "warning: BellRing\n", 0);
    let reason = evaluate(has, &only_stderr).unwrap();
    assert_eq!(
        reason,
        r#"stdout: ["no match"], stderr: ["warning: BellRing"]"#
    );
    // The text on stdout fails a probe that must not have it.
    let lit = run("lantern lit\n", "", 0);
    assert!(evaluate(has_not, &lit)
        .unwrap()
        .starts_with("stdout: [\"lantern lit\"]"));
    // Another exit code fails each probe, and the message shows stderr.
    let failed = run("", "error: no profile\n", 1);
    assert_eq!(
        evaluate(has_not, &failed).unwrap(),
        r#"exit code Some(1), stdout: [], stderr: ["error: no profile"]"#
    );
}

#[test]
fn test_each_probe_command_must_succeed() {
    for command in ["text", "commands", "check"] {
        assert!(expected_success(command), "{command}");
    }
}

#[test]
fn test_the_temporary_configuration_writes_only_in_its_folder() {
    let dir = tempfile::tempdir().unwrap();
    let ed = dir.path().join("ed");
    let config = write_config(dir.path(), &ed);
    let text = std::fs::read_to_string(config).unwrap();
    let table: toml::Table = text.parse().unwrap();
    let request = table["edcopilot"]["request_file"].as_str().unwrap();
    assert!(Path::new(request).starts_with(dir.path()), "{request}");
    assert_eq!(table["diagnostics"]["log_file"].as_str(), Some(""));
    assert_eq!(table["diagnostics"]["save_audio_dir"].as_str(), Some(""));
}
