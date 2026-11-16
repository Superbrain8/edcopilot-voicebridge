//! The real plugin against a local HTTP server, with a test key pair.
//!
//! The files of `ui/fixtures/updater` are signed with a throwaway key made
//! only for these tests (`tauri signer generate`, then `tauri signer sign
//! --app-version 0.9.0`). The private key of the test pair is not in the
//! repository. It is never the key of the project. The install action of
//! these tests only records: no test runs a file.
//!
//! The mock program has the version 0.1.0 and the updater configuration of
//! `tauri.conf.json`. Only the address and (where a test says so) the key
//! change.

use super::*;
use tauri::test::{mock_builder, mock_context, noop_assets, MockRuntime};

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures/updater");

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!("{FIXTURES}/{name}")).unwrap()
}

fn fixture_text(name: &str) -> String {
    String::from_utf8(fixture(name)).unwrap().trim().to_string()
}

/// The public key of the test pair, in the form of `tauri.conf.json`.
fn test_pubkey() -> String {
    fixture_text("test-only.key.pub")
}

/// A mock program with the updater plugin and the plugin configuration of
/// `tauri.conf.json`.
fn mock_app() -> tauri::App<MockRuntime> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tauri.conf.json");
    let config: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let mut context = mock_context(noop_assets());
    context
        .config_mut()
        .plugins
        .0
        .insert("updater".into(), config["plugins"]["updater"].clone());
    mock_builder()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .build(context)
        .unwrap()
}

