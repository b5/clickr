//! The receiving side of the protocol, run on the presenting computer.

use std::{
    collections::HashMap,
    fmt::Debug,
    future::Future,
    sync::{Arc, Mutex},
    time::Duration,
};

use iroh::{
    Endpoint, EndpointAddr, EndpointId, SecretKey, TransportAddr,
    endpoint::{Connection, RecvStream, SendStream},
    protocol::{AcceptError, ProtocolHandler, Router},
};
use iroh_tickets::endpoint::EndpointTicket;
use n0_error::{Result, StdResultExt};
use n0_future::StreamExt;
use tracing::{debug, info, warn};

use crate::{
    ACK_OK, ACK_UNKNOWN, ALPN, Bluetooth, BluetoothStatus, Command, Link, Paths, bind_endpoint,
    capability::{issue_connect, verify_connect},
    wire::{Hello, Welcome, read_message, write_message},
};

/// How long a remote has to send its [`Hello`] after connecting.
const HELLO_TIMEOUT: Duration = Duration::from_secs(10);
/// How long to wait for a rejected remote to read the answer before closing.
const REJECT_LINGER: Duration = Duration::from_secs(5);

/// Application callbacks invoked by a [`Receiver`].
pub trait ReceiverHandler: Debug + Send + Sync + 'static {
    /// Returns this receiver's nickname, sent to every remote that is let in.
    fn nickname(&self) -> String;

    /// Returns `true` if `remote` was revoked and must be turned away, even with a
    /// valid token.
    fn is_revoked(&self, remote: EndpointId) -> bool;

    /// Asks the user whether a remote without a valid token may connect.
    ///
    /// `nickname` is the name the remote reported for itself. Returning `true` issues the
    /// remote a token, so it is not asked about again.
    fn request_access(
        &self,
        remote: EndpointId,
        nickname: &str,
    ) -> impl Future<Output = bool> + Send;

    /// Called when a remote was let in, before its first command.
    fn connected(&self, remote: EndpointId, nickname: &str);

    /// Called when the last connection from a remote closed.
    fn disconnected(&self, remote: EndpointId);

    /// Called when a remote's connection opened or closed a path, or switched paths.
    ///
    /// The first call comes right after [`connected`]. With several connections from one
    /// remote, calls from each are interleaved.
    ///
    /// [`connected`]: Self::connected
    fn paths_changed(&self, remote: EndpointId, paths: &Paths);

    /// Handles a single command from a connected remote.
    fn handle(&self, remote: EndpointId, command: Command);
}

/// Live connections per remote. A remote can briefly have two while it reconnects.
type Live = Arc<Mutex<HashMap<EndpointId, Vec<Connection>>>>;

/// Accepts connections from remotes and dispatches their commands.
#[derive(Debug)]
pub struct Receiver {
    router: Router,
    live: Live,
    bluetooth: Bluetooth,
}

impl Receiver {
    /// Binds an endpoint with the given identity and starts accepting remotes.
    ///
    /// The endpoint is set up by [`bind_endpoint`]: it uses the n0 relays and address
    /// lookup, so a ticket stays usable across network changes as long as the secret key
    /// stays the same, and accepts remotes over Bluetooth when available.
    pub async fn spawn(secret_key: SecretKey, handler: impl ReceiverHandler) -> Result<Self> {
        let (endpoint, bluetooth) = bind_endpoint(secret_key).await?;
        Ok(Self::with_endpoint(endpoint, bluetooth, handler))
    }

    /// Binds a loopback-only endpoint without relays or address lookup, for tests.
    #[cfg(test)]
    pub(crate) async fn spawn_local(
        secret_key: SecretKey,
        handler: impl ReceiverHandler,
    ) -> Result<Self> {
        let endpoint = Endpoint::builder(iroh::endpoint::presets::Minimal)
            .secret_key(secret_key)
            .bind_addr("127.0.0.1:0")?
            .bind()
            .await?;
        let bluetooth = Bluetooth::unavailable("not used in tests");
        Ok(Self::with_endpoint(endpoint, bluetooth, handler))
    }

    fn with_endpoint(
        endpoint: Endpoint,
        bluetooth: Bluetooth,
        handler: impl ReceiverHandler,
    ) -> Self {
        let live = Live::default();
        let handler = Handler {
            handler: Arc::new(handler),
            secret_key: endpoint.secret_key().clone(),
            live: live.clone(),
        };
        let router = Router::builder(endpoint).accept(ALPN, handler).spawn();
        Self {
            router,
            live,
            bluetooth,
        }
    }

    /// Returns the underlying endpoint.
    pub fn endpoint(&self) -> &Endpoint {
        self.router.endpoint()
    }

    /// Returns a pairing ticket containing the current addresses of this receiver.
    ///
    /// Custom transport addresses are left out: the Bluetooth transport reports a local
    /// placeholder that means nothing to other devices, which find this receiver over
    /// Bluetooth by its endpoint id instead.
    pub fn ticket(&self) -> EndpointTicket {
        let addr = self.endpoint().addr();
        let addrs = addr
            .addrs
            .into_iter()
            .filter(|addr| !matches!(addr, TransportAddr::Custom(_)));
        EndpointTicket::new(EndpointAddr::from_parts(addr.id, addrs))
    }

    /// Returns the paths and latency of `remote`'s newest connection, or `None` if it is
    /// not connected.
    pub fn link(&self, remote: EndpointId) -> Option<Link> {
        let live = self.live.lock().expect("poisoned");
        live.get(&remote)?.last().map(Link::of)
    }

    /// Reports what the Bluetooth transport knows about `remote`.
    pub fn bluetooth(&self, remote: EndpointId) -> BluetoothStatus {
        self.bluetooth.status(remote)
    }

