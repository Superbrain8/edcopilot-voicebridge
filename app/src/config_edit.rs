//! Edits of the configuration text, and the atomic save of the file.
//!
//! An edit sets or removes one key of the known configuration. The comments,
//! the order of the keys, and the layout of the rest of the text stay the
//! same. The comment lines above a removed key stay in the file. A save
//! writes a temporary file in the same folder and then renames it. A result
//! that does not load, or that has a new problem or a problem on an edited
//! key, is never written (see `save_edits`).

use std::io::Write;
use std::path::Path;

use anyhow::{anyhow, bail, Context};
use toml_edit::{Array, DocumentMut, InlineTable, Item, Table, TableLike};

use crate::config::{is_variable_name, Config, API_KEY_ENV_PROBLEM};

/// The new value of one key.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Text(String),
    Number(f64),
    Integer(i64),
    Flag(bool),
    TextList(Vec<String>),
    /// Removes the key. The program then uses the default value.
    Absent,
}

/// The kind of value that a key takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Text,
    /// A number with a fraction. An integer is also accepted.
    Number,
    Integer,
    Flag,
    TextList,
    /// A table. The only edit is its removal.
    Table,
    /// A key of an older version that the program does not use any more.
    /// The only edit is its removal.
    Old,
}

/// Each key that an edit can set. No key holds a secret:
/// `stt.openai_compatible.api_key_env` is the name of a variable.
const KNOWN_KEYS: &[(&str, Kind)] = &[
    ("edcopilot.dir", Kind::Text),
    ("edcopilot.request_file", Kind::Text),
    ("edcopilot.voice_commands_ini", Kind::Text),
    ("edcopilot.dynamic_vars_dir", Kind::Text),
    ("edcopilot.profile", Kind::Text),
    ("data.command_map", Kind::Old),
    ("data.profile_vars", Kind::Old),
    ("data.aliases_dir", Kind::Text),
    ("data.sound_alikes", Kind::Text),
    ("data.synonyms", Kind::Text),
    ("data.fillers", Kind::Text),
    ("matching.threshold", Kind::Number),
    ("matching.exact_single_word", Kind::Flag),
    ("microphone.device", Kind::Text),
    ("microphone.mode", Kind::Text),
    ("microphone.vad_threshold_dbfs", Kind::Number),
    ("push_to_talk.key", Kind::Text),
    ("push_to_talk.joystick", Kind::Table),
    ("push_to_talk.joystick.device", Kind::Text),
    ("push_to_talk.joystick.button", Kind::Integer),
    ("push_to_talk.pre_roll_ms", Kind::Integer),
    ("push_to_talk.release_tail_ms", Kind::Integer),
    ("always_listening.wake_words", Kind::TextList),
    ("stt.engine", Kind::Text),
    ("stt.parakeet.model_dir", Kind::Text),
    ("stt.parakeet.threads", Kind::Integer),
    ("stt.parakeet.hotwords_file", Kind::Text),
    ("stt.parakeet.hotwords_score", Kind::Number),
    ("stt.openai_compatible.base_url", Kind::Text),
    ("stt.openai_compatible.api_key_env", Kind::Text),
    ("stt.openai_compatible.model", Kind::Text),
    ("stt.openai_compatible.language", Kind::Text),
    ("stt.openai_compatible.timeout_seconds", Kind::Integer),
    ("diagnostics.log_file", Kind::Text),
    ("diagnostics.save_audio_dir", Kind::Text),
];

/// Tables that the program writes as inline tables, for example
/// `joystick = { device = "MOZA", button = 5 }`.
const INLINE_TABLES: &[&str] = &["push_to_talk.joystick"];

/// The names of the known configuration keys.
pub(crate) fn known_key_names() -> impl Iterator<Item = &'static str> {
    KNOWN_KEYS.iter().map(|(name, _)| *name)
}

fn kind_of(key: &str) -> anyhow::Result<Kind> {
    KNOWN_KEYS
        .iter()
        .find(|(name, _)| *name == key)
        .map(|(_, kind)| *kind)
        .ok_or_else(|| anyhow!("{key} is not a key of the configuration"))
}

/// The largest number of characters in a text value.
pub const MAX_TEXT_CHARS: usize = 4096;
/// The largest number of entries in a list of texts.
pub const MAX_LIST_ENTRIES: usize = 64;
/// The largest number of characters in one entry of a list.
pub const MAX_ENTRY_CHARS: usize = 256;

/// Refuses a text that is too long or that has a control character. No
/// key of the configuration needs a tab or a line break, also no path. The
/// error does not repeat the value.
fn check_text(key: &str, text: &str, max: usize) -> anyhow::Result<()> {
    if text.chars().count() > max {
        bail!("{key} is too long, it must have at most {max} characters");
    }
    if text.chars().any(char::is_control) {
        bail!("{key} has a control character (for example a tab or a line break)");
    }
    Ok(())
}

/// The TOML value for `key`, or `None` for `Absent`. A value of the wrong
/// kind is an error.
fn toml_value(key: &str, value: &Value) -> anyhow::Result<Option<toml_edit::Value>> {
    let kind = kind_of(key)?;
    let out = match (kind, value) {
        (_, Value::Absent) => return Ok(None),
        (Kind::Old, _) => bail!("{key} is not used any more, an edit can only remove it"),
        // The file must never hold a key. The error does not repeat the value.
        (Kind::Text, Value::Text(text))
            if key == "stt.openai_compatible.api_key_env"
                && !text.is_empty()
                && !is_variable_name(text) =>
        {
            bail!("{API_KEY_ENV_PROBLEM}")
        }
        (Kind::Text, Value::Text(text)) => {
            check_text(key, text, MAX_TEXT_CHARS)?;
            toml_edit::Value::from(text.as_str())
        }
        (Kind::Number, Value::Number(n)) => {
            if !n.is_finite() {
                bail!("{key} must be a finite number");
            }
            toml_edit::Value::from(*n)
        }
        // An integer for a number key is written as a number with a
        // fraction, so that the file shows the kind of the key.
        (Kind::Number, Value::Integer(n)) => toml_edit::Value::from(*n as f64),
        (Kind::Integer, Value::Integer(n)) => toml_edit::Value::from(*n),
        (Kind::Flag, Value::Flag(flag)) => toml_edit::Value::from(*flag),
        (Kind::TextList, Value::TextList(list)) => {
            if list.len() > MAX_LIST_ENTRIES {
                bail!("{key} has too many entries, it must have at most {MAX_LIST_ENTRIES}");
            }
            for entry in list {
                check_text(key, entry, MAX_ENTRY_CHARS)?;
            }
            toml_edit::Value::Array(list.iter().map(String::as_str).collect::<Array>())
        }
        (kind, _) => bail!("{key} needs a value of the kind {kind:?}"),
    };
    Ok(Some(out))
}

