//! Phrase synonyms in the pipeline, on the synthetic profile. The default
//! synonym table has no key for the invented phrases of the fixture. Thus
//! the tests use an invented table of the same shape (see
//! `common::harbour_synonyms`): "make for", "make", "now we head to",
//! "maker", "make two" and "make too" give "sail to" and "sail for".

mod common;

use std::collections::HashMap;

use common::{
    cmd, default_sound_alikes, fixture_commands_loaded, fixture_vocab_with_sol, harbour_synonyms,
    hit, writer,
};
use edcopilot_voicebridge_core::{AliasMap, Matcher, Pipeline, SoundAlikes, Synonyms};

/// The fixture with its vocabulary files and "Sol" in `System11`, the
/// default sound-alikes, and the invented synonym table.
fn fixture_pipeline() -> Pipeline {
    let matcher = Matcher::new(
        fixture_commands_loaded(),
        fixture_vocab_with_sol(),
        AliasMap::new(),
        0.8,
    );
    Pipeline::new(matcher, writer())
        .with_sound_alikes(default_sound_alikes())
        .with_synonyms(harbour_synonyms())
}

/// A synonym key before the words of a phrase of several parts (a runtime
/// grammar phrase), also a key of one word.
#[test]
fn test_synonym_reaches_the_home_port() {
    let p = fixture_pipeline();
    for text in ["make for home port", "Make home port."] {
        let m = hit(&p, text);
        assert_eq!(m.token, "SailHomePort", "{text}");
        assert!(m.via_synonym, "{text}");
        assert!(!m.via_sound_alike, "{text}");
        assert_eq!(m.confidence, 1.0, "{text}");
        assert!(!m.inexact_single_required_word, "{text}");
    }
}

#[test]
fn test_the_edcopilot_phrase_matches_as_heard() {
    let p = fixture_pipeline();
    let m = hit(&p, "sail for home port");
    assert_eq!(m.token, "SailHomePort");
    assert!(!m.via_synonym);
}

/// A synonym before a slot name gives the token of the phrase.
#[test]
fn test_synonym_before_a_slot_name_gives_the_token_of_the_phrase() {
    let p = fixture_pipeline();
    let literal = hit(&p, "sail to vellin");
    let m = hit(&p, "make for Vellin");
    assert_eq!(m.token, literal.token);
    assert!(m.via_synonym);
    assert!(!m.inexact_single_required_word);
}

/// A fuzzy one-word name after a synonym follows the one-word rule.
#[test]
fn test_fuzzy_one_word_name_after_a_synonym_is_flagged() {
    let p = fixture_pipeline();
    let m = hit(&p, "make for morow");
    assert!(m.via_synonym);
    assert!(m.inexact_single_required_word);
}

/// Synonym plus sound-alike: both marks, the factor, and the replaced name
/// is the single required word.
#[test]
fn test_synonym_with_a_sound_alike_name() {
    let p = fixture_pipeline();
    let m = hit(&p, "make for seoul");
    assert_eq!(m.token, "HarbourAt:Sol:sail to");
    assert!(m.via_synonym);
    assert!(m.via_sound_alike);
    assert!((m.confidence - 0.95).abs() < 1e-6, "{}", m.confidence);
    assert!(m.inexact_single_required_word);
}

#[test]
fn test_no_match_for_a_synonym_alone_or_with_extra_words() {
    let p = fixture_pipeline();
    for text in ["make", "make for", "please make for home port"] {
        assert_eq!(p.match_text(text), None, "{text}");
    }
}

/// Two values that lead to different tokens for one text give no match.
#[test]
fn test_values_with_different_tokens_give_no_match() {
    let synonyms = Synonyms::from_pairs([("hop to", vec!["open", "close"])]);
    let both = vec![cmd("open hatch", "A"), cmd("close hatch", "B")];
    let p = Pipeline::new(
        Matcher::new(both, HashMap::new(), AliasMap::new(), 0.8),
        writer(),
    )
    .with_synonyms(synonyms.clone());
    assert_eq!(p.match_text("hop to hatch"), None);

    let one = vec![cmd("open hatch", "A")];
    let p = Pipeline::new(
        Matcher::new(one, HashMap::new(), AliasMap::new(), 0.8),
        writer(),
    )
    .with_synonyms(synonyms)
    .with_sound_alikes(SoundAlikes::default());
    assert_eq!(hit(&p, "hop to hatch").token, "A");
}

/// A literal match is never replaced by a synonym variant.
#[test]
fn test_literal_tie_is_not_decided_by_a_synonym() {
    let commands = vec![
        cmd("go hatch", "A"),
        cmd("[go; walk] hatch", "B"),
        cmd("open hatch", "C"),
    ];
    let p = Pipeline::new(
        Matcher::new(commands, HashMap::new(), AliasMap::new(), 0.8),
        writer(),
    )
    .with_synonyms(Synonyms::from_pairs([("go", vec!["open"])]));
    assert_eq!(p.match_text("go hatch"), None);
}

