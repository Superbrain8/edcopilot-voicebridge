use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::{BTreeSet, HashMap};
use std::path::Path;

/// The kinds of the slot files that a phrase of the profile can use:
/// `{TXT:System<n>}`, `{TXT:Group<n>}`, `{TXT:Voice<n>}`, `{TXT:POIType<n>}`.
const SLOT_KINDS: [&str; 4] = ["System", "Group", "Voice", "POIType"];

/// The kind of a slot key (`System12` gives `System`). `None` for a key
/// that is not a slot key.
pub(crate) fn slot_kind(key: &str) -> Option<&'static str> {
    SLOT_KINDS.into_iter().find(|kind| {
        key.strip_prefix(kind)
            .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
    })
}

/// The slot keys (`System12`, `Group3`, ...) of the `{TXT:...}` places in
/// the phrases.
pub fn slot_keys<'a>(phrases: impl Iterator<Item = &'a str>) -> BTreeSet<String> {
    let mut keys = BTreeSet::new();
    for phrase in phrases {
        let mut rest = phrase;
        while let Some(start) = rest.find("{TXT:") {
            rest = &rest[start + 5..];
            let Some(end) = rest.find('}') else {
                break;
            };
            let name = &rest[..end];
            if slot_kind(name).is_some() {
                keys.insert(name.to_string());
            }
            rest = &rest[end + 1..];
        }
    }
    keys
}

/// True when the text of a slot file has the form that the token rules of
/// the profile builder assume. The text has no byte order mark and no
/// white space at the start and at the end.
///
/// - An empty slot ("Not set ...") or an empty file gives no names. It is a
///   known form.
/// - A system file holds `[name; alternate; ...]` with one or more `;`. The
///   slot parse of the profile then takes the text from `[` to the first
///   `;`, which is the first name. A plain name with no `[`, `]` or `;` is
///   also known: the slot parse then takes the whole text.
/// - A group, voice, or point-of-interest file holds one plain name with no
///   `[`, `]`, or `;`. The profile uses the whole text of the file.
fn known_form(kind: &str, text: &str) -> bool {
    if text.is_empty() || is_not_set(text) {
        return true;
    }
    let plain = !text.contains(['[', ']', ';']);
    if holds_one_plain_name(kind) {
        return plain;
    }
    let list = text
        .strip_prefix('[')
        .and_then(|t| t.strip_suffix(']'))
        .is_some_and(|inner| inner.contains(';') && !inner.contains(['[', ']']));
    plain || list
}

/// The form check requires one plain name in each file of the slot kind
/// `kind`: a group, voice, or point-of-interest file. A system file can
/// hold a list. The profile builder keeps a token that takes the text of a
/// phrase slot directly only for such a kind: VoiceAttack writes the whole
/// text of the file, and the matcher writes the first name.
pub(crate) fn holds_one_plain_name(kind: &str) -> bool {
    kind != "System"
}

/// The warning for a slot file with a form that the program does not know.
pub fn unknown_form_warning(file: &str) -> String {
    format!("the vocabulary file {file} has a form that the program does not know, its names are not used")
}

/// [`load_dynamic_vocab`] with the check of the slot files. Each file of a
/// key in `used` must have the known form (see `known_form`). A file with
/// another form gives no names for its slot (a miss, never a wrong token),
/// and its file name goes into the second list, in sorted order. The other
/// files load as `load_dynamic_vocab` loads them.
pub fn load_dynamic_vocab_checked(
    dir: &Path,
    used: &BTreeSet<String>,
) -> (HashMap<String, Vec<String>>, Vec<String>) {
    let mut unknown = Vec::new();
    let map = load_with(dir, |key, content, file| {
        let kind = slot_kind(key).filter(|_| used.contains(key));
        if let Some(kind) = kind {
            let text = content.trim_start_matches('\u{feff}').trim();
            if !known_form(kind, text) {
                unknown.push(file.to_string());
                return Vec::new();
            }
        }
        parse_entries(content)
    });
    unknown.sort();
    (map, unknown)
}

/// Loads every `*.txt` list file in `dir`, keyed by file stem without a
/// leading `EDCoPilot`. A missing directory gives an empty map.
pub fn load_dynamic_vocab(dir: &Path) -> HashMap<String, Vec<String>> {
    load_with(dir, |_, content, _| parse_entries(content))
}