/// The table at `path`. A missing table is added when `create` is true,
/// else the result is `None`. A value that is not a table is an error.
fn table_at<'a>(
    doc: &'a mut DocumentMut,
    path: &[&str],
    create: bool,
) -> anyhow::Result<Option<&'a mut dyn TableLike>> {
    let mut item: &mut Item = doc.as_item_mut();
    for (i, segment) in path.iter().enumerate() {
        let name = path[..=i].join(".");
        let parent_inline = item.is_inline_table();
        let table = item.as_table_like_mut().ok_or_else(|| {
            anyhow!(
                "{} in the configuration is not a table",
                path[..i].join(".")
            )
        })?;
        if table.get(segment).is_none() {
            if !create {
                return Ok(None);
            }
            let new = if parent_inline || INLINE_TABLES.contains(&name.as_str()) {
                Item::Value(toml_edit::Value::InlineTable(InlineTable::new()))
            } else {
                let mut new = Table::new();
                // A table that holds only other tables gets no header line.
                new.set_implicit(i + 1 < path.len());
                Item::Table(new)
            };
            table.insert(segment, new);
        }
        item = table
            .get_mut(segment)
            .ok_or_else(|| anyhow!("cannot add the table {name}"))?;
        if !item.is_table_like() {
            bail!("{name} in the configuration is not a table");
        }
    }
    Ok(item.as_table_like_mut())
}

/// The comment lines above a key: its prefix, when the prefix has a comment.
fn comment_prefix(table: &dyn TableLike, name: &str) -> Option<String> {
    let prefix = table.key(name)?.leaf_decor().prefix()?.as_str()?;
    prefix.contains('#').then(|| prefix.to_string())
}

/// Puts `lines` in front of the prefix of a decor.
fn prepend(decor: &mut toml_edit::Decor, lines: &str) {
    let old = decor
        .prefix()
        .and_then(|p| p.as_str())
        .unwrap_or_default()
        .to_string();
    decor.set_prefix(format!("{lines}{old}"));
}

/// All standard tables of the document with their position and path.
fn tables_by_position(item: &Item, path: &mut Vec<String>, out: &mut Vec<(usize, Vec<String>)>) {
    let Some(table) = item.as_table() else {
        return;
    };
    if let Some(position) = table.position() {
        if !table.is_implicit() {
            out.push((position, path.clone()));
        }
    }
    for (key, child) in table.iter() {
        path.push(key.to_string());
        tables_by_position(child, path, out);
        path.pop();
    }
}

/// Removes a key. Its comment lines go to the next key of the table, else
/// in front of the next table, else to the end of the document. Thus the
/// help text of a file stays when a key goes away.
fn remove_key(doc: &mut DocumentMut, path: &[&str], name: &str) -> anyhow::Result<()> {
    let Some(table) = table_at(doc, path, false)? else {
        return Ok(());
    };
    if table.get(name).is_none() {
        return Ok(());
    }
    let comment = comment_prefix(table, name);
    let keys: Vec<String> = table.iter().map(|(k, _)| k.to_string()).collect();
    let next = keys
        .iter()
        .position(|k| k == name)
        .and_then(|i| keys.get(i + 1))
        .cloned();
    table.remove(name);
    let Some(comment) = comment else {
        return Ok(());
    };
    if let Some(next) = next {
        if let Some(mut key) = table.key_mut(&next) {
            prepend(key.leaf_decor_mut(), &comment);
            return Ok(());
        }
    }
    // The last key of its table: find the table that comes next in the file.
    let mut found = Vec::new();
    tables_by_position(doc.as_item(), &mut Vec::new(), &mut found);
    let own: Vec<String> = path.iter().map(|s| s.to_string()).collect();
    let own_position = found
        .iter()
        .find(|(_, p)| *p == own)
        .map(|(position, _)| *position);
    let next_table = own_position.and_then(|own_position| {
        found
            .iter()
            .filter(|(position, _)| *position > own_position)
            .min_by_key(|(position, _)| *position)
            .map(|(_, p)| p.clone())
    });
    if let Some(next_path) = next_table {
        let mut item = doc.as_item_mut();
        for segment in &next_path {
            item = &mut item[segment.as_str()];
        }
        if let Some(table) = item.as_table_mut() {
            prepend(table.decor_mut(), &comment);
            return Ok(());
        }
    }
    let trailing = doc.trailing().as_str().unwrap_or_default().to_string();
    doc.set_trailing(format!("{comment}{trailing}"));
    Ok(())
}

/// Sets one key in the parsed document. See `set_value`.
fn set_in_doc(doc: &mut DocumentMut, key: &str, value: &Value) -> anyhow::Result<()> {
    let new = toml_value(key, value)?;
    let parts: Vec<&str> = key.split('.').collect();
    let (name, path) = parts.split_last().expect("a known key has a name");
    let Some(mut new) = new else {
        return remove_key(doc, path, name);
    };
    let table = table_at(doc, path, true)?.expect("table_at adds the table");
    if let Some(item) = table.get_mut(name) {
        // The line keeps its comment and its spaces.
        if let Some(old) = item.as_value() {
            *new.decor_mut() = old.decor().clone();
        }
        *item = Item::Value(new);
    } else {
        table.insert(name, Item::Value(new));
    }
    Ok(())
}

fn parse(toml_text: &str) -> anyhow::Result<DocumentMut> {
    toml_text.parse().map_err(|e| {
        anyhow!(
            "the configuration has an error: {}",
            crate::toml_error::edit_error_text(&e, toml_text)
        )
    })
}

/// Sets one key, for example "stt.parakeet.threads". Keeps comments and order.
/// `Absent` removes the key. An unknown table or key is an error.
pub fn set_value(toml_text: &str, key: &str, value: &Value) -> anyhow::Result<String> {
    apply_edits(toml_text, &[(key.to_string(), value.clone())])
}

const BOM: char = '\u{feff}';

/// Applies the edits in their order. See `set_value`. The text keeps its
/// byte order mark and its line ends.
pub fn apply_edits(toml_text: &str, edits: &[(String, Value)]) -> anyhow::Result<String> {
    edit_text(toml_text, |doc| {
        for (key, value) in edits {
            set_in_doc(doc, key, value)?;
        }
        Ok(())
    })
}

/// The old comment line above a removed key, for `remove_keys`.
#[derive(Debug, Clone, Copy)]
pub struct OldComment<'a> {
    /// The comment line. The spaces at the ends do not count.
    pub line: &'a str,
    /// The old line also described the key `next.0` (a key of the same
    /// table). When that key directly follows the removed lines, it gets
    /// the comment line `next.1` in place of the old line.
    pub next: Option<(&'a str, &'a str)>,
}

/// The prefix of a key, or "".
fn key_prefix(table: &dyn TableLike, name: &str) -> String {
    table
        .key(name)
        .and_then(|k| k.leaf_decor().prefix())
        .and_then(|p| p.as_str())
        .unwrap_or("")
        .to_string()
}

