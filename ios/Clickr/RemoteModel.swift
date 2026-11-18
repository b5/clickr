import ClickrKit
import Foundation
import UIKit

/// Owns the connection to the computer and the saved pairing ticket.
@MainActor
final class RemoteModel: ObservableObject {
    enum Status: Equatable {
        case idle
        case connecting
        case connected(receiver: String)
        case failed(String)
    }

    @Published private(set) var status: Status = .idle
    @Published private(set) var ticket: String?
    @Published private(set) var lastRoundTripMs: UInt32?

    private static let ticketKey = "pairingTicket"

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
        ticket = UserDefaults.standard.string(forKey: Self.ticketKey)
        if ticket != nil {
            connect()
        }
    }

    /// Saves a ticket and connects to it. Returns false if the ticket is malformed.
    @discardableResult
    func pair(with rawTicket: String) -> Bool {
        let trimmed = rawTicket.trimmingCharacters(in: .whitespacesAndNewlines)
        guard isValidTicket(ticket: trimmed) else { return false }
        ticket = trimmed
        UserDefaults.standard.set(trimmed, forKey: Self.ticketKey)
        disconnect()
        connect()
        return true
    }

    /// Forgets the saved computer.
    func unpair() {
        disconnect()
        ticket = nil
        UserDefaults.standard.removeObject(forKey: Self.ticketKey)
        status = .idle
    }

    /// Connects to the paired computer, closing any existing connection first.
    func connect() {
        guard let ticket else { return }
        enqueue { [weak self] in
            guard let self else { return }
            // Close inside the queue so overlapping reconnects can't leave a connection
            // open, and wait for it so two endpoints with the same key never overlap.
            if let old = self.remote {
                self.remote = nil
                await old.close()
            }
            await self.establish(ticket: ticket)
        }
    }

    func next() { press(forward: true) }
    func previous() { press(forward: false) }

    /// Called when the app returns to the foreground; connections usually die while
    /// iOS has the app suspended.
    func resume() {
        guard ticket != nil else { return }
        enqueue { [weak self] in
            guard let self, let ticket = self.ticket else { return }
            if let remote = self.remote {
                do {
                    try await remote.ensureConnected()
                    self.status = .connected(receiver: remote.receiverName())
                } catch {
                    self.status = .failed(Self.describe(error))
                }
            } else {
                await self.establish(ticket: ticket)
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
            if self.remote == nil, let ticket = self.ticket {
                await self.establish(ticket: ticket)
            }
            guard let remote = self.remote else {
                self.notify.notificationOccurred(.error)
                return
            }
            do {
                let rtt = forward ? try await remote.nextSlide() : try await remote.previousSlide()
                self.lastRoundTripMs = rtt
                self.status = .connected(receiver: remote.receiverName())
            } catch {
                self.notify.notificationOccurred(.error)
                self.status = .failed(Self.describe(error))
            }
        }
    }

    private func establish(ticket: String) async {
        status = .connecting
        do {
            let node = try await boundNode()
            let remote = try await node.connect(ticket: ticket)
            self.remote = remote
            status = .connected(receiver: remote.receiverName())
        } catch {
            remote = nil
            status = .failed(Self.describe(error))
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

    private static func describe(_ error: Error) -> String {
        switch error {
        case let ClickrError.InvalidTicket(message),
             let ClickrError.InvalidSecretKey(message),
             let ClickrError.Connection(message):
            return message
        default:
            return error.localizedDescription
        }
    }
}
