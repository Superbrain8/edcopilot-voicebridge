//! Composite commands (VoiceAttack prefix + suffix) on the synthetic
//! profile. The fixture market group has five prefixes (one without a buy
//! or sell word), suffixes with a plain phrase and with a runtime-grammar
//! phrase, and two full commands with the phrase of a prefix.

mod common;

use common::{fixture_commands_loaded, fixture_matcher};
use edcopilot_voicebridge_core::profile::fixture::CATEGORIES;
use edcopilot_voicebridge_core::Matcher;

fn matcher() -> Matcher {
    fixture_matcher(Default::default())
}

fn token(m: &Matcher, text: &str) -> Option<String> {
    m.match_text(text).map(|hit| hit.token)
}

/// A prefix and a suffix spoken as one sentence give the token of the
/// suffix with the values of the prefix: each prefix, the optional words
/// of the prefix and of the suffix, and an item of two words.
#[test]
fn test_prefix_and_suffix_spoken_as_one_sentence() {
    let m = matcher();
    let cases = [
        ("dock sell cork", "SellDock:Cork"),
        ("give me the dock sell rate for cork", "SellDock:Cork"),
        ("dearest sell bid cork", "SellTop:Cork"),
        ("cheapest purchase bid cork", "BuyTop:Cork"),
        ("nearest harbour to buy cork", "BuyNear:Cork"),
        ("nearest quay sell cork", "SellNear:Cork"),
        ("ways of selling cork", "SellWays:Cork"),
        ("dock buy fish nets", "BuyDock:Fish Nets"),
    ];
    for (text, expected) in cases {
        assert_eq!(token(&m, text).as_deref(), Some(expected), "{text}");
    }
}

/// The prefix `nearest [quay; harbour to;] [sell; ...]` has no "quay to":
/// only the words of the profile grammar match.
#[test]
fn test_prefix_follows_the_profile_grammar() {
    let m = matcher();
    assert_eq!(token(&m, "nearest quay to sell cork"), None);
    assert_eq!(
        token(&m, "nearest sell cork").as_deref(),
        Some("SellNear:Cork")
    );
}

/// The suffix conditions read the spoken prefix: "buying" contains "buy",
/// "sale" is a sell word. A prefix without a buy or sell word leaves the
/// trade side open: no match.
#[test]
fn test_suffix_conditions_read_the_spoken_prefix() {
    let m = matcher();
    assert_eq!(
        token(&m, "ways of buying cork").as_deref(),
        Some("BuyWays:Cork")
    );
    assert_eq!(
        token(&m, "dearest sale bid cork").as_deref(),
        Some("SellTop:Cork")
    );
    assert_eq!(token(&m, "cargo report on cork"), None);
}

/// Suffixes with a runtime-grammar phrase (`{TXT:EDCP_RATE_<item>}`) take
/// part, also with an alternative name of the item.
#[test]
fn test_runtime_grammar_suffix_takes_part() {
    let m = matcher();
    assert_eq!(
        token(&m, "dock sell rope").as_deref(),
        Some("SellDock:Rope")
    );
    assert_eq!(
        token(&m, "dearest sale bid rock salt").as_deref(),
        Some("SellTop:Salt")
    );
    assert_eq!(
        token(&m, "nearest harbour to buy timber").as_deref(),
        Some("BuyNear:Timber")
    );
}

#[test]
fn test_prefix_or_suffix_alone_is_no_command() {
    let m = matcher();
    // Full commands of the fixture have these phrases
    // (`sell at dock; dock sell`, `buy at dock; dock buy`).
    assert_eq!(token(&m, "dock sell").as_deref(), Some("FindDockSell"));
    assert_eq!(token(&m, "dock buy").as_deref(), Some("FindDockBuy"));
    // Only the prefix: no full command has these phrases.
    assert_eq!(token(&m, "dearest sell bid"), None);
    assert_eq!(token(&m, "give me the cheapest buy bid"), None);
    assert_eq!(token(&m, "nearest harbour to buy"), None);
    // Only the suffix.
    assert_eq!(token(&m, "rate for cork"), None);
    assert_eq!(token(&m, "offer cork"), None);
    // A full command of the fixture: `[find; seek;] cork`.
    assert_eq!(token(&m, "cork").as_deref(), Some("SetCargo:Cork"));
}

