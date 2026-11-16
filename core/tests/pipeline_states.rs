//! The pipeline tries the next step (number variants, then sound-alike
//! variants) only when the step before found no candidate. A tie or a
//! blocked best candidate ends the search with no match.

use std::collections::HashMap;

use edcopilot_voicebridge_core::{
    AliasMap, BlockReason, Command, CommandClass, Fillers, MatchResult, Matcher, Pipeline,
    SoundAlikes, Writer,
};

fn cmd(phrase: &str, token: &str) -> Command {
    Command {
        phrase: Some(phrase.to_string()),
        token_template: token.to_string(),
        ..Default::default()
    }
}

fn pipeline(commands: Vec<Command>, table: &[(&str, &str)]) -> Pipeline {
    let matcher = Matcher::new(commands, HashMap::new(), AliasMap::new(), 0.8);
    let writer = Writer::new(std::env::temp_dir().join("voicebridge-never-written.txt"));
    Pipeline::new(matcher, writer).with_sound_alikes(SoundAlikes::from_pairs(table.iter().copied()))
}

/// "close panel" ties A and B. The variant "shut panel" matches only B.
#[test]
fn test_literal_tie_is_not_decided_by_a_sound_alike_variant() {
    let p = pipeline(
        vec![cmd("close panel", "A"), cmd("[close; shut] panel", "B")],
        &[("close", "shut")],
    );
    assert_eq!(p.match_text("close panel"), None);
}

/// "look for boot" is blocked: the only required word "bolt" is 0.75 away.
/// The variant "look for zebra" matches another token.
#[test]
fn test_blocked_literal_is_not_replaced_by_a_sound_alike_variant() {
    let p = pipeline(
        vec![cmd("[look for;] bolt", "A"), cmd("look for zebra", "B")],
        &[("boot", "zebra")],
    );
    assert_eq!(p.match_text("look for boot"), None);
}

#[test]
fn test_no_literal_candidate_lets_the_variant_match() {
    let p = pipeline(
        vec![cmd("[look for;] bolt", "A"), cmd("look for zebra", "B")],
        &[("apple", "zebra")],
    );
    let hit = p.match_text("look for apple").unwrap();
    assert_eq!(hit.token, "B");
    assert!(hit.via_sound_alike);
}

/// A literal tie is not decided by a number variant either.
#[test]
fn test_literal_tie_is_not_decided_by_a_number_variant() {
    let mut number = cmd("volume [1..9]", "C:{TXT:N}");
    number.class = CommandClass::DynamicValue;
    number.number_vars = vec!["N".to_string()];
    let p = pipeline(
        vec![
            cmd("volume five", "A"),
            cmd("[volume; level] five", "B"),
            number,
        ],
        &[],
    );
    assert_eq!(p.match_text("volume five"), None);
    assert_eq!(p.match_text("volume 5").unwrap().token, "C:5");
}

/// The raw variant score 0.83 passes 0.8, but 0.83 x 0.95 does not.
#[test]
fn test_variant_score_must_pass_the_threshold_after_the_factor() {
    let p = pipeline(vec![cmd("zebra gold bolt", "A")], &[("apple", "zebra")]);
    assert_eq!(p.match_text("zebra golf bold").unwrap().token, "A");
    assert_eq!(p.match_text("apple golf bold"), None);
}

/// Several variants with the same token: the highest confidence stays.
/// "zebra mangu" (all replacements) gives 0.9, "zebra mango" gives 1.0.
#[test]
fn test_same_token_variants_keep_the_highest_confidence() {
    let p = pipeline(
        vec![cmd("zebra mango", "A")],
        &[("apple", "zebra"), ("mango", "mangu")],
    );
    let hit = p.match_text("apple mango").unwrap();
    assert_eq!(hit.token, "A");
    assert!((hit.confidence - 0.95).abs() < 1e-6, "{}", hit.confidence);
}

/// The replaced word is the only required word: it counts as not exact.
#[test]
fn test_replaced_single_required_word_is_flagged() {
    let mut command = cmd("[go to; to;] {TXT:System1}", "Jump:{TXT:System1}");
    command.class = CommandClass::DynamicName;
    let vocab = HashMap::from([("System1".to_string(), vec!["Sol".to_string()])]);
    let matcher = Matcher::new(vec![command], vocab, AliasMap::new(), 0.8);
    let writer = Writer::new(std::env::temp_dir().join("voicebridge-never-written.txt"));
    let p = Pipeline::new(matcher, writer)
        .with_sound_alikes(SoundAlikes::from_pairs([("seoul", "sol")]));
    let hit = p.match_text("to Seoul.").unwrap();
    assert_eq!(hit.token, "Jump:Sol");
    assert!(hit.via_sound_alike);
    assert_eq!(hit.required_words, 1);
    assert!(hit.inexact_single_required_word);
    let exact = p.match_text("to sol").unwrap();
    assert!(!exact.inexact_single_required_word);
}

/// Two number readings with different tokens give no match: "pick three
/// four" is "pick 3 4" (A) and "pick 34", where 34 is a name (B).
#[test]
fn test_number_readings_with_different_tokens_give_no_match() {
    let mut a = cmd("pick [1..6] [1..6]", "A:{TXT:N}");
    a.class = CommandClass::DynamicValue;
    a.number_vars = vec!["N".to_string()];
    let mut b = cmd("pick {TXT:Name}", "B:{TXT:Name}");
    b.class = CommandClass::DynamicName;
    let vocab = HashMap::from([("Name".to_string(), vec!["34".to_string()])]);
    let writer = || Writer::new(std::env::temp_dir().join("voicebridge-never-written.txt"));
    let both = Matcher::new(vec![a.clone(), b], vocab, AliasMap::new(), 0.8);
    let p = Pipeline::new(both, writer());
    assert_eq!(p.match_text("pick three four"), None);
    assert_eq!(p.match_text("pick 3 4").unwrap().token, "A:34");

    let one = Matcher::new(vec![a], HashMap::new(), AliasMap::new(), 0.8);
    let p = Pipeline::new(one, writer());
    assert_eq!(p.match_text("pick three four").unwrap().token, "A:34");
}