/// Loads each `*.txt` file of `dir` with `parse`: the key, the content and
/// the file name give the names. A file that is not text gives no names.
fn load_with(
    dir: &Path,
    mut parse: impl FnMut(&str, &str, &str) -> Vec<String>,
) -> HashMap<String, Vec<String>> {
    let mut map = HashMap::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return map;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let is_txt = path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("txt"));
        if !is_txt || !path.is_file() {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let key = stem.strip_prefix("EDCoPilot").unwrap_or(stem).to_string();
        let file = path
            .file_name()
            .and_then(|f| f.to_str())
            .unwrap_or(stem)
            .to_string();
        let values = std::fs::read_to_string(&path)
            .map(|c| parse(&key, &c, &file))
            .unwrap_or_default();
        map.insert(key, values);
    }
    map
}

fn parse_entries(content: &str) -> Vec<String> {
    let text = content.trim_start_matches('\u{feff}').trim();
    let items: Vec<&str> = match (text.starts_with('['), text.ends_with(']')) {
        (true, true) => text[1..text.len() - 1].split(';').collect(),
        (false, false) => vec![text],
        _ => return Vec::new(),
    };
    items
        .into_iter()
        .map(str::trim)
        .filter(|e| !e.is_empty() && !is_not_set(e))
        .map(str::to_string)
        .collect()
}

/// EDCoPilot writes "Not set ..." into unused slots.
fn is_not_set(entry: &str) -> bool {
    entry
        .as_bytes()
        .get(..7)
        .is_some_and(|p| p.eq_ignore_ascii_case(b"not set"))
}

