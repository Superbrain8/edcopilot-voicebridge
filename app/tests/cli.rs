//! Runs the built `voicebridge` binary. No test needs hardware.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use tempfile::TempDir;

/// An invented synonym table for the fixture commands, with 6 entries as
/// the shipped table.
const SYNONYMS: &str = "[synonyms]\n\
    \"make for\" = [\"sail to\", \"sail for\"]\n\
    \"make\" = [\"sail for\", \"sail to\"]\n\
    \"now we head to\" = [\"sail to\", \"sail for\"]\n\
    \"maker\" = [\"sail to\", \"sail for\"]\n\
    \"make two\" = [\"sail to\", \"sail for\"]\n\
    \"make too\" = [\"sail to\", \"sail for\"]\n";

/// A temp folder with the invented EDCoPilot folder of the fixture (the
/// program reads its profile), a model folder, an invented synonym table,
/// and a configuration. `LOCALAPPDATA` of each run is a temp folder too.
struct Setup {
    dir: TempDir,
    config: PathBuf,
}

impl Setup {
    fn new(threshold: f32) -> Setup {
        let dir = tempfile::tempdir().unwrap();
        let ed = dir.path().join("ed");
        edcopilot_voicebridge_core::profile::fixture::write_fixture_edcopilot(&ed);
        let model = dir.path().join("model");
        fs::create_dir_all(&model).unwrap();
        fs::create_dir_all(dir.path().join("aliases")).unwrap();
        fs::create_dir_all(dir.path().join("local")).unwrap();
        let synonyms = dir.path().join("synonyms.toml");
        fs::write(&synonyms, SYNONYMS).unwrap();

        let core = Path::new(env!("CARGO_MANIFEST_DIR")).join("../core");
        let text = format!(
            "[edcopilot]\ndir = '{}'\n\
             [data]\naliases_dir = '{}'\n\
             sound_alikes = '{}'\nsynonyms = '{}'\nfillers = '{}'\n\
             [matching]\nthreshold = {threshold:?}\n\
             [stt.parakeet]\nmodel_dir = '{}'\nthreads = 1\nhotwords_file = '{}'\n",
            ed.display(),
            dir.path().join("aliases").display(),
            core.join("data/sound_alikes.toml").display(),
            synonyms.display(),
            core.join("data/fillers.toml").display(),
            model.display(),
            core.join("data/hotwords.txt").display(),
        );
        let config = dir.path().join("voicebridge.toml");
        fs::write(&config, text).unwrap();
        Setup { dir, config }
    }

    fn request_file(&self) -> PathBuf {
        self.dir.path().join("ed").join("EDCoPilot.request.txt")
    }

    fn run(&self, args: &[&str]) -> Output {
        let local = self.dir.path().join("local");
        program(self.dir.path())
            .arg("--config")
            .arg(&self.config)
            .args(args)
            .env("LOCALAPPDATA", &local)
            .env("XDG_RUNTIME_DIR", &local)
            .output()
            .unwrap()
    }
}

/// The program with `APPDATA` and `LOCALAPPDATA` in new folders below
/// `base`. Thus no run reads or writes the real data folders of the user.
/// A later `.env` call can give `LOCALAPPDATA` another folder.
fn program(base: &Path) -> Command {
    let roaming = base.join("env-appdata");
    let local = base.join("env-localappdata");
    fs::create_dir_all(&roaming).unwrap();
    fs::create_dir_all(&local).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_voicebridge"));
    command.env("APPDATA", roaming).env("LOCALAPPDATA", local);
    command
}

/// The program with a configuration file, in a new temporary data folder
/// that `scratch` keeps.
fn voicebridge(scratch: &TempDir, config: &Path, args: &[&str]) -> Output {
    program(scratch.path())
        .arg("--config")
        .arg(config)
        .args(args)
        .output()
        .unwrap()
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).to_string()
}

