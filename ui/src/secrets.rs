//! The key store: the API key of the cloud engine in the Windows Credential
//! Manager.
//!
//! The name of an entry is the name of the environment variable that the
//! configuration gives in `stt.openai_compatible.api_key_env`. Thus the
//! configuration file does not change.
//!
//! No type in this module shows a key in `Debug` or `Display` output. No error
//! text holds a key.

use std::sync::{Arc, Mutex};

use voicebridge::runtime::KeyLookup;

/// A place that keeps API keys. The name is the name of an environment
/// variable.
pub trait KeyStore: Send + Sync {
    fn set(&self, name: &str, key: &str) -> Result<(), String>;
    /// True when the store has a usable key for the name.
    fn has(&self, name: &str) -> bool;
    /// The key, or `None` when the store has no entry or cannot read it.
    fn get(&self, name: &str) -> Option<String>;
    /// Removes the entry. A missing entry is not an error.
    fn delete(&self, name: &str) -> Result<(), String>;
}

/// The service name of all entries in the Credential Manager.
const SERVICE: &str = "VoiceBridge";

/// The name of the kind of an error. The text is fixed. It never holds the
/// text of the error itself.
///
/// The errors of the `keyring` crate can hold secret data. `BadEncoding` and
/// `BadDataFormat` carry the raw bytes of the stored value, and their `Debug`
/// output shows the bytes. The platform errors carry text of the operating
/// system. This module never shows the `Display` or the `Debug` output of
/// such an error.
fn error_kind(error: &keyring::Error) -> &'static str {
    use keyring::Error;
    match error {
        Error::PlatformFailure(_) => "platform failure",
        Error::NoStorageAccess(_) => "no access to the storage",
        Error::NoEntry => "no entry",
        Error::BadEncoding(_) => "bad encoding",
        Error::BadDataFormat(..) => "bad data format",
        Error::BadStoreFormat(_) => "bad store format",
        Error::TooLong(..) => "value too long",
        Error::Invalid(..) => "invalid value",
        Error::Ambiguous(_) => "ambiguous entry",
        Error::NoDefaultStore => "no key store on this system",
        Error::NotSupportedByStore(_) => "not supported by the store",
        // The enum can grow in a later version of the crate.
        _ => "unknown error",
    }
}

/// The text for a failed action of the key store.
fn store_error(action: &str, error: &keyring::Error) -> String {
    format!(
        "cannot {action} the API key in the key store ({})",
        error_kind(error)
    )
}

/// A name must hold at least one character that is not white space.
fn check_name(name: &str) -> Result<(), String> {
    if name.trim().is_empty() {
        Err("the name of the API key variable is empty".to_string())
    } else {
        Ok(())
    }
}

/// The Windows Credential Manager. Service "VoiceBridge", account is the name
/// of the variable. On other systems the native store of the system.
#[derive(Debug, Clone, Copy, Default)]
pub struct CredentialManagerStore;

impl CredentialManagerStore {
    fn entry(name: &str) -> Result<keyring::Entry, keyring::Error> {
        keyring::Entry::new(SERVICE, name)
    }
}

impl KeyStore for CredentialManagerStore {
    fn set(&self, name: &str, key: &str) -> Result<(), String> {
        check_name(name)?;
        let key = key.trim();
        if key.is_empty() {
            return Err("the API key is empty".to_string());
        }
        Self::entry(name)
            .and_then(|entry| entry.set_password(key))
            .map_err(|e| store_error("save", &e))
    }

    /// Reads the stored bytes and drops them at once. No `String` holds the
    /// key.
    fn has(&self, name: &str) -> bool {
        if check_name(name).is_err() {
            return false;
        }
        Self::entry(name)
            .and_then(|entry| entry.get_secret())
            .is_ok_and(|bytes| bytes.iter().any(|b| !b.is_ascii_whitespace()))
    }

    fn get(&self, name: &str) -> Option<String> {
        if check_name(name).is_err() {
            return None;
        }
        // An error of any kind gives `None`. The caller then uses the
        // environment variable.
        Self::entry(name).ok()?.get_password().ok()
    }

    fn delete(&self, name: &str) -> Result<(), String> {
        check_name(name)?;
        match Self::entry(name).and_then(|entry| entry.delete_credential()) {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(store_error("remove", &e)),
        }
    }
}

/// A store in memory. The tests of this crate use it. It is not behind
/// `cfg(test)`, so that the tests of all modules of this crate can use it.
/// The program does not use it.
#[allow(dead_code)]
#[derive(Default)]
pub struct MemoryStore {
    entries: std::sync::Mutex<std::collections::HashMap<String, String>>,
}

