//! Endpoint setup shared by the receiver and the remote.

use std::{fmt, sync::Arc};

use iroh::{Endpoint, EndpointId, SecretKey, endpoint::presets};
use iroh_ble_transport::{BleAdapterState, BlePeerPhase, BleTransport, ConnectPath};
use n0_error::Result;
use tracing::{info, warn};

/// Binds an endpoint that reaches peers over the internet and, when available, Bluetooth
/// Low Energy.
///
/// The endpoint uses the n0 relays and address lookup, plus the BLE transport so two
/// nearby devices can talk without a shared network. If Bluetooth is unavailable (no
/// adapter, permission denied, or powered off), the endpoint binds without it and logs a
/// warning; the returned [`Bluetooth`] says why.
///
/// The BLE transport keeps scanning and advertising for the rest of the process, even
/// after the endpoint is closed, so bind one endpoint per process and reuse it.
///
/// # Errors
///
/// Returns an error if the endpoint cannot bind.
pub async fn bind_endpoint(secret_key: SecretKey) -> Result<(Endpoint, Bluetooth)> {
    let mut builder = Endpoint::builder(presets::N0).secret_key(secret_key.clone());
    let bluetooth = match BleTransport::builder().build(secret_key.public()).await {
        Ok(ble) => {
            info!("Bluetooth transport enabled");
            builder = builder
                .hooks(ble.dedup_hook())
                .add_custom_transport(ble.as_custom_transport())
                .address_lookup(ble.address_lookup());
            Bluetooth(Ok(ble))
        }
        Err(err) => {
            warn!("Bluetooth unavailable, using the internet only: {err:#}");
            Bluetooth::unavailable(format!("{err:#}"))
        }
    };
    Ok((builder.bind().await?, bluetooth))
}

/// The Bluetooth transport of an endpoint, for diagnostics.
#[derive(Debug, Clone)]
pub struct Bluetooth(std::result::Result<Arc<BleTransport>, String>);

impl Bluetooth {
    /// Returns a `Bluetooth` for an endpoint bound without it, with the reason.
    pub(crate) fn unavailable(reason: impl Into<String>) -> Self {
        Self(Err(reason.into()))
    }

    /// Reports the local adapter and what the transport knows about `peer`.
    pub fn status(&self, peer: EndpointId) -> BluetoothStatus {
        let ble = match &self.0 {
            Ok(ble) => ble,
            Err(reason) => {
                return BluetoothStatus {
                    adapter: Adapter::Unavailable(reason.clone()),
                    peer_seen: false,
                    link: None,
                };
            }
        };
        let adapter = match ble.adapter_state() {
            BleAdapterState::PoweredOn => Adapter::On,
            _ => Adapter::Off,
        };
        let device = ble.device_for_endpoint(&peer);
        let link = ble
            .snapshot_peers()
            .into_iter()
            .find(|info| {
                info.verified_endpoint == Some(peer) || Some(&info.device_id) == device.as_ref()
            })
            .map(|info| BluetoothLink {
                phase: phase_name(info.phase).to_string(),
                path: info.connect_path.map(|path| match path {
                    ConnectPath::L2cap => "L2CAP".to_string(),
                    ConnectPath::Gatt => "GATT".to_string(),
                }),
            });
        BluetoothStatus {
            adapter,
            peer_seen: ble.has_scan_hint_for_endpoint(&peer),
            link,
        }
    }
}

fn phase_name(phase: BlePeerPhase) -> &'static str {
    match phase {
        BlePeerPhase::Unknown => "unknown",
        BlePeerPhase::Discovered => "discovered",
        BlePeerPhase::PendingDial => "about to dial",
        BlePeerPhase::Connecting => "connecting",
        BlePeerPhase::Handshaking => "handshaking",
        BlePeerPhase::Connected => "connected",
        BlePeerPhase::Draining => "closing",
        BlePeerPhase::Reconnecting => "reconnecting",
        BlePeerPhase::Dead => "gone",
        BlePeerPhase::Restoring => "restoring",
        _ => "other",
    }
}

/// The local Bluetooth adapter as the transport sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Adapter {
    /// Powered on and usable.
    On,
    /// Powered off.
    Off,
    /// The transport could not start, with the reason, for example a denied permission.
    Unavailable(String),
}

/// A Bluetooth link to one peer, as the transport tracks it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BluetoothLink {
    /// Where the link is in its lifecycle, like "connecting" or "connected".
    pub phase: String,
    /// "L2CAP" or "GATT" once connected.
    pub path: Option<String>,
}

/// What the Bluetooth transport knows about one peer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BluetoothStatus {
    /// The local adapter.
    pub adapter: Adapter,
    /// Whether this device has seen the peer advertise since the transport started.
    pub peer_seen: bool,
    /// The link to the peer, if the transport has one in any phase.
    pub link: Option<BluetoothLink>,
}

impl fmt::Display for BluetoothStatus {
    /// Formats as `on, other device seen, link connected over L2CAP`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.adapter {
            Adapter::Unavailable(reason) => return write!(f, "unavailable: {reason}"),
            Adapter::Off => return f.write_str("off"),
            Adapter::On => f.write_str("on")?,
        }
        if self.peer_seen {
            f.write_str(", other device seen")?;
        } else {
            f.write_str(", other device not seen")?;
        }
        match &self.link {
            Some(BluetoothLink {
                phase,
                path: Some(path),
            }) => write!(f, ", link {phase} over {path}"),
            Some(BluetoothLink { phase, path: None }) => write!(f, ", link {phase}"),
            None => f.write_str(", no link"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_display() {
        let peer = SecretKey::generate().public();
        assert_eq!(
            Bluetooth::unavailable("permission denied")
                .status(peer)
                .to_string(),
            "unavailable: permission denied"
        );
        let status = BluetoothStatus {
            adapter: Adapter::On,
            peer_seen: true,
            link: Some(BluetoothLink {
                phase: "connected".into(),
                path: Some("L2CAP".into()),
            }),
        };
        assert_eq!(
            status.to_string(),
            "on, other device seen, link connected over L2CAP"
        );
    }
}
