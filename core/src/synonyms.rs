//! Phrase synonyms: the user's own words for a phrase of EDCoPilot
//! ("head for" for "sail out for"). A synonym works in every command
//! that uses the phrase.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;

use crate::command_map::Command;

/// The largest number of variants that `synonym_variants` gives.
pub const MAX_SYNONYM_VARIANTS: usize = 8;

/// One synonym: the spoken words and the EDCoPilot phrases they stand for.
#[derive(Debug, Clone, PartialEq)]
pub struct Synonym {
    /// The spoken phrase, in lower case.
    pub key: String,
    /// The EDCoPilot phrases, in lower case, in file order.
    pub values: Vec<String>,
    key_words: Vec<String>,
}

/// The table of synonyms.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Synonyms {
    entries: Vec<Synonym>,
}

/// A value: one phrase or a list of phrases.
#[derive(Deserialize)]
#[serde(untagged)]
enum Values {
    One(String),
    Many(Vec<String>),
}

/// The file format: one table `[synonyms]`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SynonymsFile {
    synonyms: BTreeMap<String, Values>,
}

/// The words of a phrase in lower case. `None` when a word has a character
/// that is not a letter, a digit, an apostrophe or a hyphen.
fn phrase_words(phrase: &str) -> Option<Vec<String>> {
    let words: Vec<String> = phrase.split_whitespace().map(str::to_lowercase).collect();
    let valid = |w: &String| {
        w.chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '\'' | '-'))
    };
    words.iter().all(valid).then_some(words)
}

