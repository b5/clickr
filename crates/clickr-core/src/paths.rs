//! Which network paths a connection uses, for display.

use std::{collections::BTreeSet, fmt, time::Duration};

use iroh::{TransportAddr, endpoint::Connection};

/// The kind of network path a connection runs over.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Transport {
    /// Bluetooth Low Energy, through iroh-ble-transport.
    Bluetooth,
    /// A direct IPv4 path.
    Ipv4,
    /// A direct IPv6 path.
    Ipv6,
    /// Forwarded by an iroh relay server.
    Relay,
}

impl Transport {
    /// Classifies a path's remote address.
    ///
    /// Custom addresses count as Bluetooth: it is the only custom transport clickr
    /// registers.
    fn of(addr: &TransportAddr) -> Option<Self> {
        match addr {
            TransportAddr::Relay(_) => Some(Self::Relay),
            TransportAddr::Ip(addr) if addr.ip().to_canonical().is_ipv4() => Some(Self::Ipv4),
            TransportAddr::Ip(_) => Some(Self::Ipv6),
            TransportAddr::Custom(_) => Some(Self::Bluetooth),
            _ => None,
        }
    }
}

impl fmt::Display for Transport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Transport::Bluetooth => "Bluetooth",
            Transport::Ipv4 => "IPv4",
            Transport::Ipv6 => "IPv6",
            Transport::Relay => "relay",
        })
    }
}

/// The open paths of a connection and the one it currently sends on.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Paths {
    /// The path iroh selected for sending, if any.
    pub selected: Option<Transport>,
    /// Every open path, including the selected one.
    pub open: BTreeSet<Transport>,
}

impl Paths {
    /// Reads the current paths of `connection`.
    pub fn of(connection: &Connection) -> Self {
        let mut paths = Self::default();
        for path in &connection.paths() {
            let Some(transport) = Transport::of(path.remote_addr()) else {
                continue;
            };
            paths.open.insert(transport);
            if path.is_selected() {
                paths.selected = Some(transport);
            }
        }
        paths
    }

    /// Returns the open paths other than the selected one.
    pub fn standby(&self) -> impl Iterator<Item = Transport> + '_ {
        self.open
            .iter()
            .copied()
            .filter(|transport| Some(*transport) != self.selected)
    }
}

/// A connection's paths and the round-trip time of the one in use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    /// The open and selected paths.
    pub paths: Paths,
    /// The round-trip time of the selected path, as estimated by QUIC.
    pub rtt: Option<Duration>,
}

impl Link {
    /// Reads the current paths and latency of `connection`.
    pub fn of(connection: &Connection) -> Self {
        let rtt = connection
            .paths()
            .iter()
            .find(|path| path.is_selected())
            .map(|path| path.rtt());
        Self {
            paths: Paths::of(connection),
            rtt,
        }
    }
}

impl fmt::Display for Paths {
    /// Formats as `via IPv4 (also Bluetooth, relay)`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.selected {
            Some(selected) => write!(f, "via {selected}")?,
            None => f.write_str("no path selected")?,
        }
        let standby: Vec<String> = self.standby().map(|t| t.to_string()).collect();
        if !standby.is_empty() {
            write!(f, " (also {})", standby.join(", "))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_lists_standby_paths() {
        let paths = Paths {
            selected: Some(Transport::Ipv4),
            open: [Transport::Ipv4, Transport::Bluetooth, Transport::Relay].into(),
        };
        assert_eq!(paths.to_string(), "via IPv4 (also Bluetooth, relay)");
    }

    #[test]
    fn display_without_standby_paths() {
        let paths = Paths {
            selected: Some(Transport::Bluetooth),
            open: [Transport::Bluetooth].into(),
        };
        assert_eq!(paths.to_string(), "via Bluetooth");
    }

    #[test]
    fn mapped_ipv4_counts_as_ipv4() {
        let addr: std::net::SocketAddr = "[::ffff:192.168.1.2]:1234".parse().unwrap();
        assert_eq!(
            Transport::of(&TransportAddr::Ip(addr)),
            Some(Transport::Ipv4)
        );
    }
}
