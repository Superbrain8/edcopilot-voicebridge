//! The program state: the configuration path, the key store, the running
//! session, and the last status of the session.
//!
//! Locks: `session` is held while a session starts or stops. The sink never
//! takes it. The sink takes only `status`, and only for a short time. Thus
//! a start or a stop that blocks does not block the events.
//!
//! The order of the locks: `session`, then `capture`, then `stop_flag`.
//! `capture` is also the gate between a session start and an update
//! install: a start sets `starting` under it and checks the update under
//! it, and an install checks the session and marks the update under it.
//! The lock of `updates` comes after `capture` and `status`.
//! `config_lock` comes before `key_lock` and `config_texts`. `session` also comes before
//! `key_lock`: the key lookup of a start takes it while the start holds
//! `session`. A holder of `key_lock` takes no other lock, and a holder of
//! `config_lock` never takes `session` or `capture`. `status` is the last
//! lock: its holder takes no other lock. Thus no two threads can wait for
//! each other.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use voicebridge::config::Config;
use voicebridge::runtime::{EventSink, KeyLookup, Runtime, RuntimeOptions};

use crate::catalog::Catalog;
use crate::dto::{ModelEventDto, StartInfoDto, StatusDto, StopReasonDto};
use crate::paths::StartupNotes;
use crate::secrets::{KeyLock, KeyStore};
use crate::updater::Updates;

/// A running session. `Runtime` is the real one. The tests use a fake.
pub trait SessionControl: Send {
    /// Asks the session to stop and waits for it. Safe to call twice.
    fn stop(&mut self);
    /// False after the first stop signal, also after an error stop.
    fn is_running(&self) -> bool;
}

impl SessionControl for Runtime {
    fn stop(&mut self) {
        Runtime::stop(self);
    }

    fn is_running(&self) -> bool {
        Runtime::is_running(self)
    }
}

/// Starts a session. `Err` gives one text for each problem.
pub trait Starter: Send + Sync {
    fn start(
        &self,
        config: &Config,
        options: RuntimeOptions,
        key_lookup: KeyLookup,
        sink: Arc<dyn EventSink>,
        stop: Arc<AtomicBool>,
    ) -> Result<Box<dyn SessionControl>, Vec<String>>;
}

/// Starts the real runtime.
pub struct RuntimeStarter;

impl Starter for RuntimeStarter {
    fn start(
        &self,
        config: &Config,
        options: RuntimeOptions,
        key_lookup: KeyLookup,
        sink: Arc<dyn EventSink>,
        stop: Arc<AtomicBool>,
    ) -> Result<Box<dyn SessionControl>, Vec<String>> {
        Runtime::start_with_stop(config, options, key_lookup, sink, stop)
            .map(|runtime| Box::new(runtime) as Box<dyn SessionControl>)
            .map_err(|error| error.problems)
    }
}

/// The status of the session. The sink and the commands change it.
#[derive(Debug, Default)]
pub struct Status {
    /// The number of the newest session. The sink of an older session does
    /// not change the status.
    pub generation: u64,
    pub running: bool,
    pub dry_run: bool,
    pub start_info: Option<StartInfoDto>,
    pub stop_reason: Option<StopReasonDto>,
}

impl Status {
    /// `starting` is the flag of a start in work, `AppState::starting`.
    pub fn dto(&self, starting: bool) -> StatusDto {
        StatusDto {
            running: self.running,
            starting,
            dry_run: self.dry_run,
            start_info: self.start_info.clone(),
            stop_reason: self.stop_reason.clone(),
        }
    }
}

/// Gives the lock also when another thread panicked with it.
pub fn lock<T: ?Sized>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

/// The state that Tauri manages. The commands get it.
pub struct AppState {
    config_path: PathBuf,
    keys: Arc<dyn KeyStore>,
    starter: Arc<dyn Starter>,
    /// The runtime slot. Held during a start and a stop.
    pub(crate) session: Mutex<Option<Box<dyn SessionControl>>>,
    /// True while a start is in work.
    pub(crate) starting: AtomicBool,
    /// True when the newest session uses the cloud engine, or when its
    /// start did not load the configuration yet.
    pub(crate) session_cloud: AtomicBool,
    pub(crate) status: Arc<Mutex<Status>>,
    /// The token and the cancel flag of the talk button capture, while one
    /// runs. The page gives the token, so that its cancel ends only its own
    /// capture.
    pub(crate) capture: Mutex<Option<(Option<u32>, Arc<AtomicBool>)>>,
    /// Held for the whole read, edit and write of the configuration file,
    /// and while a key is saved for the name and the host of that file.
    pub(crate) config_lock: Mutex<()>,
    /// Held for each read or write of the key and its host entry. See
    /// `secrets::KeyLock`.
    pub(crate) key_lock: KeyLock,
    /// The texts of the configuration file that `get_config` read, with
    /// their version, the newest last. A save with an older version compares
    /// the edited keys with that text. Its holder takes no other lock.
    pub(crate) config_texts: Mutex<std::collections::VecDeque<(String, String)>>,
    /// The stop flag of the newest start. `stop_session` sets it without the
    /// session lock, so that a start in work stops at its next step.
    pub(crate) stop_flag: Mutex<Option<Arc<AtomicBool>>>,
    /// The cancel flag of the model download, while one runs.
    pub(crate) model_download: Mutex<Option<Arc<AtomicBool>>>,
    /// The last step of the running download, for a window that loads
    /// again.
    pub(crate) model_step: Mutex<Option<ModelEventDto>>,
    /// The command list and the pipeline of the phrase test. Held only to
    /// read or replace the value.
    pub(crate) catalog: Mutex<Option<Arc<Catalog>>>,
    /// Held while the catalog is built, so that two calls do not build it
    /// two times. The session never takes it.
    pub(crate) catalog_build: Mutex<()>,
    /// The number of catalog calls so far. Each call takes the next number.
    pub(crate) catalog_calls: AtomicU64,
    /// Tests only: runs one time in a catalog call, right after the
    /// configuration loads and the stamp is taken.
    #[cfg(test)]
    pub(crate) catalog_hook: CatalogHook,
    /// Tests only: runs in `start_session` right after the check of the
    /// capture, so that a test can try a capture at that point.
    #[cfg(test)]
    pub(crate) start_hook: StartHook,
    /// The data folder for the models, or `None`.
    data_dir: Option<PathBuf>,
    /// Tests only: an empty folder of the saved profile report, so that a
    /// test never reads the report folder of the user.
    #[cfg(test)]
    report_dir: tempfile::TempDir,
    /// The texts of the start for the Status and Diagnostics screens.
    pub(crate) startup: Mutex<StartupNotes>,
    /// True after "Start..." of the tray menu until the page takes it. The
    /// page can load after the click, so the event alone can be lost.
    pub(crate) tray_start: AtomicBool,
    /// The update check and install.
    pub(crate) updates: Arc<Updates>,
}