/// The key that directly follows the removed key `first` and the removed
/// keys after it: the keys between have no comment line and no empty line,
/// and the key itself has none. `None` when a removed key has a comment or
/// an empty line, or when no key follows in the table.
fn directly_after(table: &dyn TableLike, first: &str, removed: &[&str]) -> Option<String> {
    let keys: Vec<String> = table.iter().map(|(k, _)| k.to_string()).collect();
    let start = keys.iter().position(|k| k == first)?;
    let blank = |name: &str| key_prefix(table, name).trim_matches([' ', '\t']).is_empty();
    for key in &keys[start + 1..] {
        if !blank(key) {
            return None;
        }
        if !removed.contains(&key.as_str()) {
            return Some(key.clone());
        }
    }
    None
}

/// Removes the keys, as `Value::Absent` does. When the comment line
/// directly above a removed key is `old.line` (the spaces at the ends do
/// not count), that line goes with the key. When `old.next` names the key
/// that directly follows the removed lines, that key gets the comment line
/// of `old.next` in place of the old line. The other comment lines stay
/// (see `remove_key`). The text keeps its byte order mark and its line
/// ends.
pub fn remove_keys(
    toml_text: &str,
    keys: &[&str],
    old: Option<OldComment>,
) -> anyhow::Result<String> {
    edit_text(toml_text, |doc| {
        // The key that gets the new comment line, with its table.
        let mut new_comment: Option<(Vec<String>, String, &str)> = None;
        for key in keys {
            let parts: Vec<&str> = key.split('.').collect();
            let (name, path) = parts.split_last().expect("a key has a name");
            if let Some(table) = table_at(doc, path, false)? {
                let prefix = table
                    .key(name)
                    .and_then(|k| k.leaf_decor().prefix())
                    .and_then(|p| p.as_str())
                    .map(str::to_string);
                let rest = prefix
                    .zip(old)
                    .and_then(|(p, c)| without_last_line(&p, c.line));
                if let Some(rest) = rest {
                    if let Some((next, line)) = old.and_then(|c| c.next) {
                        // The removed keys of the same table, by name.
                        let removed: Vec<&str> = keys
                            .iter()
                            .filter_map(|k| k.strip_prefix(&format!("{}.", path.join("."))))
                            .collect();
                        if directly_after(&*table, name, &removed).as_deref() == Some(next) {
                            let path = path.iter().map(|s| s.to_string()).collect();
                            new_comment = Some((path, next.to_string(), line));
                        }
                    }
                    if let Some(mut found) = table.key_mut(name) {
                        found.leaf_decor_mut().set_prefix(rest);
                    }
                }
            }
            set_in_doc(doc, key, &Value::Absent)?;
        }
        if let Some((path, next, line)) = new_comment {
            let path: Vec<&str> = path.iter().map(String::as_str).collect();
            if let Some(table) = table_at(doc, &path, false)? {
                if let Some(mut key) = table.key_mut(&next) {
                    let decor = key.leaf_decor_mut();
                    let prefix = decor
                        .prefix()
                        .and_then(|p| p.as_str())
                        .unwrap_or("")
                        .to_string();
                    // The spaces in front of the key come after the comment.
                    let body = prefix.trim_end_matches([' ', '\t']);
                    let indent = &prefix[body.len()..];
                    decor.set_prefix(format!("{body}{line}\n{indent}"));
                }
            }
        }
        Ok(())
    })
}

/// The prefix of a key without its last comment line, when that line is
/// `line`. `None` when the last line is another text.
fn without_last_line(prefix: &str, line: &str) -> Option<String> {
    // The spaces in front of the key come after the last line end.
    let body = prefix.trim_end_matches([' ', '\t']);
    let indent = &prefix[body.len()..];
    let lines = body.strip_suffix('\n')?;
    let lines = lines.strip_suffix('\r').unwrap_or(lines);
    let start = lines.rfind('\n').map_or(0, |i| i + 1);
    (lines[start..].trim() == line).then(|| format!("{}{indent}", &body[..start]))
}

/// Parses the text, applies `edit`, and writes the document. The text
/// keeps its byte order mark and its line ends.
fn edit_text(
    toml_text: &str,
    edit: impl FnOnce(&mut DocumentMut) -> anyhow::Result<()>,
) -> anyhow::Result<String> {
    let (bom, body) = match toml_text.strip_prefix(BOM) {
        Some(body) => (BOM.to_string(), body),
        None => (String::new(), toml_text),
    };
    let mut doc = parse(body)?;
    edit(&mut doc)?;
    let out = doc.to_string();
    // `toml_edit` writes "\n" only.
    let out = if body.contains("\r\n") {
        out.replace("\r\n", "\n").replace('\n', "\r\n")
    } else {
        out
    };
    Ok(format!("{bom}{out}"))
}

/// The folder of `path`. A bare file name gives the current folder.
pub fn parent_dir(path: &Path) -> &Path {
    match path.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir,
        _ => Path::new("."),
    }
}

/// The configuration in `text` for the file `path`, or the text of the
/// load error.
pub fn parse_config(path: &Path, text: &str) -> Result<Config, String> {
    Config::parse_file(text, path).map_err(|e| format!("{e:#}"))
}

/// The number of tries of the rename, and the wait after a failed try.
const RENAME_TRIES: u32 = 5;
const RENAME_WAIT: std::time::Duration = std::time::Duration::from_millis(50);

/// An error that another program can cause for a short time: it has the
/// file open (an antivirus program, an indexer, an editor).
fn is_transient(error: &std::io::Error) -> bool {
    /// The Windows error of a file that another program has open.
    const ERROR_SHARING_VIOLATION: i32 = 32;
    error.kind() == std::io::ErrorKind::PermissionDenied
        || (cfg!(windows) && error.raw_os_error() == Some(ERROR_SHARING_VIOLATION))
}

/// Writes the text through a temporary file in the same folder, then renames
/// it. A rename that another program blocks is tried again a few times. A
/// failure keeps the old file and leaves no temporary file.
pub fn write_atomic(path: &Path, text: &str) -> anyhow::Result<()> {
    write_atomic_named(path, text, "configuration file")
}

/// `write_atomic` for any file. `what` names the file in the error text,
/// for example "word table".
pub fn write_atomic_named(path: &Path, text: &str, what: &str) -> anyhow::Result<()> {
    let dir = parent_dir(path);
    let mut temp = tempfile::NamedTempFile::new_in(dir)
        .with_context(|| format!("cannot make a temporary file in {}", dir.display()))?;
    temp.write_all(text.as_bytes())
        .and_then(|()| temp.as_file().sync_all())
        .with_context(|| format!("cannot write a temporary file in {}", dir.display()))?;
    let mut tries = 1;
    loop {
        match temp.persist(path) {
            Ok(_) => return Ok(()),
            Err(error) if tries < RENAME_TRIES && is_transient(&error.error) => {
                temp = error.file;
                tries += 1;
                std::thread::sleep(RENAME_WAIT);
            }
            // The temporary file is removed when `error` is dropped.
            Err(error) => {
                return Err(error.error)
                    .with_context(|| format!("cannot replace {what} {}", path.display()))
            }
        }
    }
}

/// The stamp of a configuration text: a hash of the content and its length.
pub fn text_version(text: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hasher);
    format!("{:016x}-{}", hasher.finish(), text.len())
}

