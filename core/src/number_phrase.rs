//! The phrase check of a number command.
//!
//! VoiceAttack makes the number text of `{TXTNUM:x}` from the whole
//! command text. It keeps the digits, the `.` and the `-` of that text. The
//! matcher keeps only the ASCII digits of the command text, in phrase
//! order. Thus a phrase with a `.`, a `-` or another kind of numeral
//! outside its number ranges can give another number text in the program
//! than in VoiceAttack. An ASCII digit gives the same text in both. The
//! profile builder checks the literal phrase, and `resolve_runtime_grammar`
//! checks the phrase after it puts in the runtime grammar. Both use
//! [`number_text_outside_ranges`].

use crate::grammar::MAX_GROUP_DEPTH;

/// The reason of a number command that this check leaves out.
pub const NUMBER_TEXT_REASON: &str = "the phrase has a point, a minus sign or a numeral that is \
                                      not an ASCII digit outside its number ranges, the program \
                                      does not know how VoiceAttack writes the number";

/// `lo..hi` or `lo..hi, step` with digits only.
pub(crate) fn is_range(alt: &str) -> bool {
    alt.split(',')
        .next()
        .and_then(|r| r.trim().split_once(".."))
        .is_some_and(|(lo, hi)| {
            !lo.is_empty()
                && !hi.is_empty()
                && lo.bytes().all(|b| b.is_ascii_digit())
                && hi.bytes().all(|b| b.is_ascii_digit())
        })
}

/// A character that VoiceAttack can keep in a number text and that the
/// matcher does not keep in the same way: a `.`, a `-`, or a numeral that
/// is not an ASCII digit. The matcher joins each ASCII digit of the command
/// text in phrase order, as VoiceAttack does. Thus an ASCII digit gives the
/// same number text.
fn differs_in_number_text(c: char) -> bool {
    c == '.' || c == '-' || (c.is_numeric() && !c.is_ascii_digit())
}

/// The phrase without its placeholders (`{...}`). The text of a slot or a
/// variable is not phrase text. An unclosed `{` stays as text.
fn without_placeholders(phrase: &str) -> String {
    let mut out = String::with_capacity(phrase.len());
    let mut rest = phrase;
    while let Some(open) = rest.find('{') {
        let Some(close) = rest[open..].find('}').map(|c| open + c) else {
            break;
        };
        out.push_str(&rest[..open]);
        out.push(' ');
        rest = &rest[close + 1..];
    }
    out.push_str(rest);
    out
}

/// An alternative with a `-` has a twin in its group: another plain
/// alternative of the same group that is equal to it when each `-` is
/// removed or is one space (without regard to case and to the outer
/// spaces). The two spellings sound the same. The alternative must have an
/// ASCII letter (thus the twin has it too), and the twin must have no `-`
/// and no group.
fn has_twin(alt: &str, group: &[&str]) -> bool {
    let letter = |s: &str| s.bytes().any(|b| b.is_ascii_alphabetic());
    let alt = alt.trim();
    if !letter(alt) {
        return false;
    }
    let forms = [alt.replace('-', ""), alt.replace('-', " ")].map(|f| f.trim().to_lowercase());
    // The twin is equal to a form of the alternative, so it has the same
    // letters: one letter check is enough.
    group.iter().any(|other| {
        let other = other.trim();
        !other.contains(['-', '[', ']']) && forms.contains(&other.to_lowercase())
    })
}

/// The end of the group that opens at `open`: the index of its `]`, with
/// regard to groups in the group.
fn group_end(text: &str, open: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (k, c) in text[open..].char_indices() {
        match c {
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    return Some(open + k);
                }
            }
            _ => {}
        }
    }
    None
}

/// The alternatives of a group: split at each `;` that is not in a group
/// in the group.
fn alternatives(inner: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let (mut depth, mut start) = (0usize, 0);
    for (k, c) in inner.char_indices() {
        match c {
            '[' => depth += 1,
            ']' => depth = depth.saturating_sub(1),
            ';' if depth == 0 => {
                out.push(&inner[start..k]);
                start = k + 1;
            }
            _ => {}
        }
    }
    out.push(&inner[start..]);
    out
}

