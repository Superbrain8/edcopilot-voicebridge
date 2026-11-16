//! Sound-alike words: real words that a speech engine writes in place of a
//! command word ("cell" for "sell", "by" for "buy").

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::Context;
use serde::Deserialize;

/// The largest number of variants that `sound_alike_variants` gives.
pub const MAX_SOUND_ALIKE_VARIANTS: usize = 8;

/// The table `heard word -> meant word`, in lower case.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SoundAlikes {
    words: BTreeMap<String, String>,
}

/// The file format: one table `[sound_alikes]` with `heard = "meant"` lines.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SoundAlikesFile {
    sound_alikes: BTreeMap<String, String>,
}

/// True for one word of letters and digits only.
fn is_single_word(word: &str) -> bool {
    !word.is_empty() && word.chars().all(char::is_alphanumeric)
}

impl SoundAlikes {
    /// A table from `(heard, meant)` pairs. The words change to lower case.
    /// A pair is left out when a word is not one word of letters and digits,
    /// or when both words are the same.
    pub fn from_pairs<'a>(pairs: impl IntoIterator<Item = (&'a str, &'a str)>) -> SoundAlikes {
        SoundAlikes {
            words: pairs
                .into_iter()
                .map(|(heard, meant)| (heard.trim().to_lowercase(), meant.trim().to_lowercase()))
                .filter(|(heard, meant)| {
                    is_single_word(heard) && is_single_word(meant) && heard != meant
                })
                .collect(),
        }
    }

    /// Parses the TOML text of a table file.
    pub fn parse(text: &str) -> anyhow::Result<SoundAlikes> {
        Ok(SoundAlikes::parse_with_problems(text)?.0)
    }

    /// Parses the TOML text of a table file and gives one text for each
    /// entry that the table leaves out or that is not clear.
    pub fn parse_with_problems(text: &str) -> anyhow::Result<(SoundAlikes, Vec<String>)> {
        let file: SoundAlikesFile = toml::from_str(text)?;
        let mut problems = Vec::new();
        let mut seen: BTreeMap<String, &str> = BTreeMap::new();
        for (heard, meant) in &file.sound_alikes {
            let (h, m) = (heard.trim().to_lowercase(), meant.trim().to_lowercase());
            if m.is_empty() {
                problems.push(format!("sound-alike \"{heard}\" has an empty value"));
            } else if !is_single_word(&h) {
                problems.push(format!(
                    "sound-alike \"{heard}\" is not one word of letters and digits"
                ));
            } else if !is_single_word(&m) {
                problems.push(format!(
                    "sound-alike \"{heard}\" = \"{meant}\": the value is not one word of letters and digits"
                ));
            } else if h == m {
                problems.push(format!("sound-alike \"{heard}\" maps to itself"));
            }
            if let Some(other) = seen.insert(h, heard) {
                problems.push(format!(
                    "sound-alikes \"{other}\" and \"{heard}\" differ only in case"
                ));
            }
        }
        let table = SoundAlikes::from_pairs(
            file.sound_alikes
                .iter()
                .map(|(heard, meant)| (heard.as_str(), meant.as_str())),
        );
        Ok((table, problems))
    }

    /// The number of entries.
    pub fn len(&self) -> usize {
        self.words.len()
    }

    pub fn is_empty(&self) -> bool {
        self.words.is_empty()
    }

    /// The meant word for a heard word. `heard` must be in lower case.
    pub fn get(&self, heard: &str) -> Option<&str> {
        self.words.get(heard).map(String::as_str)
    }
}

/// Loads a table file. A file that is absent or not valid is an error.
pub fn load_sound_alikes(path: &Path) -> anyhow::Result<SoundAlikes> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("cannot read sound-alike file {}", path.display()))?;
    SoundAlikes::parse(&text)
        .with_context(|| format!("invalid sound-alike file {}", path.display()))
}

/// Loads a table file and never fails: a file that is absent or not valid
/// gives an empty table and a problem text. The other problems are the
/// texts of `SoundAlikes::parse_with_problems`.
pub fn load_sound_alikes_with_problems(path: &Path) -> (SoundAlikes, Vec<String>) {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) => {
            let problem = format!("cannot read sound-alike file {}: {e}", path.display());
            return (SoundAlikes::default(), vec![problem]);
        }
    };
    match SoundAlikes::parse_with_problems(&text) {
        Ok((table, problems)) => {
            let problems = problems
                .into_iter()
                .map(|p| format!("{}: {p}", path.display()))
                .collect();
            (table, problems)
        }
        Err(e) => (
            SoundAlikes::default(),
            vec![format!(
                "invalid sound-alike file {}, no sound-alikes: {e}",
                path.display()
            )],
        ),
    }
}

/// The word of a whitespace token without the punctuation around it.
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

