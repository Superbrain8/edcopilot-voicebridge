//! Sound-alike variants in the pipeline, on the synthetic profile and the
//! default table file. The probes check that a variant never changes a text
//! into a command that the speaker did not mean.

mod common;

use std::collections::HashMap;

use common::{cmd, default_sound_alikes, fixture_matcher, hit, writer};
use edcopilot_voicebridge_core::{
    AliasMap, BlockReason, Command, CommandClass, MatchResult, Matcher, Pipeline, SoundAlikes,
};

/// The writer path is never written: these tests only match.
fn pipeline(matcher: Matcher, table: SoundAlikes) -> Pipeline {
    Pipeline::new(matcher, writer()).with_sound_alikes(table)
}

fn fixture_pipeline() -> Pipeline {
    pipeline(fixture_matcher(HashMap::new()), default_sound_alikes())
}

/// "by" and "bye" in a composite prefix become "buy".
#[test]
fn test_by_and_bye_become_buy() {
    let p = fixture_pipeline();
    let m = hit(&p, "cheapest by bid cork");
    assert_eq!(m.token, "BuyTop:Cork");
    assert!(m.via_sound_alike);
    // The replaced word is in the prefix. The name "cork" is exact.
    assert!(!m.inexact_single_required_word);
    assert!((m.confidence - 0.95).abs() < 1e-6, "{}", m.confidence);
    let m = hit(&p, "dock bye cork");
    assert_eq!(m.token, "BuyDock:Cork");
    assert!(m.via_sound_alike);
}

/// "cell" is 0.75 from "sell": the text as heard already matches.
#[test]
fn test_literal_fuzzy_match_is_not_a_sound_alike() {
    let p = fixture_pipeline();
    let m = hit(&p, "dearest cell bid cork");
    assert_eq!(m.token, "SellTop:Cork");
    assert!(!m.via_sound_alike);
}

/// Two replacements: the variant with all replacements comes first and wins.
#[test]
fn test_two_and_by_become_to_buy() {
    let p = fixture_pipeline();
    for _ in 0..3 {
        let m = hit(&p, "Nearest harbour two by cork.");
        assert_eq!(m.token, "BuyNear:Cork");
        assert!(m.via_sound_alike);
        assert!(!m.inexact_single_required_word);
        assert!((m.confidence - 0.95).abs() < 1e-6, "{}", m.confidence);
    }
    assert_eq!(p.match_text("Nearest harbour two by spray."), None);
}

/// "higher" becomes "highest".
#[test]
fn test_higher_becomes_highest() {
    let p = fixture_pipeline();
    let m = hit(&p, "Higher water mark.");
    assert_eq!(m.token, "TideMark:Highest");
    assert!(m.via_sound_alike);
    assert_eq!(p.match_text("Higher spray for cork."), None);
}

#[test]
fn test_soul_is_blocked_and_seoul_becomes_sol_with_a_vocabulary_that_holds_sol() {
    let command = Command {
        phrase: Some("[sail to; steer to] {TXT:System1}".to_string()),
        token_template: "HarbourAt:{TXT:SystemName}:sail to".to_string(),
        class: CommandClass::DynamicName,
        ..Default::default()
    };
    let vocab = HashMap::from([("System1".to_string(), vec!["Sol".to_string()])]);
    let matcher = Matcher::new(vec![command], vocab, AliasMap::new(), 0.8);
    let p = pipeline(matcher, default_sound_alikes());
    let literal = hit(&p, "sail to sol");
    assert_eq!(literal.token, "HarbourAt:Sol:sail to");
    assert!(!literal.via_sound_alike);
    // "soul" is 0.75 from "sol". The slot name is scored alone and fails
    // the threshold: the match is blocked, and no variant is tried.
    assert_eq!(p.match_text("sail to soul"), None);
    // "seoul" is 0.6 from "sol": only the variant matches. The replaced word
    // is a slot name of one word, so the one-word rule applies.
    let m = hit(&p, "Sail to Seoul.");
    assert_eq!(m.token, literal.token);
    assert!(m.via_sound_alike);
    assert!(m.inexact_single_required_word);
}

/// Two variants that match different tokens give no match.
#[test]
fn test_variants_with_different_tokens_give_no_match() {
    let table = SoundAlikes::from_pairs([("apple", "zebra"), ("mango", "quilt")]);
    let both = vec![cmd("zebra mango", "A"), cmd("apple quilt", "B")];
    let matcher = Matcher::new(both, HashMap::new(), AliasMap::new(), 0.8);
    let p = pipeline(matcher, table.clone());
    assert_eq!(p.match_text("apple mango"), None);

    let one = vec![cmd("zebra mango", "A")];
    let matcher = Matcher::new(one, HashMap::new(), AliasMap::new(), 0.8);
    let p = pipeline(matcher, table);
    assert_eq!(hit(&p, "apple mango").token, "A");
}

