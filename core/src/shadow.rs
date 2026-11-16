//! Working phrases that are also phrases of a command that the program
//! cannot send.
//!
//! The matcher keeps two kinds of phrases as paths without a token (see
//! `MatchResult::Blocked`): the phrase of a value command without a number
//! variable, and the phrase of a left-out command that VoiceAttack
//! recognises by voice (a blocking phrase, see
//! `crate::profile::BlockingPhrase`). A text that is such a phrase and also
//! a phrase of a working command thus gives no command, although the
//! command list says that the working command works. `shadowed_phrases`
//! finds these phrases, so that `check` and the window can show them.

use std::collections::HashSet;

use crate::command_map::{Command, CommandClass, CommandType};
use crate::grammar::{parse_grammar, GrammarNode};
use crate::matcher::normalize;
use crate::profile::BlockingPhrase;

/// The largest number of texts of one phrase. A phrase with more texts is
/// not compared: its commands are not reported.
pub const MAX_EXPANSIONS: usize = 400;

/// A working command with a text that is also a text of a command that
/// the program cannot send.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShadowedPhrase {
    /// The phrase (grammar) of the working command.
    pub phrase: String,
    /// The text of the working command, as its phrase writes it.
    pub text: String,
}

/// True for a command whose phrase the matcher keeps as a path without a
/// token: a value command without a number variable, not a part of a
/// composite group. The same rule as in `Matcher::new`.
fn needs_value(command: &Command) -> bool {
    !command.is_composite_part()
        && command.class == CommandClass::DynamicValue
        && command.number_vars.is_empty()
}

/// The working commands (full commands that do not need a value) with at
/// least one text that is, after `normalize`, also a text of a command that
/// needs a value. One entry per command, with the first such text, in the
/// order of `commands`. See `shadowed_phrases_with` for the blocking
/// phrases too.
///
/// The texts of a phrase are the texts of its literal words, its
/// alternatives, its optional groups and its numbers. A part with a
/// vocabulary slot gives no text: the other alternatives stay. A phrase
/// with more than `MAX_EXPANSIONS` texts is not compared. Thus the result
/// can miss a phrase, but each entry is real.
pub fn shadowed_phrases(commands: &[Command]) -> Vec<ShadowedPhrase> {
    shadowed_phrases_with(commands, &[])
}

/// `shadowed_phrases` with the blocking phrases of the left-out commands:
/// a text of a full blocking phrase counts as a text of a command that the
/// program cannot send. A blocking prefix or suffix pairs with the other
/// parts of its composite group, as in the matcher: the texts of each
/// composite with one or two left-out parts count too (see
/// [`blocked_composite_texts`]).
pub fn shadowed_phrases_with(
    commands: &[Command],
    blocking: &[BlockingPhrase],
) -> Vec<ShadowedPhrase> {
    let mut value_texts: HashSet<String> = commands
        .iter()
        .filter(|c| needs_value(c))
        .filter_map(|c| c.phrase.as_deref())
        .chain(
            blocking
                .iter()
                .filter(|b| b.command_type == CommandType::Full)
                .map(|b| b.phrase.as_str()),
        )
        .filter_map(phrase_texts)
        .flatten()
        .map(|text| key(&text))
        .filter(|k| !k.is_empty())
        .collect();
    value_texts.extend(blocked_composite_texts(commands, blocking));
    if value_texts.is_empty() {
        return Vec::new();
    }
    commands
        .iter()
        .filter(|c| !c.is_composite_part() && !needs_value(c))
        .filter_map(|c| {
            let phrase = c.phrase.as_deref()?;
            let text = phrase_texts(phrase)?
                .into_iter()
                .find(|text| value_texts.contains(&key(text)))?;
            Some(ShadowedPhrase {
                phrase: phrase.to_string(),
                text,
            })
        })
        .collect()
}

