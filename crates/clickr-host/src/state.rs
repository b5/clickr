//! The receiver's persisted state: its nickname, known remotes, and revocations.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

use clickr_core::EndpointId;
use n0_error::{Result, StdResultExt};
use serde::{Deserialize, Serialize};

/// A remote that was allowed to connect.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnownDevice {
    /// The nickname the remote reported on its last connection.
    pub nickname: String,
    /// When the remote was first let in, in seconds since the Unix epoch.
    pub first_seen: u64,
    /// When the remote last connected, in seconds since the Unix epoch.
    pub last_seen: u64,
}

/// A remote whose access was revoked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Revocation {
    /// The remote's nickname when it was revoked.
    pub nickname: String,
    /// When the remote was revoked, in seconds since the Unix epoch.
    pub revoked_at: u64,
}

/// Everything the receiver persists besides its secret key.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct HostState {
    /// The nickname sent to remotes.
    pub nickname: String,
    /// Remotes that were allowed to connect.
    #[serde(default)]
    pub known: BTreeMap<EndpointId, KnownDevice>,
    /// Remotes that are turned away even with a valid token.
    #[serde(default)]
    pub revoked: BTreeMap<EndpointId, Revocation>,
}

/// [`HostState`] backed by a JSON file, rewritten after every change.
///
/// Only one process should use a state file at a time: changes made by another process
/// are not picked up and are overwritten on the next save.
#[derive(Debug)]
pub struct Store {
    path: PathBuf,
    state: Mutex<HostState>,
}

impl Store {
    /// Opens `state.json` in `dir`, starting from a fresh state if the file does not exist.
    ///
    /// `default_nickname` fills in the nickname when none was saved yet.
    ///
    /// # Errors
    ///
    /// Returns an error if the file exists but cannot be read or parsed.
    pub fn open(dir: &Path, default_nickname: impl FnOnce() -> String) -> Result<Self> {
        let path = dir.join("state.json");
        let mut state: HostState = match fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text)
                .std_context(format!("failed to parse {}", path.display()))?,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => HostState::default(),
            Err(err) => {
                return Err(err).std_context(format!("failed to read {}", path.display()));
            }
        };
        if state.nickname.is_empty() {
            state.nickname = default_nickname();
        }
        Ok(Self {
            path,
            state: Mutex::new(state),
        })
    }

    /// Returns a copy of the current state.
    pub fn snapshot(&self) -> HostState {
        self.state.lock().expect("poisoned").clone()
    }

    /// Returns the receiver's nickname.
    pub fn nickname(&self) -> String {
        self.state.lock().expect("poisoned").nickname.clone()
    }

    /// Returns `true` if `id` is revoked.
    pub fn is_revoked(&self, id: &EndpointId) -> bool {
        self.state
            .lock()
            .expect("poisoned")
            .revoked
            .contains_key(id)
    }

    /// Changes the receiver's nickname.
    ///
    /// # Errors
    ///
    /// Returns an error if the state cannot be saved.
    pub fn set_nickname(&self, nickname: String) -> Result<()> {
        self.update(|state| state.nickname = nickname)
    }

    /// Records that `id` connected as `nickname`, adding it to the known remotes if new.
    ///
    /// # Errors
    ///
    /// Returns an error if the state cannot be saved.
    pub fn record_connection(&self, id: EndpointId, nickname: &str) -> Result<()> {
        let now = now();
        self.update(|state| {
            let device = state.known.entry(id).or_insert_with(|| KnownDevice {
                nickname: String::new(),
                first_seen: now,
                last_seen: now,
            });
            nickname.clone_into(&mut device.nickname);
            device.last_seen = now;
        })
    }

    /// Revokes `id`: forgets it as a known remote and turns it away from now on.
    ///
    /// # Errors
    ///
    /// Returns an error if the state cannot be saved.
    pub fn revoke(&self, id: EndpointId) -> Result<()> {
        let now = now();
        self.update(|state| {
            let nickname = state
                .known
                .remove(&id)
                .map(|device| device.nickname)
                .unwrap_or_default();
            state.revoked.insert(
                id,
                Revocation {
                    nickname,
                    revoked_at: now,
                },
            );
        })
    }

    /// Removes the revocation of `id`, so its existing token works again.
    ///
    /// # Errors
    ///
    /// Returns an error if the state cannot be saved.
    pub fn unrevoke(&self, id: &EndpointId) -> Result<()> {
        self.update(|state| {
            state.revoked.remove(id);
        })
    }

    /// Applies `change` and saves the result, writing a temporary file and renaming it so
    /// a crash never leaves a half-written state file.
    fn update(&self, change: impl FnOnce(&mut HostState)) -> Result<()> {
        let mut state = self.state.lock().expect("poisoned");
        change(&mut state);
        let text = serde_json::to_string_pretty(&*state).std_context("failed to encode state")?;
        let tmp = self.path.with_extension("json.tmp");
        fs::write(&tmp, text).std_context("failed to write state")?;
        fs::rename(&tmp, &self.path).std_context("failed to save state")
    }
}

