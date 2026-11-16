//! The command list and the pipeline of the phrase test, built from the
//! saved configuration and kept until a file changes.
//!
//! The running session has its own pipeline. Nothing here touches it, and
//! nothing here takes a lock of the session. The pipeline of the phrase test
//! never writes the request file: the phrase test is always a dry run, and
//! the writer points to a path below the configuration file. That file is a
//! file and not a folder, so a write there always fails.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::SystemTime;

use edcopilot_voicebridge_core::profile::BuildReport;
use edcopilot_voicebridge_core::Pipeline;
use voicebridge::config::{build_pipeline_with_commands, load_commands, Config};
use voicebridge::list::ListData;

use crate::state::{lock, AppState};

/// The size and the time of the last change of one file, or `None` when
/// the file is absent.
type FileStamp = (PathBuf, Option<(u64, Option<SystemTime>)>);

/// The state of the configuration file: its size, its time, and a hash of
/// its content. The hash sees an edit that keeps the size and the time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigStamp {
    file: FileStamp,
    content: Option<u64>,
}

/// The state of all files that the list and the pipeline read. A change of
/// one file gives another stamp.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stamp {
    config: ConfigStamp,
    data: Vec<FileStamp>,
}

fn file_stamp(path: &Path) -> FileStamp {
    let meta = std::fs::metadata(path)
        .ok()
        .map(|m| (m.len(), m.modified().ok()));
    (path.to_path_buf(), meta)
}

/// The stamp of the configuration file. Take it before the file loads.
pub fn config_stamp(config_path: &Path) -> ConfigStamp {
    let file = file_stamp(config_path);
    let content = std::fs::read(config_path).ok().map(|bytes| {
        let mut hasher = DefaultHasher::new();
        bytes.hash(&mut hasher);
        hasher.finish()
    });
    ConfigStamp { file, content }
}

/// The stamps of the files in a folder, sorted by name. An absent folder
/// gives one entry for the folder.
fn folder_stamps(dir: &Path, out: &mut Vec<FileStamp>) {
    out.push(file_stamp(dir));
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut files: Vec<FileStamp> = entries
        .flatten()
        .map(|entry| file_stamp(&entry.path()))
        .collect();
    files.sort();
    out.extend(files);
}

/// The stamps of the data files that the configuration names. Take them
/// after the configuration loads. The EDCoPilot profile is one of them: an
/// update of EDCoPilot gives a new catalog.
fn data_stamps(config: &Config) -> Vec<FileStamp> {
    let ed = &config.edcopilot;
    let data = &config.data;
    let mut out = Vec::new();
    for file in [
        &ed.voice_commands_ini,
        &ed.profile,
        &data.sound_alikes,
        &data.synonyms,
        &data.fillers,
    ] {
        out.push(file_stamp(file));
    }
    folder_stamps(&data.aliases_dir, &mut out);
    folder_stamps(&ed.dynamic_vars_dir, &mut out);
    out
}

/// The stamp of the configuration file and of the data files that it names.
/// The tests use it.
#[cfg(test)]
pub fn stamp(config_path: &Path, config: &Config) -> Stamp {
    Stamp {
        config: config_stamp(config_path),
        data: data_stamps(config),
    }
}

/// The data of the Commands screen for one state of the files.
pub struct Catalog {
    pub stamp: Stamp,
    /// The number of the call that built it. A catalog of an older call
    /// never replaces the catalog of a newer call. The window gets it as the
    /// id of the catalog.
    pub ticket: u64,
    /// The loaded configuration, with the guard path as the request file.
    pub config: Config,
    pub list: ListData,
    pub pipeline: Arc<Pipeline>,
    /// The load problems of the pipeline (`LoadReport::problems`). All of
    /// them concern the command data.
    pub problems: Vec<String>,
    /// The warnings of the synonyms, the filler words, the aliases, the
    /// vocabulary files, and the load of the commands.
    pub warnings: Vec<String>,
    /// The report of the build from the EDCoPilot profile.
    pub profile: BuildReport,
}

/// The request file of the phrase test: a path below the configuration
/// file. A write there always fails.
fn guard_request_file(config_path: &Path) -> PathBuf {
    config_path
        .join("phrase-test-never-writes")
        .join("EDCoPilot.request.txt")
}