#[test]
fn test_check_valid_configuration() {
    let setup = Setup::new(0.8);
    let out = setup.run(&["check"]);
    let stdout = text(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}{}", text(&out.stderr));
    // The fixture records without the number commands that the check of
    // the runtime grammar takes out. In profile mode they are left-out
    // commands of the report, not warnings.
    let loaded = edcopilot_voicebridge_core::profile::fixture::fixture_counts().loaded;
    assert!(
        stdout.contains(&format!("loaded {loaded} commands")),
        "{stdout}"
    );
    assert!(
        stdout.contains(&format!(
            "commands from the EDCoPilot profile: {loaded} commands"
        )),
        "{stdout}"
    );
    assert_eq!(
        stdout
            .matches(
                "
warning: the command \""
            )
            .count(),
        0,
        "{stdout}"
    );
    assert!(stdout.contains("13 sound-alike entries"), "{stdout}");
    assert!(stdout.contains("6 synonym entries"), "{stdout}");
    assert!(stdout.contains("7 filler words"), "{stdout}");
}

#[test]
fn test_check_shows_the_diagnostics_and_a_missing_log_directory() {
    let setup = Setup::new(0.8);
    let mut config = fs::read_to_string(&setup.config).unwrap();
    config.push_str("[diagnostics]\nlog_file = 'logs/u.tsv'\nsave_audio_dir = 'clips'\n");
    fs::write(&setup.config, config).unwrap();
    let out = setup.run(&["check"]);
    let stdout = text(&out.stdout);
    assert_eq!(out.status.code(), Some(1), "{stdout}");
    assert!(stdout.contains("utterance log: "), "{stdout}");
    assert!(stdout.contains("audio capture: "), "{stdout}");
    assert!(stdout.contains("diagnostics.log_file"), "{stdout}");
    fs::create_dir(setup.dir.path().join("logs")).unwrap();
    let out = setup.run(&["check"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stdout));
}

#[test]
fn test_replay_needs_a_wav_file() {
    let setup = Setup::new(0.8);
    let empty = setup.dir.path().join("empty");
    fs::create_dir(&empty).unwrap();
    let out = setup.run(&["replay", &empty.to_string_lossy()]);
    assert_eq!(out.status.code(), Some(1), "{}", text(&out.stderr));
    assert!(text(&out.stderr).contains("no WAV file"));
    assert!(!setup.request_file().exists());
}

#[test]
fn test_check_bad_threshold() {
    let setup = Setup::new(2.0);
    let out = setup.run(&["check"]);
    let all = text(&out.stdout) + &text(&out.stderr);
    assert_eq!(out.status.code(), Some(1), "{all}");
    assert!(all.contains("threshold"), "{all}");
}

#[test]
fn test_commands_lists_what_works_and_takes_a_filter() {
    let setup = Setup::new(0.8);
    let out = setup.run(&["commands", "tide", "log"]);
    let stdout = text(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}{}", text(&out.stderr));
    assert!(stdout.contains("->  Guide:Panel:TideLog"), "{stdout}");
    assert!(!stdout.contains("SailHomePort"), "{stdout}");
    assert!(!stdout.contains("does not work"), "{stdout}");

    let all = setup.run(&["commands", "--all"]);
    assert!(text(&all.stdout).contains("does not work"));
}

#[test]
fn test_commands_lists_the_invented_synonyms() {
    let setup = Setup::new(0.8);
    let out = setup.run(&["commands", "make"]);
    let stdout = text(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}{}", text(&out.stderr));
    assert!(
        stdout.contains("\"make for\"  =  \"sail to\" | \"sail for\"  (synonym)"),
        "{stdout}"
    );
    let out = setup.run(&["commands", "tide", "log"]);
    assert!(!text(&out.stdout).contains("(synonym)"));
}

#[test]
fn test_commands_lists_the_filler_words() {
    let setup = Setup::new(0.8);
    let out = setup.run(&["commands", "please"]);
    let stdout = text(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}{}", text(&out.stderr));
    assert!(stdout.contains("(filler)"), "{stdout}");
    assert!(stdout.contains("7 filler words."), "{stdout}");
    let out = setup.run(&["commands", "tide", "log"]);
    assert!(!text(&out.stdout).contains("(filler)"));
}

#[test]
fn test_text_through_the_filler_step_is_marked() {
    let setup = Setup::new(0.8);
    let out = setup.run(&["text", "uh help tide log please"]);
    let stdout = text(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}{}", text(&out.stderr));
    assert!(
        stdout.contains("Guide:Panel:TideLog (0.95) (filler)"),
        "{stdout}"
    );
    assert!(!setup.request_file().exists());
}

#[test]
fn test_text_dry_run_writes_nothing() {
    let setup = Setup::new(0.8);
    let out = setup.run(&["text", "help tide log"]);
    let stdout = text(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}{}", text(&out.stderr));
    assert!(stdout.contains("Guide:Panel:TideLog"), "{stdout}");
    assert!(!setup.request_file().exists());
}