/// Variants of `text` in which heard words are replaced by the meant word.
///
/// The first variant has all replacements. When the text has more than one
/// replaceable word, one variant follows for each single replacement. The
/// result has at most `MAX_SOUND_ALIKE_VARIANTS` entries, no duplicates, and
/// never holds the original text. The comparison ignores case and the
/// punctuation around a word.
pub fn sound_alike_variants(text: &str, table: &SoundAlikes) -> Vec<String> {
    let tokens: Vec<&str> = text.split_whitespace().collect();
    // (token index, the token with the meant word in place of the heard word)
    let replacements: Vec<(usize, String)> = tokens
        .iter()
        .enumerate()
        .filter_map(|(i, token)| {
            let (start, end) = word_bounds(token);
            let meant = table.get(&token[start..end].to_lowercase())?;
            Some((i, format!("{}{meant}{}", &token[..start], &token[end..])))
        })
        .collect();
    if replacements.is_empty() {
        return Vec::new();
    }
    let build = |chosen: &[&(usize, String)]| -> String {
        let mut words: Vec<&str> = tokens.clone();
        for (i, replaced) in chosen {
            words[*i] = replaced;
        }
        words.join(" ")
    };
    let original = tokens.join(" ");
    let mut candidates = vec![build(&replacements.iter().collect::<Vec<_>>())];
    if replacements.len() > 1 {
        candidates.extend(replacements.iter().map(|r| build(&[r])));
    }
    let mut variants: Vec<String> = Vec::new();
    for candidate in candidates {
        if candidate != original && candidate != text && !variants.contains(&candidate) {
            variants.push(candidate);
        }
        if variants.len() == MAX_SOUND_ALIKE_VARIANTS {
            break;
        }
    }
    variants
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table() -> SoundAlikes {
        SoundAlikes::from_pairs([
            ("cell", "sell"),
            ("by", "buy"),
            ("bye", "buy"),
            ("soul", "sol"),
            ("higher", "highest"),
        ])
    }

    #[test]
    fn test_no_table_word_gives_no_variant() {
        assert!(sound_alike_variants("dock sell cork", &table()).is_empty());
        assert!(sound_alike_variants("", &table()).is_empty());
        assert!(sound_alike_variants("cork", &SoundAlikes::default()).is_empty());
    }

    #[test]
    fn test_one_word_gives_one_variant() {
        assert_eq!(
            sound_alike_variants("cheapest by price cork", &table()),
            vec!["cheapest buy price cork"]
        );
    }

    #[test]
    fn test_case_and_punctuation_are_ignored() {
        assert_eq!(
            sound_alike_variants("Navigate to Soul.", &table()),
            vec!["Navigate to sol."]
        );
        assert_eq!(
            sound_alike_variants("dock rate, BYE cork", &table()),
            vec!["dock rate, buy cork"]
        );
    }

    #[test]
    fn test_two_words_give_all_then_each_single() {
        assert_eq!(
            sound_alike_variants("higher water cell cork", &table()),
            vec![
                "highest water sell cork",
                "highest water cell cork",
                "higher water sell cork",
            ]
        );
    }

    #[test]
    fn test_variants_are_capped() {
        let text = ["cell"; 12].join(" ");
        let variants = sound_alike_variants(&text, &table());
        assert_eq!(variants.len(), MAX_SOUND_ALIKE_VARIANTS);
        assert_eq!(variants[0], ["sell"; 12].join(" "));
        assert!(!variants.contains(&text));
    }

    #[test]
    fn test_parse_file_format() {
        let table =
            SoundAlikes::parse("# comment\n[sound_alikes]\nCell = \"Sell\"\nby = \"buy\"\n")
                .unwrap();
        assert_eq!(table.len(), 2);
        assert_eq!(table.get("cell"), Some("sell"));
        assert!(SoundAlikes::parse("cell = \"sell\"\n").is_err());
    }

    #[test]
    fn test_default_table_file_loads() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("data/sound_alikes.toml");
        let table = load_sound_alikes(&path).unwrap();
        assert_eq!(table.get("cell"), Some("sell"));
        assert_eq!(table.get("by"), Some("buy"));
        assert_eq!(table.get("sale"), None);
    }

    #[test]
    fn test_doubtful_entries_are_problems() {
        let text = "[sound_alikes]\nsell = \"sell\"\n\"two words\" = \"x\"\n\"a.b\" = \"x\"\n\
                    empty = \"\"\nCell = \"sell\"\ncell = \"sell\"\nby = \"b y\"\nbye = \"buy\"\n";
        let (table, problems) = SoundAlikes::parse_with_problems(text).unwrap();
        assert_eq!(problems.len(), 6, "{problems:?}");
        assert!(problems.iter().any(|p| p.contains("maps to itself")));
        assert!(problems.iter().any(|p| p.contains("empty value")));
        assert!(problems.iter().any(|p| p.contains("differ only in case")));
        assert_eq!(table.get("bye"), Some("buy"));
        assert_eq!(table.get("cell"), Some("sell"));
        assert_eq!(table.get("sell"), None);
        assert_eq!(table.get("by"), None);
        assert_eq!(table.len(), 2);
    }

    #[test]
    fn test_loader_never_fails() {
        let dir = std::env::temp_dir().join(format!("voicebridge-sa-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let bad = dir.join("bad.toml");
        std::fs::write(&bad, "not = [valid").unwrap();
        let (table, problems) = load_sound_alikes_with_problems(&bad);
        assert!(table.is_empty());
        assert_eq!(problems.len(), 1);
        assert!(problems[0].contains("invalid"), "{problems:?}");
        let (table, problems) = load_sound_alikes_with_problems(&dir.join("none.toml"));
        assert!(table.is_empty());
        assert_eq!(problems.len(), 1);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_missing_file_is_an_error() {
        assert!(load_sound_alikes(Path::new("no/such/sound_alikes.toml")).is_err());
    }
}
