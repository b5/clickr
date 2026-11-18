//! Shared protocol and networking for the clickr presentation remote.
//!
//! Two roles exist:
//!
//! - A [`Receiver`] runs on the computer showing the slides. It binds an iroh endpoint,
//!   hands out an [`EndpointTicket`] for pairing, and forwards every [`Command`] it
//!   receives to a [`ReceiverHandler`].
//! - A [`Remote`] runs on the phone. It dials the receiver from a ticket and sends
//!   commands over a single long-lived bidirectional stream.
//!
//! Both roles bind their endpoint with [`bind_endpoint`], which connects over the internet
//! and, when available, Bluetooth Low Energy.
//!
//! # Wire protocol
//!
//! The remote opens one bidirectional stream per connection and starts with a handshake:
//! it sends its nickname and, if it has one, the [`Cap::Connect`] token the receiver
//! issued earlier. The receiver answers with its own nickname and either lets the remote
//! in (issuing a token if the user just allowed it), or turns it away as denied or
//! revoked. Handshake messages are postcard-encoded with a u16 little-endian length
//! prefix.
//!
//! After that, each command is one byte (see [`Command`]); the receiver answers each byte
//! with one acknowledgement byte, which lets the remote measure round-trip time and notice
//! dead connections quickly.

use std::str::FromStr;

pub use iroh::{Endpoint, EndpointId, SecretKey};
pub use iroh_tickets::endpoint::EndpointTicket;
use n0_error::{Result, StdResultExt};

mod capability;
mod endpoint;
mod paths;
mod receiver;
mod remote;
mod wire;

pub use self::{
    capability::Cap,
    endpoint::{Adapter, Bluetooth, BluetoothLink, BluetoothStatus, bind_endpoint},
    paths::{Link, Paths, Transport},
    receiver::{Receiver, ReceiverHandler},
    remote::{AccessError, Remote},
};

