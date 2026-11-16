//! Filler words: small words of natural speech that the EDCoPilot grammar
//! does not have ("for a", "the", "uh", "please"). The pipeline drops them
//! only when the text as heard and all its other forms give no candidate.

use std::collections::BTreeSet;
use std::path::Path;

use serde::Deserialize;

use crate::matcher::normalize;

/// Filler words that sound like a number, and the number word. A speech
/// engine can write "for" when the speaker said "four".
pub const NUMBER_SOUND_ALIKES: &[(&str, &str)] = &[("for", "four")];

/// The number word of a filler word that sounds like a number.
pub fn number_sound_alike(word: &str) -> Option<&'static str> {
    NUMBER_SOUND_ALIKES
        .iter()
        .find(|(w, _)| *w == word)
        .map(|(_, number)| *number)
}

/// The largest number of variants that one form of a text gives. Three
/// filler words give 7 variants. A form with more filler words gives no
/// variant at all.
pub const MAX_FILLER_VARIANTS: usize = 8;

/// The table of filler words, in lower case. Each word is in one of two
/// groups: a word that may be the last spoken word ("uh", "please"), and a
/// word that may not ("the", "for"). A sentence that stops after "the" is
/// not complete, so that "the" stays.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Fillers {
    not_last: BTreeSet<String>,
    may_be_last: BTreeSet<String>,
}

/// The file format: two lists. Both are necessary. A list can be empty.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FillersFile {
    not_last: Vec<String>,
    may_be_last: Vec<String>,
}

/// True for one word of letters only.
fn is_letter_word(word: &str) -> bool {
    !word.is_empty() && word.chars().all(char::is_alphabetic)
}

/// The usable words of a list, in lower case.
fn clean<'a>(words: impl IntoIterator<Item = &'a str>) -> BTreeSet<String> {
    words
        .into_iter()
        .map(|w| w.trim().to_lowercase())
        .filter(|w| is_letter_word(w))
        .collect()
}

impl Fillers {
    /// A table from two lists. The words change to lower case. A word that
    /// is not one word of letters is left out. A word in both lists counts
    /// as `not_last`.
    pub fn from_lists<'a>(
        not_last: impl IntoIterator<Item = &'a str>,
        may_be_last: impl IntoIterator<Item = &'a str>,
    ) -> Fillers {
        let not_last = clean(not_last);
        let may_be_last = clean(may_be_last)
            .into_iter()
            .filter(|w| !not_last.contains(w))
            .collect();
        Fillers {
            not_last,
            may_be_last,
        }
    }

    /// Parses the TOML text of a filler file. Gives one problem text for
    /// each word that the table leaves out, that is in a list two times, or
    /// that is in both lists.
    pub fn parse_with_problems(text: &str) -> anyhow::Result<(Fillers, Vec<String>)> {
        let file: FillersFile = toml::from_str(text)?;
        let mut problems = Vec::new();
        for (list, words) in [
            ("not_last", &file.not_last),
            ("may_be_last", &file.may_be_last),
        ] {
            let mut seen: BTreeSet<String> = BTreeSet::new();
            for word in words {
                let w = word.trim().to_lowercase();
                if w.is_empty() {
                    problems.push(format!("{list}: filler word is empty"));
                } else if !is_letter_word(&w) {
                    problems.push(format!(
                        "{list}: filler \"{word}\" is not one word of letters, it is left out"
                    ));
                } else if !seen.insert(w) {
                    problems.push(format!(
                        "{list}: filler \"{word}\" is in the list two times"
                    ));
                }
            }
        }
        let table = Fillers::from_lists(
            file.not_last.iter().map(String::as_str),
            file.may_be_last.iter().map(String::as_str),
        );
        for word in clean(file.may_be_last.iter().map(String::as_str)) {
            if table.not_last.contains(&word) {
                problems.push(format!(
                    "filler \"{word}\" is in both lists, it counts as not_last"
                ));
            }
        }
        Ok((table, problems))
    }

    pub fn len(&self) -> usize {
        self.not_last.len() + self.may_be_last.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// True when `word` (lower case) is a filler word.
    pub fn contains(&self, word: &str) -> bool {
        self.not_last.contains(word) || self.may_be_last.contains(word)
    }

    /// True when `word` (lower case) is a filler word that may be the last
    /// spoken word.
    pub fn may_be_last(&self, word: &str) -> bool {
        self.may_be_last.contains(word)
    }

    /// All words in alphabetical order.
    pub fn words(&self) -> Vec<&str> {
        let mut words: Vec<&str> = self
            .not_last
            .iter()
            .chain(&self.may_be_last)
            .map(String::as_str)
            .collect();
        words.sort_unstable();
        words
    }

    /// The words that may not be the last spoken word, in alphabetical order.
    pub fn not_last_words(&self) -> impl Iterator<Item = &str> {
        self.not_last.iter().map(String::as_str)
    }

    /// The words that may be the last spoken word, in alphabetical order.
    pub fn may_be_last_words(&self) -> impl Iterator<Item = &str> {
        self.may_be_last.iter().map(String::as_str)
    }
}

