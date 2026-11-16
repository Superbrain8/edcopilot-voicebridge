//! The folders and the files of the installed program.
//!
//! - The data folder of the user is `%APPDATA%\VoiceBridge` (decision D2).
//!   It holds the configuration, the copies of the word tables, and the
//!   folder `models`.
//! - The program folder holds the resources: the example configuration and
//!   the shipped word tables. The program ships no command map: it reads
//!   the commands from the EDCoPilot profile on the computer of the user.
//!
//! The word tables are data of the user. The first start copies them into
//! the data folder, and the user edits the copies. The first start never
//! overwrites a file that exists, also not after an update.
//!
//! An older version wrote `data.command_map` and `data.profile_vars` into
//! the configuration, with paths to the map files of its program folder.
//! The update step at each start removes these two keys (see
//! `remove_old_map_keys`).
//!
//! The functions take the folders as arguments. Thus the tests never touch
//! the real data folder.

use std::ffi::OsString;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::Context;
use voicebridge::config_edit::{self, Value};

/// The name of the configuration file in the data folder.
pub const CONFIG_FILE: &str = "voicebridge.toml";
/// The name of the data folder below `%APPDATA%`.
pub const DATA_FOLDER: &str = "VoiceBridge";
/// The folder of the resources below the resource folder of Tauri.
pub const RESOURCE_FOLDER: &str = "resources";
/// The example configuration in the resource folder.
pub const EXAMPLE_FILE: &str = "voicebridge.example.toml";
/// The command map that older versions shipped in `resources/data`.
pub const COMMAND_MAP_FILE: &str = "command_map_v3.json";
/// The profile variables that older versions shipped in `resources/data`.
pub const PROFILE_VARS_FILE: &str = "profile_vars.json";
/// The word tables in `resources/data` that the first start copies.
pub const TABLE_FILES: [&str; 4] = [
    "synonyms.toml",
    "sound_alikes.toml",
    "fillers.toml",
    voicebridge::tables::HOTWORDS_COPY_FILE,
];
/// The alias folder in `resources` and in the data folder.
pub const ALIASES_FOLDER: &str = "aliases";
/// The usual EDCoPilot folder. The first start uses it when it exists.
pub const DEFAULT_EDCOPILOT_DIR: &str = "C:\\EDCoPilot";
/// The number of threads of the example configuration, at most.
pub const DEFAULT_THREADS: usize = 4;

/// The data folder for the value of `%APPDATA%`. `None` when the value is
/// absent or not an absolute path.
pub fn data_dir_in(appdata: Option<OsString>) -> Option<PathBuf> {
    appdata
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .map(|p| p.join(DATA_FOLDER))
}

/// The folder for the downloaded models: `<data dir>\models`.
pub fn models_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("models")
}

/// The value of `--config <path>` or `--config=<path>` in the arguments.
pub fn config_argument(args: &[String]) -> Option<PathBuf> {
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        if arg == "--config" {
            if let Some(path) = args.next() {
                return Some(PathBuf::from(path));
            }
        } else if let Some(path) = arg.strip_prefix("--config=") {
            return Some(PathBuf::from(path));
        }
    }
    None
}

/// The configuration path: `--config`, or `<data dir>/voicebridge.toml`.
pub fn config_path(args: &[String], data_dir: &Path) -> PathBuf {
    config_argument(args).unwrap_or_else(|| data_dir.join(CONFIG_FILE))
}

/// Makes the data folder at the first start: the configuration from the
/// example with absolute data paths, and copies of the word tables.
/// It never overwrites a file that exists. The result is true when the
/// call made a file or a folder.
///
/// `edcopilot_dir` is the EDCoPilot folder for the configuration. The
/// program gives `C:\EDCoPilot` when that folder exists (see
/// `prepare_data`). Without a folder the configuration has no folder, and
/// the user sets it on the Setup screen.
pub fn first_start_with(
    data_dir: &Path,
    resource_dir: &Path,
    edcopilot_dir: Option<&Path>,
) -> anyhow::Result<bool> {
    let data_dir = plain_path(&std::path::absolute(data_dir).context("the data folder")?);
    let shipped = plain_path(
        &std::path::absolute(resource_dir.join(RESOURCE_FOLDER)).context("the resource folder")?,
    );
    // Read the example first: without it, the call makes nothing.
    let config_file = data_dir.join(CONFIG_FILE);
    let config_text = if exists(&config_file) {
        None
    } else {
        let example = shipped.join(EXAMPLE_FILE);
        let text = fs::read_to_string(&example)
            .with_context(|| format!("cannot read {}", example.display()))?;
        Some(first_configuration(&text, &data_dir, edcopilot_dir)?)
    };

    let mut made = make_dir(&data_dir)?;
    remove_leftovers(&data_dir);
    remove_leftovers(&data_dir.join(ALIASES_FOLDER));
    for name in TABLE_FILES {
        made |= copy_new(&shipped.join("data").join(name), &data_dir.join(name))?;
    }
    let aliases = data_dir.join(ALIASES_FOLDER);
    made |= make_dir(&aliases)?;
    let shipped_aliases = shipped.join(ALIASES_FOLDER);
    let entries = fs::read_dir(&shipped_aliases)
        .with_context(|| format!("cannot read {}", shipped_aliases.display()))?;
    for entry in entries {
        let path = entry?.path();
        let is_table = path
            .extension()
            .is_some_and(|x| x.eq_ignore_ascii_case("toml"));
        if path.is_file() && is_table {
            if let Some(name) = path.file_name() {
                made |= copy_new(&path, &aliases.join(name))?;
            }
        }
    }
    // The configuration comes last. A start that stops before this point
    // makes the configuration at the next start.
    if let Some(text) = config_text {
        made |= write_new(&config_file, text.as_bytes())?;
    }
    Ok(made)
}

/// The text of the first configuration: the example with absolute paths.
/// It has no `data.command_map` and no `data.profile_vars`: the program
/// reads the EDCoPilot profile.
fn first_configuration(
    example: &str,
    data_dir: &Path,
    edcopilot_dir: Option<&Path>,
) -> anyhow::Result<String> {
    let text_of = |path: &Path| Value::Text(path.display().to_string());
    let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
    let threads = DEFAULT_THREADS.min(cores) as i64;
    let edits = [
        (
            "edcopilot.dir",
            edcopilot_dir.map_or(Value::Text(String::new()), text_of),
        ),
        ("data.aliases_dir", text_of(&data_dir.join(ALIASES_FOLDER))),
        (
            "data.sound_alikes",
            text_of(&data_dir.join("sound_alikes.toml")),
        ),
        ("data.synonyms", text_of(&data_dir.join("synonyms.toml"))),
        ("data.fillers", text_of(&data_dir.join("fillers.toml"))),
        ("stt.parakeet.threads", Value::Integer(threads)),
    ];
    let mut text = example.to_string();
    for (key, value) in &edits {
        text = config_edit::set_value(&text, key, value)
            .with_context(|| format!("cannot set {key} in the example configuration"))?;
    }
    Ok(text)
}

