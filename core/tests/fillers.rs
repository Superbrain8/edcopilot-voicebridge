//! Filler words in the pipeline, on the synthetic profile and the default
//! tables. The probes check that a dropped word never changes a text into a
//! command that the speaker did not mean.

mod common;

use std::collections::HashMap;

use common::{
    cmd, default_fillers, default_sound_alikes, fixture_commands_loaded, fixture_vocab_with_sol,
    harbour_synonyms, hit, writer,
};
use edcopilot_voicebridge_core::{
    AliasMap, BlockReason, Command, Fillers, MatchResult, Matcher, Pipeline, SoundAlikes, Synonyms,
};

/// The fixture with the default sound-alikes and fillers, and the invented
/// synonym table of the harbour.
fn fixture_pipeline_with(fillers: Fillers) -> Pipeline {
    let matcher = Matcher::new(
        fixture_commands_loaded(),
        fixture_vocab_with_sol(),
        AliasMap::new(),
        0.8,
    );
    Pipeline::new(matcher, writer())
        .with_sound_alikes(default_sound_alikes())
        .with_synonyms(harbour_synonyms())
        .with_fillers(fillers)
}

fn fixture_pipeline() -> Pipeline {
    fixture_pipeline_with(default_fillers())
}

/// A pipeline on test commands with the default filler words.
fn small_pipeline(commands: Vec<Command>) -> Pipeline {
    Pipeline::new(
        Matcher::new(commands, HashMap::new(), AliasMap::new(), 0.8),
        writer(),
    )
    .with_fillers(default_fillers())
}

/// The token of `[nearest berth;] [size;] [1..4;] [old; new; spare;] steam
/// crane` (it has `{CMD}`) for the size 3. Each word before "steam crane"
/// is optional: a dropped "for" next to the size gives a match too.
const BERTH_3: &str = "Berth:Steam Crane~nearest berth size 3 steam crane";

/// A phrase with a number in the middle: "for" and "a" next to it can go,
/// and a sound-alike gives the number word.
#[test]
fn test_log_examples_reach_the_berth_command() {
    let p = fixture_pipeline();
    let m = hit(&p, "Nearest berth for a size 3 steam crane.");
    assert_eq!(m.token, BERTH_3);
    assert!(m.via_filler);
    assert!(!m.via_sound_alike);
    assert!(!m.via_synonym);
    assert!((m.confidence - 0.95).abs() < 1e-6, "{}", m.confidence);
    assert!(!m.inexact_single_required_word);

    let m = hit(&p, "Nearest berth for uh size free steam crane.");
    assert_eq!(m.token, BERTH_3);
    assert!(m.via_filler);
    assert!(m.via_sound_alike);
    assert!(
        (m.confidence - 0.95 * 0.95).abs() < 1e-6,
        "{}",
        m.confidence
    );
}

/// Without the filler words, both texts give no match.
#[test]
fn test_log_examples_need_the_filler_step() {
    let p = fixture_pipeline_with(Fillers::default());
    assert_eq!(
        p.match_text("Nearest berth for a size 3 steam crane."),
        None
    );
    assert_eq!(
        p.match_text("Nearest berth for uh size free steam crane."),
        None
    );
}

#[test]
fn test_text_as_heard_goes_first() {
    let p = fixture_pipeline();
    let m = hit(&p, "nearest berth size 3 steam crane");
    assert_eq!(m.token, BERTH_3);
    assert!(!m.via_filler);
    assert_eq!(m.confidence, 1.0);
    // The phrases need "for" and "the": they match as heard.
    let m = hit(&p, "head for the lighthouse");
    assert!(!m.via_filler);
    let m = hit(&p, "show the tide log");
    assert!(!m.via_filler);
    let m = hit(&p, "sail for home port");
    assert_eq!(m.token, "SailHomePort");
    assert!(!m.via_filler);
}

/// A filler word that the phrase needs stays in the variant that matches.
#[test]
fn test_a_needed_filler_word_stays() {
    let p = fixture_pipeline();
    let m = hit(&p, "uh head for the lighthouse");
    assert!(m.via_filler);
    let literal = hit(&p, "head for the lighthouse");
    assert_eq!(m.token, literal.token);
}

