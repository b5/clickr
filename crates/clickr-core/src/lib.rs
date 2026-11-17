//! Shared protocol and networking for the clickr presentation remote.
//!
//! Two roles exist:
//!
//! - A [`Receiver`] runs on the computer showing the slides. It binds an iroh endpoint,
//!   hands out an [`EndpointTicket`] for pairing, and forwards every [`Command`] it
//!   receives to a [`CommandHandler`].
//! - A [`Remote`] runs on the phone. It dials the receiver from a ticket and sends
//!   commands over a single long-lived bidirectional stream.
//!
//! # Wire protocol
//!
//! The remote opens one bidirectional stream per connection. Each command is one byte
//! (see [`Command`]); the receiver answers each byte with one acknowledgement byte, which
//! lets the remote measure round-trip time and notice dead connections quickly.

use std::str::FromStr;

pub use iroh::{EndpointId, SecretKey};
pub use iroh_tickets::endpoint::EndpointTicket;
use n0_error::{Result, StdResultExt};

mod receiver;
mod remote;

pub use self::{
    receiver::{CommandHandler, Receiver},
    remote::Remote,
};

/// The ALPN identifying version 0 of the clickr protocol.
pub const ALPN: &[u8] = b"n0/clickr/0";

/// Acknowledgement byte sent by the receiver after handling a command.
const ACK_OK: u8 = 0x00;
/// Acknowledgement byte sent by the receiver for an unknown command byte.
const ACK_UNKNOWN: u8 = 0xFF;

/// A command sent from the remote to the receiver.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Command {
    /// Goes back one slide, mapped to the left arrow key.
    Previous,
    /// Advances one slide, mapped to the right arrow key.
    Next,
}

impl Command {
    /// Returns the wire encoding of this command.
    pub fn to_byte(self) -> u8 {
        match self {
            Command::Previous => 0x01,
            Command::Next => 0x02,
        }
    }

    /// Parses a command from its wire encoding, returning `None` for unknown bytes.
    pub fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            0x01 => Some(Command::Previous),
            0x02 => Some(Command::Next),
            _ => None,
        }
    }
}

/// Parses a pairing ticket, ignoring surrounding whitespace.
///
/// # Errors
///
/// Returns an error if the string is not a valid [`EndpointTicket`].
pub fn parse_ticket(ticket: &str) -> Result<EndpointTicket> {
    EndpointTicket::from_str(ticket.trim()).std_context("invalid pairing ticket")
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    #[derive(Debug, Default)]
    struct Recorder {
        seen: Mutex<Vec<Command>>,
        deny: bool,
    }

    impl CommandHandler for Arc<Recorder> {
        async fn authorize(&self, _remote: EndpointId) -> bool {
            !self.deny
        }

        fn handle(&self, _remote: EndpointId, command: Command) {
            self.seen.lock().unwrap().push(command);
        }
    }

    #[test]
    fn command_roundtrip() {
        for cmd in [Command::Previous, Command::Next] {
            assert_eq!(Command::from_byte(cmd.to_byte()), Some(cmd));
        }
        assert_eq!(Command::from_byte(0x42), None);
    }

    #[tokio::test]
    async fn remote_drives_receiver() -> Result<()> {
        let recorder = Arc::new(Recorder::default());
        let receiver = Receiver::spawn_local(SecretKey::generate(), recorder.clone()).await?;
        let ticket = receiver.ticket();
        let remote = Remote::connect_local(ticket).await?;

        remote.send(Command::Next).await?;
        remote.send(Command::Next).await?;
        remote.send(Command::Previous).await?;
        assert_eq!(
            *recorder.seen.lock().unwrap(),
            vec![Command::Next, Command::Next, Command::Previous]
        );

        remote.close().await;
        receiver.shutdown().await;
        Ok(())
    }

    /// Exercises the production configuration: n0 relays, address lookup, and a ticket
    /// that round-trips through its string form.
    #[tokio::test]
    #[ignore = "requires internet access"]
    async fn remote_drives_receiver_over_n0() -> Result<()> {
        let recorder = Arc::new(Recorder::default());
        let receiver = Receiver::spawn(SecretKey::generate(), recorder.clone()).await?;
        receiver.endpoint().online().await;
        let ticket = parse_ticket(&receiver.ticket().to_string())?;
        let remote = Remote::connect(ticket, SecretKey::generate()).await?;
        remote.send(Command::Next).await?;
        assert_eq!(*recorder.seen.lock().unwrap(), vec![Command::Next]);
        remote.close().await;
        receiver.shutdown().await;
        Ok(())
    }

    #[tokio::test]
    async fn unauthorized_remote_is_rejected() -> Result<()> {
        let recorder = Arc::new(Recorder {
            deny: true,
            ..Default::default()
        });
        let receiver = Receiver::spawn_local(SecretKey::generate(), recorder.clone()).await?;
        let res = Remote::connect_local(receiver.ticket()).await;
        // The connection may be closed before or after the handshake completes, so
        // either connecting or the first send fails.
        let failed = match res {
            Err(_) => true,
            Ok(remote) => remote.send(Command::Next).await.is_err(),
        };
        assert!(failed);
        assert!(recorder.seen.lock().unwrap().is_empty());
        receiver.shutdown().await;
        Ok(())
    }
}