    /// Closes every connection from `remote`.
    ///
    /// Use this after revoking a remote so it loses access right away rather than on its
    /// next connection.
    pub fn disconnect(&self, remote: EndpointId) {
        let connections = self.live.lock().expect("poisoned").remove(&remote);
        for connection in connections.into_iter().flatten() {
            connection.close(2u32.into(), b"revoked");
        }
    }

    /// Stops accepting connections and closes the endpoint gracefully.
    pub async fn shutdown(self) {
        if let Err(err) = self.router.shutdown().await {
            warn!("router shutdown failed: {err:#}");
        }
    }
}

#[derive(Debug)]
struct Handler<H> {
    handler: Arc<H>,
    secret_key: SecretKey,
    live: Live,
}

impl<H: ReceiverHandler> ProtocolHandler for Handler<H> {
    async fn accept(&self, connection: Connection) -> Result<(), AcceptError> {
        let remote = connection.remote_id();
        if let Err(err) = self.run(&connection, remote).await {
            debug!(remote = %remote.fmt_short(), "connection ended: {err:#}");
        }
        Ok(())
    }
}

impl<H: ReceiverHandler> Handler<H> {
    async fn run(&self, connection: &Connection, remote: EndpointId) -> Result<()> {
        let (mut send, mut recv) = connection.accept_bi().await.anyerr()?;
        let hello: Hello = tokio::time::timeout(HELLO_TIMEOUT, read_message(&mut recv))
            .await
            .std_context("timed out waiting for hello")??;
        let welcome = self.admit(remote, &hello).await;
        write_message(&mut send, &welcome).await?;
        if !matches!(welcome, Welcome::Granted { .. }) {
            // Give the remote a moment to read the answer; it closes the connection itself.
            send.finish().anyerr()?;
            tokio::time::timeout(REJECT_LINGER, connection.closed())
                .await
                .ok();
            return Ok(());
        }

        info!(remote = %remote.fmt_short(), nickname = %hello.nickname, "remote connected");
        self.live
            .lock()
            .expect("poisoned")
            .entry(remote)
            .or_default()
            .push(connection.clone());
        self.handler.connected(remote, &hello.nickname);
        // The path watch ends when the connection closes, which also ends `serve`.
        let res = tokio::select! {
            res = serve(&mut send, &mut recv, &*self.handler, remote) => res,
            () = watch_paths(connection, &*self.handler, remote) => Ok(()),
        };
        if self.remove_live(remote, connection) {
            info!(remote = %remote.fmt_short(), "remote disconnected");
            self.handler.disconnected(remote);
        }
        res
    }

    /// Decides whether to let a remote in, issuing a token when the user allows it.
    async fn admit(&self, remote: EndpointId, hello: &Hello) -> Welcome {
        let nickname = &hello.nickname;
        if self.handler.is_revoked(remote) {
            info!(remote = %remote.fmt_short(), %nickname, "turned away revoked remote");
            return Welcome::Revoked;
        }
        if let Some(token) = &hello.token {
            match verify_connect(self.secret_key.public(), remote, token) {
                Ok(()) => {
                    return Welcome::Granted {
                        nickname: self.handler.nickname(),
                        token: None,
                    };
                }
                Err(err) => {
                    warn!(remote = %remote.fmt_short(), %nickname, "ignoring invalid token: {err:#}");
                }
            }
        }
        if !self.handler.request_access(remote, nickname).await {
            info!(remote = %remote.fmt_short(), %nickname, "denied remote");
            return Welcome::Denied;
        }
        info!(remote = %remote.fmt_short(), %nickname, "issued connect token");
        Welcome::Granted {
            nickname: self.handler.nickname(),
            token: Some(issue_connect(&self.secret_key, remote)),
        }
    }

    /// Forgets `connection`, returning `true` if it was the remote's last one.
    fn remove_live(&self, remote: EndpointId, connection: &Connection) -> bool {
        let mut live = self.live.lock().expect("poisoned");
        let Some(connections) = live.get_mut(&remote) else {
            // Already removed by `Receiver::disconnect`.
            return true;
        };
        connections.retain(|c| c.stable_id() != connection.stable_id());
        if connections.is_empty() {
            live.remove(&remote);
            true
        } else {
            false
        }
    }
}

/// Reports `connection`'s paths to the handler whenever they change.
async fn watch_paths(connection: &Connection, handler: &impl ReceiverHandler, remote: EndpointId) {
    let mut last = None;
    let mut stream = connection.paths_stream();
    while stream.next().await.is_some() {
        // Snapshots also arrive for changes that look the same here, such as a new
        // relay path replacing an old one; only report what the user can see.
        let paths = Paths::of(connection);
        if last.as_ref() != Some(&paths) {
            debug!(remote = %remote.fmt_short(), %paths, "paths changed");
            handler.paths_changed(remote, &paths);
            last = Some(paths);
        }
    }
}

async fn serve(
    send: &mut SendStream,
    recv: &mut RecvStream,
    handler: &impl ReceiverHandler,
    remote: EndpointId,
) -> Result<()> {
    let mut buf = [0u8; 1];
    loop {
        recv.read_exact(&mut buf).await.anyerr()?;
        let ack = match Command::from_byte(buf[0]) {
            Some(command) => {
                debug!(remote = %remote.fmt_short(), ?command, "command");
                handler.handle(remote, command);
                ACK_OK
            }
            None => {
                warn!(remote = %remote.fmt_short(), byte = buf[0], "unknown command");
                ACK_UNKNOWN
            }
        };
        send.write_all(&[ack]).await.anyerr()?;
    }
}