/// The keys of `keys` that have another value in `now` than in `then`. The
/// parsed TOML values are compared, so a change of the layout or of a
/// comment is not a change. A key that names a table compares the whole
/// table. A key that both texts do not have is not a change. A text that
/// does not parse gives an error.
pub fn changed_values(then: &str, now: &str, keys: &[&str]) -> anyhow::Result<Vec<String>> {
    let parse = |text: &str, which: &str| -> anyhow::Result<toml::Table> {
        toml::from_str(text).map_err(|e| {
            anyhow!(
                "the {which} text does not parse: {}",
                crate::toml_error::de_error_text(&e, text)
            )
        })
    };
    let then = parse(then, "old")?;
    let now = parse(now, "new")?;
    let value = |table: &toml::Table, key: &str| -> Option<toml::Value> {
        let mut parts = key.split('.');
        let mut node = table.get(parts.next()?)?;
        for part in parts {
            node = node.as_table()?.get(part)?;
        }
        Some(node.clone())
    };
    Ok(keys
        .iter()
        .filter(|key| value(&then, key) != value(&now, key))
        .map(|key| key.to_string())
        .collect())
}

/// The result of a save.
#[derive(Debug, Clone, PartialEq)]
pub struct SaveResult {
    /// With `written`: the problems that were in the file before and that
    /// the edits do not touch. Without `written`: the problems that refused
    /// the save. Empty without `written`: the text did not change.
    pub problems: Vec<String>,
    pub written: bool,
}