#[test]
fn test_text_send_without_consumer_is_not_consumed() {
    // The request file is in the temporary EDCoPilot folder of the setup.
    let setup = Setup::new(0.8);
    let out = setup.run(&["text", "help tide log", "--send"]);
    let stdout = text(&out.stdout);
    assert!(stdout.contains("Guide:Panel:TideLog"), "{stdout}");
    assert!(stdout.contains("did not consume"), "{stdout}");
    assert!(!setup.request_file().exists());
}

#[test]
fn test_text_bad_threshold_stops() {
    let setup = Setup::new(2.0);
    for args in [
        &["text", "help tide log"][..],
        &["text", "help tide log", "--send"][..],
    ] {
        let out = setup.run(args);
        let stderr = text(&out.stderr);
        let all = text(&out.stdout) + &stderr;
        assert_eq!(out.status.code(), Some(1), "{all}");
        assert!(stderr.contains("problem: matching.threshold is 2"), "{all}");
        assert!(!all.contains("Guide:Panel"), "{all}");
        assert!(!setup.request_file().exists());
    }
}

#[test]
fn test_missing_configuration_file() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("absent.toml");
    let out = voicebridge(&dir, &missing, &["check"]);
    let stderr = text(&out.stderr);
    assert_eq!(out.status.code(), Some(1), "{stderr}");
    assert!(stderr.contains("absent.toml"), "{stderr}");
}

#[test]
fn test_a_configuration_error_never_shows_the_line_or_a_value() {
    const SECRET: &str = "sk-test-1234567890abcdef";
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("voicebridge.toml");
    let head = "[edcopilot]\ndir = 'C:/E'\n";
    for (body, expected) in [
        (
            format!("[stt.openai_compatible]\napi_key_env = {SECRET}\n"),
            "line 4, column 15: the TOML syntax is not correct",
        ),
        (
            format!("[stt.parakeet]\nthreads = \"{SECRET}\"\n"),
            "line 4, column 11: the value of stt.parakeet.threads is not valid, expected usize",
        ),
        (
            format!("[stt.openai_compatible]\napi_key = \"{SECRET}\"\n"),
            "line 4, column 1: unknown key `api_key` in [stt.openai_compatible]",
        ),
    ] {
        fs::write(&config, format!("{head}{body}")).unwrap();
        for args in [&["check"][..], &["text", "x"][..], &["commands"][..]] {
            let out = voicebridge(&dir, &config, args);
            let all = text(&out.stdout) + &text(&out.stderr);
            assert_eq!(out.status.code(), Some(1), "{all}");
            assert!(!all.contains(SECRET), "{all}");
            assert!(all.contains(expected), "{all}");
        }
    }
}

/// A configuration with an empty `edcopilot.dir`, in a folder that has the
/// invented INI file and vocabulary folder of the fixture where the empty
/// folder would point. The path `edcopilot.profile` names a copy of the
/// fixture profile in another folder, so the commands load.
fn empty_dir_setup() -> (TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    edcopilot_voicebridge_core::profile::fixture::write_fixture_edcopilot(dir.path());
    let profile_dir = dir.path().join("profile");
    fs::create_dir_all(&profile_dir).unwrap();
    let profile = profile_dir.join("EDCoPilot-Profile.vap");
    fs::rename(dir.path().join("EDCoPilot-Profile.vap"), &profile).unwrap();
    let core = Path::new(env!("CARGO_MANIFEST_DIR")).join("../core");
    let text = format!(
        "[edcopilot]\ndir = ''\nprofile = '{}'\n\
         [data]\naliases_dir = '{}'\n\
         sound_alikes = '{}'\nsynonyms = '{}'\nfillers = '{}'\n\
         [stt]\nengine = 'openai_compatible'\n",
        profile.display(),
        core.join("aliases").display(),
        core.join("data/sound_alikes.toml").display(),
        core.join("data/synonyms.toml").display(),
        core.join("data/fillers.toml").display(),
    );
    let config = dir.path().join("voicebridge.toml");
    fs::write(&config, text).unwrap();
    (dir, config)
}