/// The type of `AppState::catalog_hook`.
#[cfg(test)]
pub(crate) type CatalogHook = Mutex<Option<Box<dyn FnOnce(&AppState) + Send>>>;

/// The type of `AppState::start_hook`.
#[cfg(test)]
pub(crate) type StartHook = Mutex<Option<Box<dyn Fn(&AppState) + Send>>>;

impl AppState {
    pub fn new(
        config_path: PathBuf,
        keys: Arc<dyn KeyStore>,
        starter: Arc<dyn Starter>,
    ) -> AppState {
        AppState {
            config_path,
            keys,
            starter,
            session: Mutex::new(None),
            starting: AtomicBool::new(false),
            session_cloud: AtomicBool::new(false),
            status: Arc::new(Mutex::new(Status::default())),
            capture: Mutex::new(None),
            config_lock: Mutex::new(()),
            key_lock: KeyLock::default(),
            config_texts: Mutex::new(std::collections::VecDeque::new()),
            stop_flag: Mutex::new(None),
            model_download: Mutex::new(None),
            model_step: Mutex::new(None),
            catalog: Mutex::new(None),
            catalog_build: Mutex::new(()),
            catalog_calls: AtomicU64::new(0),
            #[cfg(test)]
            catalog_hook: Mutex::new(None),
            #[cfg(test)]
            start_hook: Mutex::new(None),
            data_dir: None,
            #[cfg(test)]
            report_dir: tempfile::tempdir().expect("a temporary report folder"),
            startup: Mutex::new(StartupNotes::default()),
            tray_start: AtomicBool::new(false),
            // A build with debug assertions never checks for updates.
            updates: Arc::new(Updates::new(
                env!("CARGO_PKG_VERSION"),
                cfg!(debug_assertions),
            )),
        }
    }

    /// The folder of the saved profile report (the folder of the session
    /// lock), or `None` when the program cannot find it. In a test build,
    /// an empty temporary folder of this state.
    pub fn report_dir(&self) -> Option<PathBuf> {
        #[cfg(test)]
        return Some(self.report_dir.path().to_path_buf());
        #[cfg(not(test))]
        voicebridge::runtime::report_dir().ok()
    }

    /// The state with a data folder for the models.
    pub fn with_data_dir(mut self, data_dir: Option<PathBuf>) -> AppState {
        self.data_dir = data_dir;
        self
    }

    /// The state with another update state, for the tests.
    #[cfg(test)]
    pub fn with_updates(mut self, updates: Arc<Updates>) -> AppState {
        self.updates = updates;
        self
    }

    /// The data folder, or `None` when the program must not use one.
    pub fn data_dir(&self) -> Option<&Path> {
        self.data_dir.as_deref()
    }

    /// The problem that stops the use of a configuration, if any.
    pub fn startup_problem(&self) -> Option<String> {
        lock(&self.startup).problem.clone()
    }

    pub fn config_path(&self) -> &Path {
        &self.config_path
    }

    pub fn keys(&self) -> Arc<dyn KeyStore> {
        Arc::clone(&self.keys)
    }

    /// The lock of the key store.
    pub fn key_lock(&self) -> KeyLock {
        Arc::clone(&self.key_lock)
    }

    pub fn starter(&self) -> Arc<dyn Starter> {
        Arc::clone(&self.starter)
    }

    /// True while a session runs or starts.
    pub fn session_active(&self) -> bool {
        self.starting.load(Ordering::SeqCst) || lock(&self.status).running
    }

    /// True while a talk button capture runs. The commands check the
    /// capture under its lock, so only the tests use this function.
    #[cfg(test)]
    pub fn capture_active(&self) -> bool {
        lock(&self.capture).is_some()
    }
}