/// A one-word name from a vocabulary slot is scored on its own: a wrong
/// letter flags the match on every input path.
#[test]
fn test_one_word_slot_name_with_a_wrong_letter_is_flagged() {
    let p = fixture_pipeline();
    let literal = hit(&p, "sail to morow");
    assert!(literal.inexact_single_required_word);
    assert!(!literal.via_synonym);
    let synonym = hit(&p, "make for morow");
    assert_eq!(synonym.token, literal.token);
    assert!(synonym.inexact_single_required_word);
    let exact = hit(&p, "sail to morrow");
    assert!(!exact.inexact_single_required_word);
}

/// A slot name of two words keeps normal fuzzy matching through its average.
#[test]
fn test_two_word_slot_name_with_a_small_error_matches() {
    let p = fixture_pipeline();
    let m = hit(&p, "sail to brackmoor sund");
    assert_eq!(m.token, "HarbourAt:Brackmoor Sound:sail to");
    assert!(!m.inexact_single_required_word);
}

/// A sound-alike replacement in a one-word slot is flagged, also as heard.
#[test]
fn test_replaced_one_word_slot_name_is_flagged() {
    let p = fixture_pipeline();
    let m = hit(&p, "sail to seoul");
    assert_eq!(m.token, "HarbourAt:Sol:sail to");
    assert!(m.via_sound_alike);
    assert!(m.inexact_single_required_word);
}

/// The words of a synonym count like the words of the phrase.
#[test]
fn test_synonym_and_literal_give_the_same_result() {
    let p = fixture_pipeline();
    let literal = hit(&p, "sail for home pot");
    let synonym = hit(&p, "make for home pot");
    assert_eq!(synonym.token, literal.token);
    assert_eq!(synonym.token, "SailHomePort");
    assert_eq!(synonym.confidence, literal.confidence);
    assert_eq!(
        synonym.inexact_single_required_word,
        literal.inexact_single_required_word
    );
    assert!(!literal.inexact_single_required_word);
    assert!(synonym.via_synonym);
}

/// "sail for" is also the start of a phrase with two number ranges: the
/// synonym reaches it too.
#[test]
fn test_synonym_reaches_a_phrase_with_two_numbers() {
    let p = fixture_pipeline();
    let m = hit(&p, "make for buoy 3 4");
    assert_eq!(m.token, "SailForBuoy:34");
    assert!(m.via_synonym);
    // Number words: the separate reading wins, the digit run "34" matches
    // nothing here.
    let m = hit(&p, "make for buoy three four");
    assert_eq!(m.token, "SailForBuoy:34");
}

/// A table whose combinations exceed the cap gives no synonym match. Under
/// the cap, the same table still works.
#[test]
fn test_too_many_synonym_combinations_give_no_match() {
    let commands = vec![cmd("alpha alpha", "A"), cmd("alpha alpha alpha", "B")];
    let synonyms = Synonyms::from_pairs([("go", vec!["alpha", "beta", "gamma"])]);
    let p = Pipeline::new(
        Matcher::new(commands, HashMap::new(), AliasMap::new(), 0.8),
        writer(),
    )
    .with_synonyms(synonyms);
    // 3 x 3 = 9 combinations: over the cap of 8.
    assert_eq!(p.match_text("go go"), None);
    // 3 combinations: under the cap.
    assert_eq!(hit(&p, "go alpha").token, "A");
}

/// A run of single digits is one number: "two two zero seven" is 2207.
#[test]
fn test_digit_run_reaches_a_system_name() {
    let p = fixture_pipeline();
    for text in [
        "sail to reef two two zero seven",
        "sail to reef 2 2 0 7",
        "make for reef two two zero seven",
    ] {
        let m = hit(&p, text);
        assert_eq!(m.token, "HarbourAt:Reef 2207:sail to", "{text}");
    }
}

/// "Lay Kurs to Sol": the German word for "course" as a sound-alike.
#[test]
fn test_kurs_becomes_course() {
    let p = fixture_pipeline();
    let m = hit(&p, "Lay Kurs for home port.");
    assert_eq!(m.token, "PlotCourse:HomePort");
    assert!(m.via_sound_alike);
    let m = hit(&p, "Lay Kurs to Sol.");
    assert_eq!(m.token, "PlotCourse:Sol");
    assert!(m.via_sound_alike);
    assert!(!m.inexact_single_required_word);
    assert_eq!(p.match_text("kurs"), None);
}

