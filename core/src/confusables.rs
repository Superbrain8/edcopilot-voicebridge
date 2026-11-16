//! Hotwords that the matcher could confuse with another name.
//!
//! A hotword pulls the text toward its own spelling. If the hotword is an
//! inexact form of another name, the pull can change the name that the user
//! said into the hotword: "Siris" became "Sirius". Thus a hotword must not be
//! near another hotword or a name of a vocabulary slot. "Near" uses the word
//! similarity of the matcher: two different words are near when the matcher
//! takes one as a fuzzy form of the other.

use crate::matcher::{inexact_word_similarity, normalize};

/// What a hotword is near.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OtherKind {
    /// Another hotword.
    Hotword,
    /// A name of a vocabulary slot or a fixed name of a command.
    Name,
    /// A word of a command phrase. An equal word also counts.
    CommandWord,
}

/// A hotword and a name or word that the matcher could confuse.
#[derive(Debug, Clone, PartialEq)]
pub struct Confusion {
    pub hotword: String,
    /// The other hotword, the name, or the command word.
    pub other: String,
    pub kind: OtherKind,
    /// The word similarity of the nearest word pair. 1.0 is an equal word.
    pub similarity: f64,
}

impl Confusion {
    /// One line for a problem text that names both words.
    pub fn describe(&self) -> String {
        let kind = match self.kind {
            OtherKind::Hotword => "the hotword",
            OtherKind::Name => "the name",
            OtherKind::CommandWord => "the command word",
        };
        format!(
            "the hotword \"{}\" is near {kind} \"{}\" (word similarity {:.2})",
            self.hotword, self.other, self.similarity
        )
    }
}

/// The highest word similarity between two phrases at which the matcher
/// takes one word as an inexact form of the other. `None`: no such pair.
fn nearest_pair(a: &[String], b: &[String]) -> Option<f64> {
    a.iter()
        .flat_map(|x| b.iter().filter_map(move |y| inexact_word_similarity(x, y)))
        .max_by(f64::total_cmp)
}

/// The hotwords that are near another hotword or a name. Each pair of
/// hotwords shows one time. A name that is equal to the hotword is not a
/// confusion.
pub fn confusable_hotwords(hotwords: &[String], names: &[String]) -> Vec<Confusion> {
    let hot: Vec<Vec<String>> = hotwords.iter().map(|h| normalize(h)).collect();
    let mut out = Vec::new();
    for (i, words) in hot.iter().enumerate() {
        for (j, other) in hot.iter().enumerate().skip(i + 1) {
            if let Some(similarity) = nearest_pair(words, other) {
                out.push(Confusion {
                    hotword: hotwords[i].clone(),
                    other: hotwords[j].clone(),
                    kind: OtherKind::Hotword,
                    similarity,
                });
            }
        }
        let mut seen: Vec<&str> = Vec::new();
        for name in names {
            if seen.contains(&name.as_str()) {
                continue;
            }
            if let Some(similarity) = nearest_pair(words, &normalize(name)) {
                seen.push(name);
                out.push(Confusion {
                    hotword: hotwords[i].clone(),
                    other: name.clone(),
                    kind: OtherKind::Name,
                    similarity,
                });
            }
        }
    }
    out
}

/// The hotwords that are equal to or near a word of a command phrase. Such
/// a hotword could stand in for a command word ("course", "three").
/// `command_words` are normalised words.
pub fn command_word_conflicts(hotwords: &[String], command_words: &[String]) -> Vec<Confusion> {
    let mut out = Vec::new();
    for hotword in hotwords {
        for word in normalize(hotword) {
            for command_word in command_words {
                let similarity = if word == *command_word {
                    Some(1.0)
                } else {
                    inexact_word_similarity(&word, command_word)
                };
                if let Some(similarity) = similarity {
                    out.push(Confusion {
                        hotword: hotword.clone(),
                        other: command_word.clone(),
                        kind: OtherKind::CommandWord,
                        similarity,
                    });
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn test_siris_and_sirius_are_a_pair() {
        let found = confusable_hotwords(&strings(&["Siris", "Sirius", "Sol"]), &[]);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].hotword, "Siris");
        assert_eq!(found[0].other, "Sirius");
        assert_eq!(found[0].kind, OtherKind::Hotword);
        let text = found[0].describe();
        assert!(text.contains("Siris") && text.contains("Sirius"), "{text}");
    }

    #[test]
    fn test_hotword_near_a_vocabulary_name() {
        let names = strings(&["Sirius", "KDS 4021", "Sirius Atmospherics", "Sol"]);
        let found = confusable_hotwords(&strings(&["Siris"]), &names);
        let others: Vec<&str> = found.iter().map(|c| c.other.as_str()).collect();
        assert_eq!(others, ["Sirius", "Sirius Atmospherics"]);
        assert_eq!(found[0].kind, OtherKind::Name);
    }

    #[test]
    fn test_equal_names_and_far_names_pass() {
        let names = strings(&["Sol", "Los", "Sorrel", "sol", "Varnick"]);
        assert!(confusable_hotwords(&strings(&["Sol"]), &names).is_empty());
    }

    #[test]
    fn test_hotword_equal_or_near_a_command_word() {
        let words = strings(&["hoist", "lantern", "at", "three"]);
        let found = command_word_conflicts(&strings(&["Lantern", "Sol", "tree"]), &words);
        let pairs: Vec<(&str, &str)> = found
            .iter()
            .map(|c| (c.hotword.as_str(), c.other.as_str()))
            .collect();
        assert_eq!(pairs, [("Lantern", "lantern"), ("tree", "three")]);
        assert_eq!(found[0].similarity, 1.0);
        assert_eq!(found[0].kind, OtherKind::CommandWord);
        assert!(found[0].describe().contains("the command word \"lantern\""));
    }

    #[test]
    fn test_digits_never_count_as_near() {
        let names = strings(&["KTL 12099", "KTL 12098"]);
        assert!(confusable_hotwords(&strings(&["KTL 12099"]), &names).is_empty());
    }
}
