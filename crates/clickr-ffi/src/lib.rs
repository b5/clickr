//! Swift bindings for [`clickr_core`], generated with uniffi.
//!
//! The iOS app only needs the [`Remote`] half of the protocol, exposed here as
//! [`ClickrRemote`].

use std::{fmt, sync::Arc};

use clickr_core::{Command, Remote};

uniffi::setup_scaffolding!();

/// An error surfaced to Swift.
#[derive(Debug, uniffi::Error)]
#[uniffi(flat_error)]
pub enum ClickrError {
    /// The pairing ticket could not be parsed.
    InvalidTicket(String),
    /// The receiver could not be reached or dropped the connection.
    Connection(String),
}

impl fmt::Display for ClickrError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ClickrError::InvalidTicket(msg) => write!(f, "invalid ticket: {msg}"),
            ClickrError::Connection(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for ClickrError {}

fn conn_err(err: impl fmt::Display) -> ClickrError {
    ClickrError::Connection(format!("{err:#}"))
}

/// Returns true if `ticket` parses as a pairing ticket.
#[uniffi::export]
pub fn is_valid_ticket(ticket: String) -> bool {
    clickr_core::parse_ticket(&ticket).is_ok()
}

/// A connection from the phone to a `clickr accept` receiver.
#[derive(Debug, uniffi::Object)]
pub struct ClickrRemote {
    inner: Remote,
}

#[uniffi::export(async_runtime = "tokio")]
impl ClickrRemote {
    /// Connects to the receiver named by a pairing ticket.
    #[uniffi::constructor]
    pub async fn connect(ticket: String) -> Result<Arc<Self>, ClickrError> {
        let ticket = clickr_core::parse_ticket(&ticket)
            .map_err(|err| ClickrError::InvalidTicket(format!("{err:#}")))?;
        let inner = Remote::connect(ticket).await.map_err(conn_err)?;
        Ok(Arc::new(Self { inner }))
    }

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
    pub fn receiver_name(&self) -> String {
        self.inner.receiver_id().fmt_short().to_string()
    }

    /// Closes the connection and releases network resources.
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