/// The path without the prefix `\\?\` of a verbatim Windows path. Tauri
/// gives the program folder with this prefix. Some programs do not read
/// such a path, and it is not a path that a user types. Only a path on a
/// drive letter (`\\?\D:\`) or on a network share (`\\?\UNC\`) changes.
/// Other verbatim paths, for example `\\?\Volume{...}`, stay as they are.
pub fn plain_path(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    if let Some(rest) = text.strip_prefix("\\\\?\\UNC\\") {
        return PathBuf::from(format!("\\\\{rest}"));
    }
    if let Some(rest) = text.strip_prefix("\\\\?\\") {
        let bytes = rest.as_bytes();
        let drive = bytes.len() >= 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && bytes[2] == b'\\';
        if drive {
            return PathBuf::from(rest);
        }
    }
    path.to_path_buf()
}

/// True when a file or a folder is at the path. A link counts too.
fn exists(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}

/// Makes the folder when it does not exist. True when the call made it.
fn make_dir(path: &Path) -> anyhow::Result<bool> {
    if path.is_dir() {
        return Ok(false);
    }
    fs::create_dir_all(path).with_context(|| format!("cannot make {}", path.display()))?;
    Ok(true)
}

/// Copies the file when the target does not exist. True when the call
/// made the target.
fn copy_new(from: &Path, to: &Path) -> anyhow::Result<bool> {
    if exists(to) {
        return Ok(false);
    }
    let bytes = fs::read(from).with_context(|| format!("cannot read {}", from.display()))?;
    write_new(to, &bytes)
}

/// Writes a new file in one step. The bytes go to a temporary file in the
/// same folder first, with a flush to the disk. Then a hard link gives the
/// file its name. A hard link never replaces a file that exists, also when
/// another program makes the file at the same time. Thus a kill during the
/// write leaves only a temporary file, which the next start removes. The
/// result is true when the call made the file.
fn write_new(path: &Path, bytes: &[u8]) -> anyhow::Result<bool> {
    if exists(path) {
        return Ok(false);
    }
    let dir = path.parent().unwrap_or_else(|| Path::new(""));
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let temp = dir.join(format!(
        "{TEMP_PREFIX}{}-{name}{TEMP_SUFFIX}",
        std::process::id()
    ));
    let written = (|| -> std::io::Result<()> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()
    })();
    if let Err(e) = written {
        let _ = fs::remove_file(&temp);
        return Err(e).with_context(|| format!("cannot write {}", temp.display()));
    }
    match fs::hard_link(&temp, path) {
        Ok(()) => {
            let _ = fs::remove_file(&temp);
            Ok(true)
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            let _ = fs::remove_file(&temp);
            Ok(false)
        }
        // A volume without hard links, for example FAT32 or exFAT.
        Err(_) => move_new(&temp, path),
    }
}

/// Moves the temporary file to its name. `MoveFileExW` without
/// `MOVEFILE_REPLACE_EXISTING` is one step and never replaces a file that
/// exists. (`std::fs::rename` replaces a file on Windows, so it is not
/// used.) The temporary file is gone after the call. The result is true
/// when the call made the file.
#[cfg(windows)]
fn move_new(temp: &Path, path: &Path) -> anyhow::Result<bool> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::MoveFileExW;
    let wide = |p: &Path| -> Vec<u16> { p.as_os_str().encode_wide().chain(Some(0)).collect() };
    let (from, to) = (wide(temp), wide(path));
    // SAFETY: both texts end with 0 and live during the call. The flags
    // are 0: no replace, no copy across volumes.
    let ok = unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), 0) };
    if ok != 0 {
        return Ok(true);
    }
    let error = std::io::Error::last_os_error();
    let _ = fs::remove_file(temp);
    if error.kind() == std::io::ErrorKind::AlreadyExists {
        Ok(false)
    } else {
        Err(error).with_context(|| format!("cannot write {}", path.display()))
    }
}

/// Other systems: a hard link is the only way that never replaces a file.
#[cfg(not(windows))]
fn move_new(temp: &Path, path: &Path) -> anyhow::Result<bool> {
    let _ = fs::remove_file(temp);
    anyhow::bail!(
        "cannot write {}: the file system has no hard links",
        path.display()
    )
}

/// The first part of the name of a temporary file of the first start.
pub const TEMP_PREFIX: &str = ".vb-first-start-";
/// The last part of the name of a temporary file of the first start. No
/// loader reads a file with this end.
pub const TEMP_SUFFIX: &str = ".tmp";
/// The problem of a start without `--config` when `%APPDATA%` is unknown.
pub const NO_APPDATA: &str = "APPDATA is not set, start the program with --config <file>";
/// The problem of a development build without `--config`.
pub const DEV_BUILD: &str = "this is a development build, start it with --config <file>, \
or set VOICEBRIDGE_DEV_DATA_DIR to an absolute scratch folder";
/// The environment variable that gives a development build a data folder.
pub const DEV_DATA_DIR_VARIABLE: &str = "VOICEBRIDGE_DEV_DATA_DIR";
/// The note after the update step removed both map keys of an older
/// version: `map_keys_removed` of the two keys. The tests use it.
#[cfg(test)]
pub const MAP_KEYS_REMOVED: &str =
    "The program reads the commands from the EDCoPilot profile now. \
The update removed data.command_map and data.profile_vars from the configuration.";
/// The comment line that the old example configuration (and thus an older
/// first start) wrote directly above `data.command_map`.
pub const OLD_MAP_COMMENT: &str = "# The command map, the profile variables, and the alias folder.";
/// The comment line of the current example configuration above
/// `data.aliases_dir`. The old comment also described that key: when the
/// key directly follows the removed lines, it gets this line.
pub const ALIAS_COMMENT: &str = "# The alias folder.";

/// The names of the keys for a text: "a" or "a and b".
fn key_names(keys: &[&str]) -> String {
    keys.join(" and ")
}

/// The note after the update step removed `keys`.
pub fn map_keys_removed(keys: &[&str]) -> String {
    format!(
        "The program reads the commands from the EDCoPilot profile now. \
         The update removed {} from the configuration.",
        key_names(keys)
    )
}

/// The note at each start while the configuration has `keys` with a value
/// of the user. The program does not use them.
pub fn old_keys_kept(keys: &[&str]) -> String {
    let (verb, lines) = if keys.len() == 1 {
        ("is", "the line")
    } else {
        ("are", "the lines")
    };
    format!(
        "{} {verb} not used any more. The program reads the commands from the EDCoPilot \
         profile. You can remove {lines} from the configuration.",
        key_names(keys)
    )
}

/// The note when the update step cannot write the configuration, for
/// example a file that is read-only or that another program holds. It is a
/// note: the program does not use the keys, so the start is possible.
pub fn map_keys_not_removed(keys: &[&str], file: &Path, cause: &str) -> String {
    let lines = if keys.len() == 1 {
        "the line"
    } else {
        "the two lines"
    };
    format!(
        "The update could not remove {} from {}: {cause}. Make the file writable, or remove \
         {lines} by hand.",
        key_names(keys),
        file.display()
    )
}

/// What the program uses at the start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartupPlan {
    /// The configuration file. Empty when `problem` is set.
    pub config_path: PathBuf,
    /// The data folder for the models. `None` when the program must not use
    /// a data folder.
    pub data_dir: Option<PathBuf>,
    /// The data folder for `first_start_with` and the update step. Only a
    /// start without `--config` has it.
    pub first_start_dir: Option<PathBuf>,
    /// The problem that stops the use of a configuration.
    pub problem: Option<String>,
}