/// A digit run that lands in a range is no reading: "pick 34" with a
/// range [10..99] does not tie with "pick 3 4".
#[test]
fn test_digit_run_in_a_range_is_no_reading() {
    let number = |phrase: &str, token: &str| {
        let mut c = cmd(phrase, token);
        c.class = CommandClass::DynamicValue;
        c.number_vars = vec!["N".to_string()];
        c
    };
    let both = vec![
        number("pick [1..6] [1..6]", "A:{TXT:N}"),
        number("pick [10..99]", "B:{TXT:N}"),
    ];
    let p = pipeline(both, &[]);
    assert_eq!(p.match_text("pick three four").unwrap().token, "A:34");
}

/// `match_result` tells a blocked text from a text without a candidate.
#[test]
fn test_match_result_shows_a_blocked_text() {
    let p = pipeline(
        vec![cmd("close panel", "A"), cmd("[close; shut] panel", "B")],
        &[("close", "shut")],
    );
    assert_eq!(
        p.match_result("close panel"),
        MatchResult::Blocked(BlockReason::Tie)
    );
    assert_eq!(p.match_result("open the door"), MatchResult::NoCandidate);
    assert_eq!(
        p.match_result("shut panel").into_match().unwrap().token,
        "B"
    );
}

/// A vocabulary reload changes the generation and the names.
#[test]
fn test_vocabulary_reload_counts_and_gives_the_names() {
    let p = pipeline(vec![cmd("go to {TXT:System}", "Go:{TXT:System}")], &[]);
    assert_eq!(p.vocab_generation(), 0);
    assert!(p.names().is_empty());
    let mut vocab = HashMap::new();
    vocab.insert(
        "System".to_string(),
        vec!["Sol".to_string(), "Alioth".to_string()],
    );
    p.set_dynamic_vocab(vocab);
    assert_eq!(p.vocab_generation(), 1);
    let mut names = p.names();
    names.sort();
    assert_eq!(names, ["Alioth", "Sol"]);
}

/// Names and generation come from the same vocabulary. The command words
/// are the literal words of the phrases.
#[test]
fn test_names_and_generation_and_command_words() {
    let p = pipeline(
        vec![cmd("go [to; toward] {TXT:System}", "Go:{TXT:System}")],
        &[],
    );
    assert_eq!(p.names_and_generation(), (Vec::<String>::new(), 0));
    let mut vocab = HashMap::new();
    vocab.insert("System".to_string(), vec!["Sol".to_string()]);
    p.set_dynamic_vocab(vocab);
    assert_eq!(p.names_and_generation(), (vec!["Sol".to_string()], 1));
    assert_eq!(p.command_words(), ["go", "to", "toward"]);
}

fn value_cmd(phrase: &str, token: &str) -> Command {
    Command {
        class: CommandClass::DynamicValue,
        ..cmd(phrase, token)
    }
}

/// Two sound-alike variants of one text: one names a command that needs a
/// value, the other names a working command. The text gives no match, in
/// both orders of the variants.
#[test]
fn test_sound_alike_variant_of_a_command_without_a_token_blocks_the_other_variant() {
    // "toll gong" gives the variants "bell chime", "bell gong" and "toll
    // chime".
    let table = [("toll", "bell"), ("gong", "chime")];
    let working = || cmd("toll chime", "Chime");
    let p = pipeline(vec![working()], &table);
    let hit = p.match_text("toll gong").unwrap();
    assert_eq!(hit.token, "Chime");
    assert!(hit.via_sound_alike);
    for commands in [
        vec![working(), value_cmd("bell gong", "Bell={TXT:Rings}")],
        vec![value_cmd("bell gong", "Bell={TXT:Rings}"), working()],
    ] {
        let p = pipeline(commands, &table);
        assert_eq!(
            p.match_result("toll gong"),
            MatchResult::Blocked(BlockReason::NoToken)
        );
    }
}

/// Two filler variants of one text: one names a command that needs a
/// value, the other names a working command. The text gives no match.
#[test]
fn test_filler_variant_of_a_command_without_a_token_blocks_the_other_variant() {
    let fillers = || Fillers::from_lists(["uh", "the"], []);
    let build = |commands: Vec<Command>| {
        let matcher = Matcher::new(commands, HashMap::new(), AliasMap::new(), 0.8);
        let writer = Writer::new(std::env::temp_dir().join("voicebridge-never-written.txt"));
        Pipeline::new(matcher, writer).with_fillers(fillers())
    };
    // "uh ring the bell" gives the variants "ring the bell", "uh ring
    // bell" and "ring bell".
    let hit = build(vec![cmd("ring bell", "Ring")])
        .match_text("uh ring the bell")
        .unwrap();
    assert_eq!(hit.token, "Ring");
    assert!(hit.via_filler);
    let p = build(vec![
        cmd("ring bell", "Ring"),
        value_cmd("ring the bell", "Bell={TXT:Rings}"),
    ]);
    assert_eq!(
        p.match_result("uh ring the bell"),
        MatchResult::Blocked(BlockReason::NoToken)
    );
}