#[allow(dead_code)]
impl MemoryStore {
    pub fn new() -> Self {
        Self::default()
    }

    fn entries(&self) -> std::sync::MutexGuard<'_, std::collections::HashMap<String, String>> {
        // A panic in another test must not hide the entries.
        self.entries.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Shows the number of entries, never a key.
impl std::fmt::Debug for MemoryStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MemoryStore")
            .field("entries", &self.entries().len())
            .finish()
    }
}

impl KeyStore for MemoryStore {
    fn set(&self, name: &str, key: &str) -> Result<(), String> {
        check_name(name)?;
        let key = key.trim();
        if key.is_empty() {
            return Err("the API key is empty".to_string());
        }
        self.entries().insert(name.to_string(), key.to_string());
        Ok(())
    }

    fn has(&self, name: &str) -> bool {
        self.get(name).is_some()
    }

    fn get(&self, name: &str) -> Option<String> {
        self.entries().get(name).cloned()
    }

    fn delete(&self, name: &str) -> Result<(), String> {
        check_name(name)?;
        self.entries().remove(name);
        Ok(())
    }
}

/// The name of the store entry that holds the host of the key `name`, for
/// example `OPENAI_API_KEY.host`. The host is not secret.
pub fn host_entry(name: &str) -> String {
    format!("{name}.host")
}

/// The lock of the key store. A key and its host entry are two entries, so
/// a reader must not read them while a writer changes them. The lookup of a
/// session, `set_api_key` and `clear_api_key` hold this lock. A holder of
/// this lock takes no other lock.
pub type KeyLock = Arc<Mutex<()>>;

/// The store first, then the environment variable. The store gives its key
/// only when the entry `<name>.host` equals `host`, the origin of the
/// configured `base_url` (see `voicebridge::config::url_origin`). Thus a
/// stored key never goes to another server. The read of the store holds
/// `key_lock`.
///
/// Known limit: the environment variable is not bound to a host. The
/// console program reads it the same way.
pub fn key_lookup(store: Arc<dyn KeyStore>, host: Option<String>, key_lock: KeyLock) -> KeyLookup {
    key_lookup_with(store, host, key_lock, |name| std::env::var(name).ok())
}

/// True when the store has a key for `name` that was saved for `host`.
pub fn store_key_matches(store: &dyn KeyStore, name: &str, host: Option<&str>) -> bool {
    host.is_some() && store.get(&host_entry(name)).as_deref() == host
}

/// The stored key for `name` when its host entry equals `host`. A key
/// without a host entry is refused. The caller holds the key lock.
pub fn stored_key(store: &dyn KeyStore, name: &str, host: Option<&str>) -> Option<String> {
    store_key_matches(store, name, host)
        .then(|| store.get(name))
        .flatten()
        .filter(|key| !key.trim().is_empty())
}

/// Saves the key for `host`. The caller holds the key lock. The order of
/// the writes is: remove the host entry, write the key, write the host
/// entry. Thus a reader that does not hold the lock, or a write that stops
/// half way, finds a key without a host entry, and `stored_key` refuses
/// it. A failed write removes the key.
pub fn save_key(store: &dyn KeyStore, name: &str, key: &str, host: &str) -> Result<(), String> {
    store.delete(&host_entry(name))?;
    let written = store
        .set(name, key)
        .and_then(|()| store.set(&host_entry(name), host));
    if let Err(error) = written {
        // A key without its host is never used: remove it.
        let _ = store.delete(name);
        let _ = store.delete(&host_entry(name));
        return Err(error);
    }
    Ok(())
}

/// Removes the key, then its host entry. The caller holds the key lock.
pub fn remove_key(store: &dyn KeyStore, name: &str) -> Result<(), String> {
    store.delete(name)?;
    store.delete(&host_entry(name))
}

