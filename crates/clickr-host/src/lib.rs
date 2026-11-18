//! The receiving side of clickr as an application: identity, persisted state, access
//! decisions, and key presses.
//!
//! Both the `clickr` CLI and the Clickr Mac app run a [`Host`]. They differ only in how
//! they ask the user to approve a new remote ([`Approver`]) and how they show what is
//! going on ([`Host::subscribe`]).

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Debug,
    future::Future,
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};

use clickr_core::{Command, EndpointId, EndpointTicket, Paths, Receiver, ReceiverHandler};
use n0_error::Result;
use tokio::sync::broadcast;
use tracing::warn;

mod config;
mod keyboard;
mod state;

use self::{config::load_or_create_key, keyboard::Keyboard};
pub use self::{
    config::{config_dir, default_nickname},
    state::{HostState, KnownDevice, Revocation, Store, ago},
};

/// How long [`Host::start`] waits for a relay connection, so the ticket carries a relay
/// URL. The ticket also works without one on the local network or over Bluetooth.
const ONLINE_TIMEOUT: Duration = Duration::from_secs(5);
/// How many events a slow subscriber may fall behind before it misses some.
const EVENT_BUFFER: usize = 64;

/// Asks the user whether a new remote may connect.
pub trait Approver: Debug + Send + Sync + 'static {
    /// Returns `true` if the remote `id`, which calls itself `nickname`, may connect.
    fn approve(&self, id: EndpointId, nickname: &str) -> impl Future<Output = bool> + Send;
}

/// Something that happened on a [`Host`], for display.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// A remote connected.
    Connected {
        /// The remote's endpoint id.
        id: EndpointId,
        /// The nickname the remote reported.
        nickname: String,
    },
    /// The last connection from a remote closed.
    Disconnected {
        /// The remote's endpoint id.
        id: EndpointId,
    },
    /// A connected remote's network paths changed; see [`Host::paths`].
    PathsChanged {
        /// The remote's endpoint id.
        id: EndpointId,
        /// The remote's current paths.
        paths: Paths,
    },
    /// A remote pressed a button.
    Pressed {
        /// The remote's endpoint id.
        id: EndpointId,
        /// The command it sent.
        command: Command,
    },
    /// The persisted state changed; re-read [`Host::snapshot`].
    StateChanged,
}

/// A running receiver with its persisted state.
#[derive(Debug)]
pub struct Host {
    receiver: Receiver,
    shared: Arc<Shared>,
}

/// State shared between the [`Host`] and its [`ReceiverHandler`].
#[derive(Debug)]
struct Shared {
    store: Store,
    connected: Mutex<BTreeSet<EndpointId>>,
    paths: Mutex<BTreeMap<EndpointId, Paths>>,
    events: broadcast::Sender<Event>,
}

impl Shared {
    fn emit(&self, event: Event) {
        // No subscribers is fine: the CLI's `--yes` mode, for example, has none.
        self.events.send(event).ok();
    }
}

impl Host {
    /// Loads the identity and state from `dir` and starts accepting remotes.
    ///
    /// # Errors
    ///
    /// Returns an error if the identity or state cannot be loaded, key presses cannot be
    /// simulated, or the endpoint cannot bind.
    pub async fn start(dir: &Path, approver: impl Approver) -> Result<Self> {
        let secret_key = load_or_create_key(dir)?;
        let store = Store::open(dir, default_nickname)?;
        let keyboard = Keyboard::spawn()?;
        let (events, _) = broadcast::channel(EVENT_BUFFER);
        let shared = Arc::new(Shared {
            store,
            connected: Mutex::default(),
            paths: Mutex::default(),
            events,
        });
        let handler = Handler {
            shared: shared.clone(),
            approver,
            keyboard,
        };
        let receiver = Receiver::spawn(secret_key, handler).await?;
        if tokio::time::timeout(ONLINE_TIMEOUT, receiver.endpoint().online())
            .await
            .is_err()
        {
            warn!("no relay connection yet, ticket only contains local addresses");
        }
        Ok(Self { receiver, shared })
    }

