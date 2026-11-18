import SwiftUI

/// The remote itself: a large forward button and a smaller back button.
struct RemoteView: View {
    @EnvironmentObject private var model: RemoteModel
    @State private var confirmUnpair = false
    @State private var showDetails = false

    var body: some View {
        VStack(spacing: 16) {
            header
            SlideButton(systemImage: "arrow.left", title: "Back", tint: .gray) {
                model.previous()
            }
            .frame(maxHeight: .infinity)
            SlideButton(systemImage: "arrow.right", title: "Next", tint: .accentColor) {
                model.next()
            }
            .frame(maxHeight: .infinity)
            .layoutPriority(1)
        }
        .padding()
        .sheet(isPresented: $showDetails) {
            ConnectionDetailsView()
        }
        .confirmationDialog("Forget this computer?", isPresented: $confirmUnpair) {
            Button("Forget", role: .destructive) {
                if let id = model.selectedID { model.forget(id: id) }
            }
        } message: {
            Text("You'll need to pair and be approved again to use it.")
        }
    }

    private var header: some View {
        HStack {
            Button {
                model.close()
            } label: {
                Image(systemName: "chevron.left")
                    .imageScale(.large)
            }
            .accessibilityLabel("Computers")
            Circle()
                .fill(statusColor)
                .frame(width: 10, height: 10)
            VStack(alignment: .leading, spacing: 0) {
                Text(model.selected?.nickname ?? "")
                    .font(.headline)
                    .lineLimit(1)
                Text(statusText)
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
            Spacer()
            if canRetry {
                Button("Retry") { model.connect() }
                    .buttonStyle(.bordered)
            }
            Menu {
                Button("Connection details") { showDetails = true }
                Button("Reconnect") { model.connect() }
                Button("Forget computer", role: .destructive) { confirmUnpair = true }
            } label: {
                Image(systemName: "ellipsis.circle")
                    .imageScale(.large)
            }
        }
    }

    private var statusText: String {
        switch model.status {
        case .idle: return "Not connected"
        case .connecting: return "Connecting..."
        case .waitingForApproval: return "Waiting for approval on the computer..."
        case .connected:
            var text = "Connected"
            if let transport = model.paths?.selected {
                text += " via \(transport)"
            }
            if let rtt = model.lastRoundTripMs {
                text += " - \(rtt) ms"
            }
            return text
        case .denied: return "The computer declined this phone"
        case .revoked: return "The computer revoked this phone's access"
        case let .failed(message): return message
        }
    }

    private var canRetry: Bool {
        switch model.status {
        case .denied, .revoked, .failed: return true
        default: return false
        }
    }

    private var statusColor: Color {
        switch model.status {
        case .idle: return .gray
        case .connecting, .waitingForApproval: return .yellow
        case .connected: return .green
        case .denied, .revoked, .failed: return .red
        }
    }
}

private struct SlideButton: View {
    let systemImage: String
    let title: String
    let tint: Color
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            VStack(spacing: 8) {
                Image(systemName: systemImage)
                    .font(.system(size: 64, weight: .bold))
                Text(title)
                    .font(.title2.weight(.semibold))
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .foregroundStyle(.white)
            .background(tint, in: RoundedRectangle(cornerRadius: 28, style: .continuous))
        }
        .buttonStyle(PressableStyle())
        .accessibilityLabel(title)
    }
}

private struct PressableStyle: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .scaleEffect(configuration.isPressed ? 0.97 : 1)
            .opacity(configuration.isPressed ? 0.85 : 1)
            .animation(.easeOut(duration: 0.08), value: configuration.isPressed)
    }
}