/// A problem with the key that it names.
pub type KeyedProblem = (&'static str, String);

/// The problems that a save checks: `Config::validate` and the checks of
/// the engine files. The API key itself is not a matter of the file, so its
/// absence is not checked here.
pub fn save_problems(config: &Config) -> Vec<KeyedProblem> {
    let mut problems = config.validate_keyed();
    problems.extend(table_path_problems(config));
    problems.extend(
        crate::run::engine_problems_keyed(config, |_| Some("set".to_string()))
            .into_iter()
            .filter(|(key, _)| *key != "stt.openai_compatible.api_key_env"),
    );
    problems
}

/// The problems of the paths of the word tables: a table file must end
/// with `.toml` (`.txt` for the hotwords), compared without case, and the
/// alias folder must not be a file. An empty path is not a problem. The
/// Words screen writes rows to these paths, so a save must not set another
/// kind of file.
pub fn table_path_problems(config: &Config) -> Vec<KeyedProblem> {
    let files: [(&'static str, &Path, &str); 4] = [
        ("data.synonyms", &config.data.synonyms, ".toml"),
        ("data.sound_alikes", &config.data.sound_alikes, ".toml"),
        ("data.fillers", &config.data.fillers, ".toml"),
        (
            "stt.parakeet.hotwords_file",
            &config.stt.parakeet.hotwords_file,
            ".txt",
        ),
    ];
    let mut problems: Vec<KeyedProblem> = files
        .into_iter()
        .filter(|(_, path, end)| {
            !path.as_os_str().is_empty() && !crate::tables::has_file_end(path, end)
        })
        .map(|(key, path, end)| {
            (
                key,
                format!(
                    "{key} must name a file that ends with {end}: {}",
                    path.display()
                ),
            )
        })
        .collect();
    let aliases = &config.data.aliases_dir;
    if aliases.is_file() {
        problems.push((
            "data.aliases_dir",
            format!(
                "data.aliases_dir must name a folder, not a file: {}",
                aliases.display()
            ),
        ));
    }
    problems
}

/// True when the problem key and the edited key are the same key, or one is
/// a table that holds the other.
fn related(problem_key: &str, edit_key: &str) -> bool {
    let inside =
        |a: &str, b: &str| a.len() > b.len() && a.starts_with(b) && a[b.len()..].starts_with('.');
    problem_key == edit_key || inside(problem_key, edit_key) || inside(edit_key, problem_key)
}

/// Applies the edits and writes the result when the save rule permits it.
///
/// The save is refused when the result does not load, when it has a
/// problem that the file did not have before (the texts are compared
/// exactly, with their count), or when a problem names a key of the edits.
/// Other problems that the file had before do not refuse the save. They
/// come back as information. A text that does not change is not written.
pub fn save_edits(path: &Path, edits: &[(String, Value)]) -> anyhow::Result<SaveResult> {
    save_edits_with(path, edits, save_problems)
}

/// `save_edits` with a given check.
pub fn save_edits_with(
    path: &Path,
    edits: &[(String, Value)],
    check: impl Fn(&Config) -> Vec<KeyedProblem>,
) -> anyhow::Result<SaveResult> {
    save_edits_guarded(path, edits, check, |_| None)
}

/// `save_edits_with` with a guard. The guard gets the text of the file
/// right before the edits apply. A problem of the guard refuses the save.
pub fn save_edits_guarded(
    path: &Path,
    edits: &[(String, Value)],
    check: impl Fn(&Config) -> Vec<KeyedProblem>,
    guard: impl Fn(&str) -> Option<String>,
) -> anyhow::Result<SaveResult> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("cannot read configuration file {}", path.display()))?;
    if let Some(problem) = guard(&text) {
        return Ok(SaveResult {
            problems: vec![problem],
            written: false,
        });
    }
    let new_text = apply_edits(&text, edits)
        .with_context(|| format!("cannot change configuration file {}", path.display()))?;
    let config = match parse_config(path, &new_text) {
        Ok(config) => config,
        Err(problem) => {
            return Ok(SaveResult {
                problems: vec![problem],
                written: false,
            })
        }
    };
    let after = check(&config);
    // A file that did not load before has no problems to keep.
    let mut before: Vec<String> = parse_config(path, &text)
        .map(|old| check(&old).into_iter().map(|(_, t)| t).collect())
        .unwrap_or_default();
    let mut blocking = Vec::new();
    let mut remaining = Vec::new();
    for (key, problem) in after {
        let was_there = match before.iter().position(|old| *old == problem) {
            Some(i) => {
                before.swap_remove(i);
                true
            }
            None => false,
        };
        if !was_there || edits.iter().any(|(edit, _)| related(key, edit)) {
            blocking.push(problem);
        } else {
            remaining.push(problem);
        }
    }
    if !blocking.is_empty() {
        return Ok(SaveResult {
            problems: blocking,
            written: false,
        });
    }
    let written = new_text != text;
    if written {
        write_atomic(path, &new_text)?;
    }
    Ok(SaveResult {
        problems: remaining,
        written,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXAMPLE: &str = include_str!("../voicebridge.example.toml");

    fn edit(key: &str, value: Value) -> (String, Value) {
        (key.to_string(), value)
    }

    /// The lines that differ between two texts with the same line count.
    fn changed_lines(a: &str, b: &str) -> Vec<(String, String)> {
        assert_eq!(a.lines().count(), b.lines().count(), "{b}");
        a.lines()
            .zip(b.lines())
            .filter(|(x, y)| x != y)
            .map(|(x, y)| (x.to_string(), y.to_string()))
            .collect()
    }

    /// The example file with a model folder, so that it is valid.
    fn valid_example() -> String {
        let text = set_value(EXAMPLE, "stt.parakeet.model_dir", &Value::Text("m".into())).unwrap();
        set_value(&text, "stt.parakeet.threads", &Value::Integer(1)).unwrap()
    }

    fn load(text: &str) -> Config {
        Config::parse(text, Path::new("."), "t.toml").unwrap()
    }

    /// The keys of an older version: an edit can only remove them.
    #[test]
    fn test_an_old_key_can_only_be_removed() {
        let text = "[data]\n# Old.\ncommand_map = 'a.json'\nprofile_vars = ''\naliases_dir = 'x'\n";
        for key in ["data.command_map", "data.profile_vars"] {
            let error = set_value(text, key, &Value::Text("b.json".into())).unwrap_err();
            assert_eq!(
                format!("{error:#}"),
                format!("{key} is not used any more, an edit can only remove it")
            );
        }
        // `remove_keys` drops the given comment line above a removed key
        // only. Another comment moves to the next key, as for `Absent`.
        let old = |line| Some(OldComment { line, next: None });
        let out = remove_keys(text, &["data.command_map"], old("# Old.")).unwrap();
        assert_eq!(out, "[data]\nprofile_vars = ''\naliases_dir = 'x'\n");
        let out = remove_keys(text, &["data.command_map"], old("# Other.")).unwrap();
        assert_eq!(
            out,
            "[data]\n# Old.\nprofile_vars = ''\naliases_dir = 'x'\n"
        );
        let out = remove_keys(text, &["data.command_map"], None).unwrap();
        assert_eq!(
            out,
            set_value(text, "data.command_map", &Value::Absent).unwrap()
        );
        // CRLF stays.
        let crlf = text.replace('\n', "\r\n");
        let out = remove_keys(
            &crlf,
            &["data.command_map", "data.profile_vars"],
            old("# Old."),
        )
        .unwrap();
        assert_eq!(out, "[data]\r\naliases_dir = 'x'\r\n");
    }

    /// The old comment line also described the next key: that key gets
    /// its own comment line, but only when it directly follows the removed
    /// lines.
    #[test]
    fn test_remove_keys_gives_the_next_key_its_comment() {
        let both = ["data.command_map", "data.profile_vars"];
        let old = Some(OldComment {
            line: "# Old.",
            next: Some(("aliases_dir", "# New.")),
        });
        let text = "[data]\n# Old.\ncommand_map = ''\nprofile_vars = ''\naliases_dir = 'x'\n";
        assert_eq!(
            remove_keys(text, &both, old).unwrap(),
            "[data]\n# New.\naliases_dir = 'x'\n"
        );
        // CRLF and spaces in front of the key stay.
        let crlf =
            "[data]\r\n# Old.\r\ncommand_map = ''\r\nprofile_vars = ''\r\n  aliases_dir = 'x'\r\n";
        assert_eq!(
            remove_keys(crlf, &both, old).unwrap(),
            "[data]\r\n# New.\r\n  aliases_dir = 'x'\r\n"
        );
        // An empty line, a comment, or another key between: the old line
        // goes, and the next key gets no new line.
        for between in ["\n", "# Mine.\n", "synonyms = 's'\n"] {
            let text = format!(
                "[data]\n# Old.\ncommand_map = ''\nprofile_vars = ''\n{between}aliases_dir = 'x'\n"
            );
            let out = remove_keys(&text, &both, old).unwrap();
            assert!(!out.contains("# New."), "{between:?}: {out}");
            assert!(!out.contains("# Old."), "{between:?}: {out}");
        }
        // Another key follows: no new line.
        let text = "[data]\n# Old.\ncommand_map = ''\nsynonyms = 's'\naliases_dir = 'x'\n";
        assert_eq!(
            remove_keys(text, &["data.command_map"], old).unwrap(),
            "[data]\nsynonyms = 's'\naliases_dir = 'x'\n"
        );
        // Without the old line, nothing is new.
        let text = "[data]\ncommand_map = ''\naliases_dir = 'x'\n";
        assert_eq!(
            remove_keys(text, &["data.command_map"], old).unwrap(),
            "[data]\naliases_dir = 'x'\n"
        );
    }

    #[test]
    fn test_changed_values_compares_the_values_and_not_the_text() {
        let then = "[matching]
threshold = 0.8 # the limit
[push_to_talk]
key = 'F13'
";
        // Another layout and another key: no edited key changed.
        let now = "[matching]
threshold  =  0.8
[push_to_talk]
key = \"F13\"
pre_roll_ms = 100
";
        let keys = ["matching.threshold", "push_to_talk.key"];
        assert_eq!(
            changed_values(then, now, &keys).unwrap(),
            Vec::<String>::new()
        );
        let now = "[matching]
threshold = 0.7
[push_to_talk]
key = 'F13'
";
        assert_eq!(
            changed_values(then, now, &keys).unwrap(),
            ["matching.threshold"]
        );
        // A key that one text does not have, and a table key.
        let now = "[matching]
threshold = 0.8
[push_to_talk.joystick]
device = 'Pad'
button = 3
";
        assert_eq!(
            changed_values(
                then,
                now,
                &["push_to_talk.key", "push_to_talk.joystick", "stt.engine"]
            )
            .unwrap(),
            ["push_to_talk.key", "push_to_talk.joystick"]
        );
        assert!(changed_values(then, "not = [toml", &keys).is_err());
        assert_ne!(text_version(then), text_version(now));
        assert_eq!(text_version(then), text_version(then));
    }

    #[test]
    fn test_set_value_keeps_comments_and_order() {
        let out = set_value(EXAMPLE, "stt.parakeet.threads", &Value::Integer(6)).unwrap();
        assert_eq!(
            changed_lines(EXAMPLE, &out),
            vec![("threads = 4".to_string(), "threads = 6".to_string())]
        );
    }

    #[test]
    fn test_set_value_keeps_the_line_ends() {
        let text = "[edcopilot]\r\n# The folder.\r\ndir = 'ed'\r\n";
        let out = set_value(text, "matching.threshold", &Value::Number(0.9)).unwrap();
        assert_eq!(
            out,
            "[edcopilot]\r\n# The folder.\r\ndir = 'ed'\r\n\r\n[matching]\r\nthreshold = 0.9\r\n"
        );
        let text = "[edcopilot]\ndir = 'ed'\n";
        let out = set_value(text, "edcopilot.dir", &Value::Text("x".into())).unwrap();
        assert_eq!(out, "[edcopilot]\ndir = \"x\"\n");
    }

    #[test]
    fn test_set_value_keeps_a_comment_at_the_end_of_the_line() {
        let text = "[edcopilot]\ndir = 'ed' # the folder\n";
        let out = set_value(text, "edcopilot.dir", &Value::Text("x".into())).unwrap();
        assert_eq!(out, "[edcopilot]\ndir = \"x\" # the folder\n");
    }

    #[test]
    fn test_set_value_adds_an_absent_key_in_its_table() {
        let out = set_value(
            EXAMPLE,
            "edcopilot.request_file",
            &Value::Text("req.txt".into()),
        )
        .unwrap();
        // The new line is in [edcopilot], before [data].
        let edcopilot = out.find("[edcopilot]").unwrap();
        let line = out.find("request_file = \"req.txt\"").unwrap();
        let data = out.find("[data]").unwrap();
        assert!(edcopilot < line && line < data, "{out}");
        assert_eq!(load(&out).edcopilot.request_file, Path::new("./req.txt"));
        // All old lines stay.
        for old in EXAMPLE.lines() {
            assert!(out.contains(old), "{old} lost");
        }
        // A missing table is added.
        let text = "[edcopilot]\ndir = 'ed'\n";
        let out = set_value(text, "stt.parakeet.threads", &Value::Integer(2)).unwrap();
        assert!(out.starts_with(text), "{out}");
        assert!(out.contains("[stt.parakeet]\nthreads = 2"), "{out}");
        assert!(!out.contains("[stt]\n"), "{out}");
        assert_eq!(load(&out).stt.parakeet.threads, 2);
    }

    #[test]
    fn test_edcopilot_profile_is_a_known_key() {
        let out = set_value(EXAMPLE, "edcopilot.profile", &Value::Text("p.vap".into())).unwrap();
        assert_eq!(load(&out).edcopilot.profile, Path::new("./p.vap"));
        let back = set_value(&out, "edcopilot.profile", &Value::Absent).unwrap();
        assert_eq!(back, EXAMPLE);
    }

    #[test]
    fn test_unknown_key_is_an_error() {
        for key in ["matching.bogus", "bogus.threshold", "matching", "", "stt"] {
            let error = set_value(EXAMPLE, key, &Value::Integer(1)).unwrap_err();
            assert!(
                format!("{error:#}").contains("not a key"),
                "{key}: {error:#}"
            );
        }
        // A known key with a value of the wrong kind.
        assert!(set_value(EXAMPLE, "stt.parakeet.threads", &Value::Text("4".into())).is_err());
        assert!(set_value(EXAMPLE, "matching.threshold", &Value::Number(f64::NAN)).is_err());
        assert!(set_value(EXAMPLE, "push_to_talk.joystick", &Value::Integer(5)).is_err());
        // A table name that holds a value in the file.
        assert!(set_value("stt = 5\n", "stt.engine", &Value::Text("x".into())).is_err());
        assert!(set_value("[edcopilot\n", "edcopilot.dir", &Value::Text("x".into())).is_err());
    }

    #[test]
    fn test_a_key_is_never_written_as_the_variable_name() {
        let secret = "sk-proj-SECRET123";
        let error = set_value(
            EXAMPLE,
            "stt.openai_compatible.api_key_env",
            &Value::Text(secret.into()),
        )
        .unwrap_err();
        let text = format!("{error:#} {error:?}");
        assert!(!text.contains("SECRET"), "{text}");
        assert!(text.contains("not the key itself"), "{text}");
        let out = set_value(
            EXAMPLE,
            "stt.openai_compatible.api_key_env",
            &Value::Text("MY_STT_KEY".into()),
        )
        .unwrap();
        assert!(out.contains("api_key_env = \"MY_STT_KEY\""), "{out}");
        // An empty name is permitted: the local engine needs no key.
        let out = set_value(
            EXAMPLE,
            "stt.openai_compatible.api_key_env",
            &Value::Text(String::new()),
        )
        .unwrap();
        assert!(out.contains("api_key_env = \"\""), "{out}");
    }

    /// An old local file with an empty name has no problem, and a save
    /// writes it.
    #[test]
    fn test_save_of_a_local_file_with_an_empty_name() {
        let dir = tempfile::tempdir().unwrap();
        let file = valid_file(dir.path());
        let text = std::fs::read_to_string(&file).unwrap();
        let text = set_value(
            &text,
            "stt.openai_compatible.api_key_env",
            &Value::Text(String::new()),
        )
        .unwrap();
        std::fs::write(&file, text).unwrap();
        let result = save_edits(&file, &[edit("matching.threshold", Value::Number(0.85))]).unwrap();
        assert_eq!(
            result,
            SaveResult {
                problems: vec![],
                written: true
            }
        );
    }

    #[test]
    fn test_set_value_each_kind() {
        let out = set_value(
            EXAMPLE,
            "microphone.device",
            &Value::Text("USB \"Mic\"".into()),
        )
        .unwrap();
        assert_eq!(load(&out).microphone.device, "USB \"Mic\"");
        assert_eq!(changed_lines(EXAMPLE, &out).len(), 1);

        let out = set_value(EXAMPLE, "matching.threshold", &Value::Number(0.85)).unwrap();
        assert!(out.lines().any(|l| l == "threshold = 0.85"), "{out}");
        assert_eq!(load(&out).matching.threshold, 0.85);
        let out = set_value(EXAMPLE, "matching.threshold", &Value::Integer(1)).unwrap();
        assert!(out.lines().any(|l| l == "threshold = 1.0"), "{out}");

        let out = set_value(EXAMPLE, "push_to_talk.pre_roll_ms", &Value::Integer(0)).unwrap();
        assert_eq!(load(&out).push_to_talk.pre_roll_ms, 0);

        let out = set_value(EXAMPLE, "matching.exact_single_word", &Value::Flag(false)).unwrap();
        assert!(!load(&out).matching.exact_single_word);
        assert_eq!(changed_lines(EXAMPLE, &out).len(), 1);

        let words = vec!["computer".to_string(), "ship \"one\"".to_string()];
        let out = set_value(
            EXAMPLE,
            "always_listening.wake_words",
            &Value::TextList(words.clone()),
        )
        .unwrap();
        assert_eq!(load(&out).always_listening.wake_words, words);
        assert_eq!(changed_lines(EXAMPLE, &out).len(), 1, "{out}");
        let out = set_value(
            EXAMPLE,
            "always_listening.wake_words",
            &Value::TextList(vec![]),
        )
        .unwrap();
        assert!(load(&out).always_listening.wake_words.is_empty());
    }

    #[test]
    fn test_set_value_absent_removes_a_present_key() {
        let out = set_value(EXAMPLE, "matching.threshold", &Value::Absent).unwrap();
        assert!(!out.contains("threshold = 0.8"), "{out}");
        // The comment line above the key moves to the next key.
        assert!(out.contains("# Minimum confidence"), "{out}");
        assert_eq!(out.lines().count(), EXAMPLE.lines().count() - 1);
        // The default applies.
        assert_eq!(load(&out).matching.threshold, 0.8);
        assert!(out.contains("# A phrase of one word needs confidence 1.0."));
    }

    #[test]
    fn test_set_value_absent_on_an_absent_key_changes_nothing() {
        let out = set_value(EXAMPLE, "edcopilot.request_file", &Value::Absent).unwrap();
        assert_eq!(out, EXAMPLE);
        let out = set_value(EXAMPLE, "push_to_talk.joystick", &Value::Absent).unwrap();
        assert_eq!(out, EXAMPLE);
        // The table is absent too: no table is added.
        let text = "[edcopilot]\ndir = 'ed'\n";
        let out = set_value(text, "stt.parakeet.threads", &Value::Absent).unwrap();
        assert_eq!(out, text);
    }

    #[test]
    fn test_joystick_is_an_inline_table() {
        let out = apply_edits(
            EXAMPLE,
            &[
                edit("push_to_talk.key", Value::Absent),
                edit("push_to_talk.joystick.device", Value::Text("MOZA".into())),
                edit("push_to_talk.joystick.button", Value::Integer(5)),
            ],
        )
        .unwrap();
        assert!(
            out.contains("joystick = { device = \"MOZA\", button = 5 }"),
            "{out}"
        );
        let config = load(&out);
        assert_eq!(config.push_to_talk.key, None);
        let j = config.push_to_talk.joystick.unwrap();
        assert_eq!((j.device.as_str(), j.button), ("MOZA", 5));
        // Removal of the whole table.
        let back = apply_edits(
            &out,
            &[
                edit("push_to_talk.joystick", Value::Absent),
                edit("push_to_talk.key", Value::Text("F13".into())),
            ],
        )
        .unwrap();
        assert!(
            !back.contains(
                "
joystick = {"
            ),
            "{back}"
        );
        assert_eq!(load(&back).push_to_talk.key.as_deref(), Some("F13"));
    }

    /// A valid configuration file in `dir`, with its model folder.
    fn valid_file(dir: &Path) -> std::path::PathBuf {
        std::fs::create_dir(dir.join("m")).unwrap();
        let file = dir.join("voicebridge.toml");
        std::fs::write(&file, valid_example()).unwrap();
        file
    }

    fn temp_files(dir: &Path) -> usize {
        std::fs::read_dir(dir)
            .unwrap()
            .filter(|e| e.as_ref().unwrap().path().is_file())
            .count()
    }

    #[test]
    fn test_save_edits_writes_a_valid_result() {
        let dir = tempfile::tempdir().unwrap();
        let file = valid_file(dir.path());
        let text = std::fs::read_to_string(&file).unwrap();
        let result = save_edits(&file, &[edit("matching.threshold", Value::Number(0.9))]).unwrap();
        assert_eq!(
            result,
            SaveResult {
                problems: vec![],
                written: true
            }
        );
        let saved = std::fs::read_to_string(&file).unwrap();
        assert_eq!(changed_lines(&text, &saved).len(), 1);
        assert_eq!(Config::load(&file).unwrap().matching.threshold, 0.9);
        assert_eq!(temp_files(dir.path()), 1, "a temporary file was left");
    }

    #[test]
    fn test_save_edits_does_not_write_an_invalid_result() {
        let dir = tempfile::tempdir().unwrap();
        let file = valid_file(dir.path());
        let before = std::fs::read(&file).unwrap();
        let result = save_edits(&file, &[edit("matching.threshold", Value::Number(7.0))]).unwrap();
        assert!(!result.written);
        assert_eq!(result.problems.len(), 1, "{result:?}");
        assert!(
            result.problems[0].contains("matching.threshold"),
            "{result:?}"
        );
        assert_eq!(std::fs::read(&file).unwrap(), before);
        // A result that does not load: a negative number for an unsigned key.
        let result = save_edits(
            &file,
            &[edit("push_to_talk.pre_roll_ms", Value::Integer(-5))],
        )
        .unwrap();
        assert!(!result.written);
        assert_eq!(result.problems.len(), 1, "{result:?}");
        assert!(
            result.problems[0].contains("voicebridge.toml"),
            "{result:?}"
        );
        // The removal of a required key.
        let result = save_edits(&file, &[edit("edcopilot.dir", Value::Absent)]).unwrap();
        assert!(!result.written);
        assert_eq!(result.problems.len(), 1, "{result:?}");
        assert_eq!(std::fs::read(&file).unwrap(), before);
        assert_eq!(temp_files(dir.path()), 1, "a temporary file was left");
    }

    #[test]
    fn test_save_rule_keeps_old_problems_as_information() {
        // A fresh copy of the example: the model folder is empty.
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("voicebridge.toml");
        std::fs::write(&file, EXAMPLE).unwrap();
        let result = save_edits(&file, &[edit("matching.threshold", Value::Number(0.85))]).unwrap();
        assert!(result.written, "{result:?}");
        assert!(
            result
                .problems
                .iter()
                .any(|p| p.starts_with("stt.parakeet.model_dir is empty")),
            "{result:?}"
        );
        assert_eq!(Config::load(&file).unwrap().matching.threshold, 0.85);

        // A new problem blocks the save.
        let before = std::fs::read(&file).unwrap();
        let result = save_edits(&file, &[edit("matching.threshold", Value::Number(7.0))]).unwrap();
        assert!(!result.written);
        assert_eq!(result.problems.len(), 1, "{result:?}");
        assert!(result.problems[0].starts_with("matching.threshold is 7"));
        assert_eq!(std::fs::read(&file).unwrap(), before);

        // A folder that does not exist: a new problem on the edited key,
        // while the old problem of the empty value was on the same key.
        let missing = dir.path().join("no-model").display().to_string();
        let result = save_edits(
            &file,
            &[edit("stt.parakeet.model_dir", Value::Text(missing))],
        )
        .unwrap();
        assert!(!result.written);
        assert!(
            result.problems[0].starts_with("stt.parakeet.model_dir is not a directory"),
            "{result:?}"
        );
        assert_eq!(std::fs::read(&file).unwrap(), before);

        // A problem that stays on an edited key also blocks the save:
        // threads 0 and then threads 0 again with a second edit.
        let text = set_value(EXAMPLE, "stt.parakeet.threads", &Value::Integer(0)).unwrap();
        std::fs::write(&file, &text).unwrap();
        let result = save_edits(
            &file,
            &[
                edit("stt.parakeet.threads", Value::Integer(0)),
                edit("matching.threshold", Value::Number(0.9)),
            ],
        )
        .unwrap();
        assert!(!result.written, "{result:?}");
        assert!(result.problems[0].starts_with("stt.parakeet.threads is 0"));
        assert_eq!(std::fs::read_to_string(&file).unwrap(), text);
    }

    #[test]
    fn test_save_rule_never_writes_a_file_that_does_not_load() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("voicebridge.toml");
        // Valid TOML, but an unknown key: the result does not load either.
        let text = format!("{EXAMPLE}\n[bogus]\nx = 1\n");
        std::fs::write(&file, &text).unwrap();
        let result = save_edits(&file, &[edit("matching.threshold", Value::Number(0.85))]).unwrap();
        assert!(!result.written);
        assert!(result.problems[0].contains("bogus"), "{result:?}");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), text);
        // Not TOML at all.
        std::fs::write(&file, "[edcopilot\n").unwrap();
        assert!(save_edits(&file, &[edit("matching.threshold", Value::Number(0.85))]).is_err());
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "[edcopilot\n");
    }

    #[test]
    fn test_save_skips_a_text_that_does_not_change() {
        let dir = tempfile::tempdir().unwrap();
        let file = valid_file(dir.path());
        let modified = std::fs::metadata(&file).unwrap().modified().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        let result = save_edits(&file, &[edit("matching.threshold", Value::Number(0.8))]).unwrap();
        assert_eq!(
            result,
            SaveResult {
                problems: vec![],
                written: false
            }
        );
        assert_eq!(
            std::fs::metadata(&file).unwrap().modified().unwrap(),
            modified
        );
    }

    #[test]
    fn test_absent_moves_the_comment_lines_to_the_next_item() {
        // From key to joystick: the help text of the key stays.
        let out = apply_edits(
            EXAMPLE,
            &[
                edit("push_to_talk.key", Value::Absent),
                edit("push_to_talk.joystick.device", Value::Text("MOZA".into())),
                edit("push_to_talk.joystick.button", Value::Integer(5)),
            ],
        )
        .unwrap();
        for line in EXAMPLE.lines().filter(|l| l.starts_with('#')) {
            assert!(out.lines().any(|l| l == line), "{line} lost in\n{out}");
        }
        let help = out.find("# A keyboard key:").unwrap();
        let pre_roll = out.find("pre_roll_ms = 250").unwrap();
        assert!(help < pre_roll, "{out}");
        // The last key of a table: the lines go before the next table.
        let out = set_value(EXAMPLE, "matching.exact_single_word", &Value::Absent).unwrap();
        let comment = out.find("# A phrase of one word").unwrap();
        let next = out.find("[microphone]").unwrap();
        let threshold = out.find("threshold = 0.8").unwrap();
        assert!(threshold < comment && comment < next, "{out}");
        assert!(!out.contains("exact_single_word"), "{out}");
        // The last key of the file: the lines stay at the end.
        let out = set_value(EXAMPLE, "diagnostics.save_audio_dir", &Value::Absent).unwrap();
        assert!(out.trim_end().ends_with("stay on this computer."), "{out}");
        assert!(!out.contains("save_audio_dir"), "{out}");
        load(&out);
    }

    #[test]
    fn test_byte_order_mark_stays() {
        let text = "\u{feff}[edcopilot]\ndir = 'ed'\n";
        let out = set_value(text, "matching.threshold", &Value::Number(0.9)).unwrap();
        assert!(out.starts_with("\u{feff}[edcopilot]\n"), "{out:?}");
        let out = set_value(
            "[edcopilot]\ndir = 'ed'\n",
            "edcopilot.dir",
            &Value::Text("x".into()),
        )
        .unwrap();
        assert!(!out.starts_with('\u{feff}'));
    }

    #[test]
    fn test_values_have_length_limits_and_no_control_characters() {
        let long = "x".repeat(MAX_TEXT_CHARS + 1);
        let cases = [
            ("microphone.device", Value::Text(long.clone())),
            ("microphone.device", Value::Text("USB\nMic".into())),
            ("microphone.device", Value::Text("USB\u{7}".into())),
            (
                "always_listening.wake_words",
                Value::TextList(vec!["w".into(); MAX_LIST_ENTRIES + 1]),
            ),
            (
                "always_listening.wake_words",
                Value::TextList(vec!["y".repeat(MAX_ENTRY_CHARS + 1)]),
            ),
            (
                "always_listening.wake_words",
                Value::TextList(vec!["co\tpilot".into()]),
            ),
        ];
        for (key, value) in cases {
            let error = set_value(EXAMPLE, key, &value).unwrap_err();
            let text = format!("{error:#}");
            assert!(text.starts_with(key), "{text}");
            assert!(!text.contains("xxxx") && !text.contains("yyyy"), "{text}");
        }
        let fits = "x".repeat(MAX_TEXT_CHARS);
        assert!(set_value(EXAMPLE, "microphone.device", &Value::Text(fits)).is_ok());
        let list = vec!["w".repeat(MAX_ENTRY_CHARS); MAX_LIST_ENTRIES];
        assert!(set_value(
            EXAMPLE,
            "always_listening.wake_words",
            &Value::TextList(list)
        )
        .is_ok());
        // A path with spaces and non-ASCII letters is fine.
        let path = Value::Text("D:\\Mod\u{e8}les\\parakeet v3".into());
        assert!(set_value(EXAMPLE, "stt.parakeet.model_dir", &path).is_ok());
    }

    #[test]
    fn test_save_edits_keeps_the_file_when_the_write_fails() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("voicebridge.toml");
        // The edit fails: the old text is not a table where the key goes.
        std::fs::write(&file, "push_to_talk = 5\n").unwrap();
        let edits = [edit("push_to_talk.key", Value::Text("F13".into()))];
        assert!(save_edits(&file, &edits).is_err());
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            "push_to_talk = 5\n"
        );
        // The file is absent: the error names it.
        let missing = dir.path().join("missing.toml");
        let error = save_edits(&missing, &edits).unwrap_err();
        assert!(format!("{error:#}").contains("missing.toml"), "{error:#}");
        assert!(!missing.exists());
        // The rename fails: Windows does not replace a read-only file.
        #[cfg(windows)]
        {
            std::fs::remove_file(&file).unwrap();
            let file = valid_file(dir.path());
            let text = std::fs::read_to_string(&file).unwrap();
            let mut permissions = std::fs::metadata(&file).unwrap().permissions();
            permissions.set_readonly(true);
            std::fs::set_permissions(&file, permissions.clone()).unwrap();
            let result = save_edits(&file, &[edit("matching.threshold", Value::Number(0.9))]);
            #[allow(clippy::permissions_set_readonly_false)]
            permissions.set_readonly(false);
            std::fs::set_permissions(&file, permissions).unwrap();
            let error = result.unwrap_err();
            assert!(
                format!("{error:#}").contains("voicebridge.toml"),
                "{error:#}"
            );
            assert_eq!(std::fs::read_to_string(&file).unwrap(), text);
        }
        assert_eq!(temp_files(dir.path()), 1, "a temporary file was left");
    }

    /// Another program has the file open without delete sharing for a
    /// short time. The save tries again and then succeeds.
    #[cfg(windows)]
    #[test]
    fn test_save_tries_again_while_another_program_has_the_file_open() {
        use std::os::windows::fs::OpenOptionsExt;
        let dir = tempfile::tempdir().unwrap();
        let file = valid_file(dir.path());
        // Share mode read only: a rename over the file fails.
        let held = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(&file)
            .unwrap();
        let release = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(150));
            drop(held);
        });
        let result = save_edits(&file, &[edit("matching.threshold", Value::Number(0.9))]).unwrap();
        release.join().unwrap();
        assert!(result.written, "{result:?}");
        assert_eq!(Config::load(&file).unwrap().matching.threshold, 0.9);
        assert_eq!(temp_files(dir.path()), 1, "a temporary file was left");
    }
}
