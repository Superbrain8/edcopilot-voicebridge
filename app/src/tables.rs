//! The word tables as rows: read, check, and save.
//!
//! The tables are the alias files (one file for each language), the
//! synonyms, the sound-alikes, the filler words, and the hotwords. The paths
//! come from the saved configuration file. The web part never gives a path.
//!
//! The check does not have its own rules. It writes the rows in the file
//! format into a temporary folder, points a copy of the configuration at
//! that file, and runs `run::check_config`, the check of `voicebridge check`.
//! Thus the check of the rows is the check of the file after the save. The
//! texts name the path of the real table, not the temporary file.
//!
//! The lines of a table are the lines of that check that the same check with
//! the table empty does not have (for an alias file: the alias folder without
//! this language). Thus a line that the saved table causes stays visible. A
//! line that exists also without the table is not a line of this table.
//!
//! The check says "no problem" only when the full check ran on the rows. Each
//! cause that stops a part of the check of a table is a problem of the table:
//! the command data does not load, a file or a folder of the command data is
//! absent, or the hotwords cannot be judged (a bad score, no model pieces).
//!
//! The rows of each table:
//!
//! - Aliases: `spoken` is the token template (the key), `meant` has the
//!   phrases in grammar syntax.
//! - Synonyms: `spoken` is the spoken phrase, `meant` has the EDCoPilot
//!   phrases.
//! - Sound-alikes: `spoken` is the heard word, `meant` has the one meant
//!   word.
//! - Fillers: `spoken` is the word, `meant` has the name of its list:
//!   `not_last` or `may_be_last`.
//! - Hotwords: `spoken` is the phrase, `meant` is empty.
//!
//! A save keeps the byte order mark, the line ends, the header comment, the
//! comments of the entries, and the text of each entry that did not change.
//! The comment lines of a removed entry, and its comment at the end of the
//! line, go to the next entry, or to the end of the file. Each kept line keeps
//! its own line end. The file gets the entries in the order of the rows. A file
//! with content that the rows cannot hold (an unknown key, a value that is
//! not a text or a list of texts, a comment inside a list that changes) is
//! not written: the error says that the user must edit the file by hand.

use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, VecDeque};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{anyhow, bail, Context};
use toml_edit::{Array, DocumentMut, Item, Key, RawString, Table, Value};

use crate::config::Config;
use crate::config_edit::write_atomic_named;
use crate::run::check_config_in;
use edcopilot_voicebridge_stt::{hotwords, Hotwords};

/// One word table. `Aliases` is the alias file of one language.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TableKind {
    Aliases { language: String },
    Synonyms,
    SoundAlikes,
    Fillers,
    Hotwords,
}

/// One entry of a table. The module documentation tells the meaning of the
/// two fields for each table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub spoken: String,
    pub meant: Vec<String>,
}

/// The problems and the warnings of the rows. A problem refuses the save. A
/// warning needs a confirmation.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TableCheck {
    pub problems: Vec<String>,
    pub warnings: Vec<String>,
}

/// A table as the file has it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableFile {
    /// The file of the table.
    pub path: PathBuf,
    /// False when the file does not exist. Then `rows` is empty, and a save
    /// makes the file.
    pub exists: bool,
    pub rows: Vec<Row>,
    /// A stamp of the file content. A save refuses when the file has
    /// another stamp now.
    pub version: String,
    /// True when the configuration has no hotword file. The hotword table
    /// is then the shipped file below the configuration folder.
    pub hotwords_off: bool,
    /// The languages of the alias folder, for each table.
    pub languages: Vec<String>,
    /// The language of an alias table as the file name has it ("de" for a
    /// request of "DE" when `de.toml` exists). `None` for the other tables.
    pub language: Option<String>,
}

/// The result of a save.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableSave {
    /// False when the save was refused or when the text did not change.
    pub written: bool,
    /// The check of the rows. With problems, the save was refused.
    pub check: TableCheck,
    /// The warnings that the caller did not confirm. They refused the save.
    pub unconfirmed: Vec<String>,
    /// The stamp of the file after the call.
    pub version: String,
}

/// The two lists of the filler file.
pub const NOT_LAST: &str = "not_last";
pub const MAY_BE_LAST: &str = "may_be_last";
/// The largest number of rows of a table.
pub const MAX_ROWS: usize = 1000;
/// The largest number of characters of one entry.
pub const MAX_ENTRY_CHARS: usize = 300;
/// The largest number of values of one row.
pub const MAX_VALUES: usize = 64;
/// The largest size of a table file in bytes, before and after a save.
pub const MAX_FILE_BYTES: usize = 256 * 1024;
/// The largest number of characters of a language name.
pub const MAX_LANGUAGE_CHARS: usize = 32;
/// The shipped hotword list in a source tree, relative to the configuration
/// folder.
pub const SHIPPED_HOTWORDS_FILE: &str = "core/data/hotwords.txt";
/// The copy of the hotword list in the data folder of the user interface,
/// beside the configuration file.
pub const HOTWORDS_COPY_FILE: &str = "hotwords.txt";

/// The hotword list for a configuration without a hotword file, as an
/// absolute path. The hotword table edits it, and the hotword switch of the
/// Setup screen writes it. The rule:
///
/// - The copy `hotwords.txt` beside the configuration file, when it exists.
///   The first start of the user interface makes this copy.
/// - Else the shipped list `core/data/hotwords.txt` of a source tree, when
///   it exists.
/// - Else the copy beside the configuration file. A save makes it.
pub fn default_hotwords_file(config_path: &Path) -> PathBuf {
    let config_path = std::path::absolute(config_path).unwrap_or_else(|_| config_path.into());
    let base = config_path.parent().unwrap_or_else(|| Path::new(""));
    let copy = base.join(HOTWORDS_COPY_FILE);
    let shipped = base.join(SHIPPED_HOTWORDS_FILE);
    if !copy.is_file() && shipped.is_file() {
        shipped
    } else {
        copy
    }
}
/// The text of a save after a change of the file by another program.
pub const FILE_CHANGED: &str =
    "the file changed after the screen read it: read the table again, then make the change again";
/// The stamp of a file that does not exist.
const ABSENT: &str = "absent";

/// Names that Windows reserves for devices. A file with such a name does
/// not work.
const RESERVED_NAMES: &[&str] = &["con", "prn", "aux", "nul"];

/// Refuses a language name that is not a safe file name: 1 to 32 ASCII
/// letters, digits, `-` and `_`, and not a name of a Windows device.
pub fn check_language(language: &str) -> anyhow::Result<()> {
    let shape = !language.is_empty()
        && language.len() <= MAX_LANGUAGE_CHARS
        && language
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    let lower = language.to_ascii_lowercase();
    let device = RESERVED_NAMES.contains(&lower.as_str())
        || ((lower.starts_with("com") || lower.starts_with("lpt"))
            && lower.len() == 4
            && lower.as_bytes()[3].is_ascii_digit());
    if !shape || device {
        bail!(
            "the language name must have 1 to {MAX_LANGUAGE_CHARS} letters (a to z), digits, \"-\" or \"_\", and must not be a device name such as CON"
        );
    }
    Ok(())
}

/// The alias files of a folder: the files that `load_aliases` reads and
/// that have a valid language name, as (language, path), sorted.
fn alias_files(dir: &Path) -> Vec<(String, PathBuf)> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<(String, PathBuf)> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && p.extension()
                    .is_some_and(|x| x.eq_ignore_ascii_case("toml"))
        })
        .filter_map(|p| {
            let stem = p.file_stem()?.to_str()?.to_string();
            check_language(&stem).is_ok().then_some((stem, p))
        })
        .collect();
    files.sort();
    files
}

/// The file of a table for the loaded configuration. Relative paths start
/// at the folder of the configuration file, as in `Config::load`.
fn table_path(config: &Config, config_path: &Path, kind: &TableKind) -> anyhow::Result<PathBuf> {
    Ok(match kind {
        TableKind::Synonyms => config.data.synonyms.clone(),
        TableKind::SoundAlikes => config.data.sound_alikes.clone(),
        TableKind::Fillers => config.data.fillers.clone(),
        TableKind::Hotwords => {
            let file = &config.stt.parakeet.hotwords_file;
            if file.as_os_str().is_empty() {
                default_hotwords_file(config_path)
            } else {
                file.clone()
            }
        }
        TableKind::Aliases { language } => {
            check_language(language)?;
            let dir = &config.data.aliases_dir;
            // The name of an existing file can differ in case.
            alias_files(dir)
                .into_iter()
                .find(|(name, _)| name.eq_ignore_ascii_case(language))
                .map(|(_, path)| path)
                .unwrap_or_else(|| dir.join(format!("{language}.toml")))
        }
    })
}

/// The file name end of a table file of this kind: `.txt` for the
/// hotwords, `.toml` for the other tables.
pub fn table_file_end(kind: &TableKind) -> &'static str {
    match kind {
        TableKind::Hotwords => ".txt",
        _ => ".toml",
    }
}

/// True when the file name of `path` ends with `end`, compared without
/// case.
pub fn has_file_end(path: &Path, end: &str) -> bool {
    path.file_name()
        .map(|name| name.to_string_lossy().to_lowercase())
        .is_some_and(|name| {
            // A colon in a file name is an alternate data stream on Windows.
            name.len() > end.len() && name.ends_with(end) && !name.contains(':')
        })
}

#[cfg(test)]
mod file_end_tests {
    use super::has_file_end;
    use std::path::Path;

    #[test]
    fn test_a_file_name_with_a_colon_does_not_have_the_file_end() {
        assert!(has_file_end(Path::new("data/synonyms.TOML"), ".toml"));
        assert!(!has_file_end(Path::new("data/x:evil.toml"), ".toml"));
        assert!(!has_file_end(Path::new("data/x.toml:evil"), ".toml"));
        assert!(!has_file_end(Path::new("data/.toml"), ".toml"));
    }
}

/// The problem of a table file that the Words screen must not write: its
/// file name does not end with the end of its kind. A hand-edited
/// configuration can name any file. The read stays possible.
pub fn write_end_problem(kind: &TableKind, path: &Path) -> Option<String> {
    let end = table_file_end(kind);
    (!has_file_end(path, end)).then(|| {
        format!(
            "the Words screen writes this table only to a file that ends with {end}, \
             correct the path in the configuration file: {}",
            path.display()
        )
    })
}

/// The stamp of a file content.
fn version_of(text: Option<&str>) -> String {
    match text {
        None => ABSENT.to_string(),
        Some(text) => {
            let mut hasher = DefaultHasher::new();
            text.hash(&mut hasher);
            format!("{:016x}-{}", hasher.finish(), text.len())
        }
    }
}

/// The text of a table file. `None` when the file does not exist.
fn read_text(path: &Path) -> anyhow::Result<Option<String>> {
    match std::fs::metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e).with_context(|| format!("cannot read {}", path.display())),
        Ok(meta) if meta.len() > MAX_FILE_BYTES as u64 => bail!(
            "{} is larger than {} KiB, edit the file by hand",
            path.display(),
            MAX_FILE_BYTES / 1024
        ),
        Ok(_) => {}
    }
    std::fs::read_to_string(path)
        .map(Some)
        .with_context(|| format!("cannot read {}", path.display()))
}

/// The error of content that the rows cannot hold.
fn by_hand(what: impl std::fmt::Display) -> anyhow::Error {
    anyhow!("{what}: the screen cannot show this, edit the file by hand")
}

const BOM: char = '\u{feff}';

/// The byte order mark and the text after it.
fn split_bom(text: &str) -> (&str, &str) {
    match text.strip_prefix(BOM) {
        Some(body) => ("\u{feff}", body),
        None => ("", text),
    }
}

/// Reads the table. The rows are in the order of the file.
pub fn read_table(config_path: &Path, kind: &TableKind) -> anyhow::Result<TableFile> {
    let config = Config::load(config_path)?;
    let path = table_path(&config, config_path, kind)?;
    let text = read_text(&path)?;
    let rows = match &text {
        Some(text) => parse_rows(kind, text).with_context(|| path.display().to_string())?,
        None => Vec::new(),
    };
    // All tables give the languages, so that the screen can show them.
    let languages = alias_files(&config.data.aliases_dir)
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    let language = match kind {
        TableKind::Aliases { .. } => path
            .file_stem()
            .and_then(|s| s.to_str())
            .map(str::to_string),
        _ => None,
    };
    Ok(TableFile {
        path,
        exists: text.is_some(),
        rows,
        version: version_of(text.as_deref()),
        hotwords_off: config.stt.parakeet.hotwords_file.as_os_str().is_empty(),
        languages,
        language,
    })
}

/// The phrase of a hotword line as `Hotwords::parse` reads it. `None` for
/// a blank line and a comment.
fn hotword_phrase(line: &str) -> Option<String> {
    let phrase = line.split_whitespace().collect::<Vec<_>>().join(" ");
    (!phrase.is_empty() && !phrase.starts_with('#')).then_some(phrase)
}

/// The texts of a value: a text, or a list of texts. `None` for any other
/// value. `allow_text` false accepts only a list.
fn texts_of(item: &Item, allow_text: bool, allow_list: bool) -> Option<Vec<String>> {
    match item.as_value()? {
        Value::String(s) if allow_text => Some(vec![s.value().clone()]),
        Value::Array(a) if allow_list => a.iter().map(|v| v.as_str().map(str::to_string)).collect(),
        _ => None,
    }
}

/// The one table of a synonym or a sound-alike file, as a standard table.
/// `None` when the document does not have it.
fn named_table<'a>(doc: &'a DocumentMut, name: &str) -> anyhow::Result<Option<&'a Table>> {
    for (key, _) in doc.iter() {
        if key != name {
            return Err(by_hand(format!(
                "the key \"{key}\" is not part of this table"
            )));
        }
    }
    match doc.get(name) {
        None => Ok(None),
        Some(Item::Table(table)) => Ok(Some(table)),
        Some(_) => Err(by_hand(format!("\"{name}\" is not a table"))),
    }
}

