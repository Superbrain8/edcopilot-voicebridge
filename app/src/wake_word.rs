//! Wake word check for the always-listening mode.

/// Split text into lowercase words. Punctuation and hyphens separate words.
fn words(text: &str) -> Vec<(usize, usize, String)> {
    let mut result = Vec::new();
    let mut start: Option<usize> = None;
    for (index, c) in text.char_indices() {
        if c.is_alphanumeric() || c == '\'' {
            start.get_or_insert(index);
        } else if let Some(s) = start.take() {
            result.push((s, index, text[s..index].to_lowercase()));
        }
    }
    if let Some(s) = start {
        result.push((s, text.len(), text[s..].to_lowercase()));
    }
    result
}

/// Return the text after the first wake word that the text starts with.
///
/// An empty list returns the full text. No match returns `None`. The
/// comparison ignores case, punctuation, and hyphens and uses complete words.
pub fn strip<'a>(text: &'a str, wake_words: &[String]) -> Option<&'a str> {
    if wake_words.is_empty() {
        return Some(text);
    }
    let spoken = words(text);
    for wake in wake_words {
        let wanted: Vec<String> = words(wake).into_iter().map(|w| w.2).collect();
        if wanted.is_empty() || wanted.len() > spoken.len() {
            continue;
        }
        let starts = spoken.iter().zip(&wanted).all(|(s, w)| &s.2 == w);
        if starts {
            let end = spoken[wanted.len() - 1].1;
            let rest = text[end..].trim_start_matches(|c: char| !c.is_alphanumeric());
            return Some(rest);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wake() -> Vec<String> {
        vec!["copilot".to_string(), "co pilot".to_string()]
    }

    #[test]
    fn test_strip() {
        let w = wake();
        assert_eq!(
            strip("CoPilot, sail out for home", &w),
            Some("sail out for home")
        );
        assert_eq!(strip("co-pilot sail out", &w), Some("sail out"));
        assert_eq!(
            strip("Co pilot. Help chart log", &w),
            Some("Help chart log")
        );
        assert_eq!(strip("sail out for home", &w), None);
        assert_eq!(strip("copilots are here", &w), None);
        assert_eq!(strip("copilot", &w), Some(""));
    }

    #[test]
    fn test_no_wake_words_gives_the_full_text() {
        assert_eq!(strip("sail out for home", &[]), Some("sail out for home"));
    }
}
