//! The sending side of the protocol, run on the phone.

use std::{
    fmt,
    sync::Mutex as StdMutex,
    time::{Duration, Instant},
};

use iroh::{
    Endpoint, EndpointAddr, EndpointId,
    endpoint::{Connection, RecvStream, SendStream},
};
use iroh_tickets::endpoint::EndpointTicket;
use n0_error::{AnyError, Result, StdResultExt, anyerr};
use tokio::sync::Mutex;
use tracing::{debug, info};

use crate::{
    ACK_OK, ALPN, Command, Link,
    wire::{Hello, Welcome, read_message, write_message},
};

/// How long the receiver has to acknowledge a command on a live connection.
const SEND_TIMEOUT: Duration = Duration::from_secs(5);
/// How long reconnecting may take. Setting up a Bluetooth connection alone takes several
/// seconds, so this is well above [`SEND_TIMEOUT`].
const RECONNECT_TIMEOUT: Duration = Duration::from_secs(30);
/// How long [`Remote::connect`] waits, which covers the user approving a new remote on the
/// receiver.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(120);

/// The receiver turned the remote away.
///
/// Returned inside the [`AnyError`] of a failed [`Remote::connect`], [`Remote::send`] or
/// [`Remote::ensure_connected`]; find it with [`AnyError::downcast_ref`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessError {
    /// The user on the receiver declined this remote.
    Denied,
    /// The receiver revoked this remote's access.
    Revoked,
}

impl fmt::Display for AccessError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AccessError::Denied => f.write_str("access denied"),
            AccessError::Revoked => f.write_str("access revoked"),
        }
    }
}

impl std::error::Error for AccessError {}

/// A connection to a [`Receiver`] that sends slide commands.
///
/// The connection is re-established transparently when a send fails, so a remote
/// survives the receiver restarting or the phone switching networks.
///
/// A remote borrows its [`Endpoint`] rather than owning it: bind one endpoint per process
/// with [`bind_endpoint`] and create a remote per receiver from it.
///
/// [`Receiver`]: crate::Receiver
/// [`bind_endpoint`]: crate::bind_endpoint
#[derive(Debug)]
pub struct Remote {
    endpoint: Endpoint,
    addr: EndpointAddr,
    nickname: String,
    access: StdMutex<Access>,
    session: Mutex<Option<Session>>,
}

/// What the receiver told us in its last [`Welcome`].
#[derive(Debug, Default)]
struct Access {
    token: Option<Vec<u8>>,
    receiver_nickname: String,
    /// The current connection, kept here so [`Remote::paths`] does not wait for a send.
    conn: Option<Connection>,
}

#[derive(Debug)]
struct Session {
    conn: Connection,
    send: SendStream,
    recv: RecvStream,
}

impl Remote {
    /// Connects to the receiver named by `ticket` from `endpoint`, introducing this remote
    /// as `nickname`.
    ///
    /// Pass the `token` the receiver issued on an earlier connection, if any. Without a
    /// valid token the receiver asks its user, and this waits up to two minutes for the
    /// answer. Afterwards [`token`] holds the token to store for next time.
    ///
    /// Tokens are bound to the remote's endpoint id, so bind `endpoint` with the same
    /// secret key every time.
    ///
    /// # Errors
    ///
    /// Returns an error if the receiver is unreachable or turned this remote away (see
    /// [`AccessError`]).
    ///
    /// [`token`]: Self::token
    pub async fn connect(
        endpoint: &Endpoint,
        ticket: EndpointTicket,
        nickname: String,
        token: Option<Vec<u8>>,
    ) -> Result<Self> {
        let remote = Self {
            endpoint: endpoint.clone(),
            addr: ticket.into(),
            nickname,
            access: StdMutex::new(Access {
                token,
                receiver_nickname: String::new(),
                conn: None,
            }),
            session: Mutex::new(None),
        };
        let session = tokio::time::timeout(CONNECT_TIMEOUT, remote.open_session())
            .await
            .std_context("timed out waiting for the receiver")??;
        *remote.session.lock().await = Some(session);
        Ok(remote)
    }

    /// Binds a loopback endpoint without relays or address lookup, for tests.
    #[cfg(test)]
    pub(crate) async fn bind_local() -> Result<Endpoint> {
        Ok(Endpoint::builder(iroh::endpoint::presets::Minimal)
            .bind_addr("127.0.0.1:0")?
            .bind()
            .await?)
    }

    /// Returns the id of the receiver this remote talks to.
    pub fn receiver_id(&self) -> EndpointId {
        self.addr.id
    }

    /// Returns the nickname the receiver reported on the last connection.
    pub fn receiver_nickname(&self) -> String {
        self.access
            .lock()
            .expect("poisoned")
            .receiver_nickname
            .clone()
    }

    /// Returns the paths and latency of the current connection, or `None` when not
    /// connected.
    pub fn link(&self) -> Option<Link> {
        let access = self.access.lock().expect("poisoned");
        let conn = access.conn.as_ref()?;
        conn.close_reason().is_none().then(|| Link::of(conn))
    }