/// Two variants with different tokens give no match.
#[test]
fn test_variants_with_different_tokens_give_no_match() {
    let both = small_pipeline(vec![cmd("alpha for beta", "A"), cmd("alpha beta", "B")]);
    assert_eq!(both.match_text("alpha for the beta"), None);
    assert_eq!(
        both.match_result("alpha for the beta"),
        MatchResult::Blocked(BlockReason::Forms)
    );
    let one = small_pipeline(vec![cmd("alpha for beta", "A")]);
    let m = hit(&one, "alpha for the beta");
    assert_eq!(m.token, "A");
    assert!(m.via_filler);
}

/// A blocked text as heard ends the search: the filler step never decides.
#[test]
fn test_blocked_earlier_ends_the_search() {
    let p = small_pipeline(vec![
        cmd("the panel", "A"),
        cmd("[the; a] panel", "B"),
        cmd("panel", "C"),
    ]);
    assert_eq!(
        p.match_result("the panel"),
        MatchResult::Blocked(BlockReason::Tie)
    );
    // Without the tie, the variant "panel" alone gives C.
    let p = small_pipeline(vec![cmd("panel", "C"), cmd("open panel", "D")]);
    assert_eq!(hit(&p, "the panel").token, "C");
}

/// A sound-alike candidate below the threshold ends the search too.
#[test]
fn test_sound_alike_candidate_below_the_threshold_ends_the_search() {
    let make = |commands: Vec<Command>| {
        Pipeline::new(
            Matcher::new(commands, HashMap::new(), AliasMap::new(), 0.8),
            writer(),
        )
        .with_sound_alikes(SoundAlikes::from_pairs([("cell", "sell")]))
        .with_fillers(default_fillers())
    };
    let text = "cell the abcx efgx ijkx mnox";
    // As heard, A is (0.75 + 1 + 4 x 0.75) / 6 = 0.79: no candidate. The
    // sound-alike form "sell the ..." reaches A at 0.83, and the factor puts
    // it at 0.79, below the threshold. The search ends there.
    let p = make(vec![
        cmd("sell the abcd efgh ijkl mnop", "A"),
        cmd("sell abcx efgx ijkx mnox", "B"),
    ]);
    assert_eq!(p.match_result(text), MatchResult::NoCandidate);
    // Without A, the filler step drops "the" and finds B.
    let p = make(vec![cmd("sell abcx efgx ijkx mnox", "B")]);
    let m = hit(&p, text);
    assert_eq!(m.token, "B");
    assert!(m.via_filler);
}

/// The names with "the" match as heard and with filler words around them.
#[test]
fn test_names_with_the() {
    let p = fixture_pipeline();
    for name in ["the old mill", "tam the ferryman"] {
        let literal = hit(&p, &format!("sail to {name}"));
        assert!(!literal.via_filler, "{name}");
        let m = hit(&p, &format!("make for {name}"));
        assert_eq!(m.token, literal.token, "{name}");
        assert!(!m.via_filler, "{name}");
        let m = hit(&p, &format!("uh sail to {name} please"));
        assert_eq!(m.token, literal.token, "{name}");
        assert!(m.via_filler, "{name}");
        assert!(!m.inexact_single_required_word, "{name}");
    }
    let m = hit(&p, "Make for the old mill.");
    assert!(m.via_synonym);
    assert!(!m.via_filler);
}

/// Dropping never makes a name that was not in the text.
#[test]
fn test_dropping_makes_no_new_name() {
    let p = fixture_pipeline();
    for text in [
        "sail to old mill",
        "sail to uh old mill",
        "sail to tam ferryman",
        "sail to uh tam ferryman",
        "sail to the tam ferryman",
    ] {
        assert_eq!(p.match_text(text), None, "{text}");
    }
}

/// A dropped letter next to a name gives no match: "a" can be part of a
/// name that the vocabulary does not have.
#[test]
fn test_letter_next_to_a_name_gives_no_match() {
    let p = fixture_pipeline();
    assert_eq!(
        hit(&p, "sail to reef 2207").token,
        "HarbourAt:Reef 2207:sail to"
    );
    for text in [
        "sail to reef 2207 a",
        "sail to a morrow",
        "uh sail to morrow a",
    ] {
        assert_eq!(p.match_text(text), None, "{text}");
    }
    // "the" before a name is no letter.
    let m = hit(&p, "sail to the morrow");
    assert_eq!(m.token, "HarbourAt:Morrow:sail to");
    assert!(m.via_filler);
    assert!(!m.inexact_single_required_word);
}

