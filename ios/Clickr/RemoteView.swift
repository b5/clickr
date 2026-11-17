import SwiftUI

/// The remote itself: a large forward button and a smaller back button.
struct RemoteView: View {
    @EnvironmentObject private var model: RemoteModel
    @State private var confirmUnpair = false

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
        .confirmationDialog("Forget this computer?", isPresented: $confirmUnpair) {
            Button("Forget", role: .destructive) { model.unpair() }
        }
    }

    private var header: some View {
        HStack {
            Circle()
                .fill(statusColor)
                .frame(width: 10, height: 10)
            Text(statusText)
                .font(.subheadline)
                .lineLimit(1)
            Spacer()
            if case .failed = model.status {
                Button("Retry") { model.connect() }
                    .buttonStyle(.bordered)
            }
            Menu {
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
        case let .connected(receiver):
            if let rtt = model.lastRoundTripMs {
                return "\(receiver) - \(rtt) ms"
            }
            return "Connected to \(receiver)"
        case let .failed(message): return message
        }
    }

    private var statusColor: Color {
        switch model.status {
        case .idle: return .gray
        case .connecting: return .yellow
        case .connected: return .green
        case .failed: return .red
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