fn parse_doc(body: &str) -> anyhow::Result<DocumentMut> {
    body.parse::<DocumentMut>().map_err(|e| {
        by_hand(format!(
            "the file has an error ({})",
            crate::toml_error::edit_error_text(&e, body)
        ))
    })
}

/// The rows of a file text. An error when the rows cannot hold the text.
fn parse_rows(kind: &TableKind, text: &str) -> anyhow::Result<Vec<Row>> {
    let (_, body) = split_bom(text);
    if *kind == TableKind::Hotwords {
        return Ok(body
            .lines()
            .filter_map(hotword_phrase)
            .map(|spoken| Row {
                spoken,
                meant: Vec::new(),
            })
            .collect());
    }
    let doc = parse_doc(body)?;
    let entries = |table: &Table, allow_text: bool, allow_list: bool| -> anyhow::Result<Vec<Row>> {
        table
            .iter()
            .map(|(key, item)| {
                texts_of(item, allow_text, allow_list)
                    .map(|meant| Row {
                        spoken: key.to_string(),
                        meant,
                    })
                    .ok_or_else(|| by_hand(format!("the value of \"{key}\" has another form")))
            })
            .collect()
    };
    match kind {
        TableKind::Synonyms => match named_table(&doc, "synonyms")? {
            Some(table) => entries(table, true, true),
            None => Ok(Vec::new()),
        },
        TableKind::SoundAlikes => match named_table(&doc, "sound_alikes")? {
            Some(table) => entries(table, true, false),
            None => Ok(Vec::new()),
        },
        TableKind::Aliases { .. } => entries(doc.as_table(), false, true),
        TableKind::Fillers => {
            let mut rows = Vec::new();
            for (key, item) in doc.iter() {
                if key != NOT_LAST && key != MAY_BE_LAST {
                    return Err(by_hand(format!(
                        "the key \"{key}\" is not part of this table"
                    )));
                }
                let words = texts_of(item, false, true)
                    .ok_or_else(|| by_hand(format!("\"{key}\" is not a list of words")))?;
                rows.extend(words.into_iter().map(|spoken| Row {
                    spoken,
                    meant: vec![key.to_string()],
                }));
            }
            // The rows of `not_last` come first, as the file has them.
            rows.sort_by_key(|r| r.meant[0] != NOT_LAST);
            Ok(rows)
        }
        TableKind::Hotwords => unreachable!("handled above"),
    }
}

/// The rows without the spaces at the ends of each entry.
fn trimmed(rows: &[Row]) -> Vec<Row> {
    rows.iter()
        .map(|r| Row {
            spoken: r.spoken.trim().to_string(),
            meant: r.meant.iter().map(|m| m.trim().to_string()).collect(),
        })
        .collect()
}

/// The problem of one entry: too long or a control character.
fn entry_problem(n: usize, text: &str) -> Option<String> {
    if text.chars().count() > MAX_ENTRY_CHARS {
        Some(format!(
            "row {n}: the entry is too long, an entry has at most {MAX_ENTRY_CHARS} characters"
        ))
    } else if text.chars().any(char::is_control) {
        Some(format!("row {n}: the entry has a control character"))
    } else if text.chars().any(is_format_character) {
        Some(format!(
            "row {n}: the entry has an invisible format character (for example a zero-width space or a direction mark)"
        ))
    } else {
        None
    }
}

/// True for a character of the Unicode category Cf (format): it is not
/// visible, but it changes the text, for example U+200B (zero-width
/// space), U+202E (right-to-left override), and U+FEFF.
fn is_format_character(c: char) -> bool {
    matches!(
        c as u32,
        0x00AD
            | 0x0600..=0x0605
            | 0x061C
            | 0x06DD
            | 0x070F
            | 0x0890..=0x0891
            | 0x08E2
            | 0x180E
            | 0x200B..=0x200F
            | 0x202A..=0x202E
            | 0x2060..=0x2064
            | 0x2066..=0x206F
            | 0xFEFF
            | 0xFFF9..=0xFFFB
            | 0x110BD
            | 0x110CD
            | 0x13430..=0x1343F
            | 0x1BCA0..=0x1BCA3
            | 0x1D173..=0x1D17A
            | 0xE0001
            | 0xE0020..=0xE007F
    )
}

/// The limits of the rows and the rules of the file format. Rows with such
/// a problem cannot go into the file, so the check stops there.
fn limit_problems(kind: &TableKind, rows: &[Row]) -> Vec<String> {
    if rows.len() > MAX_ROWS {
        return vec![format!(
            "the table has {} rows, a table has at most {MAX_ROWS} rows",
            rows.len()
        )];
    }
    let mut problems = Vec::new();
    for (i, row) in rows.iter().enumerate() {
        let n = i + 1;
        let entry = |text: &str| entry_problem(n, text);
        if row.spoken.is_empty() {
            problems.push(format!("row {n}: the first field is empty"));
        }
        let first = std::iter::once(row.spoken.as_str())
            .chain(row.meant.iter().map(String::as_str))
            .find_map(entry);
        problems.extend(first);
        if row.meant.len() > MAX_VALUES {
            problems.push(format!(
                "row {n}: the row has {} values, a row has at most {MAX_VALUES} values",
                row.meant.len()
            ));
        }
        match kind {
            TableKind::SoundAlikes if row.meant.len() != 1 => {
                problems.push(format!("row {n}: a sound-alike has exactly one meant word"));
            }
            TableKind::Fillers if row.meant != [NOT_LAST] && row.meant != [MAY_BE_LAST] => {
                problems.push(format!(
                    "row {n}: the list must be \"{NOT_LAST}\" or \"{MAY_BE_LAST}\""
                ));
            }
            TableKind::Hotwords if !row.meant.is_empty() => {
                problems.push(format!("row {n}: a hotword has no second field"));
            }
            TableKind::Hotwords if row.spoken.starts_with('#') => {
                problems.push(format!(
                    "row {n}: a hotword cannot start with \"#\", the program reads such a line as a comment"
                ));
            }
            _ => {}
        }
        // A TOML table cannot have a key two times.
        let keyed = !matches!(kind, TableKind::Fillers | TableKind::Hotwords);
        if keyed && !row.spoken.is_empty() {
            if let Some(m) = rows[..i].iter().position(|r| r.spoken == row.spoken) {
                problems.push(format!(
                    "row {n}: \"{}\" is also in row {}, a key can be in the file only one time",
                    row.spoken,
                    m + 1
                ));
            }
        }
    }
    problems
}

/// The text of a new file of the table.
fn template(kind: &TableKind) -> String {
    match kind {
        TableKind::Aliases { language } => format!(
            "# Extra spoken phrases for the language \"{language}\".\n\
             # Key: the token template of the command, exactly as in the command map.\n\
             # Value: a list of phrases in VoiceAttack grammar syntax.\n"
        ),
        TableKind::Synonyms => "[synonyms]\n".to_string(),
        TableKind::SoundAlikes => "[sound_alikes]\n".to_string(),
        TableKind::Fillers => format!("{NOT_LAST} = []\n{MAY_BE_LAST} = []\n"),
        TableKind::Hotwords => "# Hotwords for the local engine (stt.parakeet.hotwords_file).\n\
             # One phrase on each line. A line that starts with \"#\" is a comment.\n"
            .to_string(),
    }
}

fn raw_has_comment(raw: Option<&RawString>) -> bool {
    raw.and_then(RawString::as_str)
        .is_some_and(|s| s.contains('#'))
}

/// True when a list has a comment inside its brackets.
fn array_has_comment(array: &Array) -> bool {
    raw_has_comment(Some(array.trailing()))
        || array
            .iter()
            .any(|v| raw_has_comment(v.decor().prefix()) || raw_has_comment(v.decor().suffix()))
}

/// A new value with the decor of the old value: the spaces and a comment at
/// the end of the line stay.
fn with_decor(mut value: Value, old: Option<&Item>) -> Item {
    if let Some(old) = old.and_then(Item::as_value) {
        *value.decor_mut() = old.decor().clone();
    }
    Item::Value(value)
}

/// The item of a row. An unchanged value is the old item. `text` true
/// writes one value as a text when the old value is a text, or always when
/// `list` is false.
fn value_item(
    key: &str,
    values: &[String],
    old: Option<&Item>,
    text: bool,
    list: bool,
) -> anyhow::Result<Item> {
    if let Some(old) = old {
        // The program trims the values: spaces at the ends are no change.
        let old_values = texts_of(old, text, list)
            .map(|v| v.iter().map(|s| s.trim().to_string()).collect::<Vec<_>>());
        if old_values.as_deref() == Some(values) {
            return Ok(old.clone());
        }
        if let Some(Value::Array(array)) = old.as_value() {
            if array_has_comment(array) {
                return Err(by_hand(format!(
                    "the list of \"{key}\" has a comment inside, and the change would remove it"
                )));
            }
        }
    }
    let old_is_text = old
        .and_then(Item::as_value)
        .is_some_and(|v| matches!(v, Value::String(_)));
    let as_text = text && values.len() == 1 && (old_is_text || !list);
    let value = if as_text {
        Value::from(values[0].as_str())
    } else {
        let mut array = Array::new();
        for v in values {
            array.push(v.as_str());
        }
        Value::Array(array)
    };
    Ok(with_decor(value, old))
}

/// Puts `lines` in front of the prefix of a decor.
fn prepend(decor: &mut toml_edit::Decor, lines: &str) {
    let old = decor
        .prefix()
        .and_then(RawString::as_str)
        .unwrap_or_default()
        .to_string();
    decor.set_prefix(format!("{lines}{old}"));
}

/// The comment lines in front of a key, when it has a comment.
fn key_comment(key: &Key) -> Option<String> {
    let prefix = key.leaf_decor().prefix()?.as_str()?;
    prefix.contains('#').then(|| prefix.to_string())
}

/// The comment at the end of the line of an entry (`a = "b" # why`), as a
/// comment line of its own.
fn line_end_comment(item: &Item) -> Option<String> {
    let suffix = item.as_value()?.decor().suffix()?.as_str()?;
    let comment = suffix.trim();
    comment.starts_with('#').then(|| {
        format!(
            "{comment}
"
        )
    })
}

/// Puts the rows into a table in the order of the rows. A row with the key
/// of an entry gets the key and its comment lines, and the old value when
/// the values did not change. The comment lines of an entry without a row
/// go to the next kept entry. `header` is a comment that must stay first.
/// Gives the comment lines that have no entry to go to.
fn put_rows(
    table: &mut Table,
    rows: &[Row],
    header: Option<String>,
    item: &dyn Fn(&Row, Option<&Item>) -> anyhow::Result<Item>,
) -> anyhow::Result<String> {
    let old: Vec<(Key, Item)> = table
        .iter()
        .filter_map(|(k, _)| table.get_key_value(k))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let mut used: Vec<Option<usize>> = vec![None; old.len()];
    let mut out: Vec<(Key, Item)> = Vec::new();
    for row in rows {
        // The program trims the keys: a key with spaces at its ends is the
        // same key, and stays as the file has it.
        let found =
            (0..old.len()).find(|&i| used[i].is_none() && old[i].0.get().trim() == row.spoken);
        match found {
            Some(i) => {
                let value = item(row, Some(&old[i].1))?;
                used[i] = Some(out.len());
                out.push((old[i].0.clone(), value));
            }
            None => out.push((Key::new(row.spoken.clone()), item(row, None)?)),
        }
    }
    let mut pending = String::new();
    for (i, slot) in used.iter().enumerate() {
        match slot {
            None => {
                pending.push_str(&key_comment(&old[i].0).unwrap_or_default());
                pending.push_str(&line_end_comment(&old[i].1).unwrap_or_default());
            }
            Some(j) if !pending.is_empty() => {
                prepend(out[*j].0.leaf_decor_mut(), &pending);
                pending.clear();
            }
            Some(_) => {}
        }
    }
    if let Some(header) = header {
        match out.first_mut() {
            Some((key, _)) => prepend(key.leaf_decor_mut(), &header),
            None => pending = format!("{header}{pending}"),
        }
    }
    table.clear();
    for (key, value) in out {
        table.insert_formatted(&key, value);
    }
    Ok(pending)
}

/// Takes the header comment of a document whose entries are at the top
/// level: the comment lines in front of the first key, or the comment of a
/// file without a key.
fn take_header(doc: &mut DocumentMut) -> Option<String> {
    let first = doc.iter().next().map(|(k, _)| k.to_string());
    match first {
        Some(first) => {
            let mut key = doc.key_mut(&first)?;
            let prefix = key.leaf_decor().prefix()?.as_str()?.to_string();
            if !prefix.contains('#') {
                return None;
            }
            key.leaf_decor_mut().set_prefix("");
            Some(prefix)
        }
        None => {
            let trailing = doc.trailing().as_str().unwrap_or_default().to_string();
            if !trailing.contains('#') {
                return None;
            }
            doc.set_trailing("");
            Some(trailing)
        }
    }
}

/// Adds comment lines to the end of the document.
fn append_trailing(doc: &mut DocumentMut, lines: &str) {
    if lines.is_empty() {
        return;
    }
    let trailing = doc.trailing().as_str().unwrap_or_default().to_string();
    doc.set_trailing(format!("{lines}{trailing}"));
}

