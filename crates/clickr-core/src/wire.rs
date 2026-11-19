//! Handshake messages exchanged at the start of every connection.

use iroh::endpoint::{RecvStream, SendStream};
use n0_error::{Result, StdResultExt, anyerr};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

/// Upper bound for one handshake message, so a peer cannot make us allocate freely.
const MAX_MESSAGE_LEN: usize = 4096;

/// The first message on a connection, sent by the remote.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Hello {
    /// The remote's nickname, shown to the user on the receiver.
    pub(crate) nickname: String,
    /// A [`Cap::Connect`] token from an earlier pairing, if the remote has one.
    ///
    /// [`Cap::Connect`]: crate::Cap::Connect
    pub(crate) token: Option<Vec<u8>>,
}

/// The receiver's answer to a [`Hello`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Welcome {
    /// The remote may send commands.
    Granted {
        /// The receiver's nickname.
        nickname: String,
        /// A newly issued token for the remote to store, set when the user just allowed
        /// it. `None` when the remote's existing token was accepted.
        token: Option<Vec<u8>>,
    },
    /// The user declined the remote.
    Denied,
    /// The receiver revoked this remote's access.
    Revoked,
}

/// Writes `message` as a u16 little-endian length followed by its postcard encoding.
pub(crate) async fn write_message(send: &mut SendStream, message: &impl Serialize) -> Result<()> {
    let bytes = postcard::to_stdvec(message).std_context("failed to encode message")?;
    let len = u16::try_from(bytes.len())
        .ok()
        .filter(|len| usize::from(*len) <= MAX_MESSAGE_LEN)
        .ok_or_else(|| anyerr!("message too large: {} bytes", bytes.len()))?;
    send.write_all(&len.to_le_bytes()).await.anyerr()?;
    send.write_all(&bytes).await.anyerr()?;
    Ok(())
}

/// Reads a message written by [`write_message`].
pub(crate) async fn read_message<T: DeserializeOwned>(recv: &mut RecvStream) -> Result<T> {
    let mut len = [0u8; 2];
    recv.read_exact(&mut len).await.anyerr()?;
    let len = usize::from(u16::from_le_bytes(len));
    if len > MAX_MESSAGE_LEN {
        return Err(anyerr!("message too large: {len} bytes"));
    }
    let mut bytes = vec![0u8; len];
    recv.read_exact(&mut bytes).await.anyerr()?;
    postcard::from_bytes(&bytes).std_context("failed to decode message")
}