/// Loads a filler file and never fails: a file that is absent or not valid
/// gives an empty table and a problem text.
pub fn load_fillers_with_problems(path: &Path) -> (Fillers, Vec<String>) {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) => {
            let problem = format!("cannot read filler file {}: {e}", path.display());
            return (Fillers::default(), vec![problem]);
        }
    };
    match Fillers::parse_with_problems(&text) {
        Ok((table, problems)) => {
            let problems = problems
                .into_iter()
                .map(|p| format!("{}: {p}", path.display()))
                .collect();
            (table, problems)
        }
        Err(e) => (
            Fillers::default(),
            vec![format!(
                "invalid filler file {}, no filler words: {e}",
                path.display()
            )],
        ),
    }
}

/// The start and the end of the word in a whitespace token, without the
/// punctuation around it.
fn word_bounds(token: &str) -> (usize, usize) {
    let start = token
        .find(|c: char| c.is_alphanumeric())
        .unwrap_or(token.len());
    let end = token
        .rfind(|c: char| c.is_alphanumeric())
        .map_or(start, |i| {
            i + token[i..].chars().next().map_or(1, char::len_utf8)
        });
    (start, end.max(start))
}

/// The word of a whitespace token in lower case, without the punctuation
/// around it.
fn core_word(token: &str) -> String {
    let (start, end) = word_bounds(token);
    token[start..end].to_lowercase()
}

/// One dropped filler word of a variant.
#[derive(Debug, Clone, PartialEq)]
pub struct Dropped {
    /// The position in the normalised words of the text.
    pub pos: usize,
    pub word: String,
    /// The nearest word on each side that is not a filler word, in the
    /// normalised words of the text. Filler words between do not count.
    pub neighbours: Vec<String>,
}

/// One form of a text without some of its filler words.
#[derive(Debug, Clone, PartialEq)]
pub struct FillerVariant {
    pub text: String,
    /// The positions of the kept words in the normalised words of the text
    /// (see `matcher::normalize`).
    pub kept: Vec<usize>,
    /// The dropped filler words.
    pub dropped: Vec<Dropped>,
    /// For each dropped word that sounds like a number ("for" and "four"):
    /// the variant with the number word in place of that one word. The
    /// pipeline uses them as a check: the speaker can have said the number.
    pub number_readings: Vec<String>,
}

impl FillerVariant {
    /// The number of normalised words of the variant.
    pub fn word_count(&self) -> usize {
        self.kept.len()
    }
}

