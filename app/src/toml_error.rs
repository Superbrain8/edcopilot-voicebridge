//! The text of a TOML error, without the content of the file.
//!
//! The `Display` text of a `toml` or a `toml_edit` error shows the line of
//! the file that has the error. The message of the error can also quote a
//! value or a key name. A user can write a secret (an API key) on a line
//! that has an error, so these texts must not go to the window, the console
//! or a log. The functions here make the text from the line, the column, the
//! key path and the expected type. The key path has only names of the known
//! configuration keys. The text never has a value or a line of the file.

use std::ops::Range;

use toml_edit::{ImDocument, Item, TableLike};

/// The text of a parse or a deserialize error of `toml`. `text` is the text
/// that the parser got.
pub fn de_error_text(error: &toml::de::Error, text: &str) -> String {
    describe(error.message(), error.span(), text)
}

/// The text of a parse error of `toml_edit`. `text` is the text that the
/// parser got.
pub fn edit_error_text(error: &toml_edit::TomlError, text: &str) -> String {
    describe(error.message(), error.span(), text)
}

/// The length limit of a quoted part of a parser message. The parser quotes
/// only the characters that it expects (for example `"` or `]`) and words
/// such as `true`. A longer quoted part can be a key from the file.
const MAX_QUOTED_CHARS: usize = 5;

/// The length limit of an unknown key name that the text shows. An API key
/// is longer.
const MAX_SHOWN_KEY_CHARS: usize = 24;

fn describe(message: &str, span: Option<Range<usize>>, text: &str) -> String {
    let summary = match ImDocument::parse(text) {
        // The text parses, so the error is from the deserializer.
        Ok(doc) => {
            // An empty span (a missing key) points at no key.
            let path = span
                .clone()
                .filter(|span| !span.is_empty())
                .and_then(|span| known_path(&doc, &span));
            value_summary(message, path.as_deref())
        }
        Err(_) => syntax_summary(message),
    };
    match span.map(|span| position(text, span.start)) {
        Some((line, column)) => format!("line {line}, column {column}: {summary}"),
        None => summary,
    }
}

/// The line and the column (1-based, in characters) of a byte offset.
fn position(text: &str, offset: usize) -> (usize, usize) {
    let mut offset = offset.min(text.len());
    while !text.is_char_boundary(offset) {
        offset -= 1;
    }
    let before = &text[..offset];
    let line = before.matches('\n').count() + 1;
    let start = before.rfind('\n').map_or(0, |i| i + 1);
    (line, before[start..].chars().count() + 1)
}

/// The summary of a parser error. The parser gives a label and the expected
/// characters. A message that quotes a longer part (a duplicate key, for
/// example) gives a general text.
fn syntax_summary(message: &str) -> String {
    let text = message
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(", ");
    let quoted_ok = text
        .split('`')
        .skip(1)
        .step_by(2)
        .all(|part| part.chars().count() <= MAX_QUOTED_CHARS);
    if text.is_empty() || !quoted_ok || text.starts_with("duplicate key") {
        "the TOML syntax is not correct".to_string()
    } else {
        format!("the TOML syntax is not correct ({text})")
    }
}

/// The summary of a deserializer error. Only the parts that the program
/// gives are kept: the key path, the names of the expected keys, and the
/// expected type, and a short unknown key name. The value is never kept.
fn value_summary(message: &str, path: Option<&str>) -> String {
    let within = path.map_or_else(String::new, |path| format!(" in [{path}]"));
    if let Some(rest) = message.strip_prefix("unknown field `") {
        let name = rest
            .split_once('`')
            .filter(|(name, after)| {
                (after.is_empty() || after.starts_with(", "))
                    && is_name(name)
                    && name.chars().count() <= MAX_SHOWN_KEY_CHARS
            })
            .map_or_else(String::new, |(name, _)| format!(" `{name}`"));
        let expected = message
            .rfind(", expected one of ")
            .map(|i| &message[i + ", expected one of ".len()..])
            .filter(|list| is_name_list(list))
            .map_or_else(String::new, |list| format!(", expected one of {list}"));
        return format!("unknown key{name}{within}{expected}");
    }
    if let Some(rest) = message.strip_prefix("missing field `") {
        return match rest.strip_suffix('`').filter(|name| is_name(name)) {
            Some(name) => format!("missing key `{name}`{within}"),
            None => format!("a key is missing{within}"),
        };
    }
    let expected = message
        .rfind(", expected ")
        .map(|i| &message[i + ", expected ".len()..])
        .filter(|expected| is_type_text(expected))
        .map_or_else(String::new, |expected| format!(", expected {expected}"));
    match path {
        Some(path) => format!("the value of {path} is not valid{expected}"),
        None => format!("a value is not valid{expected}"),
    }
}