impl Synonyms {
    /// A table from `(key, values)` pairs. A pair with an empty or invalid
    /// key, or a value that is empty, invalid or equal to the key, is left out.
    pub fn from_pairs<'a>(pairs: impl IntoIterator<Item = (&'a str, Vec<&'a str>)>) -> Synonyms {
        let mut entries = Vec::new();
        for (key, values) in pairs {
            if let Some(entry) = Synonym::new(key, &values).0 {
                entries.push(entry);
            }
        }
        Synonyms::sorted(entries)
    }

    /// Parses the TOML text of a synonym file. Gives one problem text for
    /// each entry or value that the table leaves out.
    pub fn parse_with_problems(text: &str) -> anyhow::Result<(Synonyms, Vec<String>)> {
        let file: SynonymsFile = toml::from_str(text)?;
        let mut entries: Vec<Synonym> = Vec::new();
        let mut problems = Vec::new();
        // The original text of each key, by its normalised key.
        let mut first_key: BTreeMap<String, &str> = BTreeMap::new();
        for (key, values) in &file.synonyms {
            let values: Vec<&str> = match values {
                Values::One(v) => vec![v.as_str()],
                Values::Many(v) => v.iter().map(String::as_str).collect(),
            };
            let (entry, entry_problems) = Synonym::new(key, &values);
            problems.extend(entry_problems);
            let Some(entry) = entry else {
                continue;
            };
            // Keys that differ only in case are one key: merge the values.
            if let Some(other) = first_key.get(&entry.key) {
                problems.push(format!(
                    "synonyms \"{other}\" and \"{key}\" differ only in case, their values are merged"
                ));
                let kept = entries
                    .iter_mut()
                    .find(|e| e.key == entry.key)
                    .expect("the first key has an entry");
                for value in entry.values {
                    if !kept.values.contains(&value) {
                        kept.values.push(value);
                    }
                }
                continue;
            }
            first_key.insert(entry.key.clone(), key);
            entries.push(entry);
        }
        Ok((Synonyms::sorted(entries), problems))
    }

    /// The table with the longest key first, then by key.
    fn sorted(mut entries: Vec<Synonym>) -> Synonyms {
        entries.sort_by(|a, b| {
            b.key_words
                .len()
                .cmp(&a.key_words.len())
                .then_with(|| a.key.cmp(&b.key))
        });
        Synonyms { entries }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The entries, longest key first (the order in which the matcher tries
    /// them), then by key.
    pub fn entries(&self) -> &[Synonym] {
        &self.entries
    }

    /// The longest key that matches `words` at the start. `words` are lower case.
    fn longest_at(&self, words: &[String]) -> Option<&Synonym> {
        self.entries
            .iter()
            .filter(|e| words.starts_with(&e.key_words))
            .max_by_key(|e| e.key_words.len())
    }
}

impl Synonym {
    /// The entry, or `None` with the problem texts.
    fn new(key: &str, values: &[&str]) -> (Option<Synonym>, Vec<String>) {
        let mut problems = Vec::new();
        let key_text = key.trim().to_lowercase();
        let key_words = match phrase_words(&key_text) {
            Some(words) if !words.is_empty() => words,
            Some(_) => return (None, vec!["synonym with an empty key".to_string()]),
            None => {
                return (
                    None,
                    vec![format!(
                        "synonym \"{key}\" has punctuation in a word (only ' and - are allowed)"
                    )],
                )
            }
        };
        let key_text = key_words.join(" ");
        let mut kept = Vec::new();
        for value in values {
            match phrase_words(value) {
                Some(words) if words.is_empty() => {
                    problems.push(format!("synonym \"{key}\" has an empty value"));
                }
                Some(words) if words == key_words => {
                    problems.push(format!("synonym \"{key}\" maps to itself"));
                }
                Some(words) => {
                    let value = words.join(" ");
                    if !kept.contains(&value) {
                        kept.push(value);
                    }
                }
                None => problems.push(format!(
                    "synonym \"{key}\" = \"{value}\": punctuation in a word (only ' and - are allowed)"
                )),
            }
        }
        if kept.is_empty() {
            problems.push(format!("synonym \"{key}\" has no usable value"));
            return (None, problems);
        }
        let entry = Synonym {
            key: key_text,
            values: kept,
            key_words,
        };
        (Some(entry), problems)
    }
}

/// Loads a synonym file and never fails: a file that is absent or not valid
/// gives an empty table and a problem text.
pub fn load_synonyms_with_problems(path: &Path) -> (Synonyms, Vec<String>) {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) => {
            let problem = format!("cannot read synonym file {}: {e}", path.display());
            return (Synonyms::default(), vec![problem]);
        }
    };
    match Synonyms::parse_with_problems(&text) {
        Ok((table, problems)) => {
            let problems = problems
                .into_iter()
                .map(|p| format!("{}: {p}", path.display()))
                .collect();
            (table, problems)
        }
        Err(e) => (
            Synonyms::default(),
            vec![format!(
                "invalid synonym file {}, no synonyms: {e}",
                path.display()
            )],
        ),
    }
}

/// The word of a whitespace token in lower case, without the punctuation
/// around it (apostrophes and hyphens inside the word stay).
fn core_word(token: &str) -> String {
    token
        .trim_matches(|c: char| !c.is_alphanumeric())
        .to_lowercase()
}

/// Variants of `text` in which each synonym key is replaced by its values.
///
/// Keys match whole words, without regard to case and the punctuation
/// around a word. At each position the longest key wins. With more than one
/// place, the variants are the combinations of the values, in file order.
/// More than `MAX_SYNONYM_VARIANTS` combinations give no variant at all. No
/// variant equals the original text.
pub fn synonym_variants(text: &str, table: &Synonyms) -> Vec<String> {
    synonym_variants_spoken(text, table)
        .into_iter()
        .map(|(variant, _)| variant)
        .collect()
}