/// "for" can be "four". When the digit gives a command too, the dropped
/// "for" is not clear: no match.
#[test]
fn test_for_that_can_be_four_gives_no_match() {
    let p = fixture_pipeline();
    for text in [
        "nearest berth size for steam crane",
        "nearest berth for a size for steam crane",
        "net reminder for",
        "mooring for",
    ] {
        assert_eq!(p.match_text(text), None, "{text}");
    }
    // "for" is not next to a number or a name here: it can go.
    let m = hit(&p, "Nearest berth for a size 3 steam crane.");
    assert_eq!(m.token, BERTH_3);
    let m = hit(&p, "Nearest berth for uh size free steam crane.");
    assert_eq!(m.token, BERTH_3);
}

/// "twenty for" can be "twenty four". These texts gave a wrong number
/// before the reading used the word "four" and before the rule "next to a
/// number or a name".
#[test]
fn test_for_after_a_tens_word_gives_no_match() {
    let p = fixture_pipeline();
    for text in [
        "delete mooring twenty for",
        "delete mooring group thirty for",
        "move the ferry to mooring twenty for",
        "net reminder twenty for",
        "mooring twenty for",
        "add lamp oil to the store by twenty for barrels",
        "set crate count to twenty for",
        "fog horn volume twenty for",
        "remove crate twenty for",
        "delete mooring 20 for",
        "net reminder 40 for",
        "mooring 20 for",
        "move the ferry to mooring 20 for please",
    ] {
        assert_eq!(p.match_text(text), None, "{text}");
    }
}

/// A dropped "for" next to a number or a name gives no match, on both
/// sides, also with other filler words between.
#[test]
fn test_for_next_to_a_number_or_a_name_gives_no_match() {
    let p = fixture_pipeline();
    for text in [
        "fog horn volume for 50",
        "net reminder for 120",
        "move the ferry to mooring for 2",
        "sail to reef 2207 for",
        "sail to reef for 2207",
        "sail to ktl 3081 for",
        "sail to ktl 3081 for please",
        "fog horn volume for uh 50",
        "fog horn volume fifty for please",
        "sail to morrow for please",
    ] {
        assert_eq!(p.match_text(text), None, "{text}");
    }
}

/// A sentence that stops after "a", "an", "the" or "for" is not complete:
/// such a last word stays, and the text gives no match.
#[test]
fn test_cut_off_sentence_gives_no_match() {
    let p = fixture_pipeline();
    for text in [
        "lamp off the",
        "delete mooring the",
        "belay the",
        "dump the nets for",
        "sail to morrow the",
        "lamp off the uh",
    ] {
        assert_eq!(p.match_text(text), None, "{text}");
    }
    // Words that may be last can go at the end.
    let m = hit(&p, "sail for home port please");
    assert_eq!(m.token, "SailHomePort");
    assert!(m.via_filler);
    let m = hit(&p, "uh sail for home port um");
    assert_eq!(m.token, "SailHomePort");
}

/// A dropped letter next to a number gives no match, as next to a name.
#[test]
fn test_letter_next_to_a_number_gives_no_match() {
    let p = fixture_pipeline();
    for text in [
        "delete mooring 2 a",
        "delete mooring a 2",
        "net reminder twenty a",
        "move the ferry to mooring 2 a",
        "move the ferry to mooring 2 a please",
    ] {
        assert_eq!(p.match_text(text), None, "{text}");
    }
    // The cause of the block.
    assert_eq!(
        p.match_result("delete mooring a 2"),
        MatchResult::Blocked(BlockReason::DroppedWord)
    );
}

/// A text as heard below the threshold has no candidate: the matcher does
/// not tell how close it came, so the later steps run. This pins today's
/// behaviour, so that a change is deliberate.
#[test]
fn test_as_heard_candidate_below_the_threshold_does_not_end_the_search() {
    let text = "the abcd efgh ijkl mnop qrst";
    let p = small_pipeline(vec![
        // As heard: (1 + 5 x 0.75) / 6 = 0.79, below the threshold.
        cmd("the abcx efgx ijkx mnox qrsx", "A"),
        cmd("abcd efgh ijkl mnop qrst", "B"),
    ]);
    let m = hit(&p, text);
    assert_eq!(m.token, "B");
    assert!(m.via_filler);
}

