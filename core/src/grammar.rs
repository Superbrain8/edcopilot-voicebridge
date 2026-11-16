//! Parser for the VoiceAttack-style phrase grammar used in EDCoPilot command phrases.

#[derive(Debug, Clone, PartialEq)]
pub enum GrammarNode {
    Literal(String),
    Alternation(Vec<GrammarNode>),
    Optional(Box<GrammarNode>),
    Sequence(Vec<GrammarNode>),
    ListSlot(String),
    Range(u32, u32),
    /// `[lo..hi, multiplier]`: the numbers `lo * multiplier` to
    /// `hi * multiplier`, in steps of `multiplier`.
    ScaledRange(u32, u32, u32),
    /// A phrase that the parser does not read (groups deeper than
    /// [`MAX_GROUP_DEPTH`]). It never matches.
    Never,
}

/// The deepest group nesting that a phrase parser reads. A deeper phrase
/// never matches (`parse_grammar`), can give any name (the name check of
/// the profile builder), and fails the number check. The phrases of the
/// tested profile have at most a few levels.
pub const MAX_GROUP_DEPTH: usize = 64;

/// Parses a grammar string into a tree. Never panics; malformed input is
/// handled leniently (unclosed groups close at end of input, stray or
/// unparseable pieces become literal text). A phrase with groups deeper
/// than [`MAX_GROUP_DEPTH`] gives [`GrammarNode::Never`]: the parser counts
/// the depth of its own groups and does not recurse past the limit.
pub fn parse_grammar(input: &str) -> GrammarNode {
    let chars: Vec<char> = input.chars().collect();
    let mut pos = 0;
    let mut too_deep = false;
    let alts = parse_alternatives(&chars, &mut pos, 0, &mut too_deep);
    if too_deep {
        return GrammarNode::Never;
    }
    alternatives_to_node(alts).unwrap_or(GrammarNode::Sequence(vec![]))
}

/// Reads alternatives separated by `;` up to the matching `]` (when `depth`,
/// the number of open groups, is above 0) or the end of input. Each
/// alternative is a list of sequence elements. A group deeper than
/// [`MAX_GROUP_DEPTH`] sets `too_deep` and stops the parse.
fn parse_alternatives(
    chars: &[char],
    pos: &mut usize,
    depth: usize,
    too_deep: &mut bool,
) -> Vec<Vec<GrammarNode>> {
    let in_group = depth > 0;
    let mut alts = Vec::new();
    let mut items: Vec<GrammarNode> = Vec::new();
    let mut text = String::new();

    while *pos < chars.len() {
        let c = chars[*pos];
        *pos += 1;
        match c {
            ';' => {
                flush_text(&mut text, &mut items);
                alts.push(finish_alternative(std::mem::take(&mut items)));
            }
            ']' if in_group => {
                flush_text(&mut text, &mut items);
                alts.push(finish_alternative(items));
                return alts;
            }
            '[' => {
                flush_text(&mut text, &mut items);
                if depth >= MAX_GROUP_DEPTH {
                    *too_deep = true;
                    return alts;
                }
                let inner = parse_alternatives(chars, pos, depth + 1, too_deep);
                if *too_deep {
                    return alts;
                }
                if let Some(node) = alternatives_to_node(inner) {
                    items.push(node);
                }
            }
            '<' | '{' => {
                let close = if c == '<' { '>' } else { '}' };
                match read_slot_name(chars, *pos, close) {
                    Some((name, next)) => {
                        flush_text(&mut text, &mut items);
                        items.push(GrammarNode::ListSlot(name));
                        *pos = next;
                    }
                    None => text.push(c),
                }
            }
            _ => text.push(c),
        }
    }
    flush_text(&mut text, &mut items);
    alts.push(finish_alternative(items));
    alts
}

/// Finds the closing delimiter of a slot starting at `start`. Returns the slot
/// name and the position after the delimiter. Fails if a structural character
/// comes first or the name is empty.
fn read_slot_name(chars: &[char], start: usize, close: char) -> Option<(String, usize)> {
    let mut end = start;
    while end < chars.len() {
        let c = chars[end];
        if c == close {
            let raw: String = chars[start..end].iter().collect();
            let name = raw.trim();
            let name = name.strip_prefix("TXT:").unwrap_or(name).trim();
            if name.is_empty() {
                return None;
            }
            return Some((name.to_string(), end + 1));
        }
        if matches!(c, ';' | '[' | ']' | '<' | '>' | '{' | '}') {
            return None;
        }
        end += 1;
    }
    None
}