impl Catalog {
    fn build(
        config_path: &Path,
        mut config: Config,
        stamp: Stamp,
        ticket: u64,
    ) -> Result<Catalog, String> {
        config.edcopilot.request_file = guard_request_file(config_path);
        // One load of the commands from the profile for the list and the
        // pipeline.
        let loaded = load_commands(&config).map_err(|e| format!("{e:#}"))?;
        let list = ListData::from_commands(&config, &loaded.commands);
        let (pipeline, report) = build_pipeline_with_commands(&config, loaded);
        // The warnings of the data in the order of `voicebridge check`.
        let warnings = report.warnings().cloned().collect();
        Ok(Catalog {
            stamp,
            ticket,
            config,
            list,
            pipeline: Arc::new(pipeline),
            problems: report.problems,
            warnings,
            profile: report.profile,
        })
    }
}

/// The catalog of the current files. It is built again only when a file
/// changed. The lock of the cache is held only to read or replace the
/// value. A second caller waits for a build in work, so that two calls do
/// not build two times.
///
/// The stamp of the configuration file comes before the load. Thus a save
/// during the call gives a new stamp at the next call. A catalog is not
/// kept when the configuration file changed during the build, and it never
/// replaces the catalog of a newer call. The caller gets its result in
/// both cases.
pub fn catalog(state: &AppState) -> Result<Arc<Catalog>, String> {
    let path = state.config_path();
    let before = config_stamp(path);
    let config = Config::load(path).map_err(|e| format!("{e:#}"))?;
    let stamp = Stamp {
        config: before,
        data: data_stamps(&config),
    };
    // The ticket comes after the stamps. Thus a call with an older stamp
    // does not get a newer ticket than a call that read the files later.
    let ticket = state.catalog_calls.fetch_add(1, Ordering::SeqCst) + 1;
    #[cfg(test)]
    {
        let hook = lock(&state.catalog_hook).take();
        if let Some(hook) = hook {
            hook(state);
        }
    }
    let cached = || {
        lock(&state.catalog)
            .as_ref()
            .filter(|c| c.stamp == stamp)
            .map(Arc::clone)
    };
    if let Some(found) = cached() {
        return Ok(found);
    }
    let _build = lock(&state.catalog_build);
    if let Some(found) = cached() {
        return Ok(found);
    }
    let built = Arc::new(Catalog::build(path, config, stamp.clone(), ticket)?);
    if config_stamp(path) == stamp.config {
        let mut slot = lock(&state.catalog);
        if slot.as_ref().is_none_or(|old| old.ticket < ticket) {
            *slot = Some(Arc::clone(&built));
        }
    }
    Ok(built)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_guard_request_file_is_below_the_configuration_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("voicebridge.toml");
        std::fs::write(&file, "x").unwrap();
        let guard = guard_request_file(&file);
        assert!(guard.starts_with(&file));
        // The parent of the guard is not a folder: a write fails.
        assert!(std::fs::write(&guard, "token").is_err());
        assert!(!guard.exists());
    }

    #[test]
    fn test_stamp_changes_with_a_file_of_a_folder() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("voicebridge.toml");
        std::fs::write(&file, "[edcopilot]\ndir = 'ed'\n").unwrap();
        std::fs::create_dir_all(dir.path().join("ed/Dynamic var files")).unwrap();
        let config = Config::load(&file).unwrap();
        let first = stamp(&file, &config);
        assert_eq!(first, stamp(&file, &config));
        std::fs::write(config.edcopilot.dynamic_vars_dir.join("A.txt"), "Sol\n").unwrap();
        let second = stamp(&file, &config);
        assert_ne!(first, second);
        std::fs::write(
            config.edcopilot.dynamic_vars_dir.join("A.txt"),
            "Sol\nSirius\n",
        )
        .unwrap();
        assert_ne!(second, stamp(&file, &config));
    }

    /// An edit with the same size inside the time resolution of the file
    /// system keeps the size and the time. The hash of the content sees it.
    #[test]
    fn test_config_stamp_sees_an_edit_with_the_same_size_and_time() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("voicebridge.toml");
        std::fs::write(&file, "[matching]\nthreshold = 0.8\n").unwrap();
        let time = std::fs::metadata(&file).unwrap().modified().unwrap();
        let first = config_stamp(&file);
        std::fs::write(&file, "[matching]\nthreshold = 0.9\n").unwrap();
        std::fs::File::options()
            .write(true)
            .open(&file)
            .unwrap()
            .set_modified(time)
            .unwrap();
        let second = config_stamp(&file);
        assert_eq!(first.file, second.file, "size and time are the same");
        assert_ne!(first, second);
    }
}
