//! The receiving side of the protocol, run on the presenting computer.

use std::{fmt::Debug, future::Future, sync::Arc};

use iroh::{
    Endpoint, EndpointAddr, EndpointId, SecretKey, TransportAddr,
    endpoint::Connection,
    protocol::{AcceptError, ProtocolHandler, Router},
};
use iroh_tickets::endpoint::EndpointTicket;
use n0_error::{Result, StdResultExt};
use tracing::{debug, info, warn};

use crate::{ACK_OK, ACK_UNKNOWN, ALPN, Command, bind_endpoint};

/// Application callbacks invoked by a [`Receiver`].
pub trait CommandHandler: Debug + Send + Sync + 'static {
    /// Decides whether a remote may send commands.
    ///
    /// Called once per incoming connection, before any command is read. Returning `false`
    /// closes the connection.
    fn authorize(&self, remote: EndpointId) -> impl Future<Output = bool> + Send;

    /// Handles a single command from an authorized remote.
    fn handle(&self, remote: EndpointId, command: Command);
}

/// Accepts connections from remotes and dispatches their commands.
#[derive(Debug)]
pub struct Receiver {
    router: Router,
}

impl Receiver {
    /// Binds an endpoint with the given identity and starts accepting remotes.
    ///
    /// The endpoint is set up by [`bind_endpoint`]: it uses the n0 relays and address
    /// lookup, so a ticket stays usable across network changes as long as the secret key
    /// stays the same, and accepts remotes over Bluetooth when available.
    pub async fn spawn(secret_key: SecretKey, handler: impl CommandHandler) -> Result<Self> {
        let endpoint = bind_endpoint(secret_key).await?;
        Ok(Self::with_endpoint(endpoint, handler))
    }

    /// Binds a loopback-only endpoint without relays or address lookup, for tests.
    #[cfg(test)]
    pub(crate) async fn spawn_local(
        secret_key: SecretKey,
        handler: impl CommandHandler,
    ) -> Result<Self> {
        let endpoint = Endpoint::builder(iroh::endpoint::presets::Minimal)
            .secret_key(secret_key)
            .bind_addr("127.0.0.1:0")?
            .bind()
            .await?;
        Ok(Self::with_endpoint(endpoint, handler))
    }

    fn with_endpoint(endpoint: Endpoint, handler: impl CommandHandler) -> Self {
        let router = Router::builder(endpoint)
            .accept(ALPN, Handler(Arc::new(handler)))
            .spawn();
        Self { router }
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

    /// Stops accepting connections and closes the endpoint gracefully.
    pub async fn shutdown(self) {
        if let Err(err) = self.router.shutdown().await {
            warn!("router shutdown failed: {err:#}");
        }
    }
}

#[derive(Debug)]
struct Handler<H>(Arc<H>);

impl<H: CommandHandler> ProtocolHandler for Handler<H> {
    async fn accept(&self, connection: Connection) -> Result<(), AcceptError> {
        let remote = connection.remote_id();
        if !self.0.authorize(remote).await {
            info!(remote = %remote.fmt_short(), "rejected remote");
            connection.close(1u32.into(), b"not authorized");
            return Ok(());
        }
        info!(remote = %remote.fmt_short(), "remote connected");
        if let Err(err) = serve(&connection, &*self.0, remote).await {
            debug!(remote = %remote.fmt_short(), "connection ended: {err:#}");
        }
        info!(remote = %remote.fmt_short(), "remote disconnected");
        Ok(())
    }
}

async fn serve(
    connection: &Connection,
    handler: &impl CommandHandler,
    remote: EndpointId,
) -> Result<()> {
    let (mut send, mut recv) = connection.accept_bi().await.anyerr()?;
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
