//! `voicebridge bind`: set the talk button in the configuration file.

use std::fmt;
use std::path::Path;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use crate::config::Config;
use crate::config_edit::{self, Value as EditValue};
use anyhow::{bail, Context};
use edcopilot_voicebridge_audio::{InputScanner, PressedInput};

/// The talk button that `bind` writes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Binding {
    /// A key specification for `push_to_talk.key`.
    Key(String),
    /// A value for `push_to_talk.joystick`.
    Joystick { device: String, button: u32 },
}

impl From<PressedInput> for Binding {
    fn from(input: PressedInput) -> Binding {
        match input {
            PressedInput::Key(spec) => Binding::Key(spec),
            PressedInput::Joystick { device, button } => Binding::Joystick { device, button },
        }
    }
}

impl fmt::Display for Binding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Binding::Key(spec) => write!(f, "key \"{spec}\""),
            Binding::Joystick { device, button } => {
                write!(f, "joystick \"{device}\" button {button}")
            }
        }
    }
}

impl Binding {
    /// The edits of the configuration that set this binding. A key removes
    /// `joystick`, a joystick removes `key`.
    pub fn edits(&self) -> Vec<(String, EditValue)> {
        let edit = |key: &str, value: EditValue| (key.to_string(), value);
        match self {
            Binding::Key(spec) => vec![
                edit("push_to_talk.joystick", EditValue::Absent),
                edit("push_to_talk.key", EditValue::Text(spec.clone())),
            ],
            Binding::Joystick { device, button } => vec![
                edit("push_to_talk.key", EditValue::Absent),
                edit(
                    "push_to_talk.joystick.device",
                    EditValue::Text(device.clone()),
                ),
                edit(
                    "push_to_talk.joystick.button",
                    EditValue::Integer(i64::from(*button)),
                ),
            ],
        }
    }
}

/// Sets the binding in the TOML text. Comments and the layout of the rest of
/// the text stay the same. A key removes `joystick`, a joystick removes `key`.
pub fn apply_binding(toml_text: &str, binding: &Binding) -> anyhow::Result<String> {
    config_edit::apply_edits(toml_text, &binding.edits())
}

/// `apply_binding`, then a check that the new text loads as a configuration
/// and that its push-to-talk part is valid. `path` is the file of the text.
fn checked_binding_text(text: &str, binding: &Binding, path: &Path) -> anyhow::Result<String> {
    let name = path.display().to_string();
    let new_text = apply_binding(text, binding)
        .with_context(|| format!("cannot change configuration file {name}"))?;
    let config = Config::parse_file(&new_text, path)
        .with_context(|| format!("the changed configuration file {name} would not load"))?;
    let problems = config.push_to_talk_problems();
    if !problems.is_empty() {
        bail!(
            "the changed configuration file {name} would not be valid: {}",
            problems.join(", ")
        );
    }
    Ok(new_text)
}

/// Writes the binding into the file through a temporary file in the same
/// directory, so that a failure cannot leave a half-written file. The new
/// text must load as a configuration, else the old file stays.
pub fn write_binding(path: &Path, binding: &Binding) -> anyhow::Result<()> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("cannot read configuration file {}", path.display()))?;
    let new_text = checked_binding_text(&text, binding, path)?;
    config_edit::write_atomic(path, &new_text)
}

/// What `bind` does after a scan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// No press is complete yet.
    Pending,
    /// Escape was in the press.
    Cancel,
    Bind(PressedInput),
    /// More than one input was in the press. `bind` does not guess which one.
    Several(Vec<PressedInput>),
}

/// Decides on the inputs of one complete press (all of them are up again).
pub fn decide(pressed: &[PressedInput]) -> Decision {
    let has = |spec: &str| {
        pressed
            .iter()
            .any(|p| *p == PressedInput::Key(spec.to_string()))
    };
    if has(ESCAPE) {
        return Decision::Cancel;
    }
    // Windows reports AltGr as LControl and RAlt together.
    if pressed.len() == 2 && has("LControl") && has("RAlt") {
        return Decision::Bind(PressedInput::Key("RAlt".to_string()));
    }
    match pressed {
        [one] => Decision::Bind(one.clone()),
        [] => Decision::Pending,
        several => Decision::Several(several.to_vec()),
    }
}