/// The keys of the texts of the composites that give no token: a prefix
/// and a suffix of one group, one or both of them a blocking phrase. A
/// group whose prefixes or suffixes have more than `MAX_EXPANSIONS` texts,
/// or whose composites have more than `MAX_EXPANSIONS` texts, is not
/// compared.
fn blocked_composite_texts(commands: &[Command], blocking: &[BlockingPhrase]) -> Vec<String> {
    // For each group: the texts of the prefixes and of the suffixes, each
    // with a flag for a blocking part.
    type Parts = (Vec<(String, bool)>, Vec<(String, bool)>);
    let mut groups: std::collections::BTreeMap<&str, Parts> = Default::default();
    let working = commands.iter().filter_map(|c| {
        Some((
            c.phrase.as_deref()?,
            c.command_type,
            c.composite_group.as_deref()?,
            false,
        ))
    });
    let left_out = blocking.iter().filter_map(|b| {
        Some((
            b.phrase.as_str(),
            b.command_type,
            b.composite_group.as_deref()?,
            true,
        ))
    });
    let mut too_large: HashSet<&str> = HashSet::new();
    for (phrase, command_type, group, blocked) in working.chain(left_out) {
        let side = match command_type {
            CommandType::Prefix => 0,
            CommandType::Suffix => 1,
            CommandType::Full => continue,
        };
        let Some(texts) = phrase_texts(phrase) else {
            too_large.insert(group);
            continue;
        };
        let entry = groups.entry(group).or_default();
        let list = if side == 0 {
            &mut entry.0
        } else {
            &mut entry.1
        };
        list.extend(texts.into_iter().map(|t| (t, blocked)));
        if list.len() > MAX_EXPANSIONS {
            too_large.insert(group);
        }
    }
    let mut out = Vec::new();
    for (group, (prefixes, suffixes)) in groups {
        if too_large.contains(group) {
            continue;
        }
        let pairs: Vec<(&String, &String)> = prefixes
            .iter()
            .flat_map(|(p, pb)| {
                suffixes
                    .iter()
                    .filter(move |(_, sb)| *pb || *sb)
                    .map(move |(s, _)| (p, s))
            })
            .take(MAX_EXPANSIONS + 1)
            .collect();
        if pairs.len() > MAX_EXPANSIONS {
            continue;
        }
        out.extend(
            pairs
                .into_iter()
                .map(|(p, s)| key(&format!("{p} {s}")))
                .filter(|k| !k.is_empty()),
        );
    }
    out
}

/// The normalised words of a text, joined with one space.
fn key(text: &str) -> String {
    normalize(text).join(" ")
}

/// The texts of a phrase, or `None` when the phrase has more
/// than `MAX_EXPANSIONS` texts.
fn phrase_texts(phrase: &str) -> Option<Vec<String>> {
    let texts = expand(&parse_grammar(phrase))?;
    Some(
        texts
            .into_iter()
            .map(|t| t.split_whitespace().collect::<Vec<_>>().join(" "))
            .filter(|t| !t.is_empty())
            .collect(),
    )
}

/// The texts of a node. A slot gives no text. `None` for more than
/// `MAX_EXPANSIONS` texts.
fn expand(node: &GrammarNode) -> Option<Vec<String>> {
    let texts = match node {
        // Range syntax that the parser could not read: the matcher never
        // matches it (`compile`).
        GrammarNode::Literal(text) if text.contains("..") => Vec::new(),
        GrammarNode::Literal(text) => vec![text.clone()],
        GrammarNode::Never => Vec::new(),
        // A slot gives names of the vocabulary, not literal texts: its
        // branch is not compared. The other alternatives stay.
        GrammarNode::ListSlot(_) => Vec::new(),
        GrammarNode::Range(lo, hi) => numbers(*lo, *hi, 1)?,
        GrammarNode::ScaledRange(lo, hi, multiplier) => {
            match (lo.checked_mul(*multiplier), hi.checked_mul(*multiplier)) {
                (Some(lo), Some(hi)) => numbers(lo, hi, *multiplier)?,
                _ => Vec::new(),
            }
        }
        GrammarNode::Optional(inner) => {
            let mut texts = vec![String::new()];
            texts.extend(expand(inner)?);
            texts
        }
        GrammarNode::Alternation(nodes) => {
            let mut texts = Vec::new();
            for n in nodes {
                texts.extend(expand(n)?);
                if texts.len() > MAX_EXPANSIONS {
                    return None;
                }
            }
            texts
        }
        GrammarNode::Sequence(nodes) => {
            let mut texts = vec![String::new()];
            for n in nodes {
                let parts = expand(n)?;
                if texts.len().saturating_mul(parts.len()) > MAX_EXPANSIONS {
                    return None;
                }
                texts = texts
                    .iter()
                    .flat_map(|t| parts.iter().map(move |p| format!("{t} {p}")))
                    .collect();
            }
            texts
        }
    };
    (texts.len() <= MAX_EXPANSIONS).then_some(texts)
}