/// The item word must pass the threshold on its own. "cord" and "fork" are
/// 0.75 from "cork": over the whole sentence they would pass 0.8.
#[test]
fn test_fuzzy_item_word_alone_must_pass_the_threshold() {
    let m = matcher();
    assert_eq!(token(&m, "dock sell cord"), None);
    assert_eq!(token(&m, "dock sell fork"), None);
    let cork = m.match_text("dock sell cork").unwrap();
    assert_eq!(cork.token, "SellDock:Cork");
    assert_eq!(cork.confidence, 1.0);
    assert!(!cork.inexact_single_required_word);
    // An item of two words with one small error is a normal fuzzy phrase.
    let hit = m.match_text("dock sell fish net").unwrap();
    assert_eq!(hit.token, "SellDock:Fish Nets");
    assert!(hit.confidence < 1.0 && hit.confidence >= 0.8);
    assert!(!hit.inexact_single_required_word);
}

/// The confidence of full commands does not change with the composite
/// rules: the mean of the word scores (one word at 0.75 in a phrase of 3
/// and of 5 words).
#[test]
fn test_full_command_fuzzy_confidence_is_unchanged() {
    let m = matcher();
    for (text, expected_token, expected) in [
        ("help tide logg", "Guide:Panel:TideLog", 0.916_666_7_f32),
        ("next tide logg", "TideLog:Next", 0.916_666_7),
        ("back to the tide logs", "OpenTideLogPanel", 0.95),
    ] {
        let hit = m.match_text(text).unwrap();
        assert_eq!(hit.token, expected_token, "{text}");
        assert_eq!(hit.confidence, expected, "{text}");
        assert!(!hit.inexact_single_required_word);
    }
}

/// Optional filler words ("rate", "offer", "for") do not count in the
/// suffix score. A one-word item that is not exact stays flagged, or is
/// below the threshold. Then the one-word rule rejects it.
#[test]
fn test_filler_words_do_not_hide_a_fuzzy_item_word() {
    let m = matcher();
    // Below the threshold: no match.
    for text in [
        "dock sell for cord",
        "dock sell rate cord",
        "dock sell rate for cord",
        "dock sell rate for fork",
        "dock sell offer for cord",
    ] {
        assert_eq!(m.match_text(text), None, "{text}");
    }
    // A match, flagged for the one-word rule.
    for text in ["dock sell for corks", "dock sell rate for corks"] {
        let hit = m.match_text(text).unwrap();
        assert_eq!(hit.token, "SellDock:Cork", "{text}");
        assert!(hit.inexact_single_required_word, "{text}: {hit:?}");
    }
    let cork = m.match_text("dock sell rate for cork").unwrap();
    assert_eq!(cork.token, "SellDock:Cork");
    assert_eq!(cork.confidence, 1.0);
    assert!(!cork.inexact_single_required_word);
    // The name has two words: a normal fuzzy phrase.
    let hit = m.match_text("dock sell for fish net").unwrap();
    assert_eq!(hit.token, "SellDock:Fish Nets");
    assert!(hit.confidence >= 0.8 && hit.confidence < 1.0);
    assert!(!hit.inexact_single_required_word);
    // Only filler after the prefix: no name, no match.
    assert_eq!(token(&m, "dock sell rate for"), None);
}

/// Profile maintenance commands: their write is inside a condition block
/// or comes with key presses. They are not in the map.
#[test]
fn test_profile_maintenance_commands_are_not_in_the_map() {
    let m = matcher();
    for text in ["reset the lamp profile", "rebuild the lamp profile"] {
        assert_eq!(token(&m, text), None, "{text}");
    }
}

/// A category name is no phrase: it gives no token.
#[test]
fn test_category_names_are_not_phrases() {
    let m = matcher();
    let commands = fixture_commands_loaded();
    for category in CATEGORIES {
        assert!(commands
            .iter()
            .any(|c| c.category.as_deref() == Some(category)));
        let text = category.to_lowercase();
        assert_eq!(token(&m, &text), None, "{text}");
    }
}