/// The ALPN identifying version 1 of the clickr protocol.
pub const ALPN: &[u8] = b"n0/clickr/1";

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
    use std::{
        collections::{BTreeMap, BTreeSet},
        sync::{
            Arc, Mutex,
            atomic::{AtomicBool, AtomicUsize, Ordering},
        },
        time::Duration,
    };

    use super::*;

    /// A receiver handler that records what happened and answers prompts with `allow`.
    #[derive(Debug)]
    struct Recorder {
        allow: AtomicBool,
        prompts: AtomicUsize,
        revoked: Mutex<BTreeSet<EndpointId>>,
        connected: Mutex<BTreeMap<EndpointId, String>>,
        connections: AtomicUsize,
        paths: Mutex<Option<Paths>>,
        seen: Mutex<Vec<Command>>,
        /// Makes the next command block the handler this long, like a stuck receiver.
        stall: Mutex<Option<Duration>>,
    }

    impl Recorder {
        fn new(allow: bool) -> Arc<Self> {
            Arc::new(Self {
                allow: AtomicBool::new(allow),
                prompts: AtomicUsize::new(0),
                revoked: Mutex::default(),
                connected: Mutex::default(),
                connections: AtomicUsize::new(0),
                paths: Mutex::default(),
                seen: Mutex::default(),
                stall: Mutex::default(),
            })
        }

        fn prompts(&self) -> usize {
            self.prompts.load(Ordering::SeqCst)
        }
    }

    impl ReceiverHandler for Arc<Recorder> {
        fn nickname(&self) -> String {
            "Studio Mac".into()
        }

        fn is_revoked(&self, remote: EndpointId) -> bool {
            self.revoked.lock().unwrap().contains(&remote)
        }

        async fn request_access(&self, _remote: EndpointId, _nickname: &str) -> bool {
            self.prompts.fetch_add(1, Ordering::SeqCst);
            self.allow.load(Ordering::SeqCst)
        }

        fn connected(&self, remote: EndpointId, nickname: &str) {
            self.connections.fetch_add(1, Ordering::SeqCst);
            self.connected
                .lock()
                .unwrap()
                .insert(remote, nickname.to_string());
        }

        fn disconnected(&self, remote: EndpointId) {
            self.connected.lock().unwrap().remove(&remote);
        }

        fn paths_changed(&self, _remote: EndpointId, paths: &Paths) {
            *self.paths.lock().unwrap() = Some(paths.clone());
        }

        fn handle(&self, _remote: EndpointId, command: Command) {
            self.seen.lock().unwrap().push(command);
            let stall = self.stall.lock().unwrap().take();
            if let Some(stall) = stall {
                std::thread::sleep(stall);
            }
        }
    }

    /// Connects a remote over loopback.
    async fn connect(
        endpoint: &Endpoint,
        receiver: &Receiver,
        token: Option<Vec<u8>>,
    ) -> Result<Remote> {
        Remote::connect(endpoint, receiver.ticket(), "Swift Otter".into(), token).await
    }

    fn access_error(err: &n0_error::AnyError) -> Option<AccessError> {
        err.downcast_ref::<AccessError>().copied()
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
        let recorder = Recorder::new(true);
        let receiver = Receiver::spawn_local(SecretKey::generate(), recorder.clone()).await?;
        let endpoint = Remote::bind_local().await?;
        let remote = connect(&endpoint, &receiver, None).await?;

        remote.send(Command::Next).await?;
        remote.send(Command::Next).await?;
        remote.send(Command::Previous).await?;
        assert_eq!(
            *recorder.seen.lock().unwrap(),
            vec![Command::Next, Command::Next, Command::Previous]
        );

        remote.close().await;
        endpoint.close().await;
        receiver.shutdown().await;
        Ok(())
    }

    #[tokio::test]
    async fn pairing_exchanges_nicknames_and_issues_token() -> Result<()> {
        let recorder = Recorder::new(true);
        let receiver = Receiver::spawn_local(SecretKey::generate(), recorder.clone()).await?;
        let endpoint = Remote::bind_local().await?;
        let remote = connect(&endpoint, &receiver, None).await?;

        assert_eq!(recorder.prompts(), 1);
        assert!(remote.token().is_some());
        assert_eq!(remote.receiver_nickname(), "Studio Mac");
        assert_eq!(
            recorder.connected.lock().unwrap().get(&endpoint.id()),
            Some(&"Swift Otter".to_string())
        );

        endpoint.close().await;
        receiver.shutdown().await;
        Ok(())
    }

    #[tokio::test]
    async fn token_skips_prompt_on_reconnect() -> Result<()> {
        let recorder = Recorder::new(true);
        let receiver = Receiver::spawn_local(SecretKey::generate(), recorder.clone()).await?;
        let endpoint = Remote::bind_local().await?;
        let token = connect(&endpoint, &receiver, None).await?.token();

        // Deny further prompts: a valid token must get in without one.
        recorder.allow.store(false, Ordering::SeqCst);
        let remote = connect(&endpoint, &receiver, token.clone()).await?;
        remote.send(Command::Next).await?;
        assert_eq!(recorder.prompts(), 1);
        assert_eq!(remote.token(), token);

        endpoint.close().await;
        receiver.shutdown().await;
        Ok(())
    }

    #[tokio::test]
    async fn denied_remote_gets_access_error() -> Result<()> {
        let recorder = Recorder::new(false);
        let receiver = Receiver::spawn_local(SecretKey::generate(), recorder.clone()).await?;
        let endpoint = Remote::bind_local().await?;

        let err = connect(&endpoint, &receiver, None).await.unwrap_err();
        assert_eq!(access_error(&err), Some(AccessError::Denied));
        assert!(recorder.seen.lock().unwrap().is_empty());

        endpoint.close().await;
        receiver.shutdown().await;
        Ok(())
    }

    #[tokio::test]
    async fn revocation_overrides_valid_token_until_removed() -> Result<()> {
        let recorder = Recorder::new(true);
        let receiver = Receiver::spawn_local(SecretKey::generate(), recorder.clone()).await?;
        let endpoint = Remote::bind_local().await?;
        let token = connect(&endpoint, &receiver, None).await?.token();

        recorder.revoked.lock().unwrap().insert(endpoint.id());
        let err = connect(&endpoint, &receiver, token.clone())
            .await
            .unwrap_err();
        assert_eq!(access_error(&err), Some(AccessError::Revoked));

        // Removing the revocation restores access with the same token, without a prompt.
        recorder.revoked.lock().unwrap().clear();
        recorder.allow.store(false, Ordering::SeqCst);
        connect(&endpoint, &receiver, token).await?;
        assert_eq!(recorder.prompts(), 1);

        endpoint.close().await;
        receiver.shutdown().await;
        Ok(())
    }

    #[tokio::test]
    async fn revoking_disconnects_live_remote() -> Result<()> {
        let recorder = Recorder::new(true);
        let receiver = Receiver::spawn_local(SecretKey::generate(), recorder.clone()).await?;
        let endpoint = Remote::bind_local().await?;
        let remote = connect(&endpoint, &receiver, None).await?;
        remote.send(Command::Next).await?;

        recorder.revoked.lock().unwrap().insert(endpoint.id());
        receiver.disconnect(endpoint.id());
        // The send notices the closed connection, reconnects and is turned away.
        let err = remote.send(Command::Next).await.unwrap_err();
        assert_eq!(access_error(&err), Some(AccessError::Revoked));

        endpoint.close().await;
        receiver.shutdown().await;
        Ok(())
    }

    #[tokio::test]
    async fn token_from_other_receiver_falls_back_to_prompt() -> Result<()> {
        let recorder = Recorder::new(true);
        let other = Receiver::spawn_local(SecretKey::generate(), recorder.clone()).await?;
        let receiver = Receiver::spawn_local(SecretKey::generate(), recorder.clone()).await?;
        let endpoint = Remote::bind_local().await?;
        let foreign = connect(&endpoint, &other, None).await?.token();

        recorder.allow.store(false, Ordering::SeqCst);
        let err = connect(&endpoint, &receiver, foreign).await.unwrap_err();
        assert_eq!(access_error(&err), Some(AccessError::Denied));
        assert_eq!(recorder.prompts(), 2);

        endpoint.close().await;
        receiver.shutdown().await;
        other.shutdown().await;
        Ok(())
    }

    #[tokio::test]
    async fn stolen_token_does_not_grant_access() -> Result<()> {
        let recorder = Recorder::new(true);
        let receiver = Receiver::spawn_local(SecretKey::generate(), recorder.clone()).await?;
        let owner = Remote::bind_local().await?;
        let token = connect(&owner, &receiver, None).await?.token();

        recorder.allow.store(false, Ordering::SeqCst);
        let thief = Remote::bind_local().await?;
        let err = connect(&thief, &receiver, token).await.unwrap_err();
        assert_eq!(access_error(&err), Some(AccessError::Denied));

        owner.close().await;
        thief.close().await;
        receiver.shutdown().await;
        Ok(())
    }

    #[tokio::test]
    async fn disconnect_is_reported() -> Result<()> {
        let recorder = Recorder::new(true);
        let receiver = Receiver::spawn_local(SecretKey::generate(), recorder.clone()).await?;
        let endpoint = Remote::bind_local().await?;
        let remote = connect(&endpoint, &receiver, None).await?;
        remote.send(Command::Next).await?;
        assert!(
            recorder
                .connected
                .lock()
                .unwrap()
                .contains_key(&endpoint.id())
        );

        remote.close().await;
        tokio::time::timeout(Duration::from_secs(5), async {
            while recorder
                .connected
                .lock()
                .unwrap()
                .contains_key(&endpoint.id())
            {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .std_context("disconnect not reported")?;

        endpoint.close().await;
        receiver.shutdown().await;
        Ok(())
    }

    #[tokio::test]
    async fn both_sides_see_the_transport() -> Result<()> {
        let recorder = Recorder::new(true);
        let receiver = Receiver::spawn_local(SecretKey::generate(), recorder.clone()).await?;
        let endpoint = Remote::bind_local().await?;
        let remote = connect(&endpoint, &receiver, None).await?;
        remote.send(Command::Next).await?;

        assert_eq!(
            remote.link().and_then(|link| link.paths.selected),
            Some(Transport::Ipv4)
        );
        tokio::time::timeout(Duration::from_secs(5), async {
            while recorder
                .paths
                .lock()
                .unwrap()
                .as_ref()
                .and_then(|p| p.selected)
                != Some(Transport::Ipv4)
            {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .std_context("receiver did not report the path")?;

        let link = receiver.link(endpoint.id()).std_context("no link")?;
        assert_eq!(link.paths.selected, Some(Transport::Ipv4));
        assert!(link.rtt.is_some());

        remote.close().await;
        assert_eq!(remote.link(), None);
        endpoint.close().await;
        receiver.shutdown().await;
        Ok(())
    }

    /// A receiver that stops answering must not wedge the remote: the timed-out send
    /// drops the connection, and the next send reconnects.
    #[tokio::test(flavor = "multi_thread")]
    async fn ack_timeout_drops_connection_and_next_send_reconnects() -> Result<()> {
        let recorder = Recorder::new(true);
        let receiver = Receiver::spawn_local(SecretKey::generate(), recorder.clone()).await?;
        let endpoint = Remote::bind_local().await?;
        let remote = connect(&endpoint, &receiver, None).await?;
        assert_eq!(recorder.connections.load(Ordering::SeqCst), 1);

        *recorder.stall.lock().unwrap() = Some(Duration::from_secs(6));
        assert!(remote.send(Command::Next).await.is_err());
        assert_eq!(remote.link(), None);

        remote.send(Command::Previous).await?;
        assert_eq!(recorder.connections.load(Ordering::SeqCst), 2);
        // The timed-out command reached the receiver once and was not resent.
        assert_eq!(
            *recorder.seen.lock().unwrap(),
            vec![Command::Next, Command::Previous]
        );

        endpoint.close().await;
        receiver.shutdown().await;
        Ok(())
    }

    /// Exercises the production configuration: n0 relays, address lookup, and a ticket
    /// that round-trips through its string form.
    #[tokio::test]
    #[ignore = "requires internet access"]
    async fn remote_drives_receiver_over_n0() -> Result<()> {
        let recorder = Recorder::new(true);
        let receiver = Receiver::spawn(SecretKey::generate(), recorder.clone()).await?;
        receiver.endpoint().online().await;
        let ticket = parse_ticket(&receiver.ticket().to_string())?;
        let (endpoint, _bluetooth) = bind_endpoint(SecretKey::generate()).await?;
        let remote = Remote::connect(&endpoint, ticket, "Swift Otter".into(), None).await?;
        remote.send(Command::Next).await?;
        assert_eq!(*recorder.seen.lock().unwrap(), vec![Command::Next]);
        remote.close().await;
        endpoint.close().await;
        receiver.shutdown().await;
        Ok(())
    }
}