#[test]
fn test_empty_edcopilot_dir_reads_nothing_from_the_current_folder() {
    let (dir, config) = empty_dir_setup();
    for args in [&["check"][..], &["text", "help tide log"][..]] {
        let out = program(dir.path())
            .current_dir(dir.path())
            .arg("--config")
            .arg(&config)
            .args(args)
            .env("LOCALAPPDATA", dir.path().join("local"))
            .output()
            .unwrap();
        let all = text(&out.stdout) + &text(&out.stderr);
        assert!(
            all.contains("edcopilot.dir is empty, set the EDCoPilot folder"),
            "{all}"
        );
        assert!(!dir.path().join("EDCoPilot.request.txt").exists());
        if args[0] == "check" {
            // The vocabulary folder of the current folder is not read.
            assert!(all.contains(" 0 vocabulary files with 0 names"), "{all}");
            assert!(!all.contains("not found"), "{all}");
        }
    }
}

/// A folder with the invented EDCoPilot folder of the fixture in `ed`, a
/// configuration for it, and a folder for the session lock
/// and the saved report (`local`, given as `LOCALAPPDATA`).
struct FixtureSetup {
    dir: TempDir,
    config: PathBuf,
}

impl FixtureSetup {
    fn new() -> FixtureSetup {
        let dir = tempfile::tempdir().unwrap();
        let ed = dir.path().join("ed");
        edcopilot_voicebridge_core::profile::fixture::write_fixture_edcopilot(&ed);
        fs::create_dir_all(dir.path().join("aliases")).unwrap();
        fs::create_dir_all(dir.path().join("local")).unwrap();
        let core = Path::new(env!("CARGO_MANIFEST_DIR")).join("../core");
        let text = format!(
            "[edcopilot]\ndir = '{}'\n\
             [data]\naliases_dir = '{}'\nsound_alikes = '{}'\nsynonyms = '{}'\nfillers = '{}'\n\
             [stt]\nengine = 'openai_compatible'\n\
             [stt.openai_compatible]\napi_key_env = 'VB_CLI_TEST_KEY'\n",
            ed.display(),
            dir.path().join("aliases").display(),
            core.join("data/sound_alikes.toml").display(),
            core.join("data/synonyms.toml").display(),
            core.join("data/fillers.toml").display(),
        );
        let config = dir.path().join("voicebridge.toml");
        fs::write(&config, text).unwrap();
        FixtureSetup { dir, config }
    }

    /// The report of the profile as `load_commands` gives it.
    fn loaded_report(&self) -> edcopilot_voicebridge_core::profile::BuildReport {
        let config = voicebridge::config::Config::load(&self.config).unwrap();
        voicebridge::config::load_commands(&config).unwrap().profile
    }

    /// The folder of the saved report: `<LOCALAPPDATA>\VoiceBridge`.
    fn report_dir(&self) -> PathBuf {
        self.dir.path().join("local").join("VoiceBridge")
    }

    fn run(&self, args: &[&str]) -> Output {
        let local = self.dir.path().join("local");
        program(self.dir.path())
            .arg("--config")
            .arg(&self.config)
            .args(args)
            .env("VB_CLI_TEST_KEY", "test-key")
            .env("LOCALAPPDATA", &local)
            .env("XDG_RUNTIME_DIR", &local)
            .output()
            .unwrap()
    }
}

#[test]
fn test_check_prints_the_report_lines_and_the_left_out_commands() {
    let setup = FixtureSetup::new();
    let out = setup.run(&["check"]);
    let stdout = text(&out.stdout);
    // The output of the invented profile, for `--nocapture`.
    println!("{stdout}");
    assert_eq!(out.status.code(), Some(0), "{stdout}{}", text(&out.stderr));
    // The report of the load: the build, and the number command that the
    // check of the runtime grammar takes out.
    let report = setup.loaded_report();
    let first = format!(
        "commands from the EDCoPilot profile: {} commands, {} left out",
        report.records,
        report.left_out.len()
    );
    assert!(stdout.contains(&first), "{stdout}");
    assert!(!stdout.contains("newer format"), "{stdout}");
    assert!(
        stdout.contains("left out, 1 commands: ")
            && stdout.contains(", for example \"polish the lens\""),
        "{stdout}"
    );
    assert!(!stdout.contains("left out: \""), "{stdout}");
    // The working phrase of the fixture that a value command also has.
    assert_eq!(
        stdout
            .matches(
                "warning: the phrase \"set lamp colour\" of the command \
                 \"[paint the lamp; set lamp colour]\" is also a phrase of a command that \
                 the program cannot send, the program does not send a command for it"
            )
            .count(),
        1,
        "{stdout}"
    );
    assert!(
        !stdout.contains("the EDCoPilot profile changed"),
        "{stdout}"
    );
    assert!(
        stdout.ends_with("configuration is valid\n")
            || stdout.ends_with("configuration is valid\r\n")
    );

    let out = setup.run(&["check", "--left-out"]);
    let stdout = text(&out.stdout);
    let every = stdout
        .lines()
        .filter(|l| l.starts_with("left out: \""))
        .count();
    assert_eq!(every, report.left_out.len(), "{stdout}");
    assert!(
        stdout.contains("left out: \"ring all bells\" (multi_write): "),
        "{stdout}"
    );
    assert!(!stdout.contains("left out, "), "{stdout}");
    // `check` never writes the report.
    assert!(!setup.report_dir().exists());
}