/// The system name is a second way to say a fixed command.
#[test]
fn test_system_name_of_a_fixed_command() {
    let p = fixture_pipeline();
    for (text, token) in [
        ("sail to Dunmore", "HarbourAt:Dunmore:sail to"),
        ("sail to the old mill", "HarbourAt:Dunmore:sail to"),
        ("sail to Brackwater", "HarbourAt:Brackwater:sail to"),
        ("sail to Tam The Ferryman", "HarbourAt:Brackwater:sail to"),
        ("sail to Vennick", "HarbourAt:Vennick:sail to"),
        ("sail to Halvard Crane", "HarbourAt:Vennick:sail to"),
        // A system that is equal to its name.
        ("sail to Keston", "HarbourAt:Keston:sail to"),
    ] {
        let m = hit(&p, text);
        assert_eq!(m.token, token, "{text}");
        assert!(!m.inexact_single_required_word, "{text}");
        assert_eq!(m.confidence, 1.0, "{text}");
    }
}

/// A name that is a fixed system and a vocabulary name gives one token.
#[test]
fn test_fixed_system_name_also_in_the_vocabulary_gives_no_tie() {
    let p = fixture_pipeline();
    let m = hit(&p, "sail to Ollin");
    assert_eq!(m.token, "HarbourAt:Ollin:sail to");
    let m = hit(&p, "tide at Ollin");
    assert_eq!(m.token, "HarbourInfo:Ollin:tide at Ollin");
    let m = hit(&p, "tide at Rose Quay");
    assert_eq!(m.token, "HarbourInfo:Ollin:tide at Rose Quay");
}

/// Fixed names follow the slot rule: a one-word name must be exact.
#[test]
fn test_fixed_one_word_name_with_an_error_is_flagged() {
    let p = fixture_pipeline();
    for text in [
        "sail to dunmor",
        "make for rosse",
        "sail to rosse",
        "sail to vennik",
    ] {
        let m = hit(&p, text);
        assert!(m.inexact_single_required_word, "{text}: {m:?}");
    }
    assert_eq!(p.match_text("sail to dunbar"), None);
    // A multi-word name with one small error matches through its average.
    let m = hit(&p, "sail to halvard crne");
    assert_eq!(m.token, "HarbourAt:Vennick:sail to");
    assert!(!m.inexact_single_required_word);
}

/// The synonym works on the system names of the fixed commands too.
#[test]
fn test_synonym_before_a_fixed_system_name() {
    let p = fixture_pipeline();
    let m = hit(&p, "Make for Dunmore.");
    assert_eq!(m.token, "HarbourAt:Dunmore:sail to");
    assert!(m.via_synonym);
}

/// A misheard form of a key ("Now we head to", "Maker") is a key of its
/// own, as in the default table.
#[test]
fn test_misheard_key_is_a_synonym() {
    let p = fixture_pipeline();
    let m = hit(&p, "Now we head to Sol.");
    assert_eq!(m.token, "HarbourAt:Sol:sail to");
    assert!(m.via_synonym);
    let m = hit(&p, "Maker Vellin.");
    assert_eq!(m.token, "HarbourAt:Vellin:sail to");
    assert!(m.via_synonym);
    // The key alone, or with words that are no command, gives no match.
    assert_eq!(p.match_text("now we head to"), None);
    assert_eq!(p.match_text("now we head to the store"), None);
    // A one-word name with an error stays flagged after the synonym.
    let m = hit(&p, "now we head to morow");
    assert!(m.inexact_single_required_word);
}

/// "free" is what the speech engine writes for "three".
#[test]
fn test_free_becomes_three() {
    let p = fixture_pipeline();
    let m = hit(&p, "net reminder free");
    assert_eq!(m.token, "SetNetReminder:3");
    assert!(m.via_sound_alike);
    let m = hit(&p, "Now we head to mooring free.");
    assert_eq!(m.token, "SelectMooring:3");
    assert!(m.via_synonym);
    assert!(m.via_sound_alike);
    assert_eq!(p.match_text("free"), None);
}

/// The speech engine writes "two" or "too" for "to".
#[test]
fn test_two_and_too_become_to() {
    let p = fixture_pipeline();
    for text in ["Make two Sol.", "make too sol", "sail two Sol"] {
        let m = hit(&p, text);
        assert_eq!(m.token, "HarbourAt:Sol:sail to", "{text}");
        // "make two" is a synonym key. "sail two" is a sound-alike.
        assert!(m.via_sound_alike || m.via_synonym, "{text}");
    }
    // A number that the phrase needs stays a number.
    let m = hit(&p, "make for mooring two");
    assert_eq!(m.token, "SelectMooring:2");
    assert!(!m.via_sound_alike);
    assert_eq!(p.match_text("two"), None);
    assert_eq!(p.match_text("make two"), None);
}