/// As `synonym_variants`, with one flag for each whitespace token of the
/// variant: true for a token of the text as heard, false for a token that a
/// synonym value gave.
pub fn synonym_variants_spoken(text: &str, table: &Synonyms) -> Vec<(String, Vec<bool>)> {
    if table.is_empty() {
        return Vec::new();
    }
    let tokens: Vec<&str> = text.split_whitespace().collect();
    let words: Vec<String> = tokens.iter().map(|t| core_word(t)).collect();
    // (first token, token count, entry)
    let mut places: Vec<(usize, usize, &Synonym)> = Vec::new();
    let mut i = 0;
    while i < words.len() {
        match table.longest_at(&words[i..]) {
            Some(entry) => {
                places.push((i, entry.key_words.len(), entry));
                i += entry.key_words.len();
            }
            None => i += 1,
        }
    }
    if places.is_empty() {
        return Vec::new();
    }
    // More combinations than the cap give no variant. A cut-off list could
    // hide a variant with another token from the tie check of the pipeline.
    let combinations = places
        .iter()
        .try_fold(1usize, |n, (_, _, e)| n.checked_mul(e.values.len()));
    if combinations.is_none_or(|n| n > MAX_SYNONYM_VARIANTS) {
        return Vec::new();
    }
    let original = tokens.join(" ");
    let mut variants: Vec<(String, Vec<bool>)> = Vec::new();
    // An odometer over the value indexes. The first place changes slowest.
    let mut choice = vec![0usize; places.len()];
    loop {
        let mut out: Vec<&str> = Vec::new();
        let mut spoken: Vec<bool> = Vec::new();
        let mut next = 0;
        for (place, &(start, len, entry)) in places.iter().enumerate() {
            out.extend(&tokens[next..start]);
            spoken.extend(std::iter::repeat_n(true, start - next));
            let value = entry.values[choice[place]].as_str();
            out.push(value);
            spoken.extend(std::iter::repeat_n(false, value.split_whitespace().count()));
            next = start + len;
        }
        out.extend(&tokens[next..]);
        spoken.extend(std::iter::repeat_n(true, tokens.len() - next));
        let variant = out.join(" ");
        if variant != original && variant != text && !variants.iter().any(|(v, _)| *v == variant) {
            variants.push((variant, spoken));
        }
        // Advance the odometer.
        let mut place = places.len();
        loop {
            if place == 0 {
                return variants;
            }
            place -= 1;
            choice[place] += 1;
            if choice[place] < places[place].2.values.len() {
                break;
            }
            choice[place] = 0;
        }
    }
}

/// The words of a grammar phrase: lower case, without grammar characters.
/// Slot and variable names (`{TXT:System1}`, `<Commodity>`) are no words.
fn grammar_words(phrase: &str) -> Vec<String> {
    let mut text = String::with_capacity(phrase.len());
    let mut depth = 0usize;
    for c in phrase.chars() {
        match c {
            '{' | '<' => depth += 1,
            '}' | '>' => depth = depth.saturating_sub(1),
            _ if depth == 0 => text.push(c),
            _ => {}
        }
        if matches!(c, '}' | '>') {
            text.push(' ');
        }
    }
    text.to_lowercase()
        .split(|c: char| !(c.is_alphanumeric() || matches!(c, '\'' | '-')))
        .filter(|w| !w.is_empty())
        .map(str::to_string)
        .collect()
}

