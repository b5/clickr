//! Endpoint setup shared by the receiver and the remote.

use iroh::{Endpoint, SecretKey, endpoint::presets};
use iroh_ble_transport::BleTransport;
use n0_error::Result;
use tracing::{info, warn};

/// Binds an endpoint that reaches peers over the internet and, when available, Bluetooth
/// Low Energy.
///
/// The endpoint uses the n0 relays and address lookup, plus the BLE transport so two
/// nearby devices can talk without a shared network. If Bluetooth is unavailable (no
/// adapter, permission denied, or powered off), the endpoint binds without it and logs a
/// warning.
///
/// The BLE transport keeps scanning and advertising for the rest of the process, even
/// after the endpoint is closed, so bind one endpoint per process and reuse it.
///
/// # Errors
///
/// Returns an error if the endpoint cannot bind.
pub async fn bind_endpoint(secret_key: SecretKey) -> Result<Endpoint> {
    let mut builder = Endpoint::builder(presets::N0).secret_key(secret_key.clone());
    match BleTransport::builder().build(secret_key.public()).await {
        Ok(ble) => {
            info!("Bluetooth transport enabled");
            builder = builder
                .hooks(ble.dedup_hook())
                .add_custom_transport(ble.as_custom_transport())
                .address_lookup(ble.address_lookup());
        }
        Err(err) => warn!("Bluetooth unavailable, using the internet only: {err:#}"),
    }
    Ok(builder.bind().await?)
}
