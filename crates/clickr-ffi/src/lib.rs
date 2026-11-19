//! Swift bindings for [`clickr_core`], generated with uniffi.
//!
//! The iOS app only needs the [`Remote`] half of the protocol. It binds one
//! [`ClickrNode`] for its lifetime and connects a [`ClickrRemote`] through it.

use std::{fmt, sync::Arc};

use clickr_core::{AccessError, Bluetooth, Command, Endpoint, EndpointId, Remote, SecretKey};
use n0_error::AnyError;
use tracing_subscriber::filter::LevelFilter;

uniffi::setup_scaffolding!();

/// An error surfaced to Swift.
#[derive(Debug, uniffi::Error)]
#[uniffi(flat_error)]
pub enum ClickrError {
    /// The pairing ticket could not be parsed.
    InvalidTicket(String),
    /// The secret key is not 32 bytes long.
    InvalidSecretKey(String),
    /// The user on the receiver declined this phone.
    Denied(String),
    /// The receiver revoked this phone's access.
    Revoked(String),
    /// The receiver could not be reached or dropped the connection.
    Connection(String),
}

impl fmt::Display for ClickrError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ClickrError::InvalidTicket(msg) => write!(f, "invalid ticket: {msg}"),
            ClickrError::InvalidSecretKey(msg) => write!(f, "invalid secret key: {msg}"),
            ClickrError::Denied(msg) | ClickrError::Revoked(msg) | ClickrError::Connection(msg) => {
                write!(f, "{msg}")
            }
        }
    }
}

impl std::error::Error for ClickrError {}

fn conn_err(err: AnyError) -> ClickrError {
    match err.downcast_ref::<AccessError>() {
        Some(AccessError::Denied) => ClickrError::Denied(err.to_string()),
        Some(AccessError::Revoked) => ClickrError::Revoked(err.to_string()),
        None => ClickrError::Connection(format!("{err:#}")),
    }
}

fn parse_ticket(ticket: &str) -> Result<clickr_core::EndpointTicket, ClickrError> {
    clickr_core::parse_ticket(ticket).map_err(|err| ClickrError::InvalidTicket(format!("{err:#}")))
}

/// The most verbose level of log output to emit.
#[derive(Debug, Clone, Copy, uniffi::Enum)]
pub enum LogLevel {
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}

impl From<LogLevel> for LevelFilter {
    fn from(level: LogLevel) -> Self {
        match level {
            LogLevel::Error => LevelFilter::ERROR,
            LogLevel::Warn => LevelFilter::WARN,
            LogLevel::Info => LevelFilter::INFO,
            LogLevel::Debug => LevelFilter::DEBUG,
            LogLevel::Trace => LevelFilter::TRACE,
        }
    }
}

/// Sends Rust log output at `level` and above to stderr, which Xcode shows in its console.
///
/// Only the first call takes effect; later calls are ignored.
#[uniffi::export]
pub fn init_logging(level: LogLevel) {
    let _ = tracing_subscriber::fmt()
        .with_max_level(LevelFilter::from(level))
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .try_init();
}

/// Generates a new 32-byte secret key identifying this phone to receivers.
///
/// Store it and pass it to [`ClickrNode::bind`] on every launch, so receivers that trusted
/// this phone once keep trusting it.
#[uniffi::export]
pub fn generate_secret_key() -> Vec<u8> {
    SecretKey::generate().to_bytes().to_vec()
}

/// Returns true if `ticket` parses as a pairing ticket.
#[uniffi::export]
pub fn is_valid_ticket(ticket: String) -> bool {
    clickr_core::parse_ticket(&ticket).is_ok()
}

/// Returns the endpoint id of the receiver named by `ticket`, which identifies it across
/// tickets with different addresses.
#[uniffi::export]
pub fn ticket_endpoint_id(ticket: String) -> Result<String, ClickrError> {
    Ok(parse_ticket(&ticket)?.endpoint_addr().id.to_string())
}

/// Returns the short form of an endpoint id, for display, or `id` itself if it does not
/// parse.
#[uniffi::export]
pub fn short_endpoint_id(id: String) -> String {
    id.parse::<EndpointId>()
        .map_or(id, |id| id.fmt_short().to_string())
}

/// The network paths of a connection, for display.
#[derive(Debug, Clone, uniffi::Record)]
pub struct ClickrPaths {
    /// The path in use: "Bluetooth", "IPv4", "IPv6" or "relay". `None` while no path is
    /// selected.
    pub selected: Option<String>,
    /// The other open paths, ready to take over.
    pub standby: Vec<String>,
    /// Both of the above in one line, like "via IPv4 (also Bluetooth)".
    pub summary: String,
    /// The round-trip time of the selected path in milliseconds, as estimated by QUIC.
    pub rtt_ms: Option<u32>,
}