#[test]
fn test_check_prints_the_difference_after_a_change_of_the_profile() {
    use edcopilot_voicebridge_core::profile::fixture;
    let setup = FixtureSetup::new();
    // The report of the last session start: the fixture as it is.
    fs::create_dir_all(setup.report_dir()).unwrap();
    let old = setup.loaded_report();
    let profile = setup.dir.path().join("ed").join("EDCoPilot-Profile.vap");
    voicebridge::commands_source::save_report(&setup.report_dir(), &old, &profile).unwrap();
    let out = setup.run(&["check"]);
    assert!(
        !text(&out.stdout).contains("the EDCoPilot profile changed"),
        "{}",
        text(&out.stdout)
    );
    // EDCoPilot writes a profile with one more command.
    let mut b = fixture::fixture_builder();
    b.category("Logbook")
        .fixed("open the invented extra room", "OpenRoom:Extra");
    fs::write(setup.dir.path().join("ed/EDCoPilot-Profile.vap"), b.build()).unwrap();
    let saved = fs::read(setup.report_dir().join("profile-report.json")).unwrap();
    let out = setup.run(&["check"]);
    let stdout = text(&out.stdout);
    assert!(
        stdout.contains(
            "the EDCoPilot profile changed: 1 commands added, 0 removed, 0 now left out, 0 now in the map"
        ),
        "{stdout}"
    );
    // `check` does not replace the saved report.
    assert_eq!(
        fs::read(setup.report_dir().join("profile-report.json")).unwrap(),
        saved
    );
    // A saved report of another profile file (another configuration) gives
    // no line.
    let other = setup.dir.path().join("other/EDCoPilot-Profile.vap");
    voicebridge::commands_source::save_report(&setup.report_dir(), &old, &other).unwrap();
    let out = setup.run(&["check"]);
    assert!(
        !text(&out.stdout).contains("the EDCoPilot profile changed"),
        "{}",
        text(&out.stdout)
    );
}

#[test]
fn test_text_prints_the_load_warnings_of_the_commands() {
    let setup = FixtureSetup::new();
    let out = setup.run(&["text", "polish the invented banana"]);
    let stderr = text(&out.stderr);
    assert_eq!(out.status.code(), Some(0), "{stderr}");
    assert!(
        stderr.contains("warning: the phrase \"set lamp colour\" of the command \""),
        "{stderr}"
    );
    assert!(
        text(&out.stdout).starts_with("no match"),
        "{}",
        text(&out.stdout)
    );
}

#[test]
fn test_commands_and_text_never_write_the_profile_report() {
    let setup = FixtureSetup::new();
    for args in [
        &["commands"][..],
        &["commands", "--all"][..],
        &["text", "ring bell"][..],
    ] {
        let out = setup.run(args);
        assert_eq!(
            out.status.code(),
            Some(0),
            "{args:?}: {}",
            text(&out.stderr)
        );
    }
    let out = setup.run(&["text", "ring bell"]);
    assert!(
        text(&out.stdout).contains("Bell:Ring"),
        "{}",
        text(&out.stdout)
    );
    assert!(!setup.report_dir().exists());
    assert!(!setup.dir.path().join("ed/EDCoPilot.request.txt").exists());
}