/// The new text of a TOML table file.
fn render_toml(kind: &TableKind, body: &str, rows: &[Row]) -> anyhow::Result<String> {
    // The same checks of the form as the read.
    parse_rows(kind, body)?;
    let mut doc = parse_doc(body)?;
    match kind {
        TableKind::Synonyms | TableKind::SoundAlikes => {
            let (name, text, list) = if *kind == TableKind::Synonyms {
                ("synonyms", true, true)
            } else {
                ("sound_alikes", true, false)
            };
            if doc.get(name).is_none() {
                doc.insert(name, Item::Table(Table::new()));
            }
            let table = doc
                .get_mut(name)
                .and_then(Item::as_table_mut)
                .expect("the table exists");
            let rest = put_rows(table, rows, None, &|row, old| {
                value_item(&row.spoken, &row.meant, old, text, list)
            })?;
            append_trailing(&mut doc, &rest);
        }
        TableKind::Aliases { .. } => {
            let header = take_header(&mut doc);
            let rest = put_rows(doc.as_table_mut(), rows, header, &|row, old| {
                value_item(&row.spoken, &row.meant, old, false, true)
            })?;
            append_trailing(&mut doc, &rest);
        }
        TableKind::Fillers => {
            for list in [NOT_LAST, MAY_BE_LAST] {
                let words: Vec<String> = rows
                    .iter()
                    .filter(|r| r.meant == [list])
                    .map(|r| r.spoken.clone())
                    .collect();
                let new = value_item(list, &words, doc.get(list), false, true)?;
                match doc.get_mut(list) {
                    Some(item) => *item = new,
                    None => {
                        doc.insert(list, new);
                    }
                }
            }
        }
        TableKind::Hotwords => unreachable!("not a TOML file"),
    }
    Ok(doc.to_string())
}

/// The new text of the hotword file. The lines in front of the first
/// phrase are the header. Each phrase keeps its comment lines.
fn render_hotwords(body: &str, rows: &[Row]) -> String {
    let mut header: Option<Vec<&str>> = None;
    let mut before: Vec<&str> = Vec::new();
    // (comment lines, line, phrase)
    let mut items: Vec<(Vec<&str>, &str, String)> = Vec::new();
    for line in body.lines() {
        match hotword_phrase(line) {
            None => before.push(line),
            Some(phrase) => {
                let lines = std::mem::take(&mut before);
                if header.is_none() {
                    header = Some(lines);
                    items.push((Vec::new(), line, phrase));
                } else {
                    items.push((lines, line, phrase));
                }
            }
        }
    }
    let (header, trailer) = match header {
        Some(header) => (header, before),
        None => (before, Vec::new()),
    };
    let mut used = vec![false; items.len()];
    let mut out: Vec<(Vec<String>, String)> = Vec::new();
    let mut index_of: Vec<Option<usize>> = vec![None; items.len()];
    for row in rows {
        let key = hotword_phrase(&row.spoken).unwrap_or_default();
        match (0..items.len()).find(|&i| !used[i] && items[i].2 == key) {
            Some(i) => {
                used[i] = true;
                index_of[i] = Some(out.len());
                let comments = items[i].0.iter().map(|s| s.to_string()).collect();
                out.push((comments, items[i].1.to_string()));
            }
            None => out.push((Vec::new(), row.spoken.clone())),
        }
    }
    let mut pending: Vec<String> = Vec::new();
    for (i, slot) in index_of.iter().enumerate() {
        match slot {
            None => pending.extend(
                items[i]
                    .0
                    .iter()
                    .filter(|l| l.contains('#'))
                    .map(|s| s.to_string()),
            ),
            Some(j) => {
                let mut lines = std::mem::take(&mut pending);
                lines.append(&mut out[*j].0);
                out[*j].0 = lines;
            }
        }
    }
    let mut lines: Vec<String> = header.iter().map(|s| s.to_string()).collect();
    for (comments, line) in out {
        lines.extend(comments);
        lines.push(line);
    }
    lines.extend(pending);
    lines.extend(trailer.iter().map(|s| s.to_string()));
    let mut text = lines.join("\n");
    if !text.is_empty() && (body.is_empty() || body.ends_with('\n')) {
        text.push('\n');
    }
    text
}

/// The new text of the file for the rows. `old` is the text of the file,
/// `None` when the file does not exist. The text keeps the byte order mark
/// and the line ends of the old text. The rows must be trimmed.
pub fn render(kind: &TableKind, old: Option<&str>, rows: &[Row]) -> anyhow::Result<String> {
    let fresh = template(kind);
    let (bom, body) = split_bom(old.unwrap_or(&fresh));
    // `toml_edit` and the hotword lines use "\n" only. The line ends come
    // back after the render.
    let plain = body.replace("\r\n", "\n");
    let out = match kind {
        TableKind::Hotwords => render_hotwords(&plain, rows),
        _ => render_toml(kind, &plain, rows)?,
    };
    Ok(format!("{bom}{}", restore_line_ends(body, &out)))
}

/// Gives each line of `out` the line end of the same line in `old`. A line
/// that `old` does not have gets the line end that `old` has most often.
/// Thus a file with mixed line ends stays the same where it does not change.
fn restore_line_ends(old: &str, out: &str) -> String {
    let crlf = old.matches("\r\n").count();
    let lf = old.matches('\n').count() - crlf;
    let usual = if crlf > lf { "\r\n" } else { "\n" };
    let mut ends: HashMap<&str, VecDeque<&str>> = HashMap::new();
    for piece in old.split_inclusive('\n') {
        let (line, end) = match piece.strip_suffix("\r\n") {
            Some(line) => (line, "\r\n"),
            None => match piece.strip_suffix('\n') {
                Some(line) => (line, "\n"),
                None => (piece, ""),
            },
        };
        ends.entry(line).or_default().push_back(end);
    }
    let mut text = String::with_capacity(out.len() + out.len() / 16);
    for piece in out.split_inclusive('\n') {
        match piece.strip_suffix('\n') {
            Some(line) => {
                let end = ends
                    .get_mut(line)
                    .and_then(VecDeque::pop_front)
                    .filter(|end| !end.is_empty())
                    .unwrap_or(usual);
                text.push_str(line);
                text.push_str(end);
            }
            None => text.push_str(piece),
        }
    }
    text
}

/// The rows ready for the save: the file, its text now, the new text, and
/// the check.
struct Prepared {
    path: PathBuf,
    old: Option<String>,
    /// `None` when the rows cannot go into the file.
    text: Option<String>,
    check: TableCheck,
}

fn refused(problem: String) -> TableCheck {
    TableCheck {
        problems: vec![problem],
        warnings: Vec::new(),
    }
}

/// The limits of the counts. They come before any copy of the rows and
/// before any file access.
fn count_problem(rows: &[Row]) -> Option<String> {
    if rows.len() > MAX_ROWS {
        return Some(format!(
            "the table has {} rows, a table has at most {MAX_ROWS} rows",
            rows.len()
        ));
    }
    rows.iter()
        .position(|r| r.meant.len() > MAX_VALUES)
        .map(|i| {
            format!(
                "row {}: the row has {} values, a row has at most {MAX_VALUES} values",
                i + 1,
                rows[i].meant.len()
            )
        })
}

fn prepare(config_path: &Path, kind: &TableKind, rows: &[Row]) -> anyhow::Result<Prepared> {
    let config = Config::load(config_path)?;
    let path = table_path(&config, config_path, kind)?;
    let rows = trimmed(rows);
    let limits = limit_problems(kind, &rows);
    let old = read_text(&path)?;
    let mut prepared = Prepared {
        path,
        old,
        text: None,
        check: TableCheck::default(),
    };
    if !limits.is_empty() {
        prepared.check.problems = limits;
        return Ok(prepared);
    }
    // A new file needs its folder. The program does not make folders.
    let folder = prepared
        .path
        .parent()
        .filter(|f| !f.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    if !folder.is_dir() {
        prepared.check = refused(format!("the folder {} does not exist", folder.display()));
        return Ok(prepared);
    }
    let text = match render(kind, prepared.old.as_deref(), &rows) {
        Ok(text) => text,
        Err(e) => {
            let problem = format!("{}: {e:#}", prepared.path.display());
            prepared.check = refused(problem);
            return Ok(prepared);
        }
    };
    if text.len() > MAX_FILE_BYTES {
        prepared.check = refused(format!(
            "the file would be larger than {} KiB",
            MAX_FILE_BYTES / 1024
        ));
        return Ok(prepared);
    }
    prepared.check = check_text(&config, kind, &prepared.path, &text)?;
    prepared.text = Some(text);
    Ok(prepared)
}

/// Removes the lines of `base` from `lines`, each one time.
fn without(lines: Vec<String>, base: &[String]) -> Vec<String> {
    let mut base = base.to_vec();
    lines
        .into_iter()
        .filter(|line| match base.iter().position(|b| b == line) {
            Some(i) => {
                base.swap_remove(i);
                false
            }
            None => true,
        })
        .collect()
}

/// The lines of one run of `voicebridge check`. The texts have the path of
/// the real table in place of the temporary file.
#[derive(Debug, Clone)]
struct Lines {
    problems: Vec<String>,
    warnings: Vec<String>,
    /// The load problems of the command data: the voice commands file, the
    /// vocabulary folder, and the alias folder. Without them a part of the
    /// check cannot run.
    data_problems: Vec<String>,
    /// The error when the command data does not load. Then the check of
    /// the tables did not run.
    load_error: Option<String>,
}

/// The text of the table without an entry.
fn empty_text(kind: &TableKind) -> &'static str {
    match kind {
        TableKind::Synonyms => "[synonyms]\n",
        TableKind::SoundAlikes => "[sound_alikes]\n",
        TableKind::Fillers => "not_last = []\nmay_be_last = []\n",
        TableKind::Hotwords | TableKind::Aliases { .. } => "",
    }
}

/// Runs `voicebridge check` on a copy of the configuration that reads the
/// table from `text`. `None` is the table without an entry (an alias
/// language without its file).
fn run_check(
    config: &Config,
    kind: &TableKind,
    path: &Path,
    text: Option<&str>,
) -> anyhow::Result<Lines> {
    let dir = tempfile::Builder::new()
        .prefix("voicebridge-table-")
        .tempdir()
        .context("cannot make a temporary folder for the check")?;
    let name = path
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_else(|| "table".into());
    let mut modified = config.clone();
    // The temporary path and the real path, for the texts.
    let temp_path: (PathBuf, PathBuf);
    let write = |file: &Path, text: &str| {
        std::fs::write(file, text)
            .with_context(|| format!("cannot write the temporary file {}", file.display()))
    };
    match kind {
        TableKind::Aliases { .. } => {
            let aliases = dir.path().join("aliases");
            std::fs::create_dir(&aliases).context("cannot make a temporary folder")?;
            // The other alias files as they are, and this language as the rows.
            copy_other_alias_files(&config.data.aliases_dir, &aliases, &name)?;
            if let Some(text) = text {
                write(&aliases.join(&name), text)?;
            }
            temp_path = (aliases.clone(), config.data.aliases_dir.clone());
            modified.data.aliases_dir = aliases;
        }
        _ => {
            let file = dir.path().join(&name);
            write(&file, text.unwrap_or(empty_text(kind)))?;
            match kind {
                TableKind::Synonyms => modified.data.synonyms = file.clone(),
                TableKind::SoundAlikes => modified.data.sound_alikes = file.clone(),
                TableKind::Fillers => modified.data.fillers = file.clone(),
                TableKind::Hotwords => {
                    // The check runs as with the hotwords on, also when the
                    // configuration has them off.
                    modified.stt.engine = "parakeet".to_string();
                    modified.stt.parakeet.hotwords_file = file.clone();
                }
                TableKind::Aliases { .. } => unreachable!("handled above"),
            }
            temp_path = (file, path.to_path_buf());
        }
    }
    // No key is necessary: the lines of the key are the same in each run.
    // The saved report of the profile does not count: no folder.
    let found = check_config_in(&modified, |_| None, None);
    let (temp, real) = (
        temp_path.0.display().to_string(),
        temp_path.1.display().to_string(),
    );
    let rewrite = |line: &String| line.replace(temp.as_str(), &real);
    let table_files = [
        modified.data.synonyms.display().to_string(),
        modified.data.sound_alikes.display().to_string(),
        modified.data.fillers.display().to_string(),
    ];
    let load_error = match &found.report {
        Some(_) => None,
        None => Some(
            crate::config::load_commands(&modified)
                .err()
                .map(|e| format!("{e:#}"))
                .unwrap_or_else(|| "the pipeline did not build".to_string()),
        ),
    };
    let data_problems = found
        .report
        .as_ref()
        .map(|r| {
            r.problems
                .iter()
                .filter(|p| !table_files.iter().any(|f| p.contains(f.as_str())))
                .map(rewrite)
                .collect()
        })
        .unwrap_or_default();
    Ok(Lines {
        problems: found.problems.iter().map(rewrite).collect(),
        warnings: found.warnings.iter().map(rewrite).collect(),
        data_problems,
        load_error,
    })
}

/// The largest number of base checks that the cache keeps.
const BASE_CACHE_SIZE: usize = 8;

/// The base checks by the stamp of their inputs. See `base_lines`.
static BASE_CACHE: Mutex<Vec<(u64, Lines)>> = Mutex::new(Vec::new());

/// Adds the size and the time of a file to the hash. The content of a small
/// file goes in too, because an edit can keep the size and the time.
fn hash_file(path: &Path, content: bool, hasher: &mut DefaultHasher) {
    path.hash(hasher);
    if let Ok(meta) = std::fs::metadata(path) {
        meta.len().hash(hasher);
        meta.modified().ok().hash(hasher);
        if content && meta.is_file() && meta.len() <= MAX_FILE_BYTES as u64 {
            std::fs::read(path).ok().hash(hasher);
        }
    }
}

/// Adds each file of a folder to the hash, sorted by name.
fn hash_folder(dir: &Path, content: bool, hasher: &mut DefaultHasher) {
    dir.hash(hasher);
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut files: Vec<PathBuf> = entries.filter_map(Result::ok).map(|e| e.path()).collect();
    files.sort();
    for file in files {
        hash_file(&file, content, hasher);
    }
}