/// One warning for each key whose words appear in this order in the phrase
/// of a command. The comparison is on whole words, after the grammar
/// punctuation (`[`, `]`, `;`, braces) is removed. It is a heuristic: it does
/// not follow the grammar structure, so words of two alternatives can count
/// as one sequence. Such a synonym applies only when the text as heard matches
/// no command. The warning names the first command that uses the words.
pub fn synonym_key_warnings(table: &Synonyms, commands: &[Command]) -> Vec<String> {
    let mut warnings = Vec::new();
    for entry in table.entries() {
        let has_key = |words: &[String]| {
            words
                .windows(entry.key_words.len())
                .any(|w| w == entry.key_words.as_slice())
        };
        let found = commands.iter().find_map(|c| {
            let phrase = c.phrase.as_deref()?;
            // The phrase words, and the names of its fixed name slots.
            let in_names = c
                .fixed_vocab
                .values()
                .flatten()
                .any(|name| has_key(&grammar_words(name)));
            (has_key(&grammar_words(phrase)) || in_names).then_some((phrase, &c.token_template))
        });
        if let Some((phrase, token)) = found {
            warnings.push(format!(
                "synonym \"{}\" is also in the phrase \"{}\" -> {token}: the synonym applies only when the text as heard matches no command",
                entry.key,
                phrase.trim()
            ));
        }
    }
    warnings
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table() -> Synonyms {
        Synonyms::from_pairs([
            ("navigate to", vec!["sail out to", "sail out for"]),
            ("navigate", vec!["sail out for", "sail out to"]),
        ])
    }

    #[test]
    fn test_longest_key_wins() {
        assert_eq!(
            synonym_variants("navigate to home", &table()),
            vec!["sail out to home", "sail out for home"]
        );
        assert_eq!(
            synonym_variants("Navigate home.", &table()),
            vec!["sail out for home.", "sail out to home."]
        );
    }

    /// The flags mark the tokens of the text as heard.
    #[test]
    fn test_spoken_flags() {
        assert_eq!(
            synonym_variants_spoken("navigate to the home", &table())[0],
            (
                "sail out to the home".to_string(),
                vec![false, false, false, true, true]
            )
        );
    }

    #[test]
    fn test_no_key_gives_no_variant() {
        assert!(synonym_variants("sail out for home", &table()).is_empty());
        assert!(synonym_variants("harbour panel", &table()).is_empty());
        assert!(synonym_variants("", &table()).is_empty());
        assert!(synonym_variants("navigate", &Synonyms::default()).is_empty());
    }

    #[test]
    fn test_combinations_are_in_file_order() {
        let t = Synonyms::from_pairs([("go", vec!["a", "b"])]);
        assert_eq!(
            synonym_variants("go go", &t),
            vec!["a a", "a b", "b a", "b b"]
        );
        // 2 x 2 x 2 = 8 combinations: at the cap, all are given.
        assert_eq!(synonym_variants("go go go", &t).len(), MAX_SYNONYM_VARIANTS);
    }

    /// More combinations than the cap give no variant at all: a cut-off
    /// list could hide a variant with a different token.
    #[test]
    fn test_too_many_combinations_give_no_variant() {
        let t = Synonyms::from_pairs([("go", vec!["a", "b", "c"])]);
        // 3 x 3 = 9 combinations.
        assert!(synonym_variants("go go", &t).is_empty());
        assert_eq!(synonym_variants("go", &t).len(), 3);
        let two = Synonyms::from_pairs([("go", vec!["a", "b"])]);
        // 2 x 2 x 2 x 2 = 16 combinations.
        assert!(synonym_variants("go go go go", &two).is_empty());
    }

    /// Keys that differ only in case are one key: one problem, merged values.
    #[test]
    fn test_keys_that_differ_only_in_case_are_merged() {
        let text = "[synonyms]\nNavigate = [\"sail out for\"]\n\
                    navigate = [\"sail out to\", \"sail out for\"]\n";
        let (t, problems) = Synonyms::parse_with_problems(text).unwrap();
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].contains("differ only in case"), "{problems:?}");
        assert_eq!(t.len(), 1);
        assert_eq!(t.entries()[0].values, vec!["sail out for", "sail out to"]);
        assert_eq!(
            synonym_variants("navigate home", &t),
            vec!["sail out for home", "sail out to home"]
        );
    }

    #[test]
    fn test_file_format_and_problems() {
        let text = "[synonyms]\n\"navigate to\" = [\"sail out to\", \"sail out for\"]\n\
                    navigate = \"sail out for\"\n\"\" = \"x\"\nsame = \"same\"\n\
                    \"a.b\" = \"x\"\nk = [\"ok\", \"x!y\", \"\"]\n";
        let (t, problems) = Synonyms::parse_with_problems(text).unwrap();
        assert_eq!(t.len(), 3, "{t:?}");
        assert_eq!(problems.len(), 6, "{problems:?}");
        assert!(problems.iter().any(|p| p.contains("empty key")));
        assert!(problems.iter().any(|p| p.contains("maps to itself")));
        assert!(problems.iter().any(|p| p.contains("punctuation")));
        assert!(problems.iter().any(|p| p.contains("empty value")));
        let k = t.entries().iter().find(|e| e.key == "k").unwrap();
        assert_eq!(k.values, vec!["ok"]);
        assert!(Synonyms::parse_with_problems("navigate = 1").is_err());
    }

    #[test]
    fn test_apostrophes_and_hyphens_are_allowed() {
        let t = Synonyms::from_pairs([("let's row", vec!["sail out for"])]);
        assert_eq!(t.len(), 1);
        assert_eq!(
            synonym_variants("Let's row home", &t),
            vec!["sail out for home"]
        );
    }

    #[test]
    fn test_entries_are_longest_key_first() {
        let t = Synonyms::from_pairs([
            ("navigate", vec!["sail out for"]),
            ("navigate to", vec!["sail out to"]),
        ]);
        let keys: Vec<&str> = t.entries().iter().map(|e| e.key.as_str()).collect();
        assert_eq!(keys, vec!["navigate to", "navigate"]);
    }

    #[test]
    fn test_default_file_loads_without_problems() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("data/synonyms.toml");
        let (t, problems) = load_synonyms_with_problems(&path);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(t.len(), 6);
    }

    #[test]
    fn test_missing_or_invalid_file_gives_an_empty_table() {
        let (t, problems) = load_synonyms_with_problems(Path::new("no/such/synonyms.toml"));
        assert!(t.is_empty());
        assert_eq!(problems.len(), 1);
    }

    #[test]
    fn test_key_in_a_command_phrase_gives_a_warning() {
        let commands = vec![
            Command {
                phrase: Some("[plot; navigate to] home".to_string()),
                token_template: "Home".to_string(),
                ..Default::default()
            },
            Command {
                phrase: Some("sail out for home".to_string()),
                token_template: "SailOutForHome".to_string(),
                ..Default::default()
            },
        ];
        let warnings = synonym_key_warnings(&table(), &commands);
        assert_eq!(warnings.len(), 2, "{warnings:?}");
        assert!(warnings[0].contains("\"navigate to\""), "{warnings:?}");
        assert!(warnings[0].contains("Home"), "{warnings:?}");
        let warnings = synonym_key_warnings(&table(), &commands[1..]);
        assert!(warnings.is_empty(), "{warnings:?}");
        // Whole words: "navigation" is not "navigate".
        let navigation = vec![Command {
            phrase: Some("[open;] [bookmarks; navigation] [panel;]".to_string()),
            token_template: "Bookmarks".to_string(),
            ..Default::default()
        }];
        assert!(synonym_key_warnings(&table(), &navigation).is_empty());
        // Slot and variable names are no phrase words.
        let fixed = Synonyms::from_pairs([("fixed", vec!["sail out for"])]);
        let mut engineer = Command {
            phrase: Some("sail out to {TXT:Fixed:HALVARD:Siris} <Commodity>".to_string()),
            token_template: "HarbourInfo:Siris:sail out".to_string(),
            ..Default::default()
        };
        assert!(synonym_key_warnings(&fixed, std::slice::from_ref(&engineer)).is_empty());
        // The names of a fixed name slot count.
        engineer.fixed_vocab.insert(
            "Fixed:HALVARD:Siris".to_string(),
            vec!["Halvard Crane".to_string(), "Siris".to_string()],
        );
        let halvard = Synonyms::from_pairs([("halvard", vec!["sail out for"])]);
        assert_eq!(synonym_key_warnings(&halvard, &[engineer]).len(), 1);
    }
}