/// A variant with number words also gets its number variants.
#[test]
fn test_variant_gets_number_variants() {
    let command = Command {
        phrase: Some("buy [1..20] units".to_string()),
        token_template: "Buy:{TXT:N}".to_string(),
        class: CommandClass::DynamicValue,
        number_vars: vec!["N".to_string()],
        ..Default::default()
    };
    let matcher = Matcher::new(vec![command], HashMap::new(), AliasMap::new(), 0.8);
    let p = pipeline(matcher, default_sound_alikes());
    let m = hit(&p, "by twelve units");
    assert_eq!(m.token, "Buy:12");
    assert!(m.via_sound_alike);
}

/// "by" is a phrase word here: the text and its number variant match as heard.
#[test]
fn test_phrase_word_by_matches_as_heard() {
    let p = fixture_pipeline();
    for text in [
        "add lamp oil to the store by 5 barrels",
        "add lamp oil to the store by five barrels",
    ] {
        let m = hit(&p, text);
        assert_eq!(m.token, "AddLampOil:5", "{text}");
        assert!(!m.via_sound_alike, "{text}");
    }
}

/// "lock" for "log": the engine writes "lock" where the speaker said "log".
#[test]
fn test_lock_becomes_log() {
    let p = fixture_pipeline();
    let m = hit(&p, "Open tide lock.");
    assert_eq!(m.token, "OpenTideLogPanel");
    assert!(m.via_sound_alike);
    let m = hit(&p, "help tide lock");
    assert_eq!(m.token, "Guide:Panel:TideLog");
    assert!(m.via_sound_alike);
}

/// Phrases with "lock" or "log" as heard are never changed: "lock" at the
/// start and in the middle of a phrase.
#[test]
fn test_lock_and_log_phrases_match_as_heard() {
    let p = fixture_pipeline();
    for (text, token) in [
        ("lock current ledger", "LedgerLock:Current"),
        ("open tide log", "OpenTideLogPanel"),
        ("next tide log", "TideLog:Next"),
        ("swing the lock gates", "LockGates:Swing"),
        // "log" at the start of a phrase of several words.
        ("log the wind speed", "LogWind:Speed"),
    ] {
        let m = hit(&p, text);
        assert_eq!(m.token, token, "{text}");
        assert!(!m.via_sound_alike, "{text}: {m:?}");
    }
    // The lock commands need a value that the phrase does not give. They
    // never become a log command through a variant. The variant of "lock
    // lantern" is the phrase of a log command of the fixture.
    assert_eq!(hit(&p, "log lantern").token, "LanternLog:Open");
    for text in ["lock the boathouse", "unlock the boathouse", "lock lantern"] {
        assert_eq!(
            p.match_result(text),
            MatchResult::Blocked(BlockReason::NoToken),
            "{text}"
        );
    }
    for text in [
        "lock",
        "lock the door",
        "gate lock",
        "lock tide",
        "unlock tide log",
    ] {
        assert_eq!(p.match_text(text), None, "{text}");
    }
}

/// Texts with a table word that must not become another command.
#[test]
fn test_adversarial_probes() {
    let p = fixture_pipeline();
    let none = [
        // No buy or sell command has these words.
        "stand by",
        "cell",
        "by",
        "bye",
        "lamp cell stores",
        "sort by length",
        "sort crates by size",
        "sort nets by closest",
        "source nets by quickest buy",
        "sole trader",
        "higher tide alerts list",
        "set sail",
        "higher spray for cork",
    ];
    for text in none {
        assert_eq!(p.match_text(text), None, "{text}");
    }
    // The text as heard matches. The variant is never tried.
    let literal = [
        ("lamp cell spares", "Spares:Lamp Cell"),
        ("sort nets by length", "Prefs:SortNets=\"2\""),
        ("sort stored crates by size", "Prefs:SortCrates=\"0\""),
        ("higher tide alerts", "Alerts:HigherTide"),
    ];
    for (text, token) in literal {
        let m = hit(&p, text);
        assert_eq!(m.token, token, "{text}");
        assert!(!m.via_sound_alike, "{text}");
    }
    // Never a buy or sell command through a variant.
    for text in ["lamp cell spare", "show the lamp cell", "lamp cell"] {
        if let Some(m) = p.match_text(text) {
            assert!(!m.via_sound_alike, "{text}: {m:?}");
            assert!(
                !m.token.contains("Sell") && !m.token.contains("Buy"),
                "{text}: {m:?}"
            );
        }
    }
}