/// A name of a key as the program writes it: lower-case letters, digits and
/// `_`.
fn is_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// A list such as "`a`, `b`", with names only.
fn is_name_list(list: &str) -> bool {
    list.split(", ").all(|item| {
        item.strip_prefix('`')
            .and_then(|item| item.strip_suffix('`'))
            .is_some_and(is_name)
    })
}

/// An expected type as serde gives it, for example "a string", "usize",
/// "a boolean" or "one of `a`, `b`". Quoted parts must be names.
fn is_type_text(text: &str) -> bool {
    let words_ok = text
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || " _,`".contains(c));
    let quoted_ok = text.split('`').skip(1).step_by(2).all(is_name);
    !text.is_empty() && text.len() <= 200 && words_ok && quoted_ok
}

/// The longest known key path at the error: the key whose name or value
/// holds the start of `span`, cut to the part that is a known key or table
/// of the configuration.
fn known_path(doc: &ImDocument<&str>, span: &Range<usize>) -> Option<String> {
    let mut path = Vec::new();
    find_path(doc.as_table(), span.start, &mut path);
    let mut known = None;
    for end in 1..=path.len() {
        let candidate = path[..end].join(".");
        if is_known(&candidate) {
            known = Some(candidate);
        } else {
            break;
        }
    }
    known
}

fn is_known(path: &str) -> bool {
    crate::config_edit::known_key_names().any(|key| {
        key == path
            || key
                .strip_prefix(path)
                .is_some_and(|rest| rest.starts_with('.'))
    })
}