/// `key_lookup` with a given environment. The tests use it, so that they do
/// not set variables of the process.
fn key_lookup_with(
    store: Arc<dyn KeyStore>,
    host: Option<String>,
    key_lock: KeyLock,
    env: impl Fn(&str) -> Option<String> + Send + Sync + 'static,
) -> KeyLookup {
    Arc::new(move |name: &str| {
        let stored = {
            let _keys = crate::state::lock(&key_lock);
            stored_key(&*store, name, host.as_deref())
        };
        stored.or_else(|| env(name))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A lookup for the host of the key `VB_KEY`.
    fn lookup(store: &Arc<MemoryStore>, env: Option<&'static str>) -> KeyLookup {
        const HOST: &str = "https://api.openai.com";
        if store.has("VB_KEY") {
            store.set(&host_entry("VB_KEY"), HOST).unwrap();
        }
        key_lookup_with(
            store.clone(),
            Some(HOST.to_string()),
            Arc::default(),
            move |_| env.map(str::to_string),
        )
    }

    #[test]
    fn test_store_value_wins_over_the_environment() {
        let store = Arc::new(MemoryStore::new());
        store.set("VB_KEY", "from-store").unwrap();
        let find = lookup(&store, Some("from-env"));
        assert_eq!(find("VB_KEY").as_deref(), Some("from-store"));
    }

    #[test]
    fn test_environment_is_used_when_the_store_has_no_entry() {
        let store = Arc::new(MemoryStore::new());
        let find = lookup(&store, Some("from-env"));
        assert_eq!(find("VB_KEY").as_deref(), Some("from-env"));
        let none = lookup(&store, None);
        assert_eq!(none("VB_KEY"), None);
    }

    #[test]
    fn test_store_key_is_used_only_for_its_host() {
        let store = Arc::new(MemoryStore::new());
        store.set("VB_KEY", "from-store").unwrap();
        store
            .set(&host_entry("VB_KEY"), "https://api.openai.com")
            .unwrap();
        let env = |value: Option<&'static str>| move |_: &str| value.map(str::to_string);
        let same = key_lookup_with(
            store.clone(),
            Some("https://api.openai.com".into()),
            Arc::default(),
            env(None),
        );
        assert_eq!(same("VB_KEY").as_deref(), Some("from-store"));
        let other = key_lookup_with(
            store.clone(),
            Some("https://evil.example.org".into()),
            Arc::default(),
            env(None),
        );
        assert_eq!(other("VB_KEY"), None);
        // The environment variable stays the second source.
        let other = key_lookup_with(
            store.clone(),
            Some("https://evil.example.org".into()),
            Arc::default(),
            env(Some("from-env")),
        );
        assert_eq!(other("VB_KEY").as_deref(), Some("from-env"));
        // No host (a URL that does not parse), or a key without a host.
        let none = key_lookup_with(store.clone(), None, Arc::default(), env(None));
        assert_eq!(none("VB_KEY"), None);
        store.delete(&host_entry("VB_KEY")).unwrap();
        let same = key_lookup_with(
            store,
            Some("https://api.openai.com".into()),
            Arc::default(),
            env(None),
        );
        assert_eq!(same("VB_KEY"), None);
    }

    /// The key for `https://good.example` goes only to a base URL whose
    /// request reaches that origin (the stt tests prove that the client
    /// requests the origin of `url_origin`).
    #[test]
    fn test_hostile_spellings_do_not_get_the_key() {
        use voicebridge::config::url_origin;
        let store = Arc::new(MemoryStore::new());
        store.set("VB_KEY", "from-store").unwrap();
        let good = url_origin("https://good.example/v1").unwrap();
        store.set(&host_entry("VB_KEY"), &good).unwrap();
        let cases = [
            ("https://GOOD.EXAMPLE:443/v1", true),
            ("https://good.example\\@evil.example/v1", true),
            ("https://good.ex\tample/v1", true),
            ("https://good.example@evil.example/v1", false),
            ("https://good.example.evil.example/v1", false),
            ("https://evil.example/good.example", false),
            ("http://good.example/v1", false),
            ("https://good.example:8443/v1", false),
            ("https://good.example./v1", false),
            ("https://%65vil.example/v1", false),
            ("not a url", false),
        ];
        for (base, gets_key) in cases {
            let find = key_lookup_with(store.clone(), url_origin(base), Arc::default(), |_| None);
            assert_eq!(find("VB_KEY").is_some(), gets_key, "{base:?}");
        }
    }

    /// A store that refuses the write of a host entry.
    struct NoHostStore(MemoryStore);

    impl KeyStore for NoHostStore {
        fn set(&self, name: &str, key: &str) -> Result<(), String> {
            if name.ends_with(".host") {
                return Err("cannot save the API key in the key store (platform failure)".into());
            }
            self.0.set(name, key)
        }
        fn has(&self, name: &str) -> bool {
            self.0.has(name)
        }
        fn get(&self, name: &str) -> Option<String> {
            self.0.get(name)
        }
        fn delete(&self, name: &str) -> Result<(), String> {
            self.0.delete(name)
        }
    }

    #[test]
    fn test_a_failed_host_write_leaves_no_key_and_no_old_host() {
        let store = NoHostStore(MemoryStore::new());
        store.0.set("VB_KEY", "old").unwrap();
        store
            .0
            .set(&host_entry("VB_KEY"), "https://old.example")
            .unwrap();
        assert!(save_key(&store, "VB_KEY", "new", "https://new.example").is_err());
        assert_eq!(store.get("VB_KEY"), None);
        assert_eq!(store.get(&host_entry("VB_KEY")), None);
        for host in ["https://old.example", "https://new.example"] {
            assert_eq!(stored_key(&store, "VB_KEY", Some(host)), None);
        }
    }

    #[test]
    fn test_empty_key_is_refused() {
        let store = MemoryStore::new();
        assert!(store.set("VB_KEY", "").is_err());
        assert!(!store.has("VB_KEY"));
    }

    #[test]
    fn test_whitespace_key_is_refused() {
        let store = MemoryStore::new();
        assert!(store.set("VB_KEY", " \t\r\n ").is_err());
        assert!(!store.has("VB_KEY"));
        let real = CredentialManagerStore;
        assert!(real.set("VB_KEY", "  ").is_err());
    }

    #[test]
    fn test_empty_name_is_refused() {
        let store = MemoryStore::new();
        assert!(store.set("", "key").is_err());
        assert!(store.set("  ", "key").is_err());
        let real = CredentialManagerStore;
        assert!(real.set("", "key").is_err());
        assert_eq!(real.get(""), None);
        assert!(real.delete("").is_err());
    }

    #[test]
    fn test_delete_absent_entry_is_ok() {
        let store = MemoryStore::new();
        assert!(store.delete("VB_NEVER_SET").is_ok());
        store.set("VB_KEY", "key").unwrap();
        assert!(store.has("VB_KEY"));
        store.delete("VB_KEY").unwrap();
        assert!(!store.has("VB_KEY"));
        assert_eq!(store.get("VB_KEY"), None);
    }

    #[test]
    fn test_error_text_never_holds_the_key() {
        const SECRET: &str = "sk-SECRET-VALUE";
        let bytes = SECRET.as_bytes().to_vec();
        let platform =
            || -> keyring::Error { keyring::Error::PlatformFailure(SECRET.to_string().into()) };
        let errors = vec![
            keyring::Error::BadEncoding(bytes.clone()),
            keyring::Error::BadDataFormat(bytes.clone(), SECRET.to_string().into()),
            keyring::Error::BadStoreFormat(SECRET.to_string()),
            keyring::Error::Invalid(SECRET.to_string(), SECRET.to_string()),
            keyring::Error::TooLong(SECRET.to_string(), 3),
            keyring::Error::NoStorageAccess(SECRET.to_string().into()),
            keyring::Error::NotSupportedByStore(SECRET.to_string()),
            keyring::Error::NoEntry,
            keyring::Error::NoDefaultStore,
            platform(),
        ];
        for error in &errors {
            for action in ["save", "read", "remove"] {
                let text = store_error(action, error);
                assert!(!text.contains("SECRET"), "{text}");
                assert!(!text.is_empty());
            }
        }
    }

    #[test]
    fn test_memory_store_debug_shows_no_key() {
        let store = MemoryStore::new();
        store.set("VB_KEY", "sk-SECRET-VALUE").unwrap();
        let text = format!("{store:?}");
        assert!(!text.contains("SECRET"), "{text}");
    }

    // This test uses the real Credential Manager of the user. It writes one
    // dummy entry with the name VOICEBRIDGE_TEST_KEY and removes it at the
    // end. The guard removes it also when an assertion fails.
    #[test]
    #[ignore]
    fn the_credential_manager_keeps_and_removes_a_key() {
        const NAME: &str = "VOICEBRIDGE_TEST_KEY";
        struct Cleanup;
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = CredentialManagerStore.delete(NAME);
            }
        }
        let _cleanup = Cleanup;
        let store = CredentialManagerStore;
        store.delete(NAME).unwrap();
        assert!(!store.has(NAME));
        assert_eq!(store.get(NAME), None);
        store.set(NAME, "  dummy-value-not-a-key  ").unwrap();
        assert!(store.has(NAME));
        assert_eq!(store.get(NAME).as_deref(), Some("dummy-value-not-a-key"));
        store.delete(NAME).unwrap();
        assert!(!store.has(NAME));
        assert!(store.delete(NAME).is_ok());
    }
}