/// More variants than the cap over all forms give no filler step.
#[test]
fn test_total_cap_over_all_forms() {
    let synonyms = Synonyms::from_pairs([("go", vec!["alpha", "beta", "gamma", "delta", "omega"])]);
    let make = |synonyms: Synonyms| {
        Pipeline::new(
            Matcher::new(
                vec![cmd("alpha jump", "J")],
                HashMap::new(),
                AliasMap::new(),
                0.8,
            ),
            writer(),
        )
        .with_synonyms(synonyms)
        .with_fillers(default_fillers())
    };
    // 6 forms (as heard and 5 synonym values) x 7 variants = 42: over 32.
    let p = make(synonyms);
    assert_eq!(p.match_text("uh um the go jump"), None);
    // 6 forms x 3 variants = 18: under the cap.
    let m = hit(&p, "uh um go jump");
    assert_eq!(m.token, "J");
    // Without the synonyms, the same text with 3 filler words works.
    let p = make(Synonyms::default());
    assert_eq!(hit(&p, "uh um the alpha jump").token, "J");
}

/// Synonym, filler and sound-alike together.
#[test]
fn test_synonym_filler_and_sound_alike_together() {
    let p = fixture_pipeline();
    let m = hit(&p, "Now we head to the mooring free.");
    assert_eq!(m.token, "SelectMooring:3");
    assert!(m.via_synonym);
    assert!(m.via_filler);
    assert!(m.via_sound_alike);
    let m = hit(&p, "make for the morrow please");
    assert_eq!(m.token, "HarbourAt:Morrow:sail to");
    assert!(m.via_synonym);
    assert!(m.via_filler);
    assert!(!m.via_sound_alike);
    // "Seoul" is a sound-alike of "Sol": a one-word name, so the match is
    // flagged.
    let m = hit(&p, "make for the Seoul");
    assert_eq!(m.token, "HarbourAt:Sol:sail to");
    assert!(m.via_filler && m.via_sound_alike && m.via_synonym);
    assert!(m.inexact_single_required_word);
}

/// A text with more filler words than the cap gets no filler step.
#[test]
fn test_cap_on_filler_words() {
    let p = small_pipeline(vec![cmd("hyperspace jump", "H")]);
    let m = hit(&p, "uh um the hyperspace jump");
    assert_eq!(m.token, "H");
    assert_eq!(p.match_text("uh um the a hyperspace jump"), None);
}

#[test]
fn test_only_filler_words_give_no_match() {
    let p = fixture_pipeline();
    for text in ["uh", "please", "uh the", "the a for", "um, please."] {
        assert_eq!(p.match_text(text), None, "{text}");
    }
}

/// One word that stays must be exact.
#[test]
fn test_one_remaining_word_must_be_exact() {
    let p = small_pipeline(vec![cmd("hyperspace", "H")]);
    let m = hit(&p, "the hyperspace");
    assert_eq!(m.token, "H");
    assert!(m.via_filler);
    assert!(!m.inexact_single_required_word);
    let m = hit(&p, "uh hyperspaxe");
    assert!(m.inexact_single_required_word);
    // An optional phrase of one word follows the same rule.
    let p = small_pipeline(vec![cmd("[hyperspace]", "H")]);
    let m = hit(&p, "uh hyperspaxe");
    assert!(m.inexact_single_required_word);
}

/// Texts without filler words give the same result with and without the
/// filler table.
#[test]
fn test_no_change_without_filler_words() {
    let with = fixture_pipeline();
    let without = fixture_pipeline_with(Fillers::default());
    for text in [
        "nearest berth size 3 steam crane",
        "Nearest berth size free steam crane.",
        "make for Sol",
        "Now we head to mooring free.",
        "sail to morow",
        "sail to reef 2207",
        "dock sell cork",
        "cheapest by bid cork",
        "fog horn volume fifty five",
        "net reminder free",
        "open tide lock",
        "free",
        "sail to old mill",
        "some words that match nothing",
    ] {
        assert_eq!(
            with.match_result(text),
            without.match_result(text),
            "{text}"
        );
    }
}

/// Composite commands: a filler word inside the phrase of a prefix or a
/// suffix.
#[test]
fn test_composite_commands_with_filler_words() {
    let p = fixture_pipeline();
    let literal = hit(&p, "dock sell cork");
    assert_eq!(literal.token, "SellDock:Cork");
    for text in [
        "dock sell for cork",
        "dock sell the cork",
        "uh dock sell cork",
    ] {
        let m = hit(&p, text);
        assert_eq!(m.token, "SellDock:Cork", "{text}");
    }
}
