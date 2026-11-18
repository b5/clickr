import ClickrKit
import Foundation
import UIKit

/// Owns the known computers, the phone's nickname, and the connection to the selected
/// computer.
@MainActor
final class RemoteModel: ObservableObject {
    enum Status: Equatable {
        case idle
        case connecting
        /// Connecting without a token, so the computer is asking its user.
        case waitingForApproval
        case connected
        case denied
        case revoked
        case failed(String)
    }

    @Published private(set) var status: Status = .idle
    @Published private(set) var hosts: [KnownHost] = []
    /// The computer the remote screen is showing, if any.
    @Published private(set) var selectedID: String?
    @Published private(set) var lastRoundTripMs: UInt32?
    @Published private(set) var nickname: String

    var selected: KnownHost? { selectedID.flatMap { store.host(id: $0) } }

    /// Remembers which computer was open, so relaunching returns to it.
    private static let openHostKey = "openHostID"

    private var store = KnownHosts.load()
    /// The app's single endpoint, bound on first use and kept for the life of the process.
    /// Its Bluetooth transport can't be shut down, so it must never be bound twice.
    private var node: ClickrNode?
    private var remote: ClickrRemote?
    /// This install's identity; the same key is used for every connection.
    private let secretKey = DeviceKey.load()
    /// The tail of the queue of in-flight operations; chaining keeps presses in order.
    private var tail: Task<Void, Never>?
    private let impact = UIImpactFeedbackGenerator(style: .medium)
    private let notify = UINotificationFeedbackGenerator()

    init() {
        nickname = PhoneProfile.nickname()
        hosts = store.hosts
        if let id = UserDefaults.standard.string(forKey: Self.openHostKey),
           store.host(id: id) != nil {
            open(id: id)
        }
    }

    // MARK: Known computers

    /// Saves the computer named by a ticket and opens it. Returns false if the ticket is
    /// malformed.
    @discardableResult
    func pair(with rawTicket: String) -> Bool {
        let trimmed = rawTicket.trimmingCharacters(in: .whitespacesAndNewlines)
        guard isValidTicket(ticket: trimmed), let host = store.add(ticket: trimmed) else {
            return false
        }
        hosts = store.hosts
        open(id: host.id)
        return true
    }

    /// Shows the remote for a known computer and connects to it.
    func open(id: String) {
        selectedID = id
        UserDefaults.standard.set(id, forKey: Self.openHostKey)
        lastRoundTripMs = nil
        connect()
    }

    /// Returns to the list of computers, closing the connection.
    func close() {
        disconnect()
        selectedID = nil
        UserDefaults.standard.removeObject(forKey: Self.openHostKey)
        status = .idle
    }

    /// Forgets a computer, including its token.
    func forget(id: String) {
        if selectedID == id { close() }
        store.forget(id: id)
        hosts = store.hosts
    }

    func setNickname(_ newName: String) {
        let trimmed = newName.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return }
        PhoneProfile.setNickname(trimmed)
        nickname = trimmed
    }

    // MARK: Connection

    /// Connects to the selected computer, closing any existing connection first.
    func connect() {
        guard selectedID != nil else { return }
        enqueue { [weak self] in
            guard let self else { return }
            // Close inside the queue so overlapping reconnects can't leave a connection
            // open.
            if let old = self.remote {
                self.remote = nil
                await old.close()
            }
            await self.establish()
        }
    }

    func next() { press(forward: true) }
    func previous() { press(forward: false) }

    /// Called when the app returns to the foreground; connections usually die while
    /// iOS has the app suspended.
    func resume() {
        guard selectedID != nil else { return }
        enqueue { [weak self] in
            guard let self else { return }
            if let remote = self.remote {
                do {
                    try await remote.ensureConnected()
                    self.status = .connected
                } catch {
                    self.fail(error)
                }
            } else {
                await self.establish()
            }
        }
    }

    func suspend() {
        disconnect()
        status = .idle
    }

    private func press(forward: Bool) {
        impact.impactOccurred()
        enqueue { [weak self] in
            guard let self else { return }
            if self.remote == nil {
                await self.establish()
            }
            guard let remote = self.remote else {
                self.notify.notificationOccurred(.error)
                return
            }
            do {
                let rtt = forward ? try await remote.nextSlide() : try await remote.previousSlide()
                self.lastRoundTripMs = rtt
                self.status = .connected
            } catch {
                self.notify.notificationOccurred(.error)
                self.fail(error)
            }
        }
    }

    private func establish() async {
        guard let host = selected else { return }
        status = host.token == nil ? .waitingForApproval : .connecting
        do {
            let node = try await boundNode()
            let remote = try await node.connect(
                ticket: host.ticket, nickname: nickname, token: host.token
            )
            // The user may have closed or switched computers while this was connecting.
            guard selectedID == host.id else {
                await remote.close()
                return
            }
            self.remote = remote
            record(remote, for: host)
            status = .connected
        } catch {
            remote = nil
            fail(error)
        }
    }

    /// Saves the nickname and token the computer sent.
    private func record(_ remote: ClickrRemote, for host: KnownHost) {
        var updated = host
        updated.nickname = remote.receiverNickname()
        if let token = remote.token() {
            updated.token = token
        }
        updated.lastUsed = Date()
        store.update(updated)
        hosts = store.hosts
    }

    private func fail(_ error: Error) {
        switch error {
        case ClickrError.Denied:
            status = .denied
        case ClickrError.Revoked:
            status = .revoked
        case let ClickrError.InvalidTicket(message),
             let ClickrError.InvalidSecretKey(message),
             let ClickrError.Connection(message):
            status = .failed(message)
        default:
            status = .failed(error.localizedDescription)
        }
    }

    /// Returns the app's node, binding it on first use. Only called from queued operations,
    /// so two binds can't race.
    private func boundNode() async throws -> ClickrNode {
        if let node { return node }
        let node = try await ClickrNode.bind(secretKey: secretKey)
        self.node = node
        return node
    }

    private func disconnect() {
        guard let remote else { return }
        self.remote = nil
        Task.detached { await remote.close() }
    }

    private func enqueue(_ op: @escaping @MainActor () async -> Void) {
        let previous = tail
        tail = Task { @MainActor in
            await previous?.value
            await op()
        }
    }
}