/// The key that cancels `bind`.
const ESCAPE: &str = "Escape";

/// Collects the inputs that go down after the start and decides when all of
/// them are up again. Inputs that are down at the start (the baseline) do
/// not count until they are released.
#[derive(Debug, Default)]
pub struct ReleaseTracker {
    baseline: Vec<PressedInput>,
    pressed: Vec<PressedInput>,
    down: Vec<PressedInput>,
}

impl ReleaseTracker {
    pub fn new(baseline: Vec<PressedInput>) -> ReleaseTracker {
        ReleaseTracker {
            down: baseline.clone(),
            baseline,
            pressed: Vec::new(),
        }
    }

    /// Takes one scan of the inputs that are down now.
    pub fn update(&mut self, now: &[PressedInput]) -> Decision {
        self.down = now.to_vec();
        // A baseline input counts again after its release.
        self.baseline.retain(|b| now.contains(b));
        for input in now {
            if !self.baseline.contains(input) && !self.pressed.contains(input) {
                self.pressed.push(input.clone());
            }
        }
        if self.pressed.is_empty() || self.pressed.iter().any(|p| now.contains(p)) {
            return Decision::Pending;
        }
        decide(&std::mem::take(&mut self.pressed))
    }

    /// The collected inputs that are still down after the last `update`.
    pub fn waiting_for(&self) -> Vec<PressedInput> {
        self.pressed
            .iter()
            .filter(|p| self.down.contains(p))
            .cloned()
            .collect()
    }
}

/// Poll interval of `bind`.
const BIND_POLL: Duration = Duration::from_millis(20);
/// After this time with a press still down, `bind` says what it waits for.
const RELEASE_HINT: Duration = Duration::from_secs(1);

fn join_inputs(inputs: &[PressedInput]) -> String {
    let names: Vec<String> = inputs.iter().map(ToString::to_string).collect();
    names.join(", ")
}

/// Waits for one complete press. `None`: Escape cancelled.
fn wait_for_press() -> Option<PressedInput> {
    let mut scanner = InputScanner::new();
    // Inputs that are down now, for example the Enter key that started the
    // command, do not count.
    let mut tracker = ReleaseTracker::new(scanner.scan());
    let mut since: Option<Instant> = None;
    let mut hinted = false;
    loop {
        std::thread::sleep(BIND_POLL);
        match tracker.update(&scanner.scan()) {
            Decision::Pending => {}
            Decision::Cancel => return None,
            Decision::Bind(input) => return Some(input),
            Decision::Several(inputs) => println!(
                "several inputs were pressed ({}), press only one",
                join_inputs(&inputs)
            ),
        }
        let waiting = tracker.waiting_for();
        if waiting.is_empty() {
            since = None;
            hinted = false;
            continue;
        }
        let start = *since.get_or_insert_with(Instant::now);
        if !hinted && start.elapsed() >= RELEASE_HINT {
            println!("waiting for release of: {}", join_inputs(&waiting));
            hinted = true;
        }
    }
}

/// Removes the pending key presses from the console input, so that the
/// bound key does not go to the shell.
#[cfg(windows)]
fn flush_console_input() {
    use windows_sys::Win32::System::Console::{
        FlushConsoleInputBuffer, GetStdHandle, STD_INPUT_HANDLE,
    };
    // SAFETY: GetStdHandle has no pointer inputs. FlushConsoleInputBuffer
    // gets the handle that GetStdHandle gave, it fails safely for a handle
    // that is not a console.
    unsafe {
        let handle = GetStdHandle(STD_INPUT_HANDLE);
        if !handle.is_null() && handle != windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE {
            FlushConsoleInputBuffer(handle);
        }
    }
}

#[cfg(not(windows))]
fn flush_console_input() {}