/// Adds to `path` the keys down to the deepest key whose name or value
/// holds `at`. True when a key holds it.
fn find_path(table: &dyn TableLike, at: usize, path: &mut Vec<String>) -> bool {
    for (name, _) in table.iter() {
        let Some((key, item)) = table.get_key_value(name) else {
            continue;
        };
        let holds = |span: Option<Range<usize>>| span.is_some_and(|s| s.contains(&at));
        let inner = match item {
            Item::Table(table) => Some(table as &dyn TableLike),
            Item::Value(toml_edit::Value::InlineTable(table)) => Some(table as &dyn TableLike),
            _ => None,
        };
        path.push(name.to_string());
        if let Some(inner) = inner {
            if find_path(inner, at, path) {
                return true;
            }
        }
        if holds(key.span()) || holds(item.span()) {
            return true;
        }
        path.pop();
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &str = "sk-test-1234567890abcdef";

    fn de_text(text: &str) -> String {
        let error = toml::from_str::<crate::config::Config>(text).unwrap_err();
        de_error_text(&error, text)
    }

    fn edit_text(text: &str) -> String {
        let error = text.parse::<toml_edit::DocumentMut>().unwrap_err();
        edit_error_text(&error, text)
    }

    const HEAD: &str = "[edcopilot]\ndir = 'C:/E'\n";

    #[test]
    fn test_a_value_on_a_line_that_does_not_parse_is_not_in_the_text() {
        let text = format!("{HEAD}[stt.openai_compatible]\napi_key_env = {SECRET}\n");
        for got in [de_text(&text), edit_text(&text)] {
            assert!(!got.contains(SECRET), "{got}");
            assert!(!got.contains("api_key_env ="), "{got}");
            assert_eq!(
                got,
                "line 4, column 15: the TOML syntax is not correct (invalid string, expected `\"`, `'`)"
            );
        }
    }

    #[test]
    fn test_a_value_of_the_wrong_type_is_not_in_the_text() {
        let text = format!("{HEAD}[stt.parakeet]\nthreads = \"{SECRET}\"\n");
        let got = de_text(&text);
        assert_eq!(
            got,
            "line 4, column 11: the value of stt.parakeet.threads is not valid, expected usize"
        );
        let text = format!("{HEAD}[stt.openai_compatible]\napi_key_env = [\"{SECRET}\"]\n");
        assert_eq!(
            de_text(&text),
            "line 4, column 15: the value of stt.openai_compatible.api_key_env is not valid, \
             expected a string"
        );
    }

    #[test]
    fn test_an_unknown_key_and_its_value_are_not_in_the_text() {
        let text = format!("{HEAD}[stt.openai_compatible]\napi_key = \"{SECRET}\"\n");
        assert_eq!(
            de_text(&text),
            "line 4, column 1: unknown key `api_key` in [stt.openai_compatible], expected one \
             of `base_url`, `api_key_env`, `model`, `language`, `timeout_seconds`"
        );
        // The secret as the key name.
        let text = format!("{HEAD}[stt.openai_compatible]\n{SECRET} = 1\n");
        let got = de_text(&text);
        assert!(!got.contains(SECRET), "{got}");
        assert!(got.starts_with("line 4, column 1: unknown key in [stt.openai_compatible]"));
        let text = format!("{SECRET} = 1\n{HEAD}");
        let got = de_text(&text);
        assert!(!got.contains(SECRET), "{got}");
        assert!(got.starts_with("line 1, column 1: unknown key, expected one of `edcopilot`"));
        // A long name of letters and digits only is not shown either.
        let long = "abcdefghijklmnopqrstuvwxyz0123456789";
        assert_eq!(
            de_text(&format!("{HEAD}{long} = 1\n")),
            "line 3, column 1: unknown key in [edcopilot], expected one of `dir`, \
             `request_file`, `voice_commands_ini`, `dynamic_vars_dir`, `profile`"
        );
    }

    #[test]
    fn test_a_duplicate_key_is_not_named() {
        let text = format!("{HEAD}{SECRET} = 1\n{SECRET} = 2\n");
        let got = edit_text(&text);
        assert!(!got.contains(SECRET), "{got}");
        assert!(got.ends_with(": the TOML syntax is not correct"), "{got}");
    }

    #[test]
    fn test_a_missing_key_is_named() {
        assert_eq!(
            de_text("[stt]\nengine = 'parakeet'\n"),
            "line 1, column 1: missing key `edcopilot`"
        );
    }

    #[test]
    fn test_a_value_in_a_dotted_key_and_an_inline_table_has_its_path() {
        let text =
            format!("push_to_talk.joystick = {{ device = 'X', button = '{SECRET}' }}\n{HEAD}");
        let got = de_text(&text);
        assert!(!got.contains(SECRET), "{got}");
        assert!(
            got.contains("the value of push_to_talk.joystick.button is not valid, expected u32"),
            "{got}"
        );
    }

    #[test]
    fn test_a_message_with_a_value_gives_no_expected_part_that_is_not_a_type() {
        assert_eq!(
            value_summary(
                "invalid type: string \"x\", expected sk-1",
                Some("stt.engine")
            ),
            "the value of stt.engine is not valid"
        );
        assert_eq!(
            value_summary("custom text with sk-1", None),
            "a value is not valid"
        );
    }

    /// The three forms of a secret in a configuration: on a line that does
    /// not parse, as a value of the wrong type, and as an unknown key with
    /// its value.
    pub(crate) fn secret_configs() -> Vec<String> {
        vec![
            format!("{HEAD}[stt.openai_compatible]\napi_key_env = {SECRET}\n"),
            format!("{HEAD}[stt.parakeet]\nthreads = \"{SECRET}\"\n"),
            format!("{HEAD}[stt.openai_compatible]\napi_key = \"{SECRET}\"\n"),
            format!("{HEAD}[stt.openai_compatible]\n{SECRET} = \"{SECRET}\"\n"),
        ]
    }

    #[test]
    fn test_no_entry_point_puts_the_secret_in_its_error() {
        use crate::config::Config;
        use crate::config_edit::{apply_edits, changed_values, parse_config, save_edits, Value};
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("voicebridge.toml");
        let edit = [("matching.threshold".to_string(), Value::Number(0.85))];
        for text in secret_configs() {
            std::fs::write(&file, &text).unwrap();
            let mut texts = vec![
                format!("{:#}", Config::load(&file).unwrap_err()),
                format!("{:?}", Config::load(&file).unwrap_err()),
                format!("{:#}", Config::load_as_written_text(&file).unwrap_err()),
                parse_config(&file, &text).unwrap_err(),
            ];
            // The edit and the compare need text that parses.
            if let Err(e) = apply_edits(&text, &edit) {
                texts.push(format!("{e:#}"));
            }
            if let Err(e) = changed_values(&text, &text, &["stt"]) {
                texts.push(format!("{e:#}"));
            }
            match save_edits(&file, &edit) {
                Ok(result) => texts.extend(result.problems),
                Err(e) => texts.push(format!("{e:#}")),
            }
            for got in texts {
                assert!(!got.contains(SECRET), "{got}");
                assert!(got.contains("line 4, column "), "{got}");
            }
        }
    }

    #[test]
    fn test_position_counts_characters() {
        assert_eq!(position("a\nää = x", 9), (2, 6));
        assert_eq!(position("", 4), (1, 1));
    }
}