/// The rules of the start:
///
/// - `--config <file>` wins. Then the program makes nothing in a data
///   folder. The models go to the data folder of the user (a release
///   build), or to the folder of `VOICEBRIDGE_DEV_DATA_DIR`.
/// - A release build without `--config` uses `%APPDATA%\VoiceBridge`. When
///   `%APPDATA%` is unknown, the problem `NO_APPDATA` stops the use of a
///   configuration. The program never uses the current folder.
/// - A development build without `--config` never uses the data folder of
///   the user. It uses the absolute folder of `VOICEBRIDGE_DEV_DATA_DIR`,
///   else the problem `DEV_BUILD` stops the use of a configuration.
pub fn startup_plan(
    args: &[String],
    appdata: Option<OsString>,
    dev_data_dir: Option<OsString>,
    debug: bool,
) -> StartupPlan {
    let dev_dir = dev_data_dir
        .map(PathBuf::from)
        .filter(|p| debug && p.is_absolute());
    let appdata_dir = data_dir_in(appdata);
    let user_dir = if debug { None } else { appdata_dir.clone() };
    let data_dir = dev_dir.clone().or(user_dir);
    if let Some(path) = config_argument(args) {
        // A development build with --config keeps the models folder of
        // the user, as before this rule. The download is an action of the
        // user.
        let models = if debug && dev_dir.is_none() {
            appdata_dir
        } else {
            data_dir
        };
        return StartupPlan {
            config_path: path,
            data_dir: models,
            first_start_dir: None,
            problem: None,
        };
    }
    match data_dir {
        Some(dir) => StartupPlan {
            config_path: config_path(args, &dir),
            data_dir: Some(dir.clone()),
            first_start_dir: Some(dir),
            problem: None,
        },
        None => StartupPlan {
            config_path: PathBuf::new(),
            data_dir: None,
            first_start_dir: None,
            problem: Some(if debug { DEV_BUILD } else { NO_APPDATA }.to_string()),
        },
    }
}

/// The texts of the start that the Status and Diagnostics screens show.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StartupNotes {
    /// The problem that stops the use of a configuration.
    pub problem: Option<String>,
    /// The error of a failed first start, with the data folder.
    pub first_start_error: Option<String>,
    /// The notes of the update step, for example `MAP_KEYS_REMOVED`.
    pub notes: Vec<String>,
    /// The errors of the update step. The configuration did not change.
    pub update_problems: Vec<String>,
}

/// The first start and the update step, for a start without `--config`.
/// The errors become notes: the window shows them.
pub fn prepare_data(data_dir: &Path, resource_dir: &Path) -> StartupNotes {
    prepare_data_with(data_dir, resource_dir, usual_edcopilot_dir())
}

/// `C:\EDCoPilot` when that folder exists. Only a read of the folder state:
/// the program never writes there.
fn usual_edcopilot_dir() -> Option<&'static Path> {
    let usual = Path::new(DEFAULT_EDCOPILOT_DIR);
    usual.is_dir().then_some(usual)
}

/// `prepare_data` with the EDCoPilot folder of the first start as an
/// argument. The tests give a temporary folder or none.
pub fn prepare_data_with(
    data_dir: &Path,
    resource_dir: &Path,
    edcopilot_dir: Option<&Path>,
) -> StartupNotes {
    let mut notes = StartupNotes::default();
    if let Err(error) = first_start_with(data_dir, resource_dir, edcopilot_dir) {
        notes.first_start_error = Some(format!(
            "the program could not make its data folder {}: {error:#}",
            data_dir.display()
        ));
        return notes;
    }
    match remove_old_map_keys(&data_dir.join(CONFIG_FILE)) {
        Ok(update) => notes.notes = update.notes,
        // The file does not read or does not parse: the step changed
        // nothing.
        Err(error) => notes.update_problems.push(format!("{error:#}")),
    }
    notes
}

/// The result of `remove_old_map_keys`. A write that fails is a note
/// (`map_keys_not_removed`). An error of the read is the `Err` of the
/// function.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Update {
    pub notes: Vec<String>,
}

/// The update step of a configuration of an older version. That version
/// wrote `data.command_map` and `data.profile_vars` with the absolute paths
/// of its map files: `<program folder>\resources\data\<shipped name>`.
/// The program folder can have moved since then. The program does not use
/// the two keys any more.
///
/// The step decides for each key. A key that is empty or has such a path
/// goes. A key with another value stays: it is a line of the user, and the
/// note of `old_keys_kept` says that the program does not use it. The
/// comment line `OLD_MAP_COMMENT` directly above a removed key goes too,
/// when no map key stays. The old line also described `data.aliases_dir`:
/// when that key directly follows the removed lines, it gets the line
/// `ALIAS_COMMENT` of the current example in place of the old line. The
/// other keys, the comments, and the bytes of the file stay (also the line
/// ends). A second run changes nothing. When the file cannot be written,
/// the note of `map_keys_not_removed` says so.
pub fn remove_old_map_keys(config_path: &Path) -> anyhow::Result<Update> {
    let mut update = Update::default();
    if !config_path.is_file() {
        return Ok(update);
    }
    let (config, text) = voicebridge::config::Config::load_as_written_text(config_path)?;
    let keys = [
        (
            "data.command_map",
            &config.data.old_command_map,
            COMMAND_MAP_FILE,
        ),
        (
            "data.profile_vars",
            &config.data.old_profile_vars,
            PROFILE_VARS_FILE,
        ),
    ];
    let mut remove: Vec<&str> = Vec::new();
    let mut keep: Vec<&str> = Vec::new();
    for (key, value, name) in keys {
        let Some(value) = value else { continue };
        let old = value.as_str().is_some_and(|path| {
            path.trim().is_empty() || is_first_start_path(Path::new(path), name)
        });
        if old {
            remove.push(key);
        } else {
            keep.push(key);
        }
    }
    if !keep.is_empty() {
        update.notes.push(old_keys_kept(&keep));
    }
    if remove.is_empty() {
        return Ok(update);
    }
    let comment = keep.is_empty().then_some(config_edit::OldComment {
        line: OLD_MAP_COMMENT,
        next: Some(("aliases_dir", ALIAS_COMMENT)),
    });
    let new_text = config_edit::remove_keys(&text, &remove, comment)?;
    // The removal of a key never gives a file that does not load. The
    // check is a guard for the write only.
    config_edit::parse_config(config_path, &new_text).map_err(|e| anyhow::anyhow!(e))?;
    match config_edit::write_atomic(config_path, &new_text) {
        Ok(()) => update.notes.push(map_keys_removed(&remove)),
        Err(error) => update.notes.push(map_keys_not_removed(
            &remove,
            config_path,
            &format!("{error:#}"),
        )),
    }
    Ok(update)
}

/// True when the path has the form that an older `first_start` wrote for
/// its map files: an absolute `<folder>/resources/data/<name>`, with the
/// folder names compared without case.
fn is_first_start_path(path: &Path, name: &str) -> bool {
    let part = |p: Option<&Path>| {
        p.and_then(Path::file_name)
            .map(|n| n.to_string_lossy().to_ascii_lowercase())
    };
    let data = path.parent();
    let resources = data.and_then(Path::parent);
    path.is_absolute()
        && path.file_name().is_some_and(|n| n == name)
        && part(data).as_deref() == Some("data")
        && part(resources).as_deref() == Some(RESOURCE_FOLDER)
}