/// The filler variants of a text, or `None` when the text has more filler
/// words than the cap allows.
///
/// `spoken` and `keep` have one flag for each whitespace token of `text`. A
/// token can be dropped only when its word is a filler word, it is spoken,
/// and `keep` is not set. A missing flag counts as not spoken and as keep.
///
/// A filler word of the `not_last` group stays when it is the last spoken
/// word. Words of the `may_be_last` group after it do not count: "lamp off
/// the uh" keeps "the".
///
/// The variants are all the combinations of dropped filler words, fewer
/// dropped words first. A variant never has only filler words or no word.
/// The punctuation after a dropped word moves to the word before it, so
/// that a pause in the text stays.
pub fn filler_variants_with(
    text: &str,
    fillers: &Fillers,
    spoken: &[bool],
    keep: &[bool],
) -> Option<Vec<FillerVariant>> {
    if fillers.is_empty() {
        return Some(Vec::new());
    }
    let tokens: Vec<&str> = text.split_whitespace().collect();
    let words: Vec<String> = tokens.iter().map(|t| core_word(t)).collect();
    // The first normalised word of each token, and its word count.
    let mut first_word = Vec::with_capacity(tokens.len());
    let mut word_counts = Vec::with_capacity(tokens.len());
    let mut next = 0;
    for token in &tokens {
        let count = normalize(token).len();
        first_word.push(next);
        word_counts.push(count);
        next += count;
    }
    let is_filler: Vec<bool> = words.iter().map(|w| fillers.contains(w)).collect();
    let is_spoken = |i: usize| spoken.get(i).copied().unwrap_or(false);
    // The last word. A word that may be last does not count. A word that a
    // synonym or a sound-alike gave counts: it stands for spoken words.
    let last = (0..tokens.len())
        .rev()
        .find(|&i| word_counts[i] > 0 && !(is_filler[i] && fillers.may_be_last(&words[i])));
    let places: Vec<usize> = (0..tokens.len())
        .filter(|&i| {
            is_filler[i]
                && is_spoken(i)
                && !keep.get(i).copied().unwrap_or(true)
                && !(Some(i) == last && !fillers.may_be_last(&words[i]))
        })
        .collect();
    if places.is_empty() {
        return Some(Vec::new());
    }
    // A cut-off list could hide a variant with another token from the tie
    // check of the pipeline. Thus more variants than the cap give none.
    let combinations = 1usize.checked_shl(places.len() as u32).map(|n| n - 1);
    if combinations.is_none_or(|n| n > MAX_FILLER_VARIANTS) {
        return None;
    }
    // The nearest word that is not a filler word, on each side of a token.
    let neighbours = |i: usize| -> Vec<String> {
        let real = |j: &usize| !is_filler[*j] && word_counts[*j] > 0;
        let mut out = Vec::new();
        if let Some(j) = (0..i).rev().find(real) {
            out.push(normalize(tokens[j]).pop().unwrap_or_default());
        }
        if let Some(j) = (i + 1..tokens.len()).find(real) {
            out.push(normalize(tokens[j]).swap_remove(0));
        }
        out
    };
    let mut masks: Vec<u32> = (1..(1u32 << places.len())).collect();
    masks.sort_by_key(|m| (m.count_ones(), *m));
    let mut variants: Vec<FillerVariant> = Vec::new();
    for mask in masks {
        let drop: Vec<bool> = (0..tokens.len())
            .map(|i| {
                places
                    .iter()
                    .position(|&p| p == i)
                    .is_some_and(|bit| mask & (1 << bit) != 0)
            })
            .collect();
        // At least one kept token must have a word that is not a filler.
        let real_word_kept =
            (0..tokens.len()).any(|i| !drop[i] && !is_filler[i] && word_counts[i] > 0);
        if !real_word_kept {
            continue;
        }
        let mut out: Vec<String> = Vec::new();
        let mut kept = Vec::new();
        let mut dropped = Vec::new();
        // The token index and the number word of each dropped word that
        // sounds like a number.
        let mut numbers: Vec<(usize, &str)> = Vec::new();
        for (i, token) in tokens.iter().enumerate() {
            if drop[i] {
                if let Some(number) = number_sound_alike(&words[i]) {
                    numbers.push((i, number));
                }
                dropped.push(Dropped {
                    pos: first_word[i],
                    word: words[i].clone(),
                    neighbours: neighbours(i),
                });
                let (_, end) = word_bounds(token);
                let tail = &token[end..];
                if let Some(last) = out.last_mut() {
                    if !tail.is_empty() && last.ends_with(|c: char| c.is_alphanumeric()) {
                        last.push_str(tail);
                    }
                }
                continue;
            }
            kept.extend(first_word[i]..first_word[i] + word_counts[i]);
            out.push((*token).to_string());
        }
        // The number word, so that the number parser sees "twenty four".
        let number_readings = numbers
            .iter()
            .map(|&(n, number)| {
                let words: Vec<String> = (0..tokens.len())
                    .filter(|&i| !drop[i] || i == n)
                    .map(|i| {
                        if i == n {
                            let (start, end) = word_bounds(tokens[i]);
                            format!("{}{number}{}", &tokens[i][..start], &tokens[i][end..])
                        } else {
                            tokens[i].to_string()
                        }
                    })
                    .collect();
                words.join(" ")
            })
            .collect();
        let text = out.join(" ");
        if !variants.iter().any(|v| v.text == text) {
            variants.push(FillerVariant {
                text,
                kept,
                dropped,
                number_readings,
            });
        }
    }
    Some(variants)
}