    /// Returns the token to present on future connections, if the receiver issued one.
    pub fn token(&self) -> Option<Vec<u8>> {
        self.access.lock().expect("poisoned").token.clone()
    }

    /// Sends a command and waits for the receiver to acknowledge it.
    ///
    /// Returns the round-trip time. If the current connection is broken, reconnects and
    /// sends the command again.
    ///
    /// If the receiver does not acknowledge within five seconds, the connection is
    /// dropped and this fails without resending, since the receiver may have handled the
    /// command and only the acknowledgement is late. The next send reconnects.
    ///
    /// # Errors
    ///
    /// Returns an error if the acknowledgement timed out, or if reconnecting failed or
    /// took longer than 30 seconds.
    pub async fn send(&self, command: Command) -> Result<Duration> {
        let mut guard = self.session.lock().await;
        if let Some(session) = guard.as_mut() {
            match tokio::time::timeout(SEND_TIMEOUT, session.send(command)).await {
                Ok(Ok(rtt)) => return Ok(rtt),
                Ok(Err(err)) => {
                    debug!("send failed, reconnecting: {err:#}");
                    self.drop_session(&mut guard);
                }
                Err(_) => {
                    // Keeping a connection that stopped answering would make every later
                    // command wait on it too. iroh only looks up other paths, such as
                    // Bluetooth, once the last connection to the receiver is closed.
                    debug!("receiver did not acknowledge in time, dropping the connection");
                    self.drop_session(&mut guard);
                    return Err(anyerr!("timed out waiting for the receiver"));
                }
            }
        }
        let session = tokio::time::timeout(RECONNECT_TIMEOUT, self.open_session())
            .await
            .std_context("timed out reconnecting")??;
        let session = guard.insert(session);
        tokio::time::timeout(SEND_TIMEOUT, session.send(command))
            .await
            .std_context("timed out waiting for the receiver")?
    }

    /// Closes and forgets the current connection.
    fn drop_session(&self, session: &mut Option<Session>) {
        if let Some(session) = session.take() {
            session.conn.close(0u32.into(), b"reconnecting");
        }
        self.access.lock().expect("poisoned").conn = None;
    }

    /// Checks that the receiver is reachable, reconnecting if needed.
    ///
    /// # Errors
    ///
    /// Returns an error if no connection could be established.
    pub async fn ensure_connected(&self) -> Result<()> {
        let mut guard = self.session.lock().await;
        if let Some(session) = guard.as_ref()
            && session.conn.close_reason().is_none()
        {
            return Ok(());
        }
        self.drop_session(&mut guard);
        let session = tokio::time::timeout(RECONNECT_TIMEOUT, self.open_session())
            .await
            .std_context("timed out reconnecting")??;
        *guard = Some(session);
        Ok(())
    }

    /// Connects and runs the handshake, recording what the receiver granted.
    async fn open_session(&self) -> Result<Session> {
        let hello = Hello {
            nickname: self.nickname.clone(),
            token: self.token(),
        };
        let conn = self.endpoint.connect(self.addr.clone(), ALPN).await?;
        let (mut send, mut recv) = conn.open_bi().await.anyerr()?;
        write_message(&mut send, &hello).await?;
        let welcome: Welcome = read_message(&mut recv).await?;
        let refused = match welcome {
            Welcome::Granted { nickname, token } => {
                let mut access = self.access.lock().expect("poisoned");
                access.receiver_nickname = nickname;
                access.conn = Some(conn.clone());
                if let Some(token) = token {
                    info!(receiver = %self.addr.id.fmt_short(), "received connect token");
                    access.token = Some(token);
                }
                return Ok(Session { conn, send, recv });
            }
            Welcome::Denied => AccessError::Denied,
            Welcome::Revoked => AccessError::Revoked,
        };
        conn.close(0u32.into(), b"bye");
        Err(AnyError::from_std(refused))
    }

    /// Closes the connection to the receiver.
    ///
    /// The endpoint stays open. A later [`send`] or [`ensure_connected`] reconnects.
    ///
    /// [`send`]: Self::send
    /// [`ensure_connected`]: Self::ensure_connected
    pub async fn close(&self) {
        if let Some(session) = self.session.lock().await.take() {
            session.conn.close(0u32.into(), b"bye");
        }
        self.access.lock().expect("poisoned").conn = None;
    }
}

impl Session {
    async fn send(&mut self, command: Command) -> Result<Duration> {
        let start = Instant::now();
        self.send.write_all(&[command.to_byte()]).await.anyerr()?;
        let mut ack = [0u8; 1];
        self.recv.read_exact(&mut ack).await.anyerr()?;
        if ack[0] != ACK_OK {
            return Err(anyerr!("receiver rejected command {command:?}"));
        }
        Ok(start.elapsed())
    }
}