/// A local server: `/latest.json` gives the text of `manifest` (with the
/// port), `/fake-installer.bin` the signed file, `/tampered.bin` the same
/// file with one changed byte. Any other path gives 404.
struct Server {
    server: Arc<tiny_http::Server>,
    port: u16,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Server {
    fn start(manifest: impl Fn(u16) -> String + Send + 'static) -> Server {
        let server = Arc::new(tiny_http::Server::http("127.0.0.1:0").unwrap());
        let port = server.server_addr().to_ip().unwrap().port();
        let s = Arc::clone(&server);
        let thread = std::thread::spawn(move || {
            for request in s.incoming_requests() {
                let response = match request.url() {
                    "/latest.json" => tiny_http::Response::from_string(manifest(port)).boxed(),
                    "/fake-installer.bin" => {
                        tiny_http::Response::from_data(fixture("fake-installer.bin")).boxed()
                    }
                    "/tampered.bin" => {
                        let mut bytes = fixture("fake-installer.bin");
                        bytes[0] ^= 1;
                        tiny_http::Response::from_data(bytes).boxed()
                    }
                    _ => tiny_http::Response::empty(404).boxed(),
                };
                let _ = request.respond(response);
            }
        });
        Server {
            server,
            port,
            thread: Some(thread),
        }
    }

    fn endpoint(&self) -> url::Url {
        format!("http://127.0.0.1:{}/latest.json", self.port)
            .parse()
            .unwrap()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.server.unblock();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// A `latest.json` in the static format of the release workflow.
fn manifest(version: &str, port: u16, file: &str, signature: &str) -> String {
    serde_json::json!({
        "version": version,
        "notes": "- Updates inside the program.\n- A \"quoted\" line.",
        "pub_date": "2026-12-01T10:00:00Z",
        "platforms": {
            "windows-x86_64": {
                "signature": signature,
                "url": format!("http://127.0.0.1:{port}/{file}"),
            }
        }
    })
    .to_string()
}

/// The install action of the tests: it records the bytes, and runs nothing.
#[derive(Default)]
struct Recorder(Mutex<Vec<Vec<u8>>>);

impl Recorder {
    fn action(self: &Arc<Self>) -> InstallAction {
        let me = Arc::clone(self);
        Arc::new(move |_update, bytes| {
            lock(&me.0).push(bytes.to_vec());
            Ok(())
        })
    }

    fn count(&self) -> usize {
        lock(&self.0).len()
    }
}

struct Run {
    updates: Arc<Updates>,
    recorder: Arc<Recorder>,
    check: Result<UpdateStatusDto, String>,
    _server: Server,
    _app: tauri::App<MockRuntime>,
}

/// Checks against the local server with `manifest`, then (when the check
/// offers an update) downloads it and runs the recording install action.
/// `pubkey` `None` keeps the key of `tauri.conf.json`.
fn run(manifest: impl Fn(u16) -> String + Send + 'static, pubkey: Option<String>) -> Run {
    let server = Server::start(manifest);
    let app = mock_app();
    let recorder = Arc::new(Recorder::default());
    let source = PluginSource::new(
        &app,
        Some(TestServer {
            endpoint: server.endpoint(),
            pubkey,
        }),
        || {},
        recorder.action(),
    )
    .unwrap();
    let updates = Arc::new(Updates::new("0.1.0", false));
    let check = block_on(run_check(&updates, &source, true, &FakeEmit::default()));
    if updates.status().phase == "available" {
        let (id, pending) = updates.begin_download().unwrap();
        block_on(run_download(
            Arc::clone(&updates),
            id,
            pending,
            Arc::new(FakeEmit::default()),
            Duration::ZERO,
        ));
    }
    Run {
        updates,
        recorder,
        check,
        _server: server,
        _app: app,
    }
}

fn signed(version: &'static str, file: &'static str, sig: &'static str) -> impl Fn(u16) -> String {
    move |port| manifest(version, port, file, &fixture_text(sig))
}

#[test]
fn test_a_correct_signature_gets_ready_and_runs_only_the_test_action() {
    let run = run(
        signed("0.9.0", "fake-installer.bin", "fake-installer.bin.sig"),
        Some(test_pubkey()),
    );
    let check = run.check.unwrap();
    assert_eq!(check.version.as_deref(), Some("0.9.0"));
    assert_eq!(
        check.notes.as_deref(),
        Some("- Updates inside the program.\n- A \"quoted\" line.")
    );
    assert_eq!(check.date.as_deref(), Some("2026-12-01T10:00:00Z"));
    assert_eq!(run.updates.phase(), Phase::Ready);
    assert_eq!(*lock(&run.recorder.0), vec![fixture("fake-installer.bin")]);
}

#[test]
fn test_a_changed_file_fails_and_installs_nothing() {
    let run = run(
        signed("0.9.0", "tampered.bin", "fake-installer.bin.sig"),
        Some(test_pubkey()),
    );
    run.check.unwrap();
    assert_eq!(run.recorder.count(), 0);
    let status = run.updates.status();
    assert_eq!(status.phase, "available");
    assert_eq!(
        status.error.as_deref(),
        Some(install_failed_text(Cause::Signature).as_str())
    );
}

#[test]
fn test_the_signature_of_another_file_fails_and_installs_nothing() {
    let run = run(
        signed("0.9.0", "fake-installer.bin", "other-file.bin.sig"),
        Some(test_pubkey()),
    );
    run.check.unwrap();
    assert_eq!(run.recorder.count(), 0);
    assert_eq!(
        run.updates.status().error.as_deref(),
        Some(install_failed_text(Cause::Signature).as_str())
    );
}

/// `requireSignedVersion`: a `latest.json` that pairs a higher version
/// with the file of another version fails.
#[test]
fn test_a_file_signed_for_another_version_fails() {
    let run = run(
        signed("0.9.1", "fake-installer.bin", "fake-installer.bin.sig"),
        Some(test_pubkey()),
    );
    run.check.unwrap();
    assert_eq!(run.recorder.count(), 0);
    assert_eq!(
        run.updates.status().error.as_deref(),
        Some(install_failed_text(Cause::Signature).as_str())
    );
}

/// With the key of the project, a file of the test key fails.
#[test]
fn test_the_key_of_the_project_refuses_the_test_file() {
    let run = run(
        signed("0.9.0", "fake-installer.bin", "fake-installer.bin.sig"),
        None,
    );
    run.check.unwrap();
    assert_eq!(run.recorder.count(), 0);
    assert_eq!(
        run.updates.status().error.as_deref(),
        Some(install_failed_text(Cause::Signature).as_str())
    );
}

/// Only a newer version counts: the same and an older version are up to
/// date, also with a correct signature.
#[test]
fn test_the_same_or_an_older_version_is_up_to_date() {
    for version in ["0.1.0", "v0.1.0", "0.0.9", "0.1.0-beta.1"] {
        let run = run(
            move |port| {
                manifest(
                    version,
                    port,
                    "fake-installer.bin",
                    &fixture_text("fake-installer.bin.sig"),
                )
            },
            Some(test_pubkey()),
        );
        assert_eq!(run.check.unwrap().phase, "up_to_date", "{version}");
        assert_eq!(run.recorder.count(), 0);
    }
}

/// The text of `/latest.json` for a port.
type Answer = fn(u16) -> String;

#[test]
fn test_bad_answers_of_the_server_give_a_clear_cause() {
    let cases: [(Answer, Cause); 4] = [
        (|_| "not json".to_string(), Cause::BadData),
        (|_| r#"{"version": "0.9.0"}"#.to_string(), Cause::BadData),
        (
            |port| {
                serde_json::json!({"version": "0.9.0", "platforms": {"linux-x86_64": {
                    "signature": "x", "url": format!("http://127.0.0.1:{port}/a")}}})
                .to_string()
            },
            Cause::BadData,
        ),
        (
            |_| r#"{"version": "not a version"}"#.to_string(),
            Cause::BadData,
        ),
    ];
    for (answer, cause) in cases {
        let run = run(answer, Some(test_pubkey()));
        assert_eq!(run.check.unwrap_err(), check_failed_text(cause));
        assert_eq!(run.updates.status().phase, "check_failed");
    }
}

#[test]
fn test_a_missing_file_on_the_server_gives_a_clear_cause() {
    let server = Server::start(|_| String::new());
    let app = mock_app();
    let url: url::Url = format!("http://127.0.0.1:{}/absent.json", server.port)
        .parse()
        .unwrap();
    let source = PluginSource::new(
        &app,
        Some(TestServer {
            endpoint: url,
            pubkey: Some(test_pubkey()),
        }),
        || {},
        Arc::new(Recorder::default()).action(),
    )
    .unwrap();
    let updates = Updates::new("0.1.0", false);
    let error = block_on(run_check(&updates, &source, true, &FakeEmit::default())).unwrap_err();
    assert_eq!(error, check_failed_text(Cause::Server));
}

#[test]
fn test_no_server_gives_a_clear_cause() {
    // A port that nothing listens on.
    let port = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap().port()
    };
    let app = mock_app();
    let source = PluginSource::new(
        &app,
        Some(TestServer {
            endpoint: format!("http://127.0.0.1:{port}/latest.json")
                .parse()
                .unwrap(),
            pubkey: Some(test_pubkey()),
        }),
        || {},
        Arc::new(Recorder::default()).action(),
    )
    .unwrap();
    let updates = Updates::new("0.1.0", false);
    let error = block_on(run_check(&updates, &source, true, &FakeEmit::default())).unwrap_err();
    assert_eq!(error, check_failed_text(Cause::Connection));
}