/// The filler variants of a text as heard: each filler word can be dropped.
/// More variants than `MAX_FILLER_VARIANTS` give an empty list.
pub fn filler_variants(text: &str, fillers: &Fillers) -> Vec<String> {
    let n = text.split_whitespace().count();
    filler_variants_with(text, fillers, &vec![true; n], &vec![false; n])
        .unwrap_or_default()
        .into_iter()
        .map(|v| v.text)
        .collect()
}

/// How many names a warning shows.
const NAMES_IN_A_WARNING: usize = 3;

/// One warning for each filler word that is a word of a name: the pipeline
/// does not drop the word where the whole name is in the text. The warning
/// names the first three names in alphabetical order and the number of the
/// other names.
pub fn filler_name_warnings<'a>(
    fillers: &Fillers,
    names: impl IntoIterator<Item = &'a str>,
) -> Vec<String> {
    let mut names: Vec<&str> = names.into_iter().collect();
    names.sort_unstable();
    names.dedup();
    let mut warnings = Vec::new();
    for word in fillers.words() {
        let with_word: Vec<&str> = names
            .iter()
            .copied()
            .filter(|name| normalize(name).iter().any(|w| w == word))
            .collect();
        if with_word.is_empty() {
            continue;
        }
        let shown: Vec<String> = with_word
            .iter()
            .take(NAMES_IN_A_WARNING)
            .map(|n| format!("\"{n}\""))
            .collect();
        let others = match with_word.len().saturating_sub(NAMES_IN_A_WARNING) {
            0 => String::new(),
            1 => " and 1 other name".to_string(),
            n => format!(" and {n} other names"),
        };
        warnings.push(format!(
            "filler \"{word}\" is a word of the names {}{others}: the word is not dropped where the name is in the text",
            shown.join(", ")
        ));
    }
    warnings
}

