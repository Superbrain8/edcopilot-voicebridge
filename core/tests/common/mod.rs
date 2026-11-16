//! The shared setup of the integration tests: the synthetic profile of
//! `profile::fixture` (invented phrases and tokens), loaded as the program
//! loads it, and the default word tables of `data/`.

#![allow(dead_code)]

use std::collections::HashMap;
use std::path::PathBuf;

use edcopilot_voicebridge_core::profile::fixture::{fixture_profile, write_fixture_edcopilot};
use edcopilot_voicebridge_core::profile::{build_map, BlockingPhrase};
use edcopilot_voicebridge_core::{
    load_dynamic_vocab, load_fillers_with_problems, load_sound_alikes, removed_as_blocking,
    resolve_blocking, resolve_profile_vars, resolve_runtime_grammar_with, AliasMap, Command,
    Fillers, Match, Matcher, Pipeline, SoundAlikes, Synonyms, Writer,
};

pub fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// The fixture commands as `load_commands` of the program loads them: the
/// build, the runtime grammar of the fixture INI (with the number check),
/// then the profile variables. The vocabulary files of the fixture folder.
pub fn fixture_loaded() -> (Vec<Command>, HashMap<String, Vec<String>>) {
    let (commands, _, vocab) = fixture_loaded_with_blocking();
    (commands, vocab)
}

/// `fixture_loaded` with the blocking phrases as the program loads them:
/// those of the build and those of the commands that the number check
/// takes out, with the runtime grammar and the profile variables.
pub fn fixture_loaded_with_blocking() -> (
    Vec<Command>,
    Vec<BlockingPhrase>,
    HashMap<String, Vec<String>>,
) {
    let dir = tempfile::tempdir().unwrap();
    write_fixture_edcopilot(dir.path());
    let built = build_map(&fixture_profile()).unwrap();
    let ini = dir
        .path()
        .join("working")
        .join("EDCoPilot.VoiceCommands.ini");
    let out = resolve_runtime_grammar_with(built.commands, Some(&ini), &built.profile_vars);
    let mut blocking = built.blocking;
    blocking.extend(removed_as_blocking(&out.removed));
    let blocking = resolve_blocking(blocking, Some(&ini), &built.profile_vars).phrases;
    let commands = resolve_profile_vars(out.commands, &built.profile_vars);
    let vocab = load_dynamic_vocab(&dir.path().join("Dynamic var files"));
    (commands, blocking, vocab)
}

/// The loaded fixture commands.
pub fn fixture_commands_loaded() -> Vec<Command> {
    fixture_loaded().0
}

/// The vocabulary files of the fixture folder.
pub fn fixture_vocab() -> HashMap<String, Vec<String>> {
    fixture_loaded().1
}

/// The vocabulary files of the fixture folder, and the name "Sol" in
/// `System11` (not set in the folder). The default sound-alike table
/// changes "soul", "seoul" and "sole" to "sol": the tests of these entries
/// need a slot with this name.
pub fn fixture_vocab_with_sol() -> HashMap<String, Vec<String>> {
    let mut vocab = fixture_vocab();
    vocab.insert("System11".to_string(), vec!["Sol".to_string()]);
    vocab
}

/// A vocabulary from `(slot, names)` pairs. The first name of a slot is
/// its name.
pub fn vocab(slots: &[(&str, &[&str])]) -> HashMap<String, Vec<String>> {
    slots
        .iter()
        .map(|(slot, names)| {
            (
                slot.to_string(),
                names.iter().map(|n| n.to_string()).collect(),
            )
        })
        .collect()
}

/// A matcher on the loaded fixture commands and the blocking phrases (as
/// the program builds it) with `vocab`, no aliases, and the threshold 0.8.
pub fn fixture_matcher(vocab: HashMap<String, Vec<String>>) -> Matcher {
    let (commands, blocking, _) = fixture_loaded_with_blocking();
    Matcher::new(commands, vocab, AliasMap::new(), 0.8).with_blocking(&blocking)
}

/// A writer whose path is never written: the tests only match.
pub fn writer() -> Writer {
    Writer::new(std::env::temp_dir().join("voicebridge-never-written.txt"))
}

pub fn default_sound_alikes() -> SoundAlikes {
    load_sound_alikes(&root().join("data/sound_alikes.toml")).unwrap()
}

pub fn default_fillers() -> Fillers {
    let (table, problems) = load_fillers_with_problems(&root().join("data/fillers.toml"));
    assert!(problems.is_empty(), "{problems:?}");
    table
}

/// An invented synonym table for the fixture. It has the shapes of the
/// default table: a key with "to", a key of one word, a key of several
/// words, and keys with the sound-alikes "two" and "too" of "to". Each key
/// gives the two first words of the fixture command for a harbour.
pub fn harbour_synonyms() -> Synonyms {
    let values = || vec!["sail to", "sail for"];
    Synonyms::from_pairs([
        ("make for", values()),
        ("make", vec!["sail for", "sail to"]),
        ("now we head to", values()),
        ("maker", values()),
        ("make two", values()),
        ("make too", values()),
    ])
}

/// A command with a phrase and a token, and the default other fields.
pub fn cmd(phrase: &str, token: &str) -> Command {
    Command {
        phrase: Some(phrase.to_string()),
        token_template: token.to_string(),
        ..Default::default()
    }
}

pub fn hit(p: &Pipeline, text: &str) -> Match {
    p.match_text(text)
        .unwrap_or_else(|| panic!("no match for {text:?}"))
}

pub fn token(p: &Pipeline, text: &str) -> Option<String> {
    p.match_text(text).map(|m| m.token)
}
