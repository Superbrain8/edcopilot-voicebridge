//! Hotwords (contextual biasing) for the local engine: the list, the
//! `bpe.vocab` file that sherpa-onnx needs, and the check that each hotword
//! has pieces in the model vocabulary.
//!
//! This module has no model code. The tests run without the model.

use std::collections::HashSet;
use std::path::Path;

use anyhow::{bail, Context};

/// The word-start mark of the model pieces (U+2581).
const WORD_START: char = '\u{2581}';

/// The lowest and the highest hotword score. A score above 4 changed
/// sentences that are not commands in a test with synthetic voices.
pub const MIN_SCORE: f32 = 0.1;
pub const MAX_SCORE: f32 = 4.0;

/// A list of hotword phrases and the score that the decoder adds.
#[derive(Debug, Clone, PartialEq)]
pub struct Hotwords {
    phrases: Vec<String>,
    score: f32,
}

impl Hotwords {
    /// Parses the text of a hotwords file: one phrase on each line. Blank
    /// lines and lines that start with `#` are ignored. The phrases keep
    /// their case, because the model pieces are case-sensitive. A phrase
    /// must not contain `/` or `:`, because sherpa-onnx uses these
    /// characters as separators. A UTF-8 byte order mark at the start is
    /// removed.
    pub fn parse(text: &str, score: f32) -> anyhow::Result<Hotwords> {
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        if !(MIN_SCORE..=MAX_SCORE).contains(&score) {
            bail!("the hotword score is {score}, it must be from {MIN_SCORE} to {MAX_SCORE}");
        }
        let mut phrases: Vec<String> = Vec::new();
        for (index, line) in text.lines().enumerate() {
            let phrase = line.split_whitespace().collect::<Vec<_>>().join(" ");
            if phrase.is_empty() || phrase.starts_with('#') {
                continue;
            }
            if phrase.contains(['/', ':']) {
                bail!(
                    "line {}: the hotword \"{phrase}\" has a \"/\" or a \":\"",
                    index + 1
                );
            }
            if !phrases.contains(&phrase) {
                phrases.push(phrase);
            }
        }
        Ok(Hotwords { phrases, score })
    }

    /// Reads and parses a hotwords file. The error names the file.
    pub fn load(path: &Path, score: f32) -> anyhow::Result<Hotwords> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("cannot read the hotwords file {}", path.display()))?;
        Hotwords::parse(&text, score)
            .with_context(|| format!("invalid hotwords file {}", path.display()))
    }

    pub fn phrases(&self) -> &[String] {
        &self.phrases
    }

    pub fn score(&self) -> f32 {
        self.score
    }

    pub fn is_empty(&self) -> bool {
        self.phrases.is_empty()
    }

    /// The phrases in the form of `create_stream_with_hotwords`: separated by `/`.
    pub fn stream_text(&self) -> String {
        self.phrases.join("/")
    }
}

/// The pieces of `tokens.txt` (lines of `piece id`) with their ids.
fn pieces(tokens_txt: &str) -> impl Iterator<Item = (&str, u64)> {
    tokens_txt.lines().filter_map(|line| {
        let (piece, id) = line.trim_end_matches('\r').rsplit_once(' ')?;
        Some((piece, id.trim().parse().ok()?))
    })
}

/// Makes the text of a `bpe.vocab` file from the text of `tokens.txt`.
///
/// The model package has no `bpe.vocab`. The SentencePiece scores are not
/// available, so each piece gets the score `-(1 + id / 100000)`. The encoder
/// thus selects the fewest pieces, then the lower ids. sherpa-onnx maps the
/// pieces to ids through `tokens.txt`, so only the piece strings matter.
/// The blank piece `<blk>` is not a word piece and is left out.
pub fn bpe_vocab(tokens_txt: &str) -> String {
    let mut out = String::new();
    for (piece, id) in pieces(tokens_txt) {
        if piece == "<blk>" {
            continue;
        }
        let score = -(1.0 + id as f64 / 100_000.0);
        out.push_str(&format!("{piece} {score:.6}\n"));
    }
    out
}

/// The phrases that the model pieces cannot spell. sherpa-onnx skips such a
/// phrase with only a log line, so the program must find them first.
pub fn unencodable(tokens_txt: &str, phrases: &[String]) -> Vec<String> {
    let vocab: HashSet<&str> = pieces(tokens_txt).map(|(piece, _)| piece).collect();
    let longest = vocab.iter().map(|p| p.chars().count()).max().unwrap_or(0);
    phrases
        .iter()
        .filter(|phrase| !can_spell(&vocab, longest, phrase))
        .cloned()
        .collect()
}