/// A sequence of text and groups has number text outside its ranges.
/// `depth`: the number of open groups around the text. A group deeper than
/// `MAX_GROUP_DEPTH` fails. The check counts the depth itself, so the limit
/// holds for its own reading of the text.
fn sequence_fails(text: &str, depth: usize) -> bool {
    let mut rest = text;
    while let Some(open) = rest.find('[') {
        if rest[..open].contains(differs_in_number_text) {
            return true;
        }
        let Some(close) = group_end(rest, open) else {
            // An unclosed group is text.
            break;
        };
        if depth >= MAX_GROUP_DEPTH || group_fails(&rest[open + 1..close], depth + 1) {
            return true;
        }
        rest = &rest[close + 1..];
    }
    rest.contains(differs_in_number_text)
}

/// A group has an alternative with number text outside its ranges. A
/// plain alternative with only a `-` passes when the group has its twin.
fn group_fails(inner: &str, depth: usize) -> bool {
    let group = alternatives(inner);
    group.iter().any(|alt| {
        if is_range(alt) {
            return false;
        }
        if alt.contains('[') {
            return sequence_fails(alt, depth);
        }
        if !alt.contains(differs_in_number_text) {
            return false;
        }
        let only_minus = !alt.contains(|c: char| c == '.' || c.is_numeric());
        !(only_minus && has_twin(alt, &group))
    })
}