/// `voicebridge bind`: waits for one key or joystick button and writes it to
/// the configuration file. Escape cancels with exit code 1.
pub fn bind(config_path: &Path) -> anyhow::Result<ExitCode> {
    // Find a bad file before the user presses a key.
    let text = std::fs::read_to_string(config_path)
        .with_context(|| format!("cannot read configuration file {}", config_path.display()))?;
    checked_binding_text(&text, &Binding::Key("F13".to_string()), config_path)?;

    println!("press the key or joystick button for push-to-talk (Esc cancels)");
    let input = wait_for_press();
    flush_console_input();
    let Some(input) = input else {
        println!("cancelled, the configuration file is not changed");
        return Ok(ExitCode::FAILURE);
    };
    let binding = Binding::from(input);
    write_binding(config_path, &binding)?;
    println!(
        "push-to-talk is now {binding} (written to {})",
        config_path.display()
    );
    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k(spec: &str) -> PressedInput {
        PressedInput::Key(spec.to_string())
    }

    fn moza(button: u32) -> PressedInput {
        PressedInput::Joystick {
            device: "MOZA".to_string(),
            button,
        }
    }

    #[test]
    fn test_decide() {
        assert_eq!(decide(&[k("<")]), Decision::Bind(k("<")));
        assert_eq!(decide(&[moza(5)]), Decision::Bind(moza(5)));
        assert_eq!(decide(&[k("Escape")]), Decision::Cancel);
        assert_eq!(decide(&[k("A"), k("Escape")]), Decision::Cancel);
        // AltGr gives LControl and RAlt.
        assert_eq!(
            decide(&[k("LControl"), k("RAlt")]),
            Decision::Bind(k("RAlt"))
        );
        assert_eq!(
            decide(&[k("RAlt"), k("LControl")]),
            Decision::Bind(k("RAlt"))
        );
        // Ctrl+C is not a talk key.
        assert_eq!(
            decide(&[k("LControl"), k("C")]),
            Decision::Several(vec![k("LControl"), k("C")])
        );
        assert_eq!(
            decide(&[k("F13"), moza(5)]),
            Decision::Several(vec![k("F13"), moza(5)])
        );
    }

    #[test]
    fn test_tracker_decides_on_release() {
        let mut t = ReleaseTracker::new(Vec::new());
        assert_eq!(t.update(&[]), Decision::Pending);
        assert_eq!(t.update(&[k("<")]), Decision::Pending);
        assert_eq!(t.waiting_for(), vec![k("<")]);
        assert_eq!(t.update(&[k("<")]), Decision::Pending);
        assert_eq!(t.update(&[]), Decision::Bind(k("<")));
    }

    #[test]
    fn test_tracker_ignores_the_baseline() {
        // Enter is still down from the command line.
        let mut t = ReleaseTracker::new(vec![k("Enter")]);
        assert_eq!(t.update(&[k("Enter")]), Decision::Pending);
        assert_eq!(t.update(&[k("Enter"), moza(3)]), Decision::Pending);
        assert_eq!(t.update(&[k("Enter")]), Decision::Bind(moza(3)));
        // After its release, the baseline key counts as a new press.
        let mut t = ReleaseTracker::new(vec![k("Enter")]);
        assert_eq!(t.update(&[]), Decision::Pending);
        assert_eq!(t.update(&[k("Enter")]), Decision::Pending);
        assert_eq!(t.update(&[]), Decision::Bind(k("Enter")));
    }

    #[test]
    fn test_tracker_collects_until_all_are_up() {
        let mut t = ReleaseTracker::new(Vec::new());
        assert_eq!(t.update(&[k("LControl")]), Decision::Pending);
        assert_eq!(t.update(&[k("LControl"), k("C")]), Decision::Pending);
        // C is up, LControl is still down.
        assert_eq!(t.update(&[k("LControl")]), Decision::Pending);
        assert_eq!(t.waiting_for(), vec![k("LControl")]);
        assert_eq!(
            t.update(&[]),
            Decision::Several(vec![k("LControl"), k("C")])
        );
        // A new try starts empty.
        assert_eq!(t.update(&[k("Escape")]), Decision::Pending);
        assert_eq!(t.update(&[]), Decision::Cancel);
    }

    const WITH_KEY: &str = "\
# My configuration.
[edcopilot]
dir = 'ed' # the EDCoPilot folder

[push_to_talk]
# A keyboard key.
key = \"F13\"

# Comment before the next table.
[stt]
engine = \"parakeet\"   # local
";

    const WITH_JOYSTICK: &str = "\
[edcopilot]
dir = 'ed'

[push_to_talk]
joystick = { device = \"MOZA\", button = 5 }

[stt]
engine = \"parakeet\"
";

    fn key(spec: &str) -> Binding {
        Binding::Key(spec.to_string())
    }

    fn joystick(device: &str, button: u32) -> Binding {
        Binding::Joystick {
            device: device.to_string(),
            button,
        }
    }

    fn load(text: &str) -> Config {
        Config::parse(text, Path::new("."), "t.toml").unwrap()
    }

    fn push_to_talk_problems(config: &Config) -> Vec<String> {
        config.push_to_talk_problems()
    }

    #[test]
    fn test_key_replaces_key() {
        // A character is written as it is. Its check needs the host layout,
        // so the check uses a name below.
        let out = apply_binding(WITH_KEY, &key("<")).unwrap();
        assert_eq!(load(&out).push_to_talk.key.as_deref(), Some("<"));
        assert_eq!(out, WITH_KEY.replace("key = \"F13\"", "key = \"<\""));

        let out = apply_binding(WITH_KEY, &key("RControl")).unwrap();
        let config = load(&out);
        assert_eq!(config.push_to_talk.key.as_deref(), Some("RControl"));
        assert_eq!(config.push_to_talk.joystick, None);
        assert_eq!(push_to_talk_problems(&config), Vec::<String>::new());
    }

    #[test]
    fn test_key_replaces_joystick() {
        let out = apply_binding(WITH_JOYSTICK, &key("vk:226")).unwrap();
        assert!(!out.contains("joystick"), "{out}");
        let config = load(&out);
        assert_eq!(config.push_to_talk.key.as_deref(), Some("vk:226"));
        assert_eq!(config.push_to_talk.joystick, None);
        #[cfg(windows)]
        assert_eq!(push_to_talk_problems(&config), Vec::<String>::new());
    }

    #[test]
    fn test_joystick_replaces_key() {
        let out = apply_binding(WITH_KEY, &joystick("T.16000M", 3)).unwrap();
        assert!(!out.contains("key = "), "{out}");
        let config = load(&out);
        assert_eq!(config.push_to_talk.key, None);
        let j = config.push_to_talk.joystick.clone().unwrap();
        assert_eq!((j.device.as_str(), j.button), ("T.16000M", 3));
        assert_eq!(push_to_talk_problems(&config), Vec::<String>::new());
        assert!(
            out.contains("joystick = { device = \"T.16000M\", button = 3 }"),
            "{out}"
        );
    }

    #[test]
    fn test_example_file_takes_each_binding() {
        let example = include_str!("../voicebridge.example.toml");
        for binding in [key("RControl"), joystick("MOZA", 5)] {
            let out = apply_binding(example, &binding).unwrap();
            let config = load(&out);
            assert_eq!(push_to_talk_problems(&config), Vec::<String>::new());
            assert!(out.contains("[always_listening]"), "{out}");
            assert!(out.contains("# The EDCoPilot folder. Required."), "{out}");
        }
    }

    #[test]
    fn test_comments_elsewhere_survive() {
        for binding in [key("Mouse4"), joystick("MOZA", 7)] {
            let out = apply_binding(WITH_KEY, &binding).unwrap();
            for line in [
                "# My configuration.",
                "dir = 'ed' # the EDCoPilot folder",
                "# Comment before the next table.",
                "engine = \"parakeet\"   # local",
            ] {
                assert!(out.contains(line), "{binding}: {line} lost in\n{out}");
            }
        }
    }

    #[test]
    fn test_bind_keeps_pre_roll_and_tail() {
        let text = "\
[edcopilot]
dir = 'ed'

[push_to_talk]
key = \"F13\"
# Audio before the press.
pre_roll_ms = 100
release_tail_ms = 0 # off
";
        for binding in [key("RControl"), joystick("MOZA", 5)] {
            let out = apply_binding(text, &binding).unwrap();
            assert!(
                out.contains("# Audio before the press.\npre_roll_ms = 100"),
                "{out}"
            );
            assert!(out.contains("release_tail_ms = 0 # off"), "{out}");
            let config = load(&out);
            assert_eq!(config.push_to_talk.pre_roll_ms, 100);
            assert_eq!(config.push_to_talk.release_tail_ms, 0);
            assert_eq!(push_to_talk_problems(&config), Vec::<String>::new());
        }
        // The example file keeps its values too.
        let example = include_str!("../voicebridge.example.toml");
        let out = apply_binding(example, &joystick("MOZA", 5)).unwrap();
        assert!(out.contains("pre_roll_ms = 250"), "{out}");
        assert!(out.contains("release_tail_ms = 300"), "{out}");
    }

    #[test]
    fn test_missing_table_is_added() {
        let text = "[edcopilot]\ndir = 'ed' # folder\n";
        let out = apply_binding(text, &key("RControl")).unwrap();
        assert!(out.starts_with(text), "{out}");
        assert!(out.contains("[push_to_talk]"), "{out}");
        let config = load(&out);
        assert_eq!(config.push_to_talk.key.as_deref(), Some("RControl"));
        assert_eq!(push_to_talk_problems(&config), Vec::<String>::new());
    }

    #[test]
    fn test_special_characters_are_escaped() {
        let out = apply_binding(WITH_KEY, &joystick("Pad \"X\" \\ 2", 1)).unwrap();
        let config = load(&out);
        let j = config.push_to_talk.joystick.unwrap();
        assert_eq!(j.device, "Pad \"X\" \\ 2");
    }

    #[test]
    fn test_invalid_toml_is_an_error() {
        assert!(apply_binding("[edcopilot\n", &key("F13")).is_err());
        assert!(apply_binding("push_to_talk = 5\n", &key("F13")).is_err());
    }

    #[test]
    fn test_write_binding_replaces_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("voicebridge.toml");
        std::fs::write(&file, WITH_KEY).unwrap();
        write_binding(&file, &key("RControl")).unwrap();
        let text = std::fs::read_to_string(&file).unwrap();
        assert_eq!(load(&text).push_to_talk.key.as_deref(), Some("RControl"));
        let names: Vec<_> = std::fs::read_dir(dir.path()).unwrap().collect();
        assert_eq!(names.len(), 1, "a temporary file was left");
    }

    #[test]
    fn test_write_binding_keeps_the_file_on_error() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("voicebridge.toml");
        std::fs::write(&file, "push_to_talk = 5\n").unwrap();
        assert!(write_binding(&file, &key("RControl")).is_err());
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            "push_to_talk = 5\n"
        );
        let missing = dir.path().join("missing.toml");
        let err = write_binding(&missing, &key("RControl")).unwrap_err();
        assert!(format!("{err:#}").contains("missing.toml"), "{err:#}");
    }

    #[test]
    fn test_write_binding_refuses_a_result_that_does_not_load() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("voicebridge.toml");
        // Valid TOML, but not a valid configuration.
        for text in [
            "[edcopilot]\ndir = 'ed'\nbogus = 1\n",
            "[stt]\nengine = 'parakeet'\n",
        ] {
            std::fs::write(&file, text).unwrap();
            let err = write_binding(&file, &key("RControl")).unwrap_err();
            assert!(format!("{err:#}").contains("voicebridge.toml"), "{err:#}");
            assert_eq!(std::fs::read_to_string(&file).unwrap(), text);
        }
        // A key that the check refuses.
        std::fs::write(&file, WITH_KEY).unwrap();
        let err = write_binding(&file, &key("Mouse1")).unwrap_err();
        assert!(format!("{err:#}").contains("Mouse1"), "{err:#}");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), WITH_KEY);
        let names: Vec<_> = std::fs::read_dir(dir.path()).unwrap().collect();
        assert_eq!(names.len(), 1, "a temporary file was left");
    }

    #[test]
    fn test_error_texts_say_would_not_load_or_would_not_be_valid() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("voicebridge.toml");
        std::fs::write(&file, "[edcopilot]\ndir = 'ed'\nbogus = 1\n").unwrap();
        let err = write_binding(&file, &key("RControl")).unwrap_err();
        assert!(format!("{err:#}").contains("would not load"), "{err:#}");
        std::fs::write(&file, WITH_KEY).unwrap();
        let err = write_binding(&file, &key("Mouse1")).unwrap_err();
        assert!(format!("{err:#}").contains("would not be valid"), "{err:#}");
    }

    #[test]
    fn test_binding_lines() {
        assert_eq!(key("<").to_string(), "key \"<\"");
        assert_eq!(
            joystick("MOZA", 5).to_string(),
            "joystick \"MOZA\" button 5"
        );
        let from: Binding = PressedInput::Key("F13".to_string()).into();
        assert_eq!(from, key("F13"));
    }
}