/// The stamp of all inputs of the base check: the values of the
/// configuration and each file that `check_config` reads.
fn base_stamp(config: &Config, kind: &TableKind, path: &Path) -> u64 {
    let mut hasher = DefaultHasher::new();
    // The configuration holds no key, only the name of a variable.
    format!("{config:?}").hash(&mut hasher);
    format!("{kind:?}").hash(&mut hasher);
    path.hash(&mut hasher);
    let ed = &config.edcopilot;
    let data = &config.data;
    for file in [&ed.voice_commands_ini, &ed.profile] {
        hash_file(file, false, &mut hasher);
    }
    for file in [
        &data.synonyms,
        &data.sound_alikes,
        &data.fillers,
        &config.stt.parakeet.hotwords_file,
    ] {
        hash_file(file, true, &mut hasher);
    }
    let parakeet = &config.stt.parakeet;
    hash_file(&parakeet.model_dir.join("tokens.txt"), false, &mut hasher);
    parakeet.model_dir.is_dir().hash(&mut hasher);
    hash_folder(&data.aliases_dir, true, &mut hasher);
    hash_folder(&ed.dynamic_vars_dir, false, &mut hasher);
    let log = &config.diagnostics.log_file;
    (log.is_dir(), log.parent().map(Path::is_dir)).hash(&mut hasher);
    hasher.finish()
}

fn lock_cache() -> std::sync::MutexGuard<'static, Vec<(u64, Lines)>> {
    BASE_CACHE.lock().unwrap_or_else(|e| e.into_inner())
}

/// The check with this table empty. It does not depend on the rows, so it
/// is kept until a file or a value of the configuration changes.
fn base_lines(config: &Config, kind: &TableKind, path: &Path) -> anyhow::Result<Lines> {
    let stamp = base_stamp(config, kind, path);
    let cached = lock_cache()
        .iter()
        .find(|(key, _)| *key == stamp)
        .map(|(_, lines)| lines.clone());
    if let Some(lines) = cached {
        return Ok(lines);
    }
    let lines = run_check(config, kind, path, None)?;
    let mut cache = lock_cache();
    if cache.len() >= BASE_CACHE_SIZE {
        cache.remove(0);
    }
    cache.push((stamp, lines.clone()));
    Ok(lines)
}

/// The check of the rows: the lines of `voicebridge check` with the rows in
/// the table that the check with the table empty does not have. Thus a line
/// that the saved table causes stays visible, and a line that exists
/// without the table is not a line of this table.
///
/// The check says "no problem" only when the full check ran on the rows.
/// Each cause that stops a part of the check of this table is a problem:
/// the command data does not load, a file of the command data is absent,
/// or the hotwords cannot be judged (a bad score, no model pieces).
fn check_text(
    config: &Config,
    kind: &TableKind,
    path: &Path,
    text: &str,
) -> anyhow::Result<TableCheck> {
    // Without the EDCoPilot folder, the check has no runtime grammar and no
    // vocabulary. The line of `Config::validate` is in each run, so it
    // would cancel out against the check with the table empty.
    if let Some(problem) = config.edcopilot_dir_problem() {
        return Ok(refused(format!("the check is not complete: {problem}")));
    }
    let rows = run_check(config, kind, path, Some(text))?;
    if let Some(error) = &rows.load_error {
        return Ok(refused(format!(
            "the check cannot load the command data: {error}"
        )));
    }
    let base = base_lines(config, kind, path)?;
    let mut check = TableCheck {
        problems: without(rows.problems.clone(), &base.problems),
        warnings: without(rows.warnings.clone(), &base.warnings),
    };
    for line in &rows.data_problems {
        check
            .problems
            .push(format!("the check is not complete: {line}"));
    }
    let parakeet = &config.stt.parakeet;
    // The saved configuration uses the hotwords.
    let hotwords_on =
        config.stt.engine == "parakeet" && !parakeet.hotwords_file.as_os_str().is_empty();
    if *kind == TableKind::Hotwords {
        let score_key = "stt.parakeet.hotwords_score";
        for line in rows.problems.iter().filter(|p| p.starts_with(score_key)) {
            check
                .problems
                .push(format!("the check cannot judge the hotwords: {line}"));
        }
        // The console warning of an empty list is also in the check with
        // the table empty. It stays a warning of this table.
        let empty = "stt.parakeet.hotwords_file has no hotword";
        for line in rows.warnings.iter().filter(|w| w.starts_with(empty)) {
            if !check.warnings.contains(line) {
                check.warnings.push(line.clone());
            }
        }
        let tokens = parakeet.model_dir.join("tokens.txt");
        if !tokens.is_file() {
            let text = format!(
                "the check cannot test the hotwords against the model pieces: {} is absent",
                tokens.display()
            );
            if hotwords_on {
                check.problems.push(text);
            } else {
                // `run::engine_problems` tests the spelling again at the
                // start of a session (`runtime::launch`).
                check.warnings.push(format!(
                    "{text}. The program tests them again at the session start."
                ));
            }
        }
    } else if hotwords_on && matches!(kind, TableKind::Aliases { .. }) {
        // Only the alias rows give words to the hotword check: the names
        // and the command words come from the matcher. The synonyms, the
        // sound-alikes and the filler words do not go into the matcher.
        if let Some(line) = hotword_list_problem(config, &rows.problems) {
            check
                .problems
                .push(format!("the check cannot judge the hotwords: {line}"));
        }
    }
    Ok(check)
}

/// The reason why the hotword list of the configuration cannot be judged:
/// the score problem of `voicebridge check`, or the load error of the list.
/// `None` when the list loads.
fn hotword_list_problem(config: &Config, problems: &[String]) -> Option<String> {
    let parakeet = &config.stt.parakeet;
    if !(hotwords::MIN_SCORE..=hotwords::MAX_SCORE).contains(&parakeet.hotwords_score) {
        let score = problems
            .iter()
            .find(|p| p.starts_with("stt.parakeet.hotwords_score"))
            .cloned();
        return Some(score.unwrap_or_else(|| {
            format!("stt.parakeet.hotwords_score is {}", parakeet.hotwords_score)
        }));
    }
    Hotwords::load(&parakeet.hotwords_file, parakeet.hotwords_score)
        .context("stt.parakeet.hotwords_file")
        .err()
        .map(|e| format!("{e:#}"))
}

/// Copies each file of the alias folder that `load_aliases` reads, except
/// the file `skip`. Thus the check loads all files that the program loads.
fn copy_other_alias_files(from: &Path, to: &Path, skip: &std::ffi::OsStr) -> anyhow::Result<()> {
    let Ok(entries) = std::fs::read_dir(from) else {
        return Ok(());
    };
    for path in entries.filter_map(Result::ok).map(|e| e.path()) {
        let toml = path.is_file()
            && path
                .extension()
                .is_some_and(|x| x.eq_ignore_ascii_case("toml"));
        let Some(file) = path.file_name() else {
            continue;
        };
        if !toml || file == skip || to.join(file).exists() {
            continue;
        }
        std::fs::copy(&path, to.join(file)).context("cannot copy an alias file")?;
    }
    Ok(())
}

/// Loads the rows as the program loads the file, with the real commands and
/// the other tables of the saved configuration. A configuration that does
/// not load gives its error as the problem.
pub fn check_table(config_path: &Path, kind: &TableKind, rows: &[Row]) -> TableCheck {
    if let Some(problem) = count_problem(rows) {
        return refused(problem);
    }
    match prepare(config_path, kind, rows) {
        Ok(prepared) => prepared.check,
        Err(e) => refused(format!("{e:#}")),
    }
}

#[cfg(test)]
thread_local! {
    /// Tests only: runs in `save_table` right before the last read of the
    /// file, in the thread of the save.
    static BEFORE_WRITE: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        const { std::cell::RefCell::new(None) };
}