/// The tokens of `text` that are part of a whole name: for each name with a
/// filler word, each place where all words of the name are in the text in
/// this order. One flag for each whitespace token of `text`.
pub fn tokens_in_names(text: &str, fillers: &Fillers, names: &[Vec<String>]) -> Vec<bool> {
    let tokens: Vec<&str> = text.split_whitespace().collect();
    let mut owner: Vec<usize> = Vec::new();
    for (i, token) in tokens.iter().enumerate() {
        owner.extend(std::iter::repeat_n(i, normalize(token).len()));
    }
    let words = normalize(text);
    let mut protected = vec![false; tokens.len()];
    for name in names {
        if name.is_empty() || name.len() > words.len() || !name.iter().any(|w| fillers.contains(w))
        {
            continue;
        }
        for start in 0..=words.len() - name.len() {
            if words[start..start + name.len()] == name[..] {
                for &token in &owner[start..start + name.len()] {
                    protected[token] = true;
                }
            }
        }
    }
    protected
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table() -> Fillers {
        Fillers::from_lists(["a", "an", "the", "for"], ["uh", "um", "please"])
    }

    fn all(text: &str) -> (Vec<bool>, Vec<bool>) {
        let n = text.split_whitespace().count();
        (vec![true; n], vec![false; n])
    }

    fn variants(text: &str) -> Vec<FillerVariant> {
        let (spoken, keep) = all(text);
        filler_variants_with(text, &table(), &spoken, &keep).unwrap()
    }

    #[test]
    fn test_no_filler_word_gives_no_variant() {
        assert!(filler_variants("dock sell cork", &table()).is_empty());
        assert!(filler_variants("", &table()).is_empty());
        assert!(filler_variants("the help", &Fillers::default()).is_empty());
    }

    #[test]
    fn test_one_filler_gives_one_variant() {
        assert_eq!(filler_variants("the help", &table()), vec!["help"]);
        assert_eq!(filler_variants("Uh, help.", &table()), vec!["help."]);
    }

    #[test]
    fn test_all_combinations_fewer_drops_first() {
        assert_eq!(
            filler_variants("Nearest berth for a size 3 old steam crane.", &table()),
            vec![
                "Nearest berth a size 3 old steam crane.",
                "Nearest berth for size 3 old steam crane.",
                "Nearest berth size 3 old steam crane.",
            ]
        );
    }

    #[test]
    fn test_case_and_punctuation_are_ignored_and_pauses_stay() {
        assert_eq!(
            filler_variants("bookmark one, UH, two", &table()),
            vec!["bookmark one, two"]
        );
        assert_eq!(
            filler_variants("Navigate to The Sol.", &table()),
            vec!["Navigate to Sol."]
        );
    }

    #[test]
    fn test_only_fillers_or_nothing_never_remain() {
        assert!(filler_variants("uh the", &table()).is_empty());
        assert!(filler_variants("please", &table()).is_empty());
        assert!(filler_variants("uh ,", &table()).is_empty());
        // "uh" and "um" can go, one word that is no filler stays.
        assert_eq!(
            filler_variants("uh um help", &table()).last().unwrap(),
            "help"
        );
    }

    /// A word of the `not_last` group stays at the end of the text. Words
    /// that may be last after it do not count.
    #[test]
    fn test_not_last_word_at_the_end_stays() {
        assert!(filler_variants("lamp off the", &table()).is_empty());
        assert!(filler_variants("dump nets for", &table()).is_empty());
        assert_eq!(
            filler_variants("lamp off the uh", &table()),
            vec!["lamp off the"]
        );
        assert_eq!(
            filler_variants("sail out for home please", &table()),
            vec!["sail out home please", "sail out for home", "sail out home"]
        );
        // A word that a synonym gave stands for spoken words: "the" is not
        // last, and "for" is no spoken word, so it stays.
        let text = "the sail out for";
        let spoken = [true, false, false, false];
        let keep = [false; 4];
        let v = filler_variants_with(text, &table(), &spoken, &keep).unwrap();
        let texts: Vec<&str> = v.iter().map(|v| v.text.as_str()).collect();
        assert_eq!(texts, vec!["sail out for"]);
        // A word that only the speaker said is last: "the" stays.
        let v = filler_variants_with("lamp off the", &table(), &[true; 3], &[false; 3]);
        assert_eq!(v, Some(Vec::new()));
    }

    /// More variants than the cap give none: a cut-off list could hide a
    /// variant with another token.
    #[test]
    fn test_variants_above_the_cap_give_none() {
        // 3 filler words: 7 variants.
        assert_eq!(filler_variants("a the uh help", &table()).len(), 7);
        // 4 filler words: 15 variants, over the cap.
        assert!(filler_variants("a the uh um help", &table()).is_empty());
        let (spoken, keep) = all("a the uh um help");
        assert_eq!(
            filler_variants_with("a the uh um help", &table(), &spoken, &keep),
            None
        );
        // A filler that cannot be dropped does not count.
        let keep = [true, false, false, false, false];
        assert_eq!(
            filler_variants_with("a the uh um help", &table(), &spoken, &keep)
                .unwrap()
                .len(),
            7
        );
    }

    #[test]
    fn test_positions_and_neighbours_of_dropped_words() {
        let v = variants("sail out to the co-pilot uh home");
        let both = v.last().unwrap();
        assert_eq!(both.text, "sail out to co-pilot home");
        // "co-pilot" is two normalised words.
        assert_eq!(both.kept, vec![0, 1, 2, 4, 5, 7]);
        assert_eq!(both.word_count(), 6);
        assert_eq!(
            both.dropped,
            vec![
                Dropped {
                    pos: 3,
                    word: "the".to_string(),
                    neighbours: vec!["to".to_string(), "co".to_string()],
                },
                Dropped {
                    pos: 6,
                    word: "uh".to_string(),
                    neighbours: vec!["pilot".to_string(), "home".to_string()],
                },
            ]
        );
        // Filler words between do not count.
        let v = variants("bookmark twenty uh for the x");
        let d = &v
            .iter()
            .find(|v| v.text == "bookmark twenty uh the x")
            .unwrap()
            .dropped;
        assert_eq!(d[0].neighbours, vec!["twenty", "x"]);
    }

    /// "for" can be "four": the variant keeps one reading for each dropped
    /// "for", with the number word, so that "twenty for" reads as 24.
    #[test]
    fn test_number_readings_of_a_dropped_for() {
        let v = variants("bookmark twenty for x");
        assert_eq!(v[0].text, "bookmark twenty x");
        assert_eq!(v[0].number_readings, vec!["bookmark twenty four x"]);
        let v = variants("nearest berth size for, uh, steam crane");
        let readings: Vec<(&str, Vec<&str>)> = v
            .iter()
            .map(|v| {
                let r = v.number_readings.iter().map(String::as_str).collect();
                (v.text.as_str(), r)
            })
            .collect();
        assert_eq!(
            readings,
            vec![
                (
                    "nearest berth size, uh, steam crane",
                    vec!["nearest berth size four, uh, steam crane"]
                ),
                ("nearest berth size for, steam crane", vec![]),
                (
                    "nearest berth size, steam crane",
                    vec!["nearest berth size four, steam crane"]
                ),
            ]
        );
        // Two dropped "for": one reading for each, the other one dropped.
        let v = variants("for size for crane");
        let both = v.last().unwrap();
        assert_eq!(both.text, "size crane");
        assert_eq!(
            both.number_readings,
            vec!["four size crane", "size four crane"]
        );
    }

    #[test]
    fn test_names_protect_their_words() {
        let names = vec![
            normalize("The Lantern"),
            normalize("Tom The Lamplighter"),
            normalize("Sol"),
        ];
        assert_eq!(
            tokens_in_names("navigate to the lantern", &table(), &names),
            vec![false, false, true, true]
        );
        assert_eq!(
            tokens_in_names("uh tom the lamplighter please", &table(), &names),
            vec![false, true, true, true, false]
        );
        // Only the whole name protects.
        assert_eq!(
            tokens_in_names("the sol the", &table(), &names),
            vec![false, false, false]
        );
    }

    #[test]
    fn test_name_warnings_are_sorted_and_stable() {
        let names = [
            "Tom The Lamplighter",
            "The Skipper",
            "Sol",
            "The Lantern",
            "The Beta",
            "The Lantern",
        ];
        let warnings = filler_name_warnings(&table(), names);
        assert_eq!(
            warnings,
            vec![
                "filler \"the\" is a word of the names \"The Beta\", \"The Lantern\", \"The Skipper\" and 1 other name: the word is not dropped where the name is in the text"
            ]
        );
        assert!(filler_name_warnings(&table(), ["Sol", "Theta"]).is_empty());
    }

    #[test]
    fn test_file_format_and_problems() {
        let text = "not_last = [\"a\", \"The\", \"the\", \"\", \"two words\"]\n\
                    may_be_last = [\"uh\", \"x1\", \"a\"]\n";
        let (t, problems) = Fillers::parse_with_problems(text).unwrap();
        assert_eq!(t.len(), 3, "{t:?}");
        assert!(t.contains("the"));
        assert!(!t.may_be_last("a"));
        assert!(t.may_be_last("uh"));
        assert_eq!(problems.len(), 5, "{problems:?}");
        assert!(problems.iter().any(|p| p.contains("two times")));
        assert!(problems.iter().any(|p| p.contains("empty")));
        assert!(problems.iter().any(|p| p.contains("not one word")));
        assert!(problems.iter().any(|p| p.contains("both lists")));
        // Both lists are necessary, and no other key is allowed.
        assert!(Fillers::parse_with_problems("not_last = [\"a\"]\n").is_err());
        assert!(Fillers::parse_with_problems("fillers = [\"a\"]\n").is_err());
        assert!(Fillers::parse_with_problems("not_last = []\nmay_be_last = []\nx = 1\n").is_err());
        assert!(
            Fillers::parse_with_problems("not_last = []\nmay_be_last = []\n")
                .unwrap()
                .0
                .is_empty()
        );
    }

    #[test]
    fn test_default_file_loads_without_problems() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("data/fillers.toml");
        let (t, problems) = load_fillers_with_problems(&path);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(
            t.words(),
            vec!["a", "an", "for", "please", "the", "uh", "um"]
        );
        assert!(t.may_be_last("uh") && t.may_be_last("um") && t.may_be_last("please"));
        assert!(!t.may_be_last("the") && !t.may_be_last("for"));
    }

    #[test]
    fn test_missing_or_invalid_file_gives_an_empty_table() {
        let (t, problems) = load_fillers_with_problems(Path::new("no/such/fillers.toml"));
        assert!(t.is_empty());
        assert_eq!(problems.len(), 1);
        let dir = std::env::temp_dir().join(format!("voicebridge-fillers-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let bad = dir.join("bad.toml");
        std::fs::write(&bad, "not_last = [valid").unwrap();
        let (t, problems) = load_fillers_with_problems(&bad);
        assert!(t.is_empty());
        assert_eq!(problems.len(), 1);
        assert!(problems[0].contains("invalid"), "{problems:?}");
        std::fs::remove_dir_all(&dir).ok();
    }
}