fn flush_text(text: &mut String, items: &mut Vec<GrammarNode>) {
    let words: Vec<&str> = text.split_whitespace().collect();
    if !words.is_empty() {
        items.push(GrammarNode::Literal(words.join(" ")));
    }
    text.clear();
}

/// A lone literal of the form `N..M` (N <= M) becomes a range.
fn finish_alternative(items: Vec<GrammarNode>) -> Vec<GrammarNode> {
    if let [GrammarNode::Literal(s)] = items.as_slice() {
        if let Some((lo, hi)) = parse_range(s) {
            return vec![GrammarNode::Range(lo, hi)];
        }
        if let Some((range, multiplier)) = s.split_once(',') {
            let multiplier = multiplier.trim();
            let is_number =
                !multiplier.is_empty() && multiplier.chars().all(|c| c.is_ascii_digit());
            if let (Some((lo, hi)), true) = (parse_range(range), is_number) {
                match multiplier.parse::<u32>() {
                    Ok(m) if m > 0 => return vec![GrammarNode::ScaledRange(lo, hi, m)],
                    _ => {}
                }
            }
        }
    }
    items
}

fn parse_range(s: &str) -> Option<(u32, u32)> {
    let (lo, hi) = s.split_once("..")?;
    let (lo, hi) = (lo.trim(), hi.trim());
    if !lo.chars().all(|c| c.is_ascii_digit()) || !hi.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let (lo, hi) = (lo.parse::<u32>().ok()?, hi.parse::<u32>().ok()?);
    (lo <= hi).then_some((lo, hi))
}

fn sequence_to_node(mut items: Vec<GrammarNode>) -> Option<GrammarNode> {
    match items.len() {
        0 => None,
        1 => items.pop(),
        _ => Some(GrammarNode::Sequence(items)),
    }
}

/// Combines alternatives into one node. An empty alternative makes the whole
/// group optional. Returns `None` when every alternative is empty.
fn alternatives_to_node(alts: Vec<Vec<GrammarNode>>) -> Option<GrammarNode> {
    let has_empty = alts.iter().any(|a| a.is_empty());
    let mut nodes: Vec<GrammarNode> = alts.into_iter().filter_map(sequence_to_node).collect();
    let inner = match nodes.len() {
        0 => return None,
        1 => nodes.pop()?,
        _ => GrammarNode::Alternation(nodes),
    };
    Some(if has_empty {
        GrammarNode::Optional(Box::new(inner))
    } else {
        inner
    })
}

#[cfg(test)]
mod tests {
    use super::GrammarNode::*;
    use super::*;

    fn lit(s: &str) -> GrammarNode {
        Literal(s.to_string())
    }

    #[test]
    fn test_alternation() {
        assert_eq!(
            parse_grammar("[a; b]"),
            Alternation(vec![lit("a"), lit("b")])
        );
    }

    #[test]
    fn test_trailing_semicolon_is_optional() {
        assert_eq!(parse_grammar("[x;]"), Optional(Box::new(lit("x"))));
    }

    #[test]
    fn test_sequence_with_optional() {
        assert_eq!(
            parse_grammar("[sort;] missions"),
            Sequence(vec![Optional(Box::new(lit("sort"))), lit("missions")])
        );
    }

    #[test]
    fn test_list_slot() {
        assert_eq!(parse_grammar("<Ship-Type>"), ListSlot("Ship-Type".into()));
    }

    #[test]
    fn test_range() {
        assert_eq!(parse_grammar("[1..50]"), Range(1, 50));
    }

    /// `[3..49,10]` is 30 to 490 in steps of 10 (VoiceAttack range multiplier).
    #[test]
    fn test_scaled_range() {
        assert_eq!(parse_grammar("[3..49,10]"), ScaledRange(3, 49, 10));
        assert_eq!(parse_grammar("[1..10, 50]"), ScaledRange(1, 10, 50));
        assert_eq!(
            parse_grammar("[0..12, 5;]"),
            Optional(Box::new(ScaledRange(0, 12, 5)))
        );
        assert_eq!(
            parse_grammar("volume [1..20, 5] percent"),
            Sequence(vec![
                Literal("volume".into()),
                ScaledRange(1, 20, 5),
                Literal("percent".into()),
            ])
        );
    }

    #[test]
    fn test_bad_range_multiplier_stays_literal() {
        assert_eq!(parse_grammar("[1..10,0]"), Literal("1..10,0".into()));
        assert_eq!(parse_grammar("[1..10,x]"), Literal("1..10,x".into()));
        assert_eq!(parse_grammar("[1..10,]"), Literal("1..10,".into()));
    }