    /// Returns this receiver's endpoint id.
    pub fn id(&self) -> EndpointId {
        self.receiver.endpoint().id()
    }

    /// Returns a pairing ticket for this receiver.
    pub fn ticket(&self) -> EndpointTicket {
        self.receiver.ticket()
    }

    /// Returns a copy of the persisted state.
    pub fn snapshot(&self) -> HostState {
        self.shared.store.snapshot()
    }

    /// Returns the remotes that are connected right now.
    pub fn connected(&self) -> BTreeSet<EndpointId> {
        self.shared.connected.lock().expect("poisoned").clone()
    }

    /// Returns the latest network paths of each connected remote.
    pub fn paths(&self) -> BTreeMap<EndpointId, Paths> {
        self.shared.paths.lock().expect("poisoned").clone()
    }

    /// Returns a stream of [`Event`]s from now on.
    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.shared.events.subscribe()
    }

    /// Changes the nickname sent to remotes. Connected remotes see it on reconnect.
    ///
    /// # Errors
    ///
    /// Returns an error if the state cannot be saved.
    pub fn set_nickname(&self, nickname: String) -> Result<()> {
        self.shared.store.set_nickname(nickname)?;
        self.shared.emit(Event::StateChanged);
        Ok(())
    }

    /// Revokes `id` and drops its connections.
    ///
    /// The remote is turned away, even with its token, until [`unrevoke`] is called.
    ///
    /// # Errors
    ///
    /// Returns an error if the state cannot be saved. The remote is disconnected either
    /// way.
    ///
    /// [`unrevoke`]: Self::unrevoke
    pub fn revoke(&self, id: EndpointId) -> Result<()> {
        let res = self.shared.store.revoke(id);
        self.receiver.disconnect(id);
        self.shared.emit(Event::StateChanged);
        res
    }

    /// Removes the revocation of `id`, so its existing token works again.
    ///
    /// # Errors
    ///
    /// Returns an error if the state cannot be saved.
    pub fn unrevoke(&self, id: &EndpointId) -> Result<()> {
        self.shared.store.unrevoke(id)?;
        self.shared.emit(Event::StateChanged);
        Ok(())
    }

    /// Stops accepting remotes and closes the endpoint.
    pub async fn shutdown(self) {
        self.receiver.shutdown().await;
    }
}

#[derive(Debug)]
struct Handler<A> {
    shared: Arc<Shared>,
    approver: A,
    keyboard: Keyboard,
}

impl<A: Approver> ReceiverHandler for Handler<A> {
    fn nickname(&self) -> String {
        self.shared.store.nickname()
    }

    fn is_revoked(&self, remote: EndpointId) -> bool {
        self.shared.store.is_revoked(&remote)
    }

    async fn request_access(&self, remote: EndpointId, nickname: &str) -> bool {
        self.approver.approve(remote, nickname).await
    }

    fn connected(&self, remote: EndpointId, nickname: &str) {
        if let Err(err) = self.shared.store.record_connection(remote, nickname) {
            warn!(remote = %remote.fmt_short(), "failed to save known remote: {err:#}");
        }
        self.shared
            .connected
            .lock()
            .expect("poisoned")
            .insert(remote);
        self.shared.emit(Event::Connected {
            id: remote,
            nickname: nickname.to_string(),
        });
        self.shared.emit(Event::StateChanged);
    }

    fn disconnected(&self, remote: EndpointId) {
        self.shared
            .connected
            .lock()
            .expect("poisoned")
            .remove(&remote);
        self.shared.paths.lock().expect("poisoned").remove(&remote);
        self.shared.emit(Event::Disconnected { id: remote });
    }

    fn paths_changed(&self, remote: EndpointId, paths: &Paths) {
        self.shared
            .paths
            .lock()
            .expect("poisoned")
            .insert(remote, paths.clone());
        self.shared.emit(Event::PathsChanged {
            id: remote,
            paths: paths.clone(),
        });
    }

    fn handle(&self, remote: EndpointId, command: Command) {
        self.keyboard.press(command);
        self.shared.emit(Event::Pressed {
            id: remote,
            command,
        });
    }
}
