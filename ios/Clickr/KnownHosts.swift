import ClickrKit
import Foundation

/// A computer this phone has paired with.
struct KnownHost: Codable, Identifiable, Equatable {
    /// The computer's endpoint id.
    let id: String
    /// The nickname the computer reported, or its short id before the first connection.
    var nickname: String
    /// The most recent pairing ticket, used to dial the computer.
    var ticket: String
    /// The `CONNECT` token the computer issued, presented on every connection.
    var token: Data?
    var lastUsed: Date
}

/// The known computers, persisted in `UserDefaults` and sorted by most recent use.
struct KnownHosts {
    private static let key = "knownHosts"
    /// Where version 1 of the app kept its single paired computer.
    private static let legacyTicketKey = "pairingTicket"

    private(set) var hosts: [KnownHost] = []

    static func load() -> KnownHosts {
        var store = KnownHosts()
        let defaults = UserDefaults.standard
        if let data = defaults.data(forKey: key),
           let hosts = try? JSONDecoder().decode([KnownHost].self, from: data) {
            store.hosts = hosts
        }
        if let legacy = defaults.string(forKey: legacyTicketKey) {
            store.add(ticket: legacy)
            defaults.removeObject(forKey: legacyTicketKey)
        }
        return store
    }

    func host(id: String) -> KnownHost? {
        hosts.first { $0.id == id }
    }

    /// Adds the computer named by `ticket`, or refreshes its ticket if already known.
    /// Returns the entry, or nil if the ticket is malformed.
    @discardableResult
    mutating func add(ticket: String) -> KnownHost? {
        guard let id = try? ticketEndpointId(ticket: ticket) else { return nil }
        if var existing = host(id: id) {
            existing.ticket = ticket
            existing.lastUsed = Date()
            update(existing)
            return existing
        }
        let host = KnownHost(
            id: id,
            nickname: shortEndpointId(id: id),
            ticket: ticket,
            token: nil,
            lastUsed: Date()
        )
        hosts.append(host)
        save()
        return host
    }

    mutating func update(_ host: KnownHost) {
        guard let index = hosts.firstIndex(where: { $0.id == host.id }) else { return }
        hosts[index] = host
        save()
    }

    mutating func forget(id: String) {
        hosts.removeAll { $0.id == id }
        save()
    }

    private mutating func save() {
        hosts.sort { $0.lastUsed > $1.lastUsed }
        if let data = try? JSONEncoder().encode(hosts) {
            UserDefaults.standard.set(data, forKey: Self.key)
        }
    }
}