/// Calls `on_change` on every file event in `dir`. The caller must keep the
/// returned watcher alive.
pub fn watch_dynamic_vocab(
    dir: &Path,
    on_change: impl Fn() + Send + 'static,
) -> notify::Result<RecommendedWatcher> {
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if res.is_ok() {
            on_change();
        }
    })?;
    watcher.watch(dir, RecursiveMode::NonRecursive)?;
    Ok(watcher)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    fn fixtures() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/dynamic")
    }

    #[test]
    fn test_parses_list_file() {
        let m = load_dynamic_vocab(&fixtures());
        assert_eq!(
            m["System1"],
            vec!["KDS 4021", "Kads 4021", "Harriet Quell Station"]
        );
        assert_eq!(m["Names"], vec!["Harbourmaster", "Wren"]);
    }

    #[test]
    fn test_bare_value_is_one_entry() {
        assert_eq!(
            load_dynamic_vocab(&fixtures())["SystemNew"],
            vec!["Brennoch"]
        );
    }

    #[test]
    fn test_empty_and_malformed_do_not_panic() {
        let m = load_dynamic_vocab(&fixtures());
        for k in ["Empty", "LampSignal", "Unbalanced", "UnbalancedEnd"] {
            assert!(m[k].is_empty(), "{k}");
        }
    }

    #[test]
    fn test_key_derivation_and_filtering() {
        let m = load_dynamic_vocab(&fixtures());
        assert!(m.contains_key("LampSignal"));
        assert!(!m.contains_key("EDCoPilotSystem1"));
        assert!(!m.contains_key("notes"));
        assert!(m["System96"].is_empty());
    }

    #[test]
    fn test_bom_trailing_newline_and_not_set_inside_list() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(
            d.path().join("EDCoPilotX1.txt"),
            "\u{feff}[a; NOT set b; c]\r\n",
        )
        .unwrap();
        std::fs::write(d.path().join("Bad.txt"), [0xff, 0xfe, 0x00]).unwrap();
        let m = load_dynamic_vocab(d.path());
        assert_eq!(m["X1"], vec!["a", "c"]);
        assert!(m["Bad"].is_empty());
    }

    #[test]
    fn test_missing_directory_gives_empty_map() {
        assert!(load_dynamic_vocab(&fixtures().join("nope")).is_empty());
    }

    fn keys(names: &[&str]) -> BTreeSet<String> {
        names.iter().map(|n| n.to_string()).collect()
    }

    #[test]
    fn test_a_system_file_without_a_separator_gives_no_names_and_one_warning() {
        let d = tempfile::tempdir().unwrap();
        let write = |name: &str, text: &str| std::fs::write(d.path().join(name), text).unwrap();
        write("EDCoPilotSystem1.txt", "[Alpha]");
        write("EDCoPilotSystem2.txt", "[Beta; Beta Port]");
        write("EDCoPilotSystem3.txt", "Not set - slot 3");
        // A plain name: the slot parse of the profile takes the whole text.
        write("EDCoPilotSystem4.txt", "Gamma\r\n");
        // A file that no record uses is not checked.
        write("EDCoPilotSystem5.txt", "[Delta]");
        let used = keys(&["System1", "System2", "System3", "System4"]);
        let (vocab, unknown) = load_dynamic_vocab_checked(d.path(), &used);
        assert_eq!(unknown, ["EDCoPilotSystem1.txt"]);
        assert!(vocab["System1"].is_empty());
        assert_eq!(vocab["System2"], ["Beta", "Beta Port"]);
        assert!(vocab["System3"].is_empty());
        assert_eq!(vocab["System4"], ["Gamma"]);
        assert_eq!(vocab["System5"], ["Delta"]);
        assert_eq!(
            unknown_form_warning(&unknown[0]),
            "the vocabulary file EDCoPilotSystem1.txt has a form that the program does not \
             know, its names are not used"
        );
    }

    #[test]
    fn test_a_group_file_with_alternates_gives_no_names_and_one_warning() {
        let d = tempfile::tempdir().unwrap();
        let write = |name: &str, text: &str| std::fs::write(d.path().join(name), text).unwrap();
        write("EDCoPilotGroup1.txt", "[Ann Example; Ann]");
        write("EDCoPilotGroup2.txt", "Bob Example");
        write("EDCoPilotGroup3.txt", "Not set");
        write("EDCoPilotVoice1.txt", "First;Second");
        write("EDCoPilotPOIType1.txt", "[old wrecks]");
        let used = keys(&["Group1", "Group2", "Group3", "Voice1", "POIType1", "Group9"]);
        let (vocab, unknown) = load_dynamic_vocab_checked(d.path(), &used);
        assert_eq!(
            unknown,
            [
                "EDCoPilotGroup1.txt",
                "EDCoPilotPOIType1.txt",
                "EDCoPilotVoice1.txt"
            ]
        );
        assert!(vocab["Group1"].is_empty());
        assert_eq!(vocab["Group2"], ["Bob Example"]);
        assert!(vocab["Group3"].is_empty());
        assert!(vocab["Voice1"].is_empty());
        assert!(vocab["POIType1"].is_empty());
    }

    #[test]
    fn test_valid_files_load_as_without_the_check() {
        let used = keys(&["System1", "System96", "SystemNew", "Names"]);
        let (checked, _) = load_dynamic_vocab_checked(&fixtures(), &used);
        assert_eq!(checked, load_dynamic_vocab(&fixtures()));
    }

    #[test]
    fn test_slot_keys_of_the_phrases() {
        let phrases = [
            "go to {TXT:System12} now",
            "hail {TXT:Group3}",
            "{TXT:EDCP_SAILTO} {TXT:Voice2} {TXT:POIType7}",
            "{TXT:SystemName} {TXT:System} {TXT:Groupx1}",
        ];
        assert_eq!(
            slot_keys(phrases.iter().copied()),
            keys(&["Group3", "POIType7", "System12", "Voice2"])
        );
    }

    /// The vocabulary files of the fixture of `core/src/profile/fixture.rs`
    /// have the known form: system files `[Name;...]`, group files with one
    /// plain name.
    #[test]
    fn test_the_fixture_files_have_the_known_form() {
        use crate::profile::fixture::{fixture_commands, write_fixture_edcopilot};
        let d = tempfile::tempdir().unwrap();
        write_fixture_edcopilot(d.path());
        let commands = fixture_commands();
        let used = slot_keys(commands.iter().filter_map(|c| c.phrase.as_deref()));
        assert!(
            used.contains("System1") && used.contains("Group1"),
            "{used:?}"
        );
        let dir = d.path().join("Dynamic var files");
        let (checked, unknown) = load_dynamic_vocab_checked(&dir, &used);
        assert!(unknown.is_empty(), "{unknown:?}");
        assert_eq!(checked, load_dynamic_vocab(&dir));
    }

    #[test]
    fn test_watcher_fires_on_write() {
        let d = tempfile::tempdir().unwrap();
        let (tx, rx) = mpsc::channel();
        let _w = watch_dynamic_vocab(d.path(), move || {
            let _ = tx.send(());
        })
        .unwrap();
        std::fs::write(d.path().join("EDCoPilotSystem1.txt"), "[a]").unwrap();
        rx.recv_timeout(Duration::from_secs(5)).unwrap();
    }
}