    #[test]
    fn test_nested() {
        assert_eq!(
            parse_grammar("[is there; show me;][any;] at the quay"),
            Sequence(vec![
                Optional(Box::new(Alternation(vec![lit("is there"), lit("show me")]))),
                Optional(Box::new(lit("any"))),
                lit("at the quay"),
            ])
        );
    }

    #[test]
    fn test_nested_groups_exact_tree() {
        assert_eq!(
            parse_grammar("[a [b; c]; d]"),
            Alternation(vec![
                Sequence(vec![lit("a"), Alternation(vec![lit("b"), lit("c")])]),
                lit("d"),
            ])
        );
        assert_eq!(parse_grammar("[[x;]]"), Optional(Box::new(lit("x"))));
        assert_eq!(
            parse_grammar("[a [b;] c]"),
            Sequence(vec![lit("a"), Optional(Box::new(lit("b"))), lit("c")])
        );
    }

    #[test]
    fn test_txt_slot() {
        assert_eq!(
            parse_grammar("sail out to {TXT:System96}"),
            Sequence(vec![lit("sail out to"), ListSlot("System96".into())])
        );
        assert_eq!(parse_grammar("{CMD}"), ListSlot("CMD".into()));
    }

    #[test]
    fn test_top_level_alternation() {
        assert_eq!(
            parse_grammar("a b; c d"),
            Alternation(vec![lit("a b"), lit("c d")])
        );
    }

    #[test]
    fn test_optional_alternation() {
        assert_eq!(
            parse_grammar("[a; b;]"),
            Optional(Box::new(Alternation(vec![lit("a"), lit("b")])))
        );
    }

    #[test]
    fn test_optional_range() {
        assert_eq!(parse_grammar("[1..30;]"), Optional(Box::new(Range(1, 30))));
    }

    #[test]
    fn test_empty_groups_and_input() {
        assert_eq!(parse_grammar(""), Sequence(vec![]));
        assert_eq!(parse_grammar("[]"), Sequence(vec![]));
        assert_eq!(parse_grammar("[;]"), Sequence(vec![]));
        assert_eq!(parse_grammar("[x]"), lit("x"));
    }

    #[test]
    fn test_adjacent_groups_and_case() {
        assert_eq!(
            parse_grammar("Upper Deck [Ferry;][ Bench;]"),
            Sequence(vec![
                lit("Upper Deck"),
                Optional(Box::new(lit("Ferry"))),
                Optional(Box::new(lit("Bench"))),
            ])
        );
    }

    #[test]
    fn test_real_phrase_with_ranges_and_slot() {
        assert_eq!(
            parse_grammar("remove buoy [1..6] [1..6] from {TXT:Group25}"),
            Sequence(vec![
                lit("remove buoy"),
                Range(1, 6),
                Range(1, 6),
                lit("from"),
                ListSlot("Group25".into()),
            ])
        );
    }

    /// A phrase with groups deeper than the limit never matches, and the
    /// parser does not overflow the stack.
    #[test]
    fn test_deep_groups_give_a_phrase_that_never_matches() {
        let nested = |n: usize| format!("{}harbour{}", "[".repeat(n), "]".repeat(n));
        assert_eq!(parse_grammar(&nested(MAX_GROUP_DEPTH)), lit("harbour"));
        assert_eq!(parse_grammar(&nested(MAX_GROUP_DEPTH + 1)), Never);
        assert_eq!(parse_grammar(&nested(100_000)), Never);
        assert_eq!(parse_grammar(&"[".repeat(100_000)), Never);
        assert_eq!(parse_grammar(&"[{x}".repeat(100_000)), Never);
        // `<a]` is no slot: its `]` closes the group, so the depth stays 1.
        assert_ne!(parse_grammar(&"[<a]".repeat(100_000)), Never);
    }

    #[test]
    fn test_malformed_input_does_not_panic() {
        for s in [
            "[",
            "]",
            "[[",
            "]]",
            "[a; b",
            "a]",
            "<",
            "<abc",
            "{TXT:",
            "{}",
            "<>",
            "[<a;]>",
            "[3..1]",
            "[a..b]",
            "[1..]",
            "..",
            ";;",
            ";",
            "[;;a;;]",
            "<a[b]>",
            "{x;y}",
            "[1..99999999999999]",
        ] {
            let _ = parse_grammar(s);
        }
        assert_eq!(parse_grammar("[3..1]"), lit("3..1"));
        assert_eq!(parse_grammar("a]"), lit("a]"));
        assert_eq!(
            parse_grammar("[a; b"),
            Alternation(vec![lit("a"), lit("b")])
        );
    }
}