impl From<clickr_core::Link> for ClickrPaths {
    fn from(link: clickr_core::Link) -> Self {
        let paths = link.paths;
        Self {
            selected: paths.selected.map(|t| t.to_string()),
            standby: paths.standby().map(|t| t.to_string()).collect(),
            summary: paths.to_string(),
            rtt_ms: link
                .rtt
                .map(|rtt| u32::try_from(rtt.as_millis()).unwrap_or(u32::MAX)),
        }
    }
}

/// The phone's iroh endpoint, reaching receivers over the internet and Bluetooth.
///
/// Bind one node when the app starts and keep it for the life of the process: the
/// Bluetooth transport keeps running until the process exits, so binding a node per
/// connection would leave several running at once.
#[derive(Debug, uniffi::Object)]
pub struct ClickrNode {
    endpoint: Endpoint,
    bluetooth: Bluetooth,
}

#[uniffi::export(async_runtime = "tokio")]
impl ClickrNode {
    /// Binds the endpoint, identifying as `secret_key` (see [`generate_secret_key`]).
    #[uniffi::constructor]
    pub async fn bind(secret_key: Vec<u8>) -> Result<Arc<Self>, ClickrError> {
        let bytes: [u8; 32] = secret_key.as_slice().try_into().map_err(|_| {
            ClickrError::InvalidSecretKey(format!("expected 32 bytes, got {}", secret_key.len()))
        })?;
        let (endpoint, bluetooth) = clickr_core::bind_endpoint(SecretKey::from_bytes(&bytes))
            .await
            .map_err(conn_err)?;
        Ok(Arc::new(Self {
            endpoint,
            bluetooth,
        }))
    }

    /// Describes what this phone's Bluetooth transport knows about the computer with
    /// endpoint id `peer_id`, like "on, other device seen, link connected over L2CAP".
    pub fn bluetooth_status(&self, peer_id: String) -> String {
        match peer_id.parse::<EndpointId>() {
            Ok(peer) => self.bluetooth.status(peer).to_string(),
            Err(err) => format!("invalid endpoint id: {err}"),
        }
    }

    /// Connects to the receiver named by a pairing ticket, introducing this phone as
    /// `nickname`.
    ///
    /// Pass the token from [`ClickrRemote::token`] of an earlier connection, if any.
    /// Without a valid token the receiver asks its user first, and this waits up to two
    /// minutes for the answer.
    pub async fn connect(
        &self,
        ticket: String,
        nickname: String,
        token: Option<Vec<u8>>,
    ) -> Result<Arc<ClickrRemote>, ClickrError> {
        let ticket = parse_ticket(&ticket)?;
        let inner = Remote::connect(&self.endpoint, ticket, nickname, token)
            .await
            .map_err(conn_err)?;
        Ok(Arc::new(ClickrRemote { inner }))
    }
}

/// A connection from the phone to a receiver.
#[derive(Debug, uniffi::Object)]
pub struct ClickrRemote {
    inner: Remote,
}

#[uniffi::export(async_runtime = "tokio")]
impl ClickrRemote {
    /// Sends a right arrow press. Returns the round-trip time in milliseconds.
    pub async fn next_slide(&self) -> Result<u32, ClickrError> {
        self.send(Command::Next).await
    }

    /// Sends a left arrow press. Returns the round-trip time in milliseconds.
    pub async fn previous_slide(&self) -> Result<u32, ClickrError> {
        self.send(Command::Previous).await
    }

    /// Reconnects if the connection was lost, for example after the app was suspended.
    pub async fn ensure_connected(&self) -> Result<(), ClickrError> {
        self.inner.ensure_connected().await.map_err(conn_err)
    }

    /// Returns a short form of the receiver's endpoint id, for display.
    pub fn receiver_short_id(&self) -> String {
        self.inner.receiver_id().fmt_short().to_string()
    }

    /// Returns the current connection's network paths and latency, or `None` when not
    /// connected.
    ///
    /// Reads local state only, so it is cheap to poll.
    pub fn paths(&self) -> Option<ClickrPaths> {
        self.inner.link().map(ClickrPaths::from)
    }

    /// Returns the nickname the receiver reported.
    pub fn receiver_nickname(&self) -> String {
        self.inner.receiver_nickname()
    }

    /// Returns the `CONNECT` token to store and pass to the next
    /// [`ClickrNode::connect`], if the receiver issued one.
    pub fn token(&self) -> Option<Vec<u8>> {
        self.inner.token()
    }

    /// Closes the connection. The node's endpoint stays open for later connections.
    pub async fn close(&self) {
        self.inner.close().await;
    }
}

impl ClickrRemote {
    async fn send(&self, command: Command) -> Result<u32, ClickrError> {
        let rtt = self.inner.send(command).await.map_err(conn_err)?;
        Ok(u32::try_from(rtt.as_millis()).unwrap_or(u32::MAX))
    }
}