/// True when the phrase has a `.`, a `-` or a numeral that is not an ASCII
/// digit outside its `[a..b]` ranges. One exception: a plain alternative
/// of a `[a; b]` group with a `-` and no digit passes when the group has
/// its twin (see [`has_twin`]). Text in placeholders does not count. A
/// group in a group is read as a group. A phrase with groups deeper than
/// `MAX_GROUP_DEPTH` fails.
pub fn number_text_outside_ranges(phrase: &str) -> bool {
    sequence_fails(&without_placeholders(phrase), 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ranges_and_plain_words_pass() {
        assert!(!number_text_outside_ranges("level [1..9]"));
        assert!(!number_text_outside_ranges("level [1..20, 5] [percent;]"));
        assert!(!number_text_outside_ranges("[a; b] [1..9] [0..9]"));
        assert!(!number_text_outside_ranges("point [0..9][0..9;] degrees"));
        assert!(!number_text_outside_ranges("{TXT:System3} [1..9]"));
        assert!(!number_text_outside_ranges("{TXT:EDCP_LAMP2}"));
        // A group in a group (the runtime grammar of two phrases).
        assert!(!number_text_outside_ranges(
            "[net [0..99]; net [10..140, 10]]"
        ));
        assert!(!number_text_outside_ranges("[a; [b; c]] [1..9]"));
    }

    #[test]
    fn test_point_minus_and_other_numerals_outside_ranges_fail() {
        assert!(number_text_outside_ranges("tides [1.50]"));
        assert!(number_text_outside_ranges("minus-[1..9]"));
        assert!(number_text_outside_ranges("[1.5; 0..9]"));
        assert!(number_text_outside_ranges("shift [-10..10]"));
        assert!(number_text_outside_ranges("deck . [1..9]"));
        // A numeral that is not an ASCII digit.
        assert!(number_text_outside_ranges("crane \u{0663} [1..9]"));
        assert!(number_text_outside_ranges("crane \u{00bd} [1..9]"));
        assert!(number_text_outside_ranges("[a; b\u{00b2}] [1..9]"));
        // An unclosed group is text.
        assert!(number_text_outside_ranges("[a-b [1..9]"));
        // A group in a group.
        assert!(number_text_outside_ranges("[a; [b; c.]] [1..9]"));
        assert!(number_text_outside_ranges("[net [0..9]; net-5]"));
    }

    /// The matcher joins each ASCII digit of the command text in phrase
    /// order, as VoiceAttack does. Thus a digit outside the ranges passes.
    #[test]
    fn test_ascii_digits_outside_ranges_pass() {
        assert!(!number_text_outside_ranges("crane 2 lift [1..9]"));
        assert!(!number_text_outside_ranges("buoy [0;] [1..50]"));
        assert!(!number_text_outside_ranges("buoy [1..50] [0;]"));
        assert!(!number_text_outside_ranges("[a; [b; c2]] [1..9]"));
        assert!(!number_text_outside_ranges("[net [0..9]; net 5]"));
        assert!(!number_text_outside_ranges("launch skiff [1;2]"));
    }

    #[test]
    fn test_a_minus_with_its_twin_passes() {
        assert!(!number_text_outside_ranges("[lamp; lamp-] rate [1..9]"));
        assert!(!number_text_outside_ranges("[ferryman; ferry-man] [1..9]"));
        assert!(!number_text_outside_ranges("[Ferry Man; ferry-man] [1..9]"));
        assert!(!number_text_outside_ranges("[ferry-man; FERRYMAN;] [1..9]"));
        // No twin, or the twin is not in the same group.
        assert!(number_text_outside_ranges("[ferry-man; boatman] [1..9]"));
        assert!(number_text_outside_ranges("ferryman [ferry-man;] [1..9]"));
        assert!(number_text_outside_ranges("ferry-man [1..9]"));
        // Two alternatives with a minus are not twins of each other.
        assert!(number_text_outside_ranges("[ferry-man; ferry--man] [1..9]"));
        // No exception for a point or a digit.
        assert!(number_text_outside_ranges("[mark2; mark-2] [1..9]"));
        assert!(number_text_outside_ranges("[no.; no] [1..9]"));
    }

    /// Groups deeper than the limit fail. The check does not overflow the
    /// stack.
    #[test]
    fn test_deep_groups_fail() {
        let nested = |n: usize| format!("{}lamp{} [1..9]", "[".repeat(n), "]".repeat(n));
        assert!(!number_text_outside_ranges(&nested(MAX_GROUP_DEPTH)));
        assert!(number_text_outside_ranges(&nested(MAX_GROUP_DEPTH + 1)));
        assert!(number_text_outside_ranges(&nested(100_000)));
        // The check removes the placeholders first: a scan of the raw text
        // sees depth 1 here, the check sees 100,000.
        let hidden = format!(
            "{}lamp{} [1..9]",
            "[{]}".repeat(100_000),
            "]".repeat(100_000)
        );
        assert!(number_text_outside_ranges(&hidden));
        let hidden = format!(
            "{}lamp{} [1..9]",
            "[{x}".repeat(100_000),
            "]".repeat(100_000)
        );
        assert!(number_text_outside_ranges(&hidden));
    }

    /// The alternative and its twin each have an ASCII letter. The twin is
    /// not empty, and it is in the same group at the same level.
    #[test]
    fn test_a_twin_needs_letters_and_the_same_group() {
        for phrase in [
            "[-;] [1..9]",
            "[--;] [1..9]",
            "[- ; ] [1..9]",
            "[-; ;] [1..9]",
            "[lamp-; lamp] [-;] [1..9]",
            // The twin in a group in the group.
            "[ferry-man; [ferryman; x]] [1..9]",
            "[[ferry-man; x]; ferryman] [1..9]",
            // The alternative (and thus its twin) has no ASCII letter.
            "[\u{00e9}-; \u{00e9}] [1..9]",
        ] {
            assert!(number_text_outside_ranges(phrase), "{phrase}");
        }
        assert!(!number_text_outside_ranges(
            " [ Ferry-Man ;ferryman ] [1..9]"
        ));
        assert!(!number_text_outside_ranges(
            "[a [ferry-man; ferry man]] [1..9]"
        ));
    }
}