#[test]
fn test_empty_edcopilot_dir_reads_no_profile_from_the_current_folder() {
    let setup = FixtureSetup::new();
    // The current folder has a profile where an empty folder would point.
    edcopilot_voicebridge_core::profile::fixture::write_fixture_edcopilot(setup.dir.path());
    let text_of = fs::read_to_string(&setup.config).unwrap();
    let first = text_of.lines().nth(1).unwrap().to_string();
    fs::write(&setup.config, text_of.replace(&first, "dir = ''")).unwrap();
    for args in [
        &["check"][..],
        &["text", "ring bell"][..],
        &["commands"][..],
    ] {
        let out = program(setup.dir.path())
            .current_dir(setup.dir.path())
            .arg("--config")
            .arg(&setup.config)
            .args(args)
            .env("VB_CLI_TEST_KEY", "test-key")
            .env("LOCALAPPDATA", setup.dir.path().join("local"))
            .output()
            .unwrap();
        let all = text(&out.stdout) + &text(&out.stderr);
        assert_eq!(out.status.code(), Some(1), "{args:?}: {all}");
        assert!(
            all.contains("edcopilot.dir is empty, set the EDCoPilot folder"),
            "{all}"
        );
        if args[0] == "check" {
            // One problem, not one of the folder and one of the profile.
            assert_eq!(all.matches("edcopilot.dir is empty").count(), 1, "{all}");
        }
        assert!(
            !all.contains("commands from the EDCoPilot profile"),
            "{all}"
        );
        assert!(!all.contains("Bell:Ring"), "{all}");
    }
}

#[test]
fn test_text_send_without_the_edcopilot_folder_stops_and_writes_nothing() {
    // A temporary folder and a scratch configuration: no EDCoPilot folder.
    let (dir, config) = empty_dir_setup();
    let before: Vec<_> = fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    let scratch = tempfile::tempdir().unwrap();
    let out = program(scratch.path())
        .current_dir(dir.path())
        .arg("--config")
        .arg(&config)
        .args(["text", "help tide log", "--send"])
        .output()
        .unwrap();
    let stderr = text(&out.stderr);
    assert_eq!(out.status.code(), Some(1), "{stderr}");
    assert!(
        stderr.contains("problem: edcopilot.dir is empty, set the EDCoPilot folder"),
        "{stderr}"
    );
    let after: Vec<_> = fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(before, after);
}

/// The band of the number of commands after the load of a sane current
/// profile. The profile of 1.11.755 gives 2,485. A load that keeps
/// commands that it must leave out gives more.
const LOADED: std::ops::RangeInclusive<usize> = 2_000..=2_700;

/// The band of the number of commands that the load leaves out as not
/// understood. The profile of 1.11.755 gives 503.
const NOT_UNDERSTOOD: std::ops::RangeInclusive<usize> = 300..=700;

/// The lowest count of each reason group of `not_understood` with a
/// recorded count of 5 or more: half of the recorded count, rounded up.
/// The profile of 1.11.755 gives 482, 8 and 7.
const REASON_MINIMUMS: [(&str, usize); 3] = [
    (
        "the command sets a part of its token in a way that the program does not model",
        241,
    ),
    (
        "action type 16 (execute a command) after the write: the program cannot find the \
         command that it runs",
        4,
    ),
    (
        "an action before the write is switched off in the profile, the program cannot know \
         what VoiceAttack does",
        4,
    ),
];