/// Removes the temporary files of a first start that stopped during a
/// write. Only names with `TEMP_PREFIX` and `TEMP_SUFFIX`, only in this
/// folder.
fn remove_leftovers(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.filter_map(Result::ok) {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let ours = name.starts_with(TEMP_PREFIX) && name.ends_with(TEMP_SUFFIX);
        if ours && entry.file_type().is_ok_and(|t| t.is_file()) {
            let _ = fs::remove_file(entry.path());
        }
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use voicebridge::config::Config;
    use voicebridge::run::{check_config_in, CheckReport};

    /// `check_config` with an empty temporary folder of the saved report:
    /// a test never reads the report folder of the user.
    fn check_config(config: &Config, env: impl Fn(&str) -> Option<String>) -> CheckReport {
        let dir = tempfile::tempdir().unwrap();
        check_config_in(config, env, Some(dir.path()))
    }

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    /// The `bundle.resources` of `tauri.conf.json`: source path, path in
    /// the program folder.
    fn bundle_resources() -> Vec<(String, String)> {
        let conf = Path::new(env!("CARGO_MANIFEST_DIR")).join("tauri.conf.json");
        let conf: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(conf).unwrap()).unwrap();
        conf["bundle"]["resources"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(from, to)| (from.clone(), to.as_str().unwrap().to_string()))
            .collect()
    }

    /// A resource folder with the files that the bundle ships, from the
    /// source tree. The layout is the layout of `bundle.resources` in
    /// `tauri.conf.json`.
    pub fn resource_folder() -> tempfile::TempDir {
        let ui = Path::new(env!("CARGO_MANIFEST_DIR"));
        let dir = tempfile::tempdir().unwrap();
        for (from, to) in bundle_resources() {
            let (from, to) = (ui.join(from), dir.path().join(to));
            fs::create_dir_all(to.parent().unwrap()).unwrap();
            if from.is_dir() {
                fs::create_dir_all(&to).unwrap();
                for entry in fs::read_dir(&from).unwrap() {
                    let path = entry.unwrap().path();
                    fs::copy(&path, to.join(path.file_name().unwrap())).unwrap();
                }
            } else {
                fs::copy(&from, &to).unwrap();
            }
        }
        dir
    }

    #[test]
    fn test_the_bundle_ships_the_files_that_the_first_start_reads() {
        let conf = Path::new(env!("CARGO_MANIFEST_DIR")).join("tauri.conf.json");
        let conf: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(conf).unwrap()).unwrap();
        let resources = &conf["bundle"]["resources"];
        assert_eq!(
            resources,
            &serde_json::json!({
                "../app/voicebridge.example.toml": "resources/voicebridge.example.toml",
                "../core/data/synonyms.toml": "resources/data/synonyms.toml",
                "../core/data/sound_alikes.toml": "resources/data/sound_alikes.toml",
                "../core/data/fillers.toml": "resources/data/fillers.toml",
                "../core/data/hotwords.txt": "resources/data/hotwords.txt",
                "../core/aliases": "resources/aliases",
            })
        );
        assert_eq!(
            conf["bundle"]["windows"]["nsis"]["installMode"],
            "currentUser"
        );
        // The workspace links the static C runtime itself, so the Tauri
        // setting is off.
        assert_eq!(conf["build"]["windows"]["staticVCRuntime"], false);
        // Each word table that the first start copies is in the bundle.
        let res = resource_folder();
        for name in TABLE_FILES {
            assert!(
                res.path().join("resources/data").join(name).is_file(),
                "{name}"
            );
        }
    }

    /// The bundle ships the word tables and the example configuration. It
    /// ships no command map and no profile variables: the program reads the
    /// EDCoPilot profile of the user.
    #[test]
    fn test_the_bundle_ships_no_command_map() {
        let ui = Path::new(env!("CARGO_MANIFEST_DIR"));
        for (from, to) in bundle_resources() {
            let path = ui.join(&from);
            let files: Vec<PathBuf> = if path.is_dir() {
                fs::read_dir(&path)
                    .unwrap()
                    .map(|e| e.unwrap().path())
                    .collect()
            } else {
                vec![path]
            };
            for file in files {
                let name = file.file_name().unwrap().to_string_lossy().to_string();
                assert!(
                    name != COMMAND_MAP_FILE && name != PROFILE_VARS_FILE,
                    "{from} -> {to} ships {name}"
                );
                assert!(!name.ends_with(".json"), "{from} -> {to} ships {name}");
            }
        }
        let res = resource_folder();
        let data = res.path().join("resources/data");
        assert!(!data.join(COMMAND_MAP_FILE).exists());
        assert!(!data.join(PROFILE_VARS_FILE).exists());
    }

    #[test]
    fn test_config_argument_wins() {
        let data = Path::new("C:\\Users\\x\\AppData\\Roaming\\VoiceBridge");
        assert_eq!(
            config_path(&args(&["ui.exe", "--config", "D:\\vb\\my.toml"]), data),
            PathBuf::from("D:\\vb\\my.toml")
        );
        assert_eq!(
            config_path(&args(&["ui.exe", "--config=my.toml"]), data),
            PathBuf::from("my.toml")
        );
        assert_eq!(
            config_path(&args(&["ui.exe"]), data),
            data.join("voicebridge.toml")
        );
        // `--config` without a value is not a path.
        assert_eq!(
            config_path(&args(&["ui.exe", "--config"]), data),
            data.join("voicebridge.toml")
        );
    }

    #[test]
    fn test_data_dir_needs_an_absolute_appdata() {
        assert_eq!(
            data_dir_in(Some("C:\\Users\\x\\AppData\\Roaming".into())),
            Some(PathBuf::from("C:\\Users\\x\\AppData\\Roaming\\VoiceBridge"))
        );
        assert_eq!(data_dir_in(Some("relative".into())), None);
        assert_eq!(data_dir_in(None), None);
        assert_eq!(
            models_dir(Path::new("D:\\d")),
            PathBuf::from("D:\\d\\models")
        );
    }

    #[test]
    fn test_first_start_makes_the_files_and_reports_true() {
        let res = resource_folder();
        let home = tempfile::tempdir().unwrap();
        let data = home.path().join(DATA_FOLDER);
        assert!(first_start_with(&data, res.path(), None).unwrap());
        for name in TABLE_FILES {
            assert_eq!(
                fs::read(data.join(name)).unwrap(),
                fs::read(res.path().join("resources/data").join(name)).unwrap(),
                "{name}"
            );
        }
        assert!(data.join("aliases/de.toml").is_file());
        // No map file anywhere: the program reads the EDCoPilot profile.
        assert!(!data.join(COMMAND_MAP_FILE).exists());
        assert!(!data.join(PROFILE_VARS_FILE).exists());

        let config = Config::load(&data.join(CONFIG_FILE)).unwrap();
        assert!(!config.has_old_map_keys());
        assert_eq!(config.data.aliases_dir, data.join("aliases"));
        assert_eq!(config.data.synonyms, data.join("synonyms.toml"));
        assert_eq!(config.data.sound_alikes, data.join("sound_alikes.toml"));
        assert_eq!(config.data.fillers, data.join("fillers.toml"));
        // The file has absolute paths, not paths relative to its folder.
        let written = Config::load_as_written(&data.join(CONFIG_FILE)).unwrap();
        assert!(written.data.synonyms.is_absolute());
        // The hotwords, the utterance log, and the audio capture are off.
        assert!(config.stt.parakeet.hotwords_file.as_os_str().is_empty());
        assert!(config.diagnostics.log_file.as_os_str().is_empty());
        assert!(config.diagnostics.save_audio_dir.as_os_str().is_empty());
        assert!(config.edcopilot.dir.as_os_str().is_empty());
        // The comments of the example stay.
        let text = fs::read_to_string(data.join(CONFIG_FILE)).unwrap();
        assert!(text.contains("The files hold your voice and stay on this computer."));
        assert!(text.starts_with("# VoiceBridge configuration."));
    }

    #[test]
    fn test_plain_path_removes_the_verbatim_prefix() {
        assert_eq!(
            plain_path(Path::new("\\\\?\\D:\\Program\\VoiceBridge")),
            PathBuf::from("D:\\Program\\VoiceBridge")
        );
        assert_eq!(
            plain_path(Path::new("\\\\?\\UNC\\server\\share\\vb")),
            PathBuf::from("\\\\server\\share\\vb")
        );
        assert_eq!(
            plain_path(Path::new("D:\\Program\\VoiceBridge")),
            PathBuf::from("D:\\Program\\VoiceBridge")
        );
    }

    #[test]
    fn test_plain_path_keeps_a_verbatim_path_without_a_drive_letter() {
        for path in [
            "\\\\?\\Volume{0b6d1a5e-1111-2222-3333-444455556666}\\vb",
            "\\\\?\\GLOBALROOT\\Device\\HarddiskVolume3\\vb",
            "\\\\?\\1:\\x",
        ] {
            assert_eq!(plain_path(Path::new(path)), PathBuf::from(path), "{path}");
        }
    }

    #[test]
    #[cfg(windows)]
    fn test_the_move_fallback_places_a_file_and_never_replaces_one() {
        // The fallback for a volume without hard links.
        let dir = tempfile::tempdir().unwrap();
        let temp = dir
            .path()
            .join(format!("{TEMP_PREFIX}1-a.toml{TEMP_SUFFIX}"));
        let target = dir.path().join("a.toml");
        fs::write(&temp, "shipped").unwrap();
        assert!(move_new(&temp, &target).unwrap());
        assert_eq!(fs::read_to_string(&target).unwrap(), "shipped");
        assert!(!temp.exists());
        // A file that exists stays, and the temporary file goes.
        fs::write(&temp, "newer").unwrap();
        assert!(!move_new(&temp, &target).unwrap());
        assert_eq!(fs::read_to_string(&target).unwrap(), "shipped");
        assert!(!temp.exists());
    }

    #[test]
    fn test_an_existing_file_is_never_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("synonyms.toml");
        fs::write(&file, "mine").unwrap();
        assert!(!write_new(&file, b"shipped").unwrap());
        assert_eq!(fs::read_to_string(&file).unwrap(), "mine");
        let new = dir.path().join("fillers.toml");
        assert!(write_new(&new, b"shipped").unwrap());
        assert_eq!(fs::read_to_string(&new).unwrap(), "shipped");
        // No temporary file stays.
        let names: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names.len(), 2, "{names:?}");
    }

    #[test]
    fn test_a_leftover_temporary_file_is_removed_and_never_read() {
        let res = resource_folder();
        let home = tempfile::tempdir().unwrap();
        let data = home.path().join(DATA_FOLDER);
        first_start_with(&data, res.path(), None).unwrap();
        // A start that was killed during a write leaves a part.
        let part_config = data.join(format!("{TEMP_PREFIX}1-voicebridge.toml{TEMP_SUFFIX}"));
        let part_alias = data
            .join(ALIASES_FOLDER)
            .join(format!("{TEMP_PREFIX}1-fr.toml{TEMP_SUFFIX}"));
        fs::write(&part_config, "[edcopilot").unwrap();
        fs::write(&part_alias, "'not toml").unwrap();
        // The alias loader does not read the part.
        let config = Config::load(&data.join(CONFIG_FILE)).unwrap();
        let found = check_config(&config, |_| None);
        assert!(
            !found.problems.iter().any(|p| p.contains(TEMP_PREFIX)),
            "{:?}",
            found.problems
        );
        // A file of the user with another name stays.
        let mine = data.join("vb-notes.txt");
        fs::write(&mine, "mine").unwrap();
        first_start_with(&data, res.path(), None).unwrap();
        assert!(!part_config.exists());
        assert!(!part_alias.exists());
        assert!(mine.exists());
    }

    /// The text of a configuration that an older first start wrote: the
    /// map keys with the comment line of the old example above them.
    fn old_install_text(program: &Path, data: &Path) -> String {
        let shipped = program.join(RESOURCE_FOLDER).join("data");
        format!(
            "# VoiceBridge configuration.\n\n[edcopilot]\n# The folder.\ndir = \"\"\n\n\
             [data]\n{OLD_MAP_COMMENT}\n\
             command_map = '{}'\nprofile_vars = '{}'\naliases_dir = '{}'\n\
             # The synonyms.\nsynonyms = '{}'\n\n[matching]\nthreshold = 0.85 # mine\n",
            shipped.join(COMMAND_MAP_FILE).display(),
            shipped.join(PROFILE_VARS_FILE).display(),
            data.join(ALIASES_FOLDER).display(),
            data.join("synonyms.toml").display(),
        )
    }

    /// The text without the lines that start with one of `starts`.
    fn without_lines(text: &str, starts: &[&str]) -> String {
        text.split_inclusive('\n')
            .filter(|l| !starts.iter().any(|s| l.starts_with(s)))
            .collect()
    }

    /// The line of a key with a path as an older first start wrote it.
    fn key_line(key: &str, path: &Path) -> String {
        format!("{key} = '{}'\n", path.display())
    }

    /// The update gives the alias folder the comment line of the current
    /// example: the example must have it directly above the key.
    #[test]
    fn test_the_example_has_the_alias_comment_above_the_alias_folder() {
        let example = include_str!("../../app/voicebridge.example.toml");
        assert!(
            example.contains(&format!("{ALIAS_COMMENT}\naliases_dir = "))
                || example.contains(&format!("{ALIAS_COMMENT}\r\naliases_dir = ")),
            "the example has no line {ALIAS_COMMENT:?} directly above aliases_dir"
        );
    }

    #[test]
    fn test_the_notes_name_the_keys() {
        let both = ["data.command_map", "data.profile_vars"];
        assert_eq!(map_keys_removed(&both), MAP_KEYS_REMOVED);
        assert_eq!(
            map_keys_removed(&["data.profile_vars"]),
            "The program reads the commands from the EDCoPilot profile now. The update removed \
             data.profile_vars from the configuration."
        );
        assert_eq!(
            old_keys_kept(&["data.command_map"]),
            "data.command_map is not used any more. The program reads the commands from the \
             EDCoPilot profile. You can remove the line from the configuration."
        );
        assert_eq!(
            old_keys_kept(&both),
            "data.command_map and data.profile_vars are not used any more. The program reads \
             the commands from the EDCoPilot profile. You can remove the lines from the \
             configuration."
        );
        assert_eq!(
            map_keys_not_removed(
                &both,
                Path::new("D:\\vb\\voicebridge.toml"),
                "access denied"
            ),
            "The update could not remove data.command_map and data.profile_vars from \
             D:\\vb\\voicebridge.toml: access denied. Make the file writable, or remove the two \
             lines by hand."
        );
    }

    #[test]
    fn test_update_removes_the_map_keys_that_an_older_first_start_wrote() {
        let home = tempfile::tempdir().unwrap();
        let data = home.path().join(DATA_FOLDER);
        fs::create_dir_all(&data).unwrap();
        let config = data.join(CONFIG_FILE);
        // The program folder of the old version, also after a move: the
        // files there are gone or still there. Both count.
        for program in [home.path().join("Program"), home.path().join("Moved")] {
            let text = old_install_text(&program, &data);
            fs::write(&config, &text).unwrap();
            let update = remove_old_map_keys(&config).unwrap();
            assert_eq!(update.notes, [MAP_KEYS_REMOVED], "{update:?}");
            let after = fs::read_to_string(&config).unwrap();
            // Exactly the two key lines go. The old comment line above them
            // becomes the comment line of the alias folder, which directly
            // follows. The other lines, the comments and the comment at the
            // end of a line stay as they were.
            let expected = without_lines(&text, &["command_map", "profile_vars"])
                .replace(OLD_MAP_COMMENT, ALIAS_COMMENT);
            assert_eq!(after, expected);
            let loaded = Config::load_as_written(&config).unwrap();
            assert!(!loaded.has_old_map_keys());
            // A second start changes nothing and gives no note.
            let again = remove_old_map_keys(&config).unwrap();
            assert_eq!(again, Update::default());
            assert_eq!(fs::read_to_string(&config).unwrap(), after);
        }
    }

    /// The real form of an installed configuration: CRLF line ends, literal
    /// strings in single quotes, no byte order mark, and an absolute path
    /// below the local application data folder. The file after the step
    /// has the same bytes, without the removed lines.
    #[test]
    fn test_update_of_the_installed_form_keeps_every_other_byte() {
        let home = tempfile::tempdir().unwrap();
        let config = home.path().join(CONFIG_FILE);
        let program = PathBuf::from("C:\\Users\\pilot\\AppData\\Local\\VoiceBridge");
        let data = PathBuf::from("C:\\Users\\pilot\\AppData\\Roaming\\VoiceBridge");
        let text = old_install_text(&program, &data).replace('\n', "\r\n");
        assert!(text.contains(
            "command_map = 'C:\\Users\\pilot\\AppData\\Local\\VoiceBridge\\resources\\data\\command_map_v3.json'\r\n"
        ));
        fs::write(&config, text.as_bytes()).unwrap();
        let update = remove_old_map_keys(&config).unwrap();
        assert_eq!(update.notes, [MAP_KEYS_REMOVED]);
        let after = fs::read(&config).unwrap();
        let expected = without_lines(&text, &["command_map", "profile_vars"])
            .replace(OLD_MAP_COMMENT, ALIAS_COMMENT);
        assert_eq!(after, expected.as_bytes());
        // Only these bytes changed: the two key lines with their CRLF, and
        // the old comment line, which is now the alias comment line.
        let removed = text.len() - expected.len();
        let keys: usize = text
            .split_inclusive("\r\n")
            .filter(|l| l.starts_with("command_map") || l.starts_with("profile_vars"))
            .map(str::len)
            .sum();
        assert_eq!(removed, keys + OLD_MAP_COMMENT.len() - ALIAS_COMMENT.len());
        assert!(!after.starts_with(&[0xEF, 0xBB, 0xBF]));
        assert!(String::from_utf8(after)
            .unwrap()
            .contains(&format!("[data]\r\n{ALIAS_COMMENT}\r\naliases_dir = ")));
    }

    /// Each key on its own: an old key goes, a key of the user stays with a
    /// note. The old comment stays while a map key stays.
    #[test]
    fn test_update_decides_for_each_key() {
        let home = tempfile::tempdir().unwrap();
        let data = home.path().join(DATA_FOLDER);
        fs::create_dir_all(&data).unwrap();
        let config = data.join(CONFIG_FILE);
        let shipped = home
            .path()
            .join("Program")
            .join(RESOURCE_FOLDER)
            .join("data");
        let old_map = key_line("command_map", &shipped.join(COMMAND_MAP_FILE));
        let old_vars = key_line("profile_vars", &shipped.join(PROFILE_VARS_FILE));
        let mine_map = key_line(
            "command_map",
            &home.path().join("mine").join(COMMAND_MAP_FILE),
        );
        let mine_vars = key_line("profile_vars", &shipped.join("my_vars.json"));
        let rest = format!(
            "aliases_dir = '{}'\n\n[matching]\nthreshold = 0.85\n",
            data.join(ALIASES_FOLDER).display()
        );
        let file = |keys: &str| {
            format!("[edcopilot]\ndir = \"\"\n\n[data]\n{OLD_MAP_COMMENT}\n{keys}{rest}")
        };
        // (keys of the file, the lines that go, the notes). When the old
        // comment line goes and the alias folder directly follows, the
        // line becomes `ALIAS_COMMENT`: `OLD_MAP_COMMENT` is then in the
        // list of the lines that go.
        let cases: Vec<(String, Vec<&str>, Vec<String>)> = vec![
            // The map is old, the variables are of the user.
            (
                format!("{old_map}{mine_vars}"),
                vec!["command_map"],
                vec![
                    old_keys_kept(&["data.profile_vars"]),
                    map_keys_removed(&["data.command_map"]),
                ],
            ),
            // The map is of the user, the variables are empty.
            (
                format!("{mine_map}profile_vars = ''\n"),
                vec!["profile_vars"],
                vec![
                    old_keys_kept(&["data.command_map"]),
                    map_keys_removed(&["data.profile_vars"]),
                ],
            ),
            // Only one key, old: it goes, and the old comment becomes the
            // comment of the alias folder.
            (
                old_vars.clone(),
                vec!["profile_vars", OLD_MAP_COMMENT],
                vec![map_keys_removed(&["data.profile_vars"])],
            ),
            // A value of only spaces counts as empty.
            (
                "command_map = '   '\nprofile_vars = ' '\n".to_string(),
                vec!["command_map", "profile_vars", OLD_MAP_COMMENT],
                vec![MAP_KEYS_REMOVED.to_string()],
            ),
            // Only one key, of the user: nothing changes.
            (
                mine_map.clone(),
                vec![],
                vec![old_keys_kept(&["data.command_map"])],
            ),
            // A relative path is not the form of a first start.
            (
                "command_map = 'resources/data/command_map_v3.json'\n".to_string(),
                vec![],
                vec![old_keys_kept(&["data.command_map"])],
            ),
            // A value that is not a text is of the user.
            (
                "command_map = 7\nprofile_vars = ''\n".to_string(),
                vec!["profile_vars"],
                vec![
                    old_keys_kept(&["data.command_map"]),
                    map_keys_removed(&["data.profile_vars"]),
                ],
            ),
        ];
        for (keys, gone, notes) in cases {
            let text = file(&keys);
            fs::write(&config, &text).unwrap();
            let update = remove_old_map_keys(&config).unwrap();
            assert_eq!(update.notes, notes, "{keys}");
            let after = fs::read_to_string(&config).unwrap();
            let expected = if gone.contains(&OLD_MAP_COMMENT) {
                let keys: Vec<&str> = gone
                    .iter()
                    .copied()
                    .filter(|l| *l != OLD_MAP_COMMENT)
                    .collect();
                without_lines(&text, &keys).replace(OLD_MAP_COMMENT, ALIAS_COMMENT)
            } else {
                without_lines(&text, &gone)
            };
            assert_eq!(after, expected, "{keys}");
            // A second start removes nothing more.
            let again = remove_old_map_keys(&config).unwrap();
            assert!(
                again
                    .notes
                    .iter()
                    .all(|n| !n.starts_with("The program reads")),
                "{keys}: {again:?}"
            );
            assert_eq!(fs::read_to_string(&config).unwrap(), after, "{keys}");
        }
    }

    /// A comment line above a removed key that is not the old comment of
    /// the example stays: it can be a line of the user.
    #[test]
    fn test_update_keeps_a_comment_of_the_user_above_a_removed_key() {
        let home = tempfile::tempdir().unwrap();
        let config = home.path().join(CONFIG_FILE);
        let shipped = home.path().join("P").join(RESOURCE_FOLDER).join("data");
        let text = format!(
            "[edcopilot]\ndir = \"\"\n\n[data]\n# My own note.\n{}{}aliases_dir = 'a'\n",
            key_line("command_map", &shipped.join(COMMAND_MAP_FILE)),
            key_line("profile_vars", &shipped.join(PROFILE_VARS_FILE)),
        );
        fs::write(&config, &text).unwrap();
        let update = remove_old_map_keys(&config).unwrap();
        assert_eq!(update.notes, [MAP_KEYS_REMOVED]);
        assert_eq!(
            fs::read_to_string(&config).unwrap(),
            "[edcopilot]\ndir = \"\"\n\n[data]\n# My own note.\naliases_dir = 'a'\n"
        );
    }

    /// A configuration that the step cannot write: a note with the cause,
    /// not a problem. The file stays as it was. Only on Windows: the
    /// read-only flag stops the write there. On other systems the owner or
    /// a root user can still write the file.
    #[cfg(windows)]
    #[test]
    fn test_a_failed_update_is_a_note_with_the_cause() {
        let home = tempfile::tempdir().unwrap();
        let data = home.path().join(DATA_FOLDER);
        fs::create_dir_all(&data).unwrap();
        let config = data.join(CONFIG_FILE);
        let text = old_install_text(&home.path().join("P"), &data);
        fs::write(&config, &text).unwrap();
        let original = fs::metadata(&config).unwrap().permissions();
        let mut read_only = original.clone();
        read_only.set_readonly(true);
        fs::set_permissions(&config, read_only).unwrap();
        let update = remove_old_map_keys(&config);
        fs::set_permissions(&config, original).unwrap();
        let update = update.unwrap();
        assert_eq!(update.notes.len(), 1, "{update:?}");
        let start = format!(
            "The update could not remove data.command_map and data.profile_vars from {}: ",
            config.display()
        );
        assert!(update.notes[0].starts_with(&start), "{update:?}");
        assert!(
            update.notes[0].ends_with(". Make the file writable, or remove the two lines by hand."),
            "{update:?}"
        );
        assert_eq!(fs::read_to_string(&config).unwrap(), text);
        // The keys are not used: the configuration still checks as valid
        // for the commands, with the warning of the old keys.
        let loaded = Config::load(&config).unwrap();
        assert!(loaded.has_old_map_keys());
    }

    #[test]
    fn test_update_of_a_new_configuration_changes_nothing() {
        let res = resource_folder();
        let home = tempfile::tempdir().unwrap();
        let data = home.path().join(DATA_FOLDER);
        let notes = prepare_data_with(&data, res.path(), None);
        assert_eq!(notes, StartupNotes::default());
        let config = data.join(CONFIG_FILE);
        let text = fs::read_to_string(&config).unwrap();
        assert_eq!(remove_old_map_keys(&config).unwrap(), Update::default());
        assert_eq!(fs::read_to_string(&config).unwrap(), text);
        // An absent file is not an error.
        assert_eq!(
            remove_old_map_keys(&home.path().join("none.toml")).unwrap(),
            Update::default()
        );
    }

    #[test]
    fn test_prepare_data_gives_the_note_of_the_update() {
        let res = resource_folder();
        let home = tempfile::tempdir().unwrap();
        let data = home.path().join(DATA_FOLDER);
        fs::create_dir_all(&data).unwrap();
        let config = data.join(CONFIG_FILE);
        fs::write(&config, old_install_text(&home.path().join("P"), &data)).unwrap();
        let notes = prepare_data_with(&data, res.path(), None);
        assert_eq!(notes.notes, [MAP_KEYS_REMOVED]);
        assert!(notes.update_problems.is_empty());
        // A file that does not load: a problem, and the file stays.
        fs::write(&config, "[edcopilot\n").unwrap();
        let notes = prepare_data_with(&data, res.path(), None);
        assert!(notes.notes.is_empty());
        assert_eq!(notes.update_problems.len(), 1, "{notes:?}");
        assert_eq!(fs::read_to_string(&config).unwrap(), "[edcopilot\n");
    }

    #[test]
    fn test_an_update_problem_never_shows_the_line_or_a_value() {
        const SECRET: &str = "sk-test-1234567890abcdef";
        let res = resource_folder();
        let home = tempfile::tempdir().unwrap();
        let data = home.path().join(DATA_FOLDER);
        fs::create_dir_all(&data).unwrap();
        let config = data.join(CONFIG_FILE);
        let head = "[edcopilot]\ndir = 'C:/E'\n";
        for body in [
            format!("[stt.openai_compatible]\napi_key_env = {SECRET}\n"),
            format!("[stt.parakeet]\nthreads = \"{SECRET}\"\n"),
            format!("[stt.openai_compatible]\napi_key = \"{SECRET}\"\n"),
        ] {
            fs::write(&config, format!("{head}{body}")).unwrap();
            let notes = prepare_data_with(&data, res.path(), None);
            assert_eq!(notes.update_problems.len(), 1, "{notes:?}");
            let problem = &notes.update_problems[0];
            assert!(!problem.contains(SECRET), "{problem}");
            assert!(problem.contains("line 4, column "), "{problem}");
        }
    }

    fn os(text: &str) -> Option<OsString> {
        Some(OsString::from(text))
    }

    #[test]
    fn test_startup_rules_for_the_configuration_path() {
        let appdata = os("C:\\Users\\x\\AppData\\Roaming");
        let data = PathBuf::from("C:\\Users\\x\\AppData\\Roaming\\VoiceBridge");
        // A release build without --config: the data folder.
        let plan = startup_plan(&args(&["ui.exe"]), appdata.clone(), None, false);
        assert_eq!(plan.config_path, data.join(CONFIG_FILE));
        assert_eq!(plan.first_start_dir.as_deref(), Some(data.as_path()));
        assert_eq!(plan.data_dir.as_deref(), Some(data.as_path()));
        assert_eq!(plan.problem, None);
        // --config wins, also in a debug build, and makes nothing.
        for debug in [false, true] {
            let given = args(&["ui.exe", "--config", "D:\\vb.toml"]);
            let plan = startup_plan(&given, appdata.clone(), None, debug);
            assert_eq!(plan.config_path, PathBuf::from("D:\\vb.toml"));
            assert_eq!(plan.first_start_dir, None);
            assert_eq!(plan.problem, None);
        }
        // No APPDATA: no configuration in the current folder.
        for bad in [None, os("relative")] {
            let plan = startup_plan(&args(&["ui.exe"]), bad, None, false);
            assert_eq!(plan.problem.as_deref(), Some(NO_APPDATA));
            assert_eq!(plan.first_start_dir, None);
            assert_eq!(plan.config_path, PathBuf::new());
        }
    }

    #[test]
    fn test_a_debug_build_never_uses_the_data_folder_of_the_user() {
        let appdata = os("C:\\Users\\x\\AppData\\Roaming");
        let plan = startup_plan(&args(&["ui.exe"]), appdata.clone(), None, true);
        assert_eq!(plan.problem.as_deref(), Some(DEV_BUILD));
        assert_eq!(plan.first_start_dir, None);
        assert_eq!(plan.data_dir, None);
        assert_eq!(plan.config_path, PathBuf::new());
        // The opt-in for live checks: a scratch data folder.
        let scratch = os("D:\\scratch\\vb");
        let plan = startup_plan(&args(&["ui.exe"]), appdata.clone(), scratch, true);
        let dir = PathBuf::from("D:\\scratch\\vb");
        assert_eq!(plan.config_path, dir.join(CONFIG_FILE));
        assert_eq!(plan.first_start_dir.as_deref(), Some(dir.as_path()));
        assert_eq!(plan.data_dir.as_deref(), Some(dir.as_path()));
        assert_eq!(plan.problem, None);
        // A relative opt-in folder is refused.
        let plan = startup_plan(&args(&["ui.exe"]), appdata, os("scratch"), true);
        assert_eq!(plan.problem.as_deref(), Some(DEV_BUILD));
        // A release build ignores the opt-in.
        let plan = startup_plan(&args(&["ui.exe"]), os("C:\\A"), os("D:\\scratch"), false);
        assert_eq!(
            plan.config_path,
            PathBuf::from("C:\\A\\VoiceBridge").join(CONFIG_FILE)
        );
    }

    #[test]
    fn test_prepare_data_keeps_the_error_of_a_failed_first_start_with_the_folder() {
        let res = resource_folder();
        fs::remove_file(res.path().join("resources").join(EXAMPLE_FILE)).unwrap();
        let home = tempfile::tempdir().unwrap();
        let data = home.path().join(DATA_FOLDER);
        let notes = prepare_data_with(&data, res.path(), None);
        let error = notes.first_start_error.expect("an error");
        assert!(error.contains(&data.display().to_string()), "{error}");
        assert!(error.contains(EXAMPLE_FILE), "{error}");
        // A good start has no error and no note.
        let res = resource_folder();
        let notes = prepare_data_with(&data, res.path(), None);
        assert_eq!(notes, StartupNotes::default());
    }

    #[test]
    #[cfg(windows)]
    fn test_first_start_writes_no_verbatim_path() {
        // Tauri gives the program folder with the prefix \\?\.
        let res = resource_folder();
        let verbatim = PathBuf::from(format!("\\\\?\\{}", res.path().display()));
        let home = tempfile::tempdir().unwrap();
        let data = home.path().join(DATA_FOLDER);
        first_start_with(&data, &verbatim, None).unwrap();
        let text = fs::read_to_string(data.join(CONFIG_FILE)).unwrap();
        assert!(!text.contains("\\\\?\\"), "{text}");
        assert!(!text.contains("\\?\\"), "{text}");
        let config = Config::load(&data.join(CONFIG_FILE)).unwrap();
        assert!(config.data.aliases_dir.is_dir());
    }

    #[test]
    fn test_first_start_with_the_edcopilot_folder_sets_it() {
        let res = resource_folder();
        let home = tempfile::tempdir().unwrap();
        let ed = home.path().join("EDCoPilot");
        fs::create_dir_all(&ed).unwrap();
        let data = home.path().join(DATA_FOLDER);
        assert!(first_start_with(&data, res.path(), Some(&ed)).unwrap());
        let config = Config::load_as_written(&data.join(CONFIG_FILE)).unwrap();
        assert_eq!(config.edcopilot.dir, ed);
        // Nothing is written in the EDCoPilot folder.
        assert_eq!(fs::read_dir(&ed).unwrap().count(), 0);
    }

    #[test]
    fn test_first_start_never_overwrites() {
        let res = resource_folder();
        let home = tempfile::tempdir().unwrap();
        let data = home.path().join(DATA_FOLDER);
        assert!(first_start_with(&data, res.path(), None).unwrap());
        let synonyms = data.join("synonyms.toml");
        fs::write(&synonyms, "[synonyms]\n\"head for\" = \"sail out for\"\n").unwrap();
        let config = data.join(CONFIG_FILE);
        let edited = fs::read_to_string(&config)
            .unwrap()
            .replace("threshold = 0.8", "threshold = 0.9");
        fs::write(&config, &edited).unwrap();
        let synonyms_before = fs::read(&synonyms).unwrap();

        assert!(!first_start_with(&data, res.path(), None).unwrap());
        assert_eq!(fs::read(&synonyms).unwrap(), synonyms_before);
        assert_eq!(fs::read_to_string(&config).unwrap(), edited);

        // A table that the user removed comes back from the resources.
        fs::remove_file(data.join("fillers.toml")).unwrap();
        assert!(first_start_with(&data, res.path(), None).unwrap());
        assert!(data.join("fillers.toml").is_file());
    }

    #[test]
    fn test_first_start_configuration_has_no_map_keys_and_is_valid_except_for_the_folders_of_the_user(
    ) {
        let res = resource_folder();
        let home = tempfile::tempdir().unwrap();
        let data = home.path().join(DATA_FOLDER);
        first_start_with(&data, res.path(), None).unwrap();
        let text = fs::read_to_string(data.join(CONFIG_FILE)).unwrap();
        assert!(!text.contains("command_map"), "{text}");
        assert!(!text.contains("profile_vars"), "{text}");
        // Without the EDCoPilot folder: the folder and the model only.
        let config = Config::load(&data.join(CONFIG_FILE)).unwrap();
        let found = check_config(&config, |_| None);
        assert!(!found.problems.is_empty());
        for problem in &found.problems {
            assert!(
                problem.contains("edcopilot.dir") || problem.contains("stt.parakeet.model_dir"),
                "{problem}"
            );
        }
        assert!(found.problems.iter().any(|p| p.contains("edcopilot.dir")));
        assert!(found
            .problems
            .iter()
            .any(|p| p.contains("stt.parakeet.model_dir")));

        // With an EDCoPilot folder: the commands come from its profile, and
        // only the model folder is a problem.
        let ed = home.path().join("ed");
        edcopilot_voicebridge_core::profile::fixture::write_fixture_edcopilot(&ed);
        let other = home.path().join("other");
        first_start_with(&other, res.path(), Some(&ed)).unwrap();
        let config = Config::load(&other.join(CONFIG_FILE)).unwrap();
        let found = check_config(&config, |_| None);
        let report = found.report.expect("the data loads");
        assert!(
            report.profile.records > 0,
            "the commands come from the profile"
        );
        assert!(found.load_problem.is_none());
        assert_eq!(found.problems.len(), 1, "{:?}", found.problems);
        assert!(found.problems[0].contains("stt.parakeet.model_dir"));
    }

    #[test]
    fn test_first_start_without_the_example_is_an_error_and_writes_no_configuration() {
        let res = resource_folder();
        fs::remove_file(res.path().join("resources").join(EXAMPLE_FILE)).unwrap();
        let home = tempfile::tempdir().unwrap();
        let data = home.path().join(DATA_FOLDER);
        let error = first_start_with(&data, res.path(), None).unwrap_err();
        assert!(format!("{error:#}").contains(EXAMPLE_FILE), "{error:#}");
        assert!(!data.join(CONFIG_FILE).exists());
    }
}