/// True when the pieces can spell the phrase, with the word-start mark
/// before each word.
fn can_spell(vocab: &HashSet<&str>, longest: usize, phrase: &str) -> bool {
    let marked: Vec<char> = phrase
        .split_whitespace()
        .flat_map(|word| std::iter::once(WORD_START).chain(word.chars()))
        .collect();
    if marked.is_empty() {
        return false;
    }
    // reach[i]: the pieces can spell the first i characters.
    let mut reach = vec![false; marked.len() + 1];
    reach[0] = true;
    for start in 0..marked.len() {
        if !reach[start] {
            continue;
        }
        let mut piece = String::new();
        for (len, &c) in marked[start..].iter().take(longest).enumerate() {
            piece.push(c);
            if vocab.contains(piece.as_str()) {
                reach[start + len + 1] = true;
            }
        }
    }
    reach[marked.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKENS: &str = "<blk> 8192\n<unk> 0\n\u{2581}s 5\n\u{2581}sol 7\no 9\nl 10\nS 11\n\u{2581}S 12\n\u{2581}sail 13\n\u{2581}out 14\n";

    #[test]
    fn test_parse_keeps_case_and_skips_comments() {
        let h = Hotwords::parse("# names\nSol\n\n  sail   out \nSol\nsol\r\n", 2.0).unwrap();
        assert_eq!(h.phrases(), ["Sol", "sail out", "sol"]);
        assert_eq!(h.score(), 2.0);
        assert_eq!(h.stream_text(), "Sol/sail out/sol");
        assert!(!h.is_empty());
        assert!(Hotwords::parse("# only a comment\n", 2.0)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn test_parse_removes_a_byte_order_mark() {
        let h = Hotwords::parse("\u{feff}Sol\nVarnick\n", 1.5).unwrap();
        assert_eq!(h.phrases(), ["Sol", "Varnick"]);
        let h = Hotwords::parse("\u{feff}# comment\nSol\n", 1.5).unwrap();
        assert_eq!(h.phrases(), ["Sol"]);
    }

    #[test]
    fn test_parse_rejects_separators() {
        for bad in ["Sol/Siris", "Sol :3.0"] {
            let err = Hotwords::parse(&format!("ok\n{bad}\n"), 2.0).unwrap_err();
            let text = err.to_string();
            assert!(text.contains("line 2"), "{text}");
            assert!(text.contains(bad), "{text}");
        }
    }

    #[test]
    fn test_parse_rejects_a_score_out_of_range() {
        for bad in [0.0, -1.0, 4.5, f32::NAN, f32::INFINITY] {
            let err = Hotwords::parse("Sol", bad).unwrap_err().to_string();
            assert!(err.contains("score"), "{err}");
        }
        assert!(Hotwords::parse("Sol", MIN_SCORE).is_ok());
        assert!(Hotwords::parse("Sol", MAX_SCORE).is_ok());
    }

    #[test]
    fn test_load_names_the_file() {
        let path = std::env::temp_dir().join("voicebridge-no-such-hotwords.txt");
        let err = format!("{:#}", Hotwords::load(&path, 2.0).unwrap_err());
        assert!(err.contains("voicebridge-no-such-hotwords.txt"), "{err}");
    }

    #[test]
    fn test_bpe_vocab_follows_the_spike_rule() {
        let vocab = bpe_vocab(TOKENS);
        let lines: Vec<&str> = vocab.lines().collect();
        assert_eq!(lines[0], "<unk> -1.000000");
        assert_eq!(lines[1], "\u{2581}s -1.000050");
        assert!(!vocab.contains("<blk>"));
        assert_eq!(lines.len(), 9);
        // Windows line ends in tokens.txt give the same result.
        assert_eq!(bpe_vocab(&TOKENS.replace('\n', "\r\n")), vocab);
    }

    #[test]
    fn test_unencodable_finds_phrases_without_pieces() {
        let phrases: Vec<String> = ["Sol", "sol", "sail out", "Zeta", "Sols"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        // "Sol" = "\u{2581}S" + "o" + "l". "Zeta" has no pieces. "Sols"
        // needs "s" inside a word, and only the word-start form exists.
        assert_eq!(unencodable(TOKENS, &phrases), ["Zeta", "Sols"]);
    }
}