/// The numbers from `lo` to `hi` in steps of `step`. `None` for more than
/// `MAX_EXPANSIONS` numbers.
fn numbers(lo: u32, hi: u32, step: u32) -> Option<Vec<String>> {
    if hi < lo || step == 0 {
        return Some(Vec::new());
    }
    let count = (hi - lo) / step + 1;
    if count as usize > MAX_EXPANSIONS {
        return None;
    }
    Some((0..count).map(|i| (lo + i * step).to_string()).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::LeftOutClass;

    fn cmd(phrase: &str, template: &str, class: CommandClass) -> Command {
        Command {
            phrase: Some(phrase.to_string()),
            token_template: template.to_string(),
            class,
            ..Default::default()
        }
    }

    fn value(phrase: &str) -> Command {
        cmd(phrase, "Sail={TXT:Trim}", CommandClass::DynamicValue)
    }

    fn fixed(phrase: &str) -> Command {
        cmd(phrase, "SailTrim", CommandClass::Fixed)
    }

    #[test]
    fn test_finds_a_working_text_that_a_value_command_also_has() {
        let commands = vec![
            value("[trim; ease] the [main;] sail"),
            fixed("[set; trim] the Main-Sail"),
            fixed("trim the foresail"),
        ];
        assert_eq!(
            shadowed_phrases(&commands),
            [ShadowedPhrase {
                phrase: "[set; trim] the Main-Sail".into(),
                text: "trim the Main-Sail".into(),
            }]
        );
        // Without the value command: nothing.
        assert!(shadowed_phrases(&commands[1..]).is_empty());
    }

    #[test]
    fn test_numbers_and_optional_words_are_texts() {
        let commands = vec![
            value("reef [the;] sail 2"),
            number_command("reef sail [1..3]"),
            fixed("reef the sail [4..9, 2]"),
        ];
        let found = shadowed_phrases(&commands);
        assert_eq!(
            found,
            [ShadowedPhrase {
                phrase: "reef sail [1..3]".into(),
                text: "reef sail 2".into(),
            }]
        );
        let commands = vec![value("reef the sail 10"), fixed("reef the sail [4..9, 2]")];
        assert_eq!(shadowed_phrases(&commands)[0].text, "reef the sail 10");
    }

    fn number_command(phrase: &str) -> Command {
        Command {
            number_vars: vec!["N".into()],
            ..cmd(phrase, "Reef={TXT:N}", CommandClass::DynamicValue)
        }
    }

    /// A value command with a number variable works: its texts do not
    /// count. A composite part is no full command.
    #[test]
    fn test_only_value_commands_without_a_number_variable_count() {
        let commands = vec![number_command("reef sail [1..3]"), fixed("reef sail 2")];
        assert!(shadowed_phrases(&commands).is_empty());
        let prefix = Command {
            command_type: CommandType::Prefix,
            composite_group: Some("trade".into()),
            ..value("trim the sail")
        };
        assert!(shadowed_phrases(&[prefix, fixed("trim the sail")]).is_empty());
        let suffix = Command {
            command_type: CommandType::Suffix,
            composite_group: Some("trade".into()),
            ..fixed("trim the sail")
        };
        assert!(shadowed_phrases(&[value("trim the sail"), suffix]).is_empty());
    }

    /// A part with a slot gives no text, the other alternatives stay. A
    /// phrase with too many texts is not compared.
    #[test]
    fn test_slots_and_large_phrases_are_not_compared() {
        let commands = vec![value("trim the {TXT:Sail1}"), fixed("trim the mainsail")];
        assert!(shadowed_phrases(&commands).is_empty());
        let commands = vec![value("trim the mainsail"), fixed("trim the {TXT:Sail1}")];
        assert!(shadowed_phrases(&commands).is_empty());
        let commands = vec![
            value("[trim the {TXT:Sail1}; trim the mainsail]"),
            fixed("[set the {TXT:Sail2}; trim the mainsail]"),
        ];
        assert_eq!(shadowed_phrases(&commands)[0].text, "trim the mainsail");
        // 21 * 21 = 441 texts.
        let large = "trim [0..20] [0..20] mainsail";
        let commands = vec![value("trim 1 1 mainsail"), fixed(large)];
        assert!(shadowed_phrases(&commands).is_empty());
        let commands = vec![value(large), fixed("trim 1 1 mainsail")];
        assert!(shadowed_phrases(&commands).is_empty());
        // 20 * 20 = 400 texts: compared.
        let commands = vec![
            value("trim 1 1 mainsail"),
            fixed("trim [1..20] [1..20] mainsail"),
        ];
        assert_eq!(shadowed_phrases(&commands).len(), 1);
        assert!(phrase_texts("[1..400]").is_some());
        assert!(phrase_texts("[0..400]").is_none());
    }

    fn blocking(phrase: &str, command_type: CommandType) -> BlockingPhrase {
        BlockingPhrase {
            phrase: phrase.to_string(),
            class: LeftOutClass::NoWrite,
            command_type,
            composite_group: (command_type != CommandType::Full).then(|| "trade".to_string()),
        }
    }

    /// The phrase of a left-out command shadows a working text as a value
    /// command does. A left-out prefix or suffix does not.
    #[test]
    fn test_a_blocking_phrase_shadows_a_working_text() {
        let commands = vec![fixed("[hoist; lower] the flag"), fixed("ring the bell")];
        let left_out = [blocking("lower [the;] flag", CommandType::Full)];
        assert_eq!(
            shadowed_phrases_with(&commands, &left_out),
            [ShadowedPhrase {
                phrase: "[hoist; lower] the flag".into(),
                text: "lower the flag".into(),
            }]
        );
        assert!(shadowed_phrases(&commands).is_empty());
        for part in [CommandType::Prefix, CommandType::Suffix] {
            let left_out = [blocking("lower the flag", part)];
            assert!(shadowed_phrases_with(&commands, &left_out).is_empty());
        }
    }

    fn part(phrase: &str, command_type: CommandType, group: &str) -> Command {
        Command {
            phrase: Some(phrase.to_string()),
            command_type,
            composite_group: Some(group.to_string()),
            ..Default::default()
        }
    }

    /// A composite of a left-out part and a working part of one group
    /// blocks in the matcher. Its texts shadow a working full command of
    /// the same text. A composite of two working parts does not.
    #[test]
    fn test_a_composite_with_a_left_out_part_shadows_a_working_text() {
        let commands = vec![
            fixed("[sell; buy] cork"),
            part("[sell; offer]", CommandType::Prefix, "trade"),
            part("[cork; rope]", CommandType::Suffix, "trade"),
        ];
        assert!(shadowed_phrases(&commands).is_empty());
        let prefix = [blocking("sell", CommandType::Prefix)];
        assert_eq!(
            shadowed_phrases_with(&commands, &prefix),
            [ShadowedPhrase {
                phrase: "[sell; buy] cork".into(),
                text: "sell cork".into(),
            }]
        );
        let suffix = [blocking("cork", CommandType::Suffix)];
        assert_eq!(
            shadowed_phrases_with(&commands, &suffix)[0].text,
            "sell cork"
        );
        // A left-out part of another group: nothing.
        let other = [BlockingPhrase {
            composite_group: Some("quay".into()),
            ..blocking("sell", CommandType::Prefix)
        }];
        assert!(shadowed_phrases_with(&commands, &other).is_empty());
    }

    /// The texts of a phrase: alternatives, optional groups, and a range
    /// syntax that the parser cannot read.
    #[test]
    fn test_phrase_texts() {
        assert_eq!(
            phrase_texts("[open; show;] the [log; chart]").unwrap(),
            [
                "the log",
                "the chart",
                "open the log",
                "open the chart",
                "show the log",
                "show the chart"
            ]
        );
        assert!(phrase_texts("set [-10..10]").unwrap().is_empty());
    }
}