/// Formats a Unix timestamp as a rough age, like "3 hours ago".
pub fn ago(timestamp: u64) -> String {
    let secs = now().saturating_sub(timestamp);
    let (amount, unit) = match secs {
        0..60 => return "just now".to_string(),
        60..3600 => (secs / 60, "minute"),
        3600..86_400 => (secs / 3600, "hour"),
        _ => (secs / 86_400, "day"),
    };
    let plural = if amount == 1 { "" } else { "s" };
    format!("{amount} {unit}{plural} ago")
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

#[cfg(test)]
mod tests {
    use clickr_core::SecretKey;

    use super::*;

    fn open(dir: &Path) -> Store {
        Store::open(dir, || "Studio Mac".into()).unwrap()
    }

    #[test]
    fn new_store_uses_default_nickname() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(open(dir.path()).nickname(), "Studio Mac");
    }

    #[test]
    fn empty_saved_nickname_gets_default() {
        let dir = tempfile::tempdir().unwrap();
        let phone = SecretKey::generate().public();
        Store::open(dir.path(), String::new)
            .unwrap()
            .revoke(phone)
            .unwrap();
        assert_eq!(open(dir.path()).nickname(), "Studio Mac");
    }

    #[test]
    fn state_survives_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let phone = SecretKey::generate().public();
        let store = open(dir.path());
        store.set_nickname("Podium".into()).unwrap();
        store.record_connection(phone, "Swift Otter").unwrap();

        let state = open(dir.path()).snapshot();
        assert_eq!(state.nickname, "Podium");
        assert_eq!(state.known[&phone].nickname, "Swift Otter");
    }

    #[test]
    fn revoke_moves_device_and_unrevoke_clears_it() {
        let dir = tempfile::tempdir().unwrap();
        let phone = SecretKey::generate().public();
        let store = open(dir.path());
        store.record_connection(phone, "Swift Otter").unwrap();

        store.revoke(phone).unwrap();
        assert!(store.is_revoked(&phone));
        let state = open(dir.path()).snapshot();
        assert!(!state.known.contains_key(&phone));
        assert_eq!(state.revoked[&phone].nickname, "Swift Otter");

        store.unrevoke(&phone).unwrap();
        assert!(!store.is_revoked(&phone));
        assert!(open(dir.path()).snapshot().revoked.is_empty());
    }

    #[test]
    fn reconnect_updates_nickname_and_keeps_first_seen() {
        let dir = tempfile::tempdir().unwrap();
        let phone = SecretKey::generate().public();
        let store = open(dir.path());
        store.record_connection(phone, "Swift Otter").unwrap();
        let first = store.snapshot().known[&phone].first_seen;

        store.record_connection(phone, "Calm Heron").unwrap();
        let device = &store.snapshot().known[&phone];
        assert_eq!(device.nickname, "Calm Heron");
        assert_eq!(device.first_seen, first);
    }
}
