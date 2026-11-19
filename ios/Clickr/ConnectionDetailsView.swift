import SwiftUI

/// The selected computer's endpoint id, the connection's network path and latency, and
/// what Bluetooth knows about the computer. Updates while open.
struct ConnectionDetailsView: View {
    @EnvironmentObject private var model: RemoteModel
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            List {
                Section("Computer") {
                    LabeledContent("Name", value: model.selected?.nickname ?? "-")
                    VStack(alignment: .leading, spacing: 4) {
                        Text("Endpoint ID")
                        Text(model.selected?.id ?? "-")
                            .font(.caption.monospaced())
                            .foregroundStyle(.secondary)
                            .textSelection(.enabled)
                    }
                }
                Section("Connection") {
                    LabeledContent("Using", value: model.paths?.selected ?? "not connected")
                    if let standby = model.paths?.standby, !standby.isEmpty {
                        LabeledContent("Also open", value: standby.joined(separator: ", "))
                    }
                    LabeledContent("Latency", value: model.paths?.rttMs.map { "\($0) ms" } ?? "-")
                }
                Section {
                    Text(model.bluetoothStatus ?? "not started yet")
                } header: {
                    Text("Bluetooth")
                } footer: {
                    Text("\"Other device seen\" means this phone has picked up the computer's Bluetooth advertisement.")
                }
            }
            .navigationTitle("Connection")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Done") { dismiss() }
                }
            }
        }
    }
}
