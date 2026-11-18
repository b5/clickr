//! RCAN capability tokens that grant a remote access to a receiver.
//!
//! When the user allows a new remote, the receiver issues it a [`Cap::Connect`] token
//! signed with the receiver's key. The remote stores the token and presents it on every
//! later connection, so it is admitted without asking again.

use std::fmt;

use iroh::{EndpointId, SecretKey};
use n0_error::{Result, anyerr};
use rcan::{Authorizer, Expires, Rcan};
use serde::{Deserialize, Serialize};

/// A capability a receiver grants to a remote.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Cap {
    /// Allows connecting to the receiver with this endpoint id and sending commands.
    Connect(EndpointId),
}

impl fmt::Display for Cap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Cap::Connect(id) => write!(f, "CONNECT: {id}"),
        }
    }
}

impl rcan::Capability for Cap {
    fn permits(&self, other: &Self) -> bool {
        self == other
    }
}

/// Issues a token that lets `remote` connect to the receiver identified by `receiver`.
///
/// The token never expires; access is withdrawn by revoking the remote on the receiver.
pub(crate) fn issue_connect(receiver: &SecretKey, remote: EndpointId) -> Vec<u8> {
    let signing_key = ed25519_dalek::SigningKey::from_bytes(&receiver.to_bytes());
    Rcan::issuing_builder(
        &signing_key,
        remote.as_verifying_key(),
        Cap::Connect(receiver.public()),
    )
    .sign(Expires::Never)
    .encode()
}

/// Checks that `token` lets `remote` connect to `receiver`.
///
/// `remote` must be the authenticated peer of the connection the token arrived on, so a
/// token copied to another device is useless.
///
/// # Errors
///
/// Returns an error if the token does not decode, has a bad signature, was issued by a
/// different receiver, was issued to a different remote, or grants a different
/// capability.
pub(crate) fn verify_connect(receiver: EndpointId, remote: EndpointId, token: &[u8]) -> Result<()> {
    // `Rcan::decode` checks the signature. rcan reports errors as `anyhow::Error`, which
    // does not implement `std::error::Error`, so they are converted by message.
    let rcan = Rcan::<Cap>::decode(token).map_err(|err| anyerr!("invalid token: {err:#}"))?;
    Authorizer::new(receiver.as_verifying_key())
        .check_invocation_from(remote.as_verifying_key(), Cap::Connect(receiver), &[&rcan])
        .map_err(|err| anyerr!("token does not grant access: {err:#}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn issued_token_verifies() {
        let receiver = SecretKey::generate();
        let remote = SecretKey::generate().public();
        let token = issue_connect(&receiver, remote);
        verify_connect(receiver.public(), remote, &token).unwrap();
    }

    #[test]
    fn token_rejected_for_other_remote() {
        let receiver = SecretKey::generate();
        let token = issue_connect(&receiver, SecretKey::generate().public());
        let thief = SecretKey::generate().public();
        assert!(verify_connect(receiver.public(), thief, &token).is_err());
    }

    #[test]
    fn token_rejected_by_other_receiver() {
        let remote = SecretKey::generate().public();
        let token = issue_connect(&SecretKey::generate(), remote);
        let other = SecretKey::generate().public();
        assert!(verify_connect(other, remote, &token).is_err());
    }

    #[test]
    fn tampered_token_rejected() {
        let receiver = SecretKey::generate();
        let remote = SecretKey::generate().public();
        let mut token = issue_connect(&receiver, remote);
        let last = token.len() - 1;
        token[last] ^= 0x01;
        assert!(verify_connect(receiver.public(), remote, &token).is_err());
    }

    #[test]
    fn garbage_token_rejected() {
        let receiver = SecretKey::generate().public();
        let remote = SecretKey::generate().public();
        assert!(verify_connect(receiver, remote, b"not a token").is_err());
    }

    #[test]
    fn cap_displays_as_connect() {
        let id = SecretKey::generate().public();
        assert_eq!(Cap::Connect(id).to_string(), format!("CONNECT: {id}"));
    }
}
