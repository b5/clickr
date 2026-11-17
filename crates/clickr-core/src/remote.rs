//! The sending side of the protocol, run on the phone.

use std::time::{Duration, Instant};

use iroh::{
    Endpoint, EndpointAddr, EndpointId,
    endpoint::{Connection, RecvStream, SendStream, presets},
};
use iroh_tickets::endpoint::EndpointTicket;
use n0_error::{Result, StdResultExt, anyerr};
use tokio::sync::Mutex;
use tracing::debug;

use crate::{ACK_OK, ALPN, Command};

/// How long a single command may take, including a reconnect, before it fails.
const SEND_TIMEOUT: Duration = Duration::from_secs(5);

/// A connection to a [`Receiver`] that sends slide commands.
///
/// The connection is re-established transparently when a send fails, so a remote
/// survives the receiver restarting or the phone switching networks.
///
/// [`Receiver`]: crate::Receiver
#[derive(Debug)]
pub struct Remote {
    endpoint: Endpoint,
    addr: EndpointAddr,
    session: Mutex<Option<Session>>,
}

#[derive(Debug)]
struct Session {
    conn: Connection,
    send: SendStream,
    recv: RecvStream,
}

impl Remote {
    /// Binds a fresh endpoint and connects to the receiver named by `ticket`.
    ///
    /// # Errors
    ///
    /// Returns an error if the endpoint cannot bind or the receiver is unreachable.
    pub async fn connect(ticket: EndpointTicket) -> Result<Self> {
        let endpoint = Endpoint::bind(presets::N0).await?;
        Self::with_endpoint(endpoint, ticket).await
    }

    /// Connects over loopback without relays or address lookup, for tests.
    #[cfg(test)]
    pub(crate) async fn connect_local(ticket: EndpointTicket) -> Result<Self> {
        let endpoint = Endpoint::builder(presets::Minimal)
            .bind_addr("127.0.0.1:0")?
            .bind()
            .await?;
        Self::with_endpoint(endpoint, ticket).await
    }

    async fn with_endpoint(endpoint: Endpoint, ticket: EndpointTicket) -> Result<Self> {
        let addr: EndpointAddr = ticket.into();
        let session = Session::open(&endpoint, addr.clone()).await?;
        Ok(Self {
            endpoint,
            addr,
            session: Mutex::new(Some(session)),
        })
    }

    /// Returns the id of the receiver this remote talks to.
    pub fn receiver_id(&self) -> EndpointId {
        self.addr.id
    }

    /// Sends a command and waits for the receiver to acknowledge it.
    ///
    /// Returns the round-trip time. If the current connection is broken, reconnects once
    /// and retries.
    ///
    /// # Errors
    ///
    /// Returns an error if the command could not be delivered within five seconds.
    pub async fn send(&self, command: Command) -> Result<Duration> {
        tokio::time::timeout(SEND_TIMEOUT, self.send_inner(command))
            .await
            .std_context("timed out")?
    }

    async fn send_inner(&self, command: Command) -> Result<Duration> {
        let mut guard = self.session.lock().await;
        if let Some(session) = guard.as_mut() {
            match session.send(command).await {
                Ok(rtt) => return Ok(rtt),
                Err(err) => {
                    debug!("send failed, reconnecting: {err:#}");
                    session.conn.close(0u32.into(), b"reconnecting");
                    *guard = None;
                }
            }
        }
        let session = guard.insert(Session::open(&self.endpoint, self.addr.clone()).await?);
        session.send(command).await
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
        *guard = Some(Session::open(&self.endpoint, self.addr.clone()).await?);
        Ok(())
    }

    /// Closes the connection and the endpoint.
    pub async fn close(&self) {
        if let Some(session) = self.session.lock().await.take() {
            session.conn.close(0u32.into(), b"bye");
        }
        self.endpoint.close().await;
    }
}

impl Session {
    async fn open(endpoint: &Endpoint, addr: EndpointAddr) -> Result<Self> {
        let conn = endpoint.connect(addr, ALPN).await?;
        let (send, recv) = conn.open_bi().await.anyerr()?;
        Ok(Self { conn, send, recv })
    }

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