/// The live check of the load and of the console program on the real
/// EDCoPilot folder of `VOICEBRIDGE_EDCOPILOT_DIR`. It reads the folder and
/// writes nothing there. The configuration and the report folder are
/// temporary. Run it with:
///
/// ```text
/// VOICEBRIDGE_EDCOPILOT_DIR=C:\EDCoPilot cargo test -p voicebridge \
///     --test cli -- --ignored --nocapture
/// ```
#[test]
#[ignore = "needs the EDCoPilot folder in VOICEBRIDGE_EDCOPILOT_DIR"]
fn test_real_profile_loads_with_counts_in_the_recorded_band() {
    use edcopilot_voicebridge_core::profile::{build_map, LeftOutClass};
    use edcopilot_voicebridge_core::{load_dynamic_vocab_checked, slot_keys};

    let ed = PathBuf::from(
        std::env::var_os("VOICEBRIDGE_EDCOPILOT_DIR").expect("VOICEBRIDGE_EDCOPILOT_DIR"),
    );
    let core = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../core")
        .canonicalize()
        .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let local = dir.path().join("local");
    fs::create_dir_all(&local).unwrap();
    let config = dir.path().join("live.toml");
    // The request file of the real folder stays as it is.
    let request = dir.path().join("EDCoPilot.request.txt");
    fs::write(
        &config,
        format!(
            "[edcopilot]\ndir = '{}'\nrequest_file = '{}'\n\
             [data]\naliases_dir = '{}'\nsound_alikes = '{}'\nsynonyms = '{}'\nfillers = '{}'\n\
             [stt]\nengine = 'openai_compatible'\n\
             [stt.openai_compatible]\napi_key_env = 'VB_CLI_TEST_KEY'\n",
            ed.display(),
            request.display(),
            core.join("aliases").display(),
            core.join("data/sound_alikes.toml").display(),
            core.join("data/synonyms.toml").display(),
            core.join("data/fillers.toml").display(),
        ),
    )
    .unwrap();
    let config_value = voicebridge::config::Config::load(&config).unwrap();
    assert_eq!(config_value.edcopilot.request_file, request);
    let run = |args: &[&str]| {
        let out = program(dir.path())
            .arg("--config")
            .arg(&config)
            .args(args)
            .env("VB_CLI_TEST_KEY", "test-key")
            .env("LOCALAPPDATA", &local)
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
        text(&out.stdout)
    };

    // The load: the build, and the number commands that the check of the
    // runtime grammar takes out.
    let loaded = voicebridge::config::load_commands(&config_value).unwrap();
    let report = &loaded.profile;
    let not_understood = report.left_out_count(LeftOutClass::NotUnderstood);
    println!(
        "profile (VoiceAttack {}): {} commands after the load, {} left out, {} not understood, \
         {} blocking phrases ({} skipped)",
        report.source.voiceattack_version,
        report.records,
        report.left_out.len(),
        not_understood,
        loaded.blocking.len(),
        loaded.blocking_skipped
    );
    // Each blocking phrase is the phrase of a left-out command.
    assert!(loaded.blocking.len() <= report.left_out.len());
    let reasons = report.class_reasons(LeftOutClass::NotUnderstood);
    let listing: String = reasons
        .iter()
        .map(|(reason, n, _)| format!("  {n:4}  {reason}\n"))
        .collect();
    print!("{listing}");
    assert_eq!(loaded.commands.len(), report.records);
    assert!(
        LOADED.contains(&report.records),
        "{} commands, outside {LOADED:?}. Reasons of not_understood:\n{listing}",
        report.records
    );
    assert!(
        NOT_UNDERSTOOD.contains(&not_understood),
        "{not_understood} commands not understood, outside {NOT_UNDERSTOOD:?}. Reasons:\n{listing}"
    );
    for (reason, minimum) in REASON_MINIMUMS {
        let n = reasons
            .iter()
            .find(|(r, _, _)| r == reason)
            .map_or(0, |(_, n, _)| *n);
        assert!(
            n >= minimum,
            "{n} commands with the reason \"{reason}\", fewer than {minimum}. Reasons:\n{listing}"
        );
    }

    // The slot files of the real folder have the known form.
    let vap = fs::read(ed.join("EDCoPilot-Profile.vap")).unwrap();
    let commands = build_map(&vap).unwrap().commands;
    let used = slot_keys(commands.iter().filter_map(|c| c.phrase.as_deref()));
    let (_, unknown) = load_dynamic_vocab_checked(&ed.join("Dynamic var files"), &used);
    println!(
        "slot files: {} slots used by records, {} files with another form",
        used.len(),
        unknown.len()
    );
    assert!(unknown.is_empty(), "{} files", unknown.len());

    // `check` shows the report line and writes no report. `commands --all`
    // has one row for each command.
    let check = run(&["check"]);
    assert!(
        check.contains(&format!(
            "commands from the EDCoPilot profile: {} commands, {} left out",
            report.records,
            report.left_out.len()
        )),
        "the check has no report line"
    );
    assert!(check.contains("configuration is valid"));
    assert!(!local
        .join("VoiceBridge")
        .join("profile-report.json")
        .exists());
    let all = run(&["commands", "--all"]);
    let rows = all.lines().filter(|l| l.contains("  ->  ")).count();
    println!("commands --all: {} lines, {rows} rows", all.lines().count());
    assert!(rows >= *LOADED.start(), "{rows} rows");
}
