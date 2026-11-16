//! The shipped default word tables of `data/` load with no problem and
//! have the expected shape. The test checks the form of each entry, not
//! the words.

use std::path::PathBuf;

use edcopilot_voicebridge_core::{
    load_fillers_with_problems, load_sound_alikes_with_problems, load_synonyms_with_problems,
};

fn data() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("data")
}

/// One or more words in lower case, with letters, digits, `'` and `-`.
fn lower_words(text: &str) -> bool {
    !text.is_empty()
        && text.split(' ').all(|w| {
            !w.is_empty()
                && w.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '\'' || c == '-')
        })
}

#[test]
fn test_the_shipped_default_tables_load_and_have_the_expected_shape() {
    let (synonyms, problems) = load_synonyms_with_problems(&data().join("synonyms.toml"));
    assert!(problems.is_empty(), "{problems:?}");
    assert!(!synonyms.is_empty());
    for entry in synonyms.entries() {
        assert!(lower_words(&entry.key), "{:?}", entry.key);
        assert!(!entry.values.is_empty(), "{:?}", entry.key);
        assert!(entry.values.iter().all(|v| lower_words(v)), "{entry:?}");
    }

    let path = data().join("sound_alikes.toml");
    let (sound_alikes, problems) = load_sound_alikes_with_problems(&path);
    assert!(problems.is_empty(), "{problems:?}");
    assert!(!sound_alikes.is_empty());
    // Each entry of the file: one heard word gives one other word.
    let text = std::fs::read_to_string(&path).unwrap();
    let table: toml::Table = text.parse().unwrap();
    let entries = table["sound_alikes"].as_table().unwrap();
    assert_eq!(entries.len(), sound_alikes.len());
    for (heard, meant) in entries {
        let meant = meant.as_str().unwrap();
        assert!(!heard.contains(' ') && lower_words(heard), "{heard:?}");
        assert!(!meant.contains(' ') && lower_words(meant), "{meant:?}");
        assert_ne!(heard, meant);
        assert_eq!(sound_alikes.get(heard), Some(meant));
    }

    let (fillers, problems) = load_fillers_with_problems(&data().join("fillers.toml"));
    assert!(problems.is_empty(), "{problems:?}");
    assert!(fillers.not_last_words().count() > 0);
    assert!(fillers.may_be_last_words().count() > 0);
    for word in fillers.words() {
        assert!(!word.contains(' ') && lower_words(word), "{word:?}");
    }
}

/// The repository ships no data that comes from EDCoPilot: no file under
/// `core/data` is a command map or a file of profile variables. The test
/// reads the folder on the disk, not the git index. The program builds the
/// commands from the EDCoPilot profile of the user.
#[test]
fn test_no_file_under_core_data_is_a_command_map() {
    let mut names: Vec<String> = std::fs::read_dir(data())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().to_lowercase())
        .collect();
    names.sort();
    for name in &names {
        assert!(
            !name.ends_with(".json")
                && !name.contains("command_map")
                && !name.contains("profile_vars"),
            "{name}"
        );
    }
    assert_eq!(
        names,
        [
            "fillers.toml",
            "hotwords.txt",
            "sound_alikes.toml",
            "synonyms.toml"
        ]
    );
}