/// Writes the rows when the check has no problem, when each warning is in
/// `confirmed`, and when the file has the stamp `version` of the read.
/// Keeps the header comment. The check runs again here: the result of an
/// earlier check does not count. The file is read again right before the
/// write, so a change during the check refuses the save.
pub fn save_table(
    config_path: &Path,
    kind: &TableKind,
    rows: &[Row],
    confirmed: &[String],
    version: &str,
) -> anyhow::Result<TableSave> {
    if let Some(problem) = count_problem(rows) {
        return Ok(TableSave {
            written: false,
            check: refused(problem),
            unconfirmed: Vec::new(),
            version: version.to_string(),
        });
    }
    let prepared = prepare(config_path, kind, rows)?;
    let current = version_of(prepared.old.as_deref());
    let refuse = |check: TableCheck, unconfirmed: Vec<String>, version: String| TableSave {
        written: false,
        check,
        unconfirmed,
        version,
    };
    if let Some(problem) = write_end_problem(kind, &prepared.path) {
        return Ok(refuse(refused(problem), Vec::new(), current));
    }
    if current != version {
        return Ok(refuse(
            refused(FILE_CHANGED.to_string()),
            Vec::new(),
            current,
        ));
    }
    let Some(text) = prepared
        .text
        .as_ref()
        .filter(|_| prepared.check.problems.is_empty())
    else {
        return Ok(refuse(prepared.check, Vec::new(), current));
    };
    let unconfirmed: Vec<String> = prepared
        .check
        .warnings
        .iter()
        .filter(|w| !confirmed.contains(w))
        .cloned()
        .collect();
    if !unconfirmed.is_empty() {
        return Ok(refuse(prepared.check, unconfirmed, current));
    }
    if prepared.old.as_deref() == Some(text.as_str()) {
        return Ok(refuse(prepared.check, Vec::new(), current));
    }
    #[cfg(test)]
    BEFORE_WRITE.with(|hook| {
        if let Some(hook) = hook.borrow_mut().take() {
            hook();
        }
    });
    let now = version_of(read_text(&prepared.path)?.as_deref());
    if now != current {
        return Ok(refuse(refused(FILE_CHANGED.to_string()), Vec::new(), now));
    }
    write_atomic_named(&prepared.path, text, "word table")?;
    Ok(TableSave {
        written: true,
        version: version_of(Some(text)),
        check: prepared.check,
        unconfirmed: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::run::{check_config_in, CheckReport};

    /// `check_config` with an empty temporary folder of the saved report:
    /// a test never reads the report folder of the user.
    fn check_config(config: &Config, env: impl Fn(&str) -> Option<String>) -> CheckReport {
        let dir = tempfile::tempdir().unwrap();
        check_config_in(config, env, Some(dir.path()))
    }

    /// The folder `core` of the repository.
    fn core() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../core")
            .canonicalize()
            .unwrap()
    }

    /// A configuration in a temporary folder with COPIES of the shipped
    /// sound-alike, filler and hotword tables, an invented synonym table and
    /// alias file, and the invented EDCoPilot folder of the fixture.
    struct Setup {
        dir: tempfile::TempDir,
        config: PathBuf,
    }

    impl Setup {
        fn path(&self, relative: &str) -> PathBuf {
            self.dir.path().join(relative)
        }

        fn text(&self, relative: &str) -> String {
            std::fs::read_to_string(self.path(relative)).unwrap()
        }
    }

    /// An invented synonym table for the fixture commands, in the format
    /// and with the comment layout of the shipped table.
    const SYNONYMS: &str = "# Phrase synonyms of the invented harbour test table.\n\
        # A synonym works in every command that uses the phrase.\n\
        #\n\
        # Format, under the one table below:\n\
        #   \"spoken phrase\" = [\"phrase\", \"other phrase\"]\n\
        \n\
        [synonyms]\n\
        \"make for\" = [\"sail to\", \"sail for\"]\n\
        \"make\" = [\"sail for\", \"sail to\"]\n\
        # The speech engine writes this text for \"make for\".\n\
        \"now we head to\" = [\"sail to\", \"sail for\"]\n\
        \"maker\" = [\"sail to\", \"sail for\"]\n\
        # \"make for\" with \"for\" heard as \"two\" or \"too\".\n\
        \"make two\" = [\"sail to\", \"sail for\"]\n\
        \"make too\" = [\"sail to\", \"sail for\"]\n";

    /// An invented German alias file for a fixture token, in the format of
    /// the shipped alias files.
    const ALIASES_DE: &str = "# Extra spoken phrases for the German language.\n\
        # Key: the command's token template, exactly as in the command map.\n\
        # Value: list of phrases in VoiceAttack grammar syntax.\n\
        # Use single quotes for keys: templates hold double quotes and braces.\n\
        'OpenTideLogPanel' = [\"gezeitenbuch [öffnen; zeigen;]\"]\n";

    fn setup_with(hotwords: &str) -> Setup {
        let dir = tempfile::tempdir().unwrap();
        let core = core();
        std::fs::create_dir_all(dir.path().join("data")).unwrap();
        std::fs::create_dir_all(dir.path().join("aliases")).unwrap();
        for name in ["sound_alikes.toml", "fillers.toml", "hotwords.txt"] {
            std::fs::copy(
                core.join("data").join(name),
                dir.path().join("data").join(name),
            )
            .unwrap();
        }
        std::fs::write(dir.path().join("data/synonyms.toml"), SYNONYMS).unwrap();
        std::fs::write(dir.path().join("aliases/de.toml"), ALIASES_DE).unwrap();
        // The invented EDCoPilot folder of the fixture: the program reads
        // its profile.
        edcopilot_voicebridge_core::profile::fixture::write_fixture_edcopilot(
            &dir.path().join("ed"),
        );
        let config = dir.path().join("voicebridge.toml");
        let text = format!(
            "[edcopilot]\ndir = 'ed'\n\n[data]\n\
             aliases_dir = 'aliases'\nsound_alikes = 'data/sound_alikes.toml'\n\
             synonyms = 'data/synonyms.toml'\nfillers = 'data/fillers.toml'\n\n\
             [stt.parakeet]\nmodel_dir = 'model'\nthreads = 1\nhotwords_file = '{hotwords}'\n",
        );
        std::fs::write(&config, text).unwrap();
        // Model pieces that spell each ASCII word, for the hotword check.
        std::fs::create_dir_all(dir.path().join("model")).unwrap();
        let mut tokens = String::from("<blk> 0\n\u{2581} 1\n");
        for (i, c) in ('a'..='z').chain('A'..='Z').chain('0'..='9').enumerate() {
            tokens.push_str(&format!("{c} {}\n", i + 2));
        }
        std::fs::write(dir.path().join("model/tokens.txt"), tokens).unwrap();
        Setup { dir, config }
    }

    /// The setup with the hotwords on.
    fn setup() -> Setup {
        setup_with("data/hotwords.txt")
    }

    fn row(spoken: &str, meant: &[&str]) -> Row {
        Row {
            spoken: spoken.to_string(),
            meant: meant.iter().map(|m| m.to_string()).collect(),
        }
    }

    fn de() -> TableKind {
        TableKind::Aliases {
            language: "de".to_string(),
        }
    }

    /// Each kind with the file of its copy.
    fn kinds() -> Vec<(TableKind, &'static str)> {
        vec![
            (TableKind::Synonyms, "data/synonyms.toml"),
            (TableKind::SoundAlikes, "data/sound_alikes.toml"),
            (TableKind::Fillers, "data/fillers.toml"),
            (TableKind::Hotwords, "data/hotwords.txt"),
            (de(), "aliases/de.toml"),
        ]
    }

    fn rows_of(s: &Setup, kind: &TableKind) -> Vec<Row> {
        read_table(&s.config, kind).unwrap().rows
    }

    /// Saves with the confirmation of each warning of the check.
    fn save_confirmed(s: &Setup, kind: &TableKind, rows: &[Row]) -> TableSave {
        let check = check_table(&s.config, kind, rows);
        let version = read_table(&s.config, kind).unwrap().version;
        save_table(&s.config, kind, rows, &check.warnings, &version).unwrap()
    }

    #[test]
    fn test_empty_edcopilot_dir_makes_the_check_incomplete_and_refuses_each_save() {
        let s = setup();
        let text = s.text("voicebridge.toml").replace("dir = 'ed'", "dir = ''");
        std::fs::write(&s.config, text).unwrap();
        let expected = format!(
            "the check is not complete: {}",
            crate::config::EDCOPILOT_DIR_EMPTY
        );
        for (kind, file) in kinds() {
            let before = std::fs::read(s.path(file)).unwrap();
            let mut rows = rows_of(&s, &kind);
            let check = check_table(&s.config, &kind, &rows);
            assert_eq!(check.problems, std::slice::from_ref(&expected), "{file}");
            // A change: the last entry goes.
            rows.pop();
            let version = read_table(&s.config, &kind).unwrap().version;
            let saved = save_table(&s.config, &kind, &rows, &[], &version).unwrap();
            assert!(!saved.written, "{file}");
            assert!(
                saved.check.problems.contains(&expected),
                "{file}: {saved:?}"
            );
            assert_eq!(std::fs::read(s.path(file)).unwrap(), before, "{file}");
        }
    }

    #[test]
    fn test_read_then_save_gives_the_same_file() {
        let s = setup();
        for (kind, file) in kinds() {
            let before = std::fs::read(s.path(file)).unwrap();
            let table = read_table(&s.config, &kind).unwrap();
            assert!(table.exists, "{file}");
            assert!(!table.rows.is_empty(), "{file}");
            let text = render(&kind, Some(&s.text(file)), &table.rows).unwrap();
            assert_eq!(text.as_bytes(), &before[..], "{file}");
            let saved = save_confirmed(&s, &kind, &table.rows);
            assert!(saved.check.problems.is_empty(), "{file}: {saved:?}");
            assert!(!saved.written, "{file}: the text did not change");
            assert_eq!(std::fs::read(s.path(file)).unwrap(), before, "{file}");
        }
    }

    /// The shipped synonym table through the check of the window: it
    /// reads, it has no duplicate key and no problem, and each warning is
    /// of the known kind. The fixture does not have the phrases of the
    /// shipped table, thus the test does not compare texts.
    #[test]
    fn test_shipped_synonym_table_is_clean_in_shape() {
        let s = setup();
        std::fs::copy(
            core().join("data/synonyms.toml"),
            s.dir.path().join("data/synonyms.toml"),
        )
        .unwrap();
        let table = read_table(&s.config, &TableKind::Synonyms).unwrap();
        assert!(table.exists);
        assert!(!table.rows.is_empty());
        let mut keys: Vec<&str> = table.rows.iter().map(|r| r.spoken.as_str()).collect();
        keys.sort_unstable();
        let count = keys.len();
        keys.dedup();
        assert_eq!(keys.len(), count, "duplicate keys");
        let check = check_table(&s.config, &TableKind::Synonyms, &table.rows);
        assert!(check.problems.is_empty(), "{check:?}");
        // The one kind of warning: a synonym key in a phrase of the fixture.
        for warning in &check.warnings {
            assert!(
                warning.starts_with("synonym \"") && warning.contains("is also in the phrase"),
                "{warning}"
            );
        }
    }

    /// The rows of each table: the invented synonyms and aliases of the
    /// setup, and the shipped sound-alike, filler and hotword files.
    #[test]
    fn test_rows_of_the_invented_synonyms_and_the_shipped_tables() {
        let s = setup();
        let synonyms = rows_of(&s, &TableKind::Synonyms);
        assert_eq!(synonyms[0], row("make for", &["sail to", "sail for"]));
        assert_eq!(synonyms.len(), 6);
        // Each table gives the languages of the alias folder.
        let table = read_table(&s.config, &TableKind::Synonyms).unwrap();
        assert_eq!(table.languages, ["de"]);
        let sound = rows_of(&s, &TableKind::SoundAlikes);
        assert_eq!(sound[0], row("cell", &["sell"]));
        let fillers = rows_of(&s, &TableKind::Fillers);
        assert_eq!(fillers[0], row("a", &[NOT_LAST]));
        assert_eq!(fillers[4], row("uh", &[MAY_BE_LAST]));
        assert_eq!(fillers.len(), 7);
        let hot = rows_of(&s, &TableKind::Hotwords);
        assert_eq!(hot[0], row("Sol", &[]));
        assert_eq!(hot.len(), 7);
        let aliases = rows_of(&s, &de());
        assert_eq!(
            aliases,
            [row("OpenTideLogPanel", &["gezeitenbuch [öffnen; zeigen;]"])]
        );
    }

    #[test]
    fn test_synonym_key_that_is_a_command_word_gives_a_warning() {
        let s = setup();
        let mut rows = rows_of(&s, &TableKind::Synonyms);
        rows.push(row("ring bell", &["toll bell"]));
        let check = check_table(&s.config, &TableKind::Synonyms, &rows);
        assert!(check.problems.is_empty(), "{check:?}");
        assert_eq!(check.warnings.len(), 1, "{check:?}");
        assert!(
            check.warnings[0].starts_with("synonym \"ring bell\" is also in the phrase"),
            "{check:?}"
        );
    }

    #[test]
    fn test_hotword_near_another_name_is_a_problem() {
        let s = setup();
        let before = s.text("data/hotwords.txt");
        let mut rows = rows_of(&s, &TableKind::Hotwords);
        rows.push(row("Siris", &[]));
        rows.push(row("Sirius", &[]));
        let check = check_table(&s.config, &TableKind::Hotwords, &rows);
        assert!(
            check
                .problems
                .iter()
                .any(|p| p.contains("the hotword \"Siris\" is near the hotword \"Sirius\"")),
            "{check:?}"
        );
        let version = read_table(&s.config, &TableKind::Hotwords).unwrap().version;
        let saved = save_table(&s.config, &TableKind::Hotwords, &rows, &[], &version).unwrap();
        assert!(!saved.written);
        assert_eq!(saved.check, check);
        assert_eq!(s.text("data/hotwords.txt"), before);
    }

    #[test]
    fn test_filler_that_the_loader_refuses_is_a_problem() {
        let s = setup();
        let mut rows = rows_of(&s, &TableKind::Fillers);
        rows.push(row("s3", &[NOT_LAST]));
        let check = check_table(&s.config, &TableKind::Fillers, &rows);
        assert_eq!(check.problems.len(), 1, "{check:?}");
        assert!(
            check.problems[0]
                .ends_with("not_last: filler \"s3\" is not one word of letters, it is left out"),
            "{check:?}"
        );
        // The path in the text is the path of the table, not of a temporary file.
        let path = Config::load(&s.config).unwrap().data.fillers;
        assert!(
            check.problems[0].starts_with(&path.display().to_string()),
            "{check:?}"
        );
        // A word in both lists.
        let mut rows = rows_of(&s, &TableKind::Fillers);
        rows.push(row("the", &[MAY_BE_LAST]));
        let check = check_table(&s.config, &TableKind::Fillers, &rows);
        assert!(
            check.problems[0].ends_with("filler \"the\" is in both lists, it counts as not_last"),
            "{check:?}"
        );
        // A list that is not one of the two lists of the file.
        let rows = vec![row("uh", &["sometimes"])];
        let check = check_table(&s.config, &TableKind::Fillers, &rows);
        assert_eq!(
            check.problems,
            ["row 1: the list must be \"not_last\" or \"may_be_last\""]
        );
    }

    #[test]
    fn test_save_with_a_problem_keeps_the_file() {
        let s = setup();
        for (kind, file, bad) in [
            (
                TableKind::Synonyms,
                "data/synonyms.toml",
                row("make.", &["sail to"]),
            ),
            (
                TableKind::SoundAlikes,
                "data/sound_alikes.toml",
                row("cell", &["cell"]),
            ),
            (
                TableKind::Fillers,
                "data/fillers.toml",
                row("two words", &[NOT_LAST]),
            ),
            (
                TableKind::Hotwords,
                "data/hotwords.txt",
                row("Sol/Achenar", &[]),
            ),
            (de(), "aliases/de.toml", row("X", &["a\tb"])),
        ] {
            let before = std::fs::read(s.path(file)).unwrap();
            let mut rows = rows_of(&s, &kind);
            rows.push(bad);
            let check = check_table(&s.config, &kind, &rows);
            assert!(!check.problems.is_empty(), "{file}");
            let version = read_table(&s.config, &kind).unwrap().version;
            let saved = save_table(&s.config, &kind, &rows, &check.warnings, &version).unwrap();
            assert!(!saved.written, "{file}");
            assert_eq!(saved.check.problems, check.problems, "{file}");
            assert_eq!(std::fs::read(s.path(file)).unwrap(), before, "{file}");
        }
    }

    /// The comment lines at the top of a file, up to the first entry.
    fn header(text: &str) -> String {
        text.lines()
            .take_while(|l| l.starts_with('#') || l.trim().is_empty())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn test_save_keeps_the_header_comment() {
        let s = setup();
        for (kind, file, new) in [
            (
                TableKind::Synonyms,
                "data/synonyms.toml",
                row("row me to", &["sail to"]),
            ),
            (
                TableKind::SoundAlikes,
                "data/sound_alikes.toml",
                row("sel", &["sell"]),
            ),
            (
                TableKind::Fillers,
                "data/fillers.toml",
                row("er", &[MAY_BE_LAST]),
            ),
            (
                TableKind::Hotwords,
                "data/hotwords.txt",
                row("Shinrarta Dezhra", &[]),
            ),
            (
                de(),
                "aliases/de.toml",
                row("SailHomePort", &["nach hause"]),
            ),
        ] {
            let before = s.text(file);
            // The first row goes away, and the new row comes first.
            let mut rows = rows_of(&s, &kind);
            rows.remove(0);
            rows.insert(0, new.clone());
            let saved = save_confirmed(&s, &kind, &rows);
            assert!(saved.written, "{file}: {saved:?}");
            let after = s.text(file);
            assert_eq!(header(&after), header(&before), "{file}:\n{after}");
            // The filler file has two lists: the rows come back by list.
            if kind == TableKind::Fillers {
                rows.sort_by_key(|r| r.meant[0] != NOT_LAST);
            }
            assert_eq!(rows_of(&s, &kind), rows, "{file}");
            // All rows away: the header stays.
            let saved = save_confirmed(&s, &kind, &[]);
            assert!(saved.written, "{file}: {saved:?}");
            let empty = s.text(file);
            assert!(
                header(&empty).starts_with(&header(&before)),
                "{file}:\n{empty}"
            );
            assert!(rows_of(&s, &kind).is_empty(), "{file}");
        }
    }

    /// The comment of a removed entry stays in the file.
    #[test]
    fn test_comment_of_a_removed_entry_stays() {
        let s = setup();
        let before = s.text("data/synonyms.toml");
        let mut rows = rows_of(&s, &TableKind::Synonyms);
        rows.retain(|r| r.spoken != "now we head to");
        assert!(save_confirmed(&s, &TableKind::Synonyms, &rows).written);
        let after = s.text("data/synonyms.toml");
        let comments = |t: &str| t.lines().filter(|l| l.starts_with('#')).count();
        assert_eq!(comments(&after), comments(&before), "{after}");
        assert!(!after.contains("\"now we head to\""));

        let before = s.text("data/hotwords.txt");
        let mut rows = rows_of(&s, &TableKind::Hotwords);
        rows.retain(|r| r.spoken != "Sol");
        assert!(save_confirmed(&s, &TableKind::Hotwords, &rows).written);
        let after = s.text("data/hotwords.txt");
        assert_eq!(comments(&after), comments(&before), "{after}");
        assert!(after.contains("\r\n"), "the line ends stay");
    }

    /// The lines of `voicebridge check` that concern one table: the
    /// problems that name its file (or the hotword key), and its warnings.
    fn console_lines(config: &Path, kind: &TableKind) -> TableCheck {
        let config = Config::load(config).unwrap();
        let found = check_config(&config, |_| None);
        let report = found.report.clone().unwrap();
        let file = match kind {
            TableKind::Synonyms => Some(config.data.synonyms.display().to_string()),
            TableKind::SoundAlikes => Some(config.data.sound_alikes.display().to_string()),
            TableKind::Fillers => Some(config.data.fillers.display().to_string()),
            _ => None,
        };
        let hot = "stt.parakeet.hotwords_file";
        let problem = |p: &String| match (kind, &file) {
            (TableKind::Hotwords, _) => p.starts_with(hot),
            (TableKind::Aliases { .. }, _) => p.starts_with(hot),
            (_, Some(f)) => p.contains(f.as_str()),
            _ => false,
        };
        let warning = |w: &String| match kind {
            TableKind::Synonyms => report.synonym_warnings.contains(w),
            TableKind::Fillers => report.filler_warnings.contains(w),
            TableKind::Hotwords => w.starts_with(hot),
            _ => false,
        };
        TableCheck {
            problems: found
                .problems
                .iter()
                .filter(|p| problem(p))
                .cloned()
                .collect(),
            warnings: found
                .warnings
                .iter()
                .filter(|w| warning(w))
                .cloned()
                .collect(),
        }
    }

    /// Checks the rows, writes them as a save writes them, and compares the
    /// check with the lines of `voicebridge check` for the written file.
    /// Rows with a problem are written directly, because a save refuses
    /// them.
    fn assert_check_is_the_console_check(s: &Setup, kind: &TableKind, file: &str, rows: &[Row]) {
        let check = check_table(&s.config, kind, rows);
        assert!(
            !check.problems.is_empty() || !check.warnings.is_empty(),
            "{file}: the sample must give a problem or a warning"
        );
        if check.problems.is_empty() {
            assert!(save_confirmed(s, kind, rows).written, "{file}");
        } else {
            let text = render(kind, Some(&s.text(file)), rows).unwrap();
            std::fs::write(s.path(file), text).unwrap();
        }
        assert_eq!(console_lines(&s.config, kind), check, "{file}");
    }

    #[test]
    fn test_synonym_check_is_the_console_check_after_the_save() {
        let s = setup();
        let kind = TableKind::Synonyms;
        let mut rows = rows_of(&s, &kind);
        rows.push(row("ring bell", &["toll bell"]));
        assert_check_is_the_console_check(&s, &kind, "data/synonyms.toml", &rows);
        let mut rows = rows_of(&s, &kind);
        rows.push(row("Make For", &["sail for"]));
        rows.push(row("row!", &["sail for"]));
        assert_check_is_the_console_check(&s, &kind, "data/synonyms.toml", &rows);
    }

    #[test]
    fn test_sound_alike_check_is_the_console_check_after_the_save() {
        let s = setup();
        let kind = TableKind::SoundAlikes;
        let mut rows = rows_of(&s, &kind);
        rows.push(row("Cell", &["sell"]));
        rows.push(row("two words", &["x"]));
        assert_check_is_the_console_check(&s, &kind, "data/sound_alikes.toml", &rows);
    }

    #[test]
    fn test_filler_check_is_the_console_check_after_the_save() {
        let s = setup();
        let kind = TableKind::Fillers;
        // A word of a name gives a warning. The vocabulary of the fixture
        // has the name "Seven Gulls".
        let mut rows = rows_of(&s, &kind);
        rows.push(row("gulls", &[MAY_BE_LAST]));
        let check = check_table(&s.config, &kind, &rows);
        assert!(
            check
                .warnings
                .iter()
                .any(|w| w.contains("filler \"gulls\"")),
            "{check:?}"
        );
        assert_check_is_the_console_check(&s, &kind, "data/fillers.toml", &rows);
        let mut rows = rows_of(&s, &kind);
        rows.push(row("uh", &[MAY_BE_LAST]));
        assert_check_is_the_console_check(&s, &kind, "data/fillers.toml", &rows);
    }

    #[test]
    fn test_hotword_check_is_the_console_check_after_the_save() {
        let s = setup();
        let kind = TableKind::Hotwords;
        let mut rows = rows_of(&s, &kind);
        rows.push(row("Siris", &[]));
        rows.push(row("Sirius", &[]));
        assert_check_is_the_console_check(&s, &kind, "data/hotwords.txt", &rows);
    }

    #[test]
    fn test_alias_check_is_the_console_check_after_the_save() {
        let s = setup();
        let kind = de();
        // An alias phrase with the word of a hotword: the hotword is then
        // equal to a command word, and `check` refuses the hotword list.
        let mut rows = rows_of(&s, &kind);
        rows.push(row("SailHomePort", &["nach alioth"]));
        let check = check_table(&s.config, &kind, &rows);
        assert!(
            check
                .problems
                .iter()
                .any(|p| p.contains("the hotword \"Alioth\" is near the command word \"alioth\"")),
            "{check:?}"
        );
        assert_check_is_the_console_check(&s, &kind, "aliases/de.toml", &rows);
    }

    #[test]
    fn test_a_warning_needs_its_confirmation() {
        let s = setup();
        let kind = TableKind::Synonyms;
        let before = s.text("data/synonyms.toml");
        let mut rows = rows_of(&s, &kind);
        rows.push(row("ring bell", &["toll bell"]));
        let check = check_table(&s.config, &kind, &rows);
        let version = read_table(&s.config, &kind).unwrap().version;
        let saved = save_table(&s.config, &kind, &rows, &[], &version).unwrap();
        assert!(!saved.written);
        assert_eq!(saved.unconfirmed, check.warnings);
        assert_eq!(s.text("data/synonyms.toml"), before);
        // A confirmation of another warning is not a confirmation of this one.
        let other = vec!["synonym \"make\" is also in the phrase".to_string()];
        let saved = save_table(&s.config, &kind, &rows, &other, &version).unwrap();
        assert!(!saved.written);
        assert_eq!(saved.unconfirmed, check.warnings);
        // The confirmation of the shown warning permits the save.
        let saved = save_table(&s.config, &kind, &rows, &check.warnings, &version).unwrap();
        assert!(saved.written, "{saved:?}");
        assert!(saved.unconfirmed.is_empty());
        assert_ne!(saved.version, version);
        assert_eq!(saved.version, read_table(&s.config, &kind).unwrap().version);
    }

    #[test]
    fn test_save_refuses_when_the_file_changed_after_the_read() {
        let s = setup();
        let kind = TableKind::SoundAlikes;
        let table = read_table(&s.config, &kind).unwrap();
        // A hand edit after the read.
        let edited = s.text("data/sound_alikes.toml") + "lok = \"log\"\n";
        std::fs::write(s.path("data/sound_alikes.toml"), &edited).unwrap();
        let mut rows = table.rows.clone();
        rows.push(row("sel", &["sell"]));
        let saved = save_table(&s.config, &kind, &rows, &[], &table.version).unwrap();
        assert!(!saved.written);
        assert_eq!(saved.check.problems, [FILE_CHANGED]);
        assert_eq!(s.text("data/sound_alikes.toml"), edited);
    }

    #[test]
    fn test_language_names() {
        for good in ["de", "fr", "pt-BR", "en_GB", "x1"] {
            assert!(check_language(good).is_ok(), "{good}");
        }
        for bad in [
            "",
            "..",
            "a/b",
            "a\\b",
            "de.toml",
            "c:",
            "CON",
            "nul",
            "Com1",
            "lpt9",
            " de",
            "ü",
            &"a".repeat(33),
        ] {
            assert!(check_language(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn test_new_language_makes_the_file_only_on_save() {
        let s = setup();
        let kind = TableKind::Aliases {
            language: "fr".to_string(),
        };
        let table = read_table(&s.config, &kind).unwrap();
        assert!(!table.exists);
        assert!(table.rows.is_empty());
        assert_eq!(table.languages, ["de"]);
        assert!(!s.path("aliases/fr.toml").exists());
        let rows = vec![row("OpenTideLogPanel", &["ouvrir le journal des marées"])];
        assert!(check_table(&s.config, &kind, &rows).problems.is_empty());
        assert!(!s.path("aliases/fr.toml").exists());
        let saved = save_confirmed(&s, &kind, &rows);
        assert!(saved.written, "{saved:?}");
        assert_eq!(rows_of(&s, &kind), rows);
        assert!(s.text("aliases/fr.toml").starts_with("# "));
        assert_eq!(
            read_table(&s.config, &kind).unwrap().languages,
            ["de", "fr"]
        );
        // The program loads the new file.
        let config = Config::load(&s.config).unwrap();
        let map = edcopilot_voicebridge_core::load_aliases(&config.data.aliases_dir);
        assert!(map["OpenTideLogPanel"].contains(&"ouvrir le journal des marées".to_string()));
        // A bad name is refused before any file access.
        let bad = TableKind::Aliases {
            language: "../x".to_string(),
        };
        assert!(read_table(&s.config, &bad).is_err());
        assert!(!check_table(&s.config, &bad, &rows).problems.is_empty());
    }

    #[test]
    fn test_limits_of_the_rows() {
        let s = setup();
        let kind = TableKind::Synonyms;
        let many: Vec<Row> = (0..=MAX_ROWS)
            .map(|i| row(&format!("word{i}"), &["sail to"]))
            .collect();
        let check = check_table(&s.config, &kind, &many);
        assert_eq!(
            check.problems.len(),
            1,
            "{:?}",
            &check.problems[..3.min(check.problems.len())]
        );
        assert!(check.problems[0].contains("rows"), "{check:?}");
        let long = "a".repeat(MAX_ENTRY_CHARS + 1);
        for (rows, text) in [
            (
                vec![row(&long, &["x"])],
                "row 1: the entry is too long, an entry has at most 300 characters",
            ),
            (
                vec![row("a\u{7}b", &["x"])],
                "row 1: the entry has a control character",
            ),
            (
                vec![row("a", &["x\nb"])],
                "row 1: the entry has a control character",
            ),
            (vec![row("", &["x"])], "row 1: the first field is empty"),
            (
                vec![row("a", &["x"]), row("a", &["y"])],
                "row 2: \"a\" is also in row 1, a key can be in the file only one time",
            ),
        ] {
            let check = check_table(&s.config, &kind, &rows);
            assert_eq!(check.problems, [text], "{rows:?}");
        }
        let values: Vec<String> = (0..=MAX_VALUES).map(|i| format!("v{i}")).collect();
        let rows = vec![Row {
            spoken: "a".into(),
            meant: values,
        }];
        assert!(check_table(&s.config, &kind, &rows).problems[0].contains("values"));
        // Kind rules of the file format.
        let check = check_table(&s.config, &TableKind::SoundAlikes, &[row("a", &["b", "c"])]);
        assert_eq!(
            check.problems,
            ["row 1: a sound-alike has exactly one meant word"]
        );
        let check = check_table(&s.config, &TableKind::Hotwords, &[row("# Sol", &[])]);
        assert_eq!(
            check.problems,
            ["row 1: a hotword cannot start with \"#\", the program reads such a line as a comment"]
        );
    }

    #[test]
    fn test_content_that_the_rows_cannot_hold_is_refused() {
        let s = setup();
        let refuse = |file: &str, text: &str, kind: &TableKind| {
            std::fs::write(s.path(file), text).unwrap();
            let error = read_table(&s.config, kind).unwrap_err();
            assert!(
                format!("{error:#}").contains("edit the file by hand"),
                "{error:#}"
            );
            let valid = match kind {
                TableKind::Fillers => row("x", &[NOT_LAST]),
                _ => row("x", &["y"]),
            };
            let check = check_table(&s.config, kind, &[valid]);
            assert!(
                check.problems[0].contains("edit the file by hand"),
                "{check:?}"
            );
        };
        refuse(
            "data/synonyms.toml",
            "[synonyms]\n\"a\" = \"b\"\n[other]\nx = 1\n",
            &TableKind::Synonyms,
        );
        refuse(
            "data/synonyms.toml",
            "[synonyms]\n\"a\" = 5\n",
            &TableKind::Synonyms,
        );
        refuse("aliases/de.toml", "'X' = \"not a list\"\n", &de());
        refuse(
            "data/fillers.toml",
            "not_last = []\nmay_be_last = []\nextra = []\n",
            &TableKind::Fillers,
        );
        refuse(
            "data/sound_alikes.toml",
            "this is not toml",
            &TableKind::SoundAlikes,
        );
        // A comment inside a list stays when the list does not change. A
        // change of that list is refused.
        let text = "[synonyms]\n\"a\" = [\n  \"b\", # why\n  \"c\",\n]\n";
        std::fs::write(s.path("data/synonyms.toml"), text).unwrap();
        let rows = rows_of(&s, &TableKind::Synonyms);
        assert_eq!(rows, [row("a", &["b", "c"])]);
        assert_eq!(
            render(&TableKind::Synonyms, Some(text), &rows).unwrap(),
            text
        );
        let check = check_table(&s.config, &TableKind::Synonyms, &[row("a", &["b"])]);
        assert!(
            check.problems[0].contains("edit the file by hand"),
            "{check:?}"
        );
    }

    #[test]
    fn test_default_hotwords_file_is_the_copy_beside_the_configuration() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("voicebridge.toml");
        // No list at all: the file beside the configuration.
        assert_eq!(
            default_hotwords_file(&config),
            dir.path().join("hotwords.txt")
        );
        // A source tree: the shipped list.
        let shipped = dir.path().join("core/data/hotwords.txt");
        std::fs::create_dir_all(shipped.parent().unwrap()).unwrap();
        std::fs::write(&shipped, "Sol\n").unwrap();
        assert_eq!(default_hotwords_file(&config), shipped);
        // The data folder of the user interface: the copy wins.
        std::fs::write(dir.path().join("hotwords.txt"), "Sol\n").unwrap();
        assert_eq!(
            default_hotwords_file(&config),
            dir.path().join("hotwords.txt")
        );
        // A relative configuration path gives an absolute path.
        assert!(default_hotwords_file(Path::new("voicebridge.toml")).is_absolute());
    }

    #[test]
    fn test_hotwords_off_edits_the_default_file() {
        let s = setup_with("");
        let shipped = s.path("core/data/hotwords.txt");
        std::fs::create_dir_all(shipped.parent().unwrap()).unwrap();
        std::fs::copy(s.path("data/hotwords.txt"), &shipped).unwrap();
        let table = read_table(&s.config, &TableKind::Hotwords).unwrap();
        assert!(table.hotwords_off);
        assert_eq!(table.path, shipped);
        assert_eq!(table.rows.len(), 7);
        // The check runs as with the hotwords on.
        let mut rows = table.rows.clone();
        rows.push(row("Siris", &[]));
        rows.push(row("Sirius", &[]));
        let check = check_table(&s.config, &TableKind::Hotwords, &rows);
        assert!(!check.problems.is_empty(), "{check:?}");
        let on = setup();
        assert!(
            !read_table(&on.config, &TableKind::Hotwords)
                .unwrap()
                .hotwords_off
        );
    }

    #[test]
    fn test_byte_order_mark_and_line_ends_stay() {
        let s = setup();
        let text = "\u{feff}# head\r\n[sound_alikes]\r\ncell = \"sell\"\r\n";
        std::fs::write(s.path("data/sound_alikes.toml"), text).unwrap();
        let rows = vec![row("cell", &["sell"]), row("bye", &["buy"])];
        assert!(save_confirmed(&s, &TableKind::SoundAlikes, &rows).written);
        let after = s.text("data/sound_alikes.toml");
        assert_eq!(
            after,
            "\u{feff}# head\r\n[sound_alikes]\r\ncell = \"sell\"\r\nbye = \"buy\"\r\n"
        );
    }

    /// The lines of `voicebridge check` without a filter, as multisets:
    /// the lines of the check of the saved files that the check with the
    /// table empty does not have.
    fn lines_the_table_causes(s: &Setup, kind: &TableKind, file: &str) -> TableCheck {
        let config = Config::load(&s.config).unwrap();
        let after = check_config(&config, |_| None);
        let saved = std::fs::read(s.path(file)).unwrap();
        match kind {
            TableKind::Aliases { .. } => std::fs::remove_file(s.path(file)).unwrap(),
            TableKind::Synonyms => std::fs::write(s.path(file), "[synonyms]\n").unwrap(),
            TableKind::SoundAlikes => std::fs::write(s.path(file), "[sound_alikes]\n").unwrap(),
            TableKind::Fillers => {
                std::fs::write(s.path(file), "not_last = []\nmay_be_last = []\n").unwrap()
            }
            TableKind::Hotwords => std::fs::write(s.path(file), "").unwrap(),
        }
        let empty = check_config(&config, |_| None);
        std::fs::write(s.path(file), saved).unwrap();
        let minus = |lines: &[String], base: &[String]| {
            let mut base = base.to_vec();
            let mut out: Vec<String> = Vec::new();
            for line in lines {
                match base.iter().position(|b| b == line) {
                    Some(i) => {
                        base.remove(i);
                    }
                    None => out.push(line.clone()),
                }
            }
            out.sort();
            out
        };
        TableCheck {
            problems: minus(&after.problems, &empty.problems),
            warnings: minus(&after.warnings, &empty.warnings),
        }
    }

    fn sorted(check: TableCheck) -> TableCheck {
        let mut check = check;
        check.problems.sort();
        check.warnings.sort();
        check
    }

    /// The check of the rows, compared with the lines of the unfiltered
    /// `voicebridge check` after the rows are in the file.
    fn assert_unfiltered(s: &Setup, kind: &TableKind, file: &str, rows: &[Row]) {
        let check = check_table(&s.config, kind, rows);
        let text = render(kind, Some(&s.text(file)), &trimmed(rows)).unwrap();
        std::fs::write(s.path(file), text).unwrap();
        assert_eq!(
            sorted(check),
            lines_the_table_causes(s, kind, file),
            "{file}"
        );
    }

    #[test]
    fn test_check_is_the_unfiltered_console_check_for_each_table() {
        let s = setup();
        let with = |kind: &TableKind, extra: Vec<Row>| {
            let mut rows = rows_of(&s, kind);
            rows.extend(extra);
            rows
        };
        let cases = [
            (
                TableKind::Synonyms,
                "data/synonyms.toml",
                vec![row("ring bell", &["toll bell"]), row("Make For", &["x"])],
            ),
            (
                TableKind::SoundAlikes,
                "data/sound_alikes.toml",
                vec![row("Cell", &["sell"]), row("two words", &["x"])],
            ),
            (
                TableKind::Fillers,
                "data/fillers.toml",
                vec![row("gulls", &[MAY_BE_LAST]), row("s3", &[NOT_LAST])],
            ),
            (
                TableKind::Hotwords,
                "data/hotwords.txt",
                vec![row("Siris", &[]), row("Sirius", &[])],
            ),
            (
                de(),
                "aliases/de.toml",
                vec![
                    row("SailHomePort", &["nach alioth"]),
                    row("NoSuchToken", &["gibt es nicht"]),
                ],
            ),
        ];
        for (kind, file, extra) in cases {
            let rows = with(&kind, extra);
            assert_unfiltered(&s, &kind, file, &rows);
            // The unchanged rows of the file give the same lines too.
            let rows = rows_of(&s, &kind);
            assert_unfiltered(&s, &kind, file, &rows);
        }
    }

    /// A conflict that the saved alias file already has stays visible, and
    /// the save is refused.
    #[test]
    fn test_a_conflict_that_the_saved_alias_file_has_stays_visible() {
        let s = setup();
        let text = s.text("aliases/de.toml") + "'SailHomePort' = [\"nach alioth\"]\r\n";
        std::fs::write(s.path("aliases/de.toml"), &text).unwrap();
        let rows = rows_of(&s, &de());
        let check = check_table(&s.config, &de(), &rows);
        assert!(
            check
                .problems
                .iter()
                .any(|p| p.contains("the hotword \"Alioth\" is near the command word \"alioth\"")),
            "{check:?}"
        );
        // A change of another row: the save is refused.
        let mut changed = rows.clone();
        changed[0].meant.push("protokoll".into());
        let version = read_table(&s.config, &de()).unwrap().version;
        let saved = save_table(&s.config, &de(), &changed, &[], &version).unwrap();
        assert!(!saved.written);
        assert_eq!(s.text("aliases/de.toml"), text);
    }

    /// For each other table, a line that the saved table causes stays
    /// visible for the unchanged rows.
    #[test]
    fn test_a_line_of_the_saved_table_stays_visible_for_each_kind() {
        let s = setup();
        let append = |file: &str, extra: &str| {
            let text = s.text(file) + extra;
            std::fs::write(s.path(file), text).unwrap();
        };
        append("data/synonyms.toml", "\"ring bell\" = \"toll bell\"\n");
        append("data/sound_alikes.toml", "Cell = \"sell\"\n");
        append("data/hotwords.txt", "Siris\r\nSirius\r\n");
        let cases = [
            (
                TableKind::Synonyms,
                "synonym \"ring bell\" is also in the phrase",
            ),
            (
                TableKind::SoundAlikes,
                "sound-alikes \"Cell\" and \"cell\" differ only in case",
            ),
            (TableKind::Fillers, "filler \"the\" is a word of the names"),
            (
                TableKind::Hotwords,
                "the hotword \"Siris\" is near the hotword \"Sirius\"",
            ),
        ];
        for (kind, text) in cases {
            let check = check_table(&s.config, &kind, &rows_of(&s, &kind));
            let all: Vec<&String> = check.problems.iter().chain(&check.warnings).collect();
            assert!(all.iter().any(|l| l.contains(text)), "{kind:?}: {check:?}");
        }
    }

    /// Each kind: a check that cannot load the command data is a problem,
    /// and the save is refused.
    #[test]
    fn test_a_missing_profile_is_a_problem_for_each_kind() {
        let s = setup();
        // The EDCoPilot folder has no profile.
        std::fs::remove_file(s.path("ed/EDCoPilot-Profile.vap")).unwrap();
        for (kind, file) in kinds() {
            let before = s.text(file);
            let rows = rows_of(&s, &kind);
            let check = check_table(&s.config, &kind, &rows);
            assert_eq!(check.problems.len(), 1, "{file}: {check:?}");
            assert!(
                check.problems[0].starts_with("the check cannot load the command data: "),
                "{check:?}"
            );
            let version = read_table(&s.config, &kind).unwrap().version;
            let mut changed = rows.clone();
            changed.pop();
            let saved = save_table(&s.config, &kind, &changed, &[], &version).unwrap();
            assert!(!saved.written, "{file}");
            assert_eq!(s.text(file), before, "{file}");
        }
    }

    /// The profile is an input of the check: a check after a clean check
    /// does not use the kept result when the profile is gone or changed.
    #[test]
    fn test_table_check_is_not_complete_without_the_profile() {
        let s = setup();
        let kind = TableKind::Synonyms;
        let rows = rows_of(&s, &kind);
        let config = Config::load(&s.config).unwrap();
        let path = table_path(&config, &s.config, &kind).unwrap();
        let clean = check_table(&s.config, &kind, &rows);
        assert!(clean.problems.is_empty(), "{clean:?}");
        let profile = s.path("ed/EDCoPilot-Profile.vap");
        let before = base_stamp(&config, &kind, &path);
        let bytes = std::fs::read(&profile).unwrap();
        std::fs::write(&profile, [bytes.as_slice(), b"x"].concat()).unwrap();
        assert_ne!(base_stamp(&config, &kind, &path), before);
        std::fs::remove_file(&profile).unwrap();
        let check = check_table(&s.config, &kind, &rows);
        assert_eq!(check.problems.len(), 1, "{check:?}");
        assert!(
            check.problems[0].starts_with("the check cannot load the command data: "),
            "{check:?}"
        );
    }

    /// A file that is not a profile stops the load of the command data:
    /// the check of each table is not clean.
    #[test]
    fn test_a_file_that_is_not_a_profile_makes_the_table_check_refuse() {
        let s = setup();
        let profile = s.path("ed/EDCoPilot-Profile.vap");
        std::fs::write(&profile, "not a profile").unwrap();
        let expected = format!(
            "the check cannot load the command data: the file {} is not an EDCoPilot profile \
             that the program understands: ",
            std::path::absolute(&profile).unwrap().display()
        );
        for (kind, _) in kinds() {
            let check = check_table(&s.config, &kind, &rows_of(&s, &kind));
            assert_eq!(check.problems.len(), 1, "{kind:?}: {check:?}");
            assert!(
                check.problems[0].starts_with(&expected),
                "{kind:?}: {check:?}"
            );
        }
    }

    /// An absent file of the command data stops a part of the check: that
    /// is a problem of each table.
    #[test]
    fn test_an_absent_vocabulary_folder_makes_the_check_incomplete() {
        let s = setup();
        std::fs::remove_dir_all(s.path("ed/Dynamic var files")).unwrap();
        for (kind, _) in kinds() {
            let check = check_table(&s.config, &kind, &rows_of(&s, &kind));
            assert!(
                check
                    .problems
                    .iter()
                    .any(|p| p
                        .starts_with("the check is not complete: vocabulary directory not found")),
                "{kind:?}: {check:?}"
            );
        }
    }

    #[test]
    fn test_an_absent_voice_commands_file_makes_the_check_incomplete() {
        let s = setup();
        std::fs::remove_file(s.path("ed/working/EDCoPilot.VoiceCommands.ini")).unwrap();
        for (kind, _) in kinds() {
            let check = check_table(&s.config, &kind, &rows_of(&s, &kind));
            assert!(
                check
                    .problems
                    .iter()
                    .any(|p| p
                        .starts_with("the check is not complete: voice commands file not found")),
                "{kind:?}: {check:?}"
            );
        }
    }

    #[test]
    fn test_a_bad_hotword_score_is_a_problem() {
        let s = setup();
        let text = std::fs::read_to_string(&s.config).unwrap();
        std::fs::write(&s.config, text + "hotwords_score = 9.0\n").unwrap();
        let kind = TableKind::Hotwords;
        let before = s.text("data/hotwords.txt");
        let mut rows = rows_of(&s, &kind);
        rows.push(row("Siris", &[]));
        let check = check_table(&s.config, &kind, &rows);
        assert!(
            check.problems.iter().any(|p| p.starts_with(
                "the check cannot judge the hotwords: stt.parakeet.hotwords_score is 9"
            )),
            "{check:?}"
        );
        let version = read_table(&s.config, &kind).unwrap().version;
        assert!(
            !save_table(&s.config, &kind, &rows, &[], &version)
                .unwrap()
                .written
        );
        assert_eq!(s.text("data/hotwords.txt"), before);
        // Without the model pieces the hotwords cannot be judged either.
        let s = setup();
        std::fs::remove_file(s.path("model/tokens.txt")).unwrap();
        let check = check_table(&s.config, &kind, &rows_of(&s, &kind));
        assert!(
            check
                .problems
                .iter()
                .any(|p| p
                    .starts_with("the check cannot test the hotwords against the model pieces")),
            "{check:?}"
        );
    }

    /// The check of the table with another file is the check of the files
    /// now: no old result is used after a file changed.
    #[test]
    fn test_a_change_of_another_file_is_seen_by_the_next_check() {
        let s = setup();
        let mut rows = rows_of(&s, &de());
        rows.push(row("SailHomePort", &["nach alioth"]));
        // Another language has the same conflict: it is not a line of this table.
        std::fs::write(
            s.path("aliases/fr.toml"),
            "'OpenTideLogPanel' = [\"alioth journal\"]\n",
        )
        .unwrap();
        let check = check_table(&s.config, &de(), &rows);
        assert!(check.problems.is_empty(), "{check:?}");
        std::fs::remove_file(s.path("aliases/fr.toml")).unwrap();
        let check = check_table(&s.config, &de(), &rows);
        assert!(
            check.problems.iter().any(|p| p.contains("\"Alioth\"")),
            "{check:?}"
        );
    }

    #[test]
    fn test_alias_key_that_no_command_takes_is_a_warning() {
        let s = setup();
        let mut rows = rows_of(&s, &de());
        rows.push(row("NoSuchToken", &["gibt es nicht"]));
        let check = check_table(&s.config, &de(), &rows);
        assert!(check.problems.is_empty(), "{check:?}");
        let path = Config::load(&s.config)
            .unwrap()
            .data
            .aliases_dir
            .join("de.toml");
        assert_eq!(
            check.warnings,
            [format!(
                "alias file {}: the token \"NoSuchToken\" matches no command that works, the program ignores its phrases",
                path.display()
            )]
        );
        let saved = save_confirmed(&s, &de(), &rows);
        assert!(saved.written, "{saved:?}");
    }

    #[test]
    fn test_save_refuses_a_change_right_before_the_write() {
        let s = setup();
        let kind = TableKind::SoundAlikes;
        let table = read_table(&s.config, &kind).unwrap();
        let mut rows = table.rows.clone();
        rows.push(row("sel", &["sell"]));
        let file = s.path("data/sound_alikes.toml");
        let edited = s.text("data/sound_alikes.toml") + "lok = \"log\"\n";
        let hook_file = file.clone();
        let hook_text = edited.clone();
        BEFORE_WRITE.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move || {
                std::fs::write(&hook_file, &hook_text).unwrap();
            }))
        });
        let saved = save_table(&s.config, &kind, &rows, &[], &table.version).unwrap();
        assert!(!saved.written);
        assert_eq!(saved.check.problems, [FILE_CHANGED]);
        assert_eq!(s.text("data/sound_alikes.toml"), edited);
    }

    #[test]
    fn test_language_case_resolves_to_the_existing_file() {
        let s = setup();
        let upper = TableKind::Aliases {
            language: "DE".to_string(),
        };
        let table = read_table(&s.config, &upper).unwrap();
        assert!(table.exists);
        assert_eq!(table.language.as_deref(), Some("de"));
        assert_eq!(table.rows.len(), 1);
        let mut rows = table.rows.clone();
        rows.push(row("SailHomePort", &["nach hause"]));
        let saved = save_table(&s.config, &upper, &rows, &[], &table.version).unwrap();
        assert!(saved.written, "{saved:?}");
        let names: Vec<_> = std::fs::read_dir(s.path("aliases"))
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(names, ["de.toml"]);
    }

    #[test]
    fn test_mixed_line_ends_stay() {
        let s = setup();
        let text = "# head\r\n[sound_alikes]\ncell = \"sell\"\r\nby = \"buy\"\n";
        std::fs::write(s.path("data/sound_alikes.toml"), text).unwrap();
        let rows = rows_of(&s, &TableKind::SoundAlikes);
        assert_eq!(
            render(&TableKind::SoundAlikes, Some(text), &rows).unwrap(),
            text
        );
        let hot = "# head\r\nSol\nVarnick\r\n";
        let rows = vec![row("Sol", &[]), row("Varnick", &[])];
        assert_eq!(render(&TableKind::Hotwords, Some(hot), &rows).unwrap(), hot);
        // A kept line keeps its own line end.
        let rows = vec![row("Sol", &[]), row("Varnick", &[]), row("Alioth", &[])];
        let out = render(&TableKind::Hotwords, Some(hot), &rows).unwrap();
        assert!(out.starts_with("# head\r\nSol\nVarnick\r\n"), "{out:?}");
    }

    /// A comment at the end of the line of an entry stays when the entry
    /// stays or changes. It moves to the next entry when the entry goes away.
    #[test]
    fn test_end_of_line_comment_stays_or_moves() {
        let text = "[sound_alikes]\ncell = \"sell\" # why\nby = \"buy\"\n";
        let kind = TableKind::SoundAlikes;
        let changed = render(
            &kind,
            Some(text),
            &[row("cell", &["cel"]), row("by", &["buy"])],
        )
        .unwrap();
        assert_eq!(
            changed,
            "[sound_alikes]\ncell = \"cel\" # why\nby = \"buy\"\n"
        );
        let removed = render(&kind, Some(text), &[row("by", &["buy"])]).unwrap();
        assert!(removed.contains("# why"), "{removed}");
        assert_eq!(parse_rows(&kind, &removed).unwrap(), [row("by", &["buy"])]);
        // The last entry goes away: the comment goes to the end of the file.
        let text = "[sound_alikes]\nby = \"buy\"\ncell = \"sell\" # why\n";
        let removed = render(&kind, Some(text), &[row("by", &["buy"])]).unwrap();
        assert!(removed.contains("# why"), "{removed}");
    }

    #[test]
    fn test_limits_come_before_any_file_access() {
        let missing = Path::new("Z:\\no\\such\\voicebridge.toml");
        let many: Vec<Row> = (0..=MAX_ROWS)
            .map(|i| row(&format!("w{i}"), &["x"]))
            .collect();
        let check = check_table(missing, &TableKind::Synonyms, &many);
        assert_eq!(check.problems.len(), 1);
        assert!(check.problems[0].contains("rows"), "{check:?}");
        let values: Vec<&str> = (0..=MAX_VALUES).map(|_| "v").collect();
        let check = check_table(missing, &TableKind::Synonyms, &[row("a", &values)]);
        assert!(check.problems[0].contains("values"), "{check:?}");
        let saved = save_table(missing, &TableKind::Synonyms, &many, &[], "v1").unwrap();
        assert!(!saved.written);
        assert!(saved.check.problems[0].contains("rows"));
    }

    #[test]
    fn test_format_characters_are_refused() {
        let s = setup();
        for text in [
            "a\u{200B}b",
            "\u{202E}abc",
            "x\u{FEFF}",
            "a\u{00AD}b",
            "a\u{2066}b",
        ] {
            let check = check_table(&s.config, &TableKind::Hotwords, &[row(text, &[])]);
            assert_eq!(
                check.problems,
                ["row 1: the entry has an invisible format character (for example a zero-width space or a direction mark)"],
                "{text:?}"
            );
        }
    }

    #[test]
    fn test_missing_folder_is_named() {
        let s = setup();
        std::fs::remove_dir_all(s.path("aliases")).unwrap();
        let fr = TableKind::Aliases {
            language: "fr".to_string(),
        };
        let rows = vec![row("OpenTideLogPanel", &["journal"])];
        let check = check_table(&s.config, &fr, &rows);
        let folder = Config::load(&s.config).unwrap().data.aliases_dir;
        assert_eq!(
            check.problems,
            [format!("the folder {} does not exist", folder.display())]
        );
        let s = setup_with("nowhere/hot.txt");
        let check = check_table(&s.config, &TableKind::Hotwords, &[row("Sol", &[])]);
        assert!(check.problems[0].starts_with("the folder "), "{check:?}");
        assert!(check.problems[0].contains("nowhere"), "{check:?}");
    }

    /// Texts with quotes, backslashes, "#", keys that need quotes, and an
    /// empty list go into the file and come back the same.
    #[test]
    fn test_escaping_round_trip() {
        let synonyms = vec![
            row("say \"hi\"", &["a \\ b", "c # d"]),
            row("plain", &[]),
            row("it's", &["x'y"]),
        ];
        let text = render(&TableKind::Synonyms, Some("[synonyms]\n"), &synonyms).unwrap();
        assert_eq!(
            parse_rows(&TableKind::Synonyms, &text).unwrap(),
            synonyms,
            "{text}"
        );
        let aliases = vec![
            row("SailTo:\"{TXT:HarbourName}\"", &["kurs \"nach\" <System>"]),
            row("Back\\slash # key", &[]),
        ];
        let text = render(&de(), None, &aliases).unwrap();
        assert_eq!(parse_rows(&de(), &text).unwrap(), aliases, "{text}");
        // The loader of the program reads the same values.
        let map: edcopilot_voicebridge_core::AliasMap = toml::from_str(&text).unwrap();
        assert_eq!(
            map["SailTo:\"{TXT:HarbourName}\""],
            ["kurs \"nach\" <System>"]
        );
        let sound = vec![row("a#b", &["c\"d"])];
        let text = render(&TableKind::SoundAlikes, None, &sound).unwrap();
        assert_eq!(
            parse_rows(&TableKind::SoundAlikes, &text).unwrap(),
            sound,
            "{text}"
        );
    }

    fn append_config(s: &Setup, extra: &str) {
        let text = std::fs::read_to_string(&s.config).unwrap();
        std::fs::write(&s.config, text + extra).unwrap();
    }

    /// The hotwords are on, but the list cannot be judged: an alias row
    /// that is near a hotword must not check clean.
    #[test]
    fn test_aliases_cannot_be_judged_when_the_hotwords_cannot_load() {
        for (extra, start) in [
            (
                "hotwords_score = 9.0\n",
                "the check cannot judge the hotwords: stt.parakeet.hotwords_score is 9",
            ),
            (
                "",
                "the check cannot judge the hotwords: stt.parakeet.hotwords_file: cannot read the hotwords file",
            ),
        ] {
            let s = if extra.is_empty() {
                setup_with("data/missing.txt")
            } else {
                setup()
            };
            append_config(&s, extra);
            let before = s.text("aliases/de.toml");
            let mut rows = rows_of(&s, &de());
            rows.push(row("SailHomePort", &["nach alioth"]));
            let check = check_table(&s.config, &de(), &rows);
            assert!(
                check.problems.iter().any(|p| p.starts_with(start)),
                "{start}: {check:?}"
            );
            let version = read_table(&s.config, &de()).unwrap().version;
            let saved = save_table(&s.config, &de(), &rows, &check.warnings, &version).unwrap();
            assert!(!saved.written);
            assert_eq!(s.text("aliases/de.toml"), before);
        }
        // With the hotwords off nothing is to judge.
        let s = setup_with("");
        append_config(&s, "hotwords_score = 9.0\n");
        let mut rows = rows_of(&s, &de());
        rows.push(row("SailHomePort", &["nach alioth"]));
        let check = check_table(&s.config, &de(), &rows);
        assert!(check.problems.is_empty(), "{check:?}");
    }

    /// No model pieces: a problem only for the local engine with the
    /// hotwords on. Else a warning, because the session start tests the
    /// list again.
    #[test]
    fn test_missing_model_pieces_for_the_four_combinations() {
        for (hotwords, cloud, problem) in [
            ("data/hotwords.txt", false, true),
            ("", false, false),
            ("data/hotwords.txt", true, false),
            ("", true, false),
        ] {
            let s = setup_with(hotwords);
            if cloud {
                append_config(&s, "\n[stt]\nengine = 'openai_compatible'\n");
            }
            std::fs::remove_file(s.path("model/tokens.txt")).unwrap();
            let kind = TableKind::Hotwords;
            let rows = vec![row("Sol", &[])];
            let check = check_table(&s.config, &kind, &rows);
            let start = "the check cannot test the hotwords against the model pieces";
            let in_problems = check.problems.iter().any(|p| p.starts_with(start));
            let in_warnings = check.warnings.iter().any(|w| {
                w.starts_with(start)
                    && w.ends_with("The program tests them again at the session start.")
            });
            assert_eq!(
                (in_problems, in_warnings),
                (problem, !problem),
                "{hotwords} {cloud}: {check:?}"
            );
        }
    }

    #[test]
    fn test_an_empty_hotword_list_keeps_the_console_warning() {
        let s = setup();
        let check = check_table(&s.config, &TableKind::Hotwords, &[]);
        let path = Config::load(&s.config).unwrap().stt.parakeet.hotwords_file;
        assert_eq!(
            check.warnings,
            [format!(
                "stt.parakeet.hotwords_file has no hotword, the second pass is off: {}",
                path.display()
            )]
        );
        assert!(check.problems.is_empty(), "{check:?}");
    }

    /// An entry with spaces at its ends stays as the file has it when the
    /// user does not change it. The page sends the rows without the spaces.
    #[test]
    fn test_an_untrimmed_entry_stays_as_it_is() {
        let s = setup();
        let text = "[synonyms]\n\" row to \" = [\" sail to\"]\n";
        std::fs::write(s.path("data/synonyms.toml"), text).unwrap();
        let table = read_table(&s.config, &TableKind::Synonyms).unwrap();
        let mut rows = trimmed(&table.rows);
        rows.push(row("row me for", &["sail for"]));
        let saved =
            save_table(&s.config, &TableKind::Synonyms, &rows, &[], &table.version).unwrap();
        assert!(saved.written, "{saved:?}");
        assert!(
            s.text("data/synonyms.toml").starts_with(text),
            "{}",
            s.text("data/synonyms.toml")
        );
        let fillers = "not_last = [\" the\"]\nmay_be_last = []\n";
        let rows = vec![row("the", &[NOT_LAST]), row("uh", &[MAY_BE_LAST])];
        let out = render(&TableKind::Fillers, Some(fillers), &rows).unwrap();
        assert!(out.starts_with("not_last = [\" the\"]\n"), "{out}");
    }

    /// The time of one check with the real data. Run it by hand:
    /// `VOICEBRIDGE_UI_CONFIG=<scratch config> cargo test -p voicebridge
    /// --lib measure_check_time -- --ignored --nocapture`. The scratch
    /// configuration must point at copies of the tables.
    #[test]
    #[ignore]
    fn measure_check_time() {
        let config =
            PathBuf::from(std::env::var("VOICEBRIDGE_UI_CONFIG").expect("VOICEBRIDGE_UI_CONFIG"));
        let kind = TableKind::Synonyms;
        let rows = read_table(&config, &kind).unwrap().rows;
        for round in 0..3 {
            let start = std::time::Instant::now();
            let check = check_table(&config, &kind, &rows);
            println!(
                "check {round}: {} ms, {check:?}",
                start.elapsed().as_millis()
            );
        }
    }
}
