import SwiftUI

/// Pairs with a computer by scanning or pasting its ticket. Shown as a sheet.
struct PairingView: View {
    @EnvironmentObject private var model: RemoteModel
    @Environment(\.dismiss) private var dismiss
    @State private var pasted = ""
    @State private var invalid = false
    @State private var scanning = false

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    Text("Open Clickr on your Mac and show the pairing code, or run `clickr accept`, then scan the code.")
                    Button {
                        scanning = true
                    } label: {
                        Label("Scan pairing code", systemImage: "qrcode.viewfinder")
                    }
                }
                Section("Or paste the ticket") {
                    TextField("endpoint...", text: $pasted, axis: .vertical)
                        .font(.system(.footnote, design: .monospaced))
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .lineLimit(3...6)
                    Button("Pair") { submit(pasted) }
                        .disabled(pasted.isEmpty)
                }
            }
            .navigationTitle("Pair computer")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                }
            }
            .sheet(isPresented: $scanning) {
                QRScannerView { code in
                    scanning = false
                    submit(code)
                }
                .ignoresSafeArea()
            }
            .alert("That is not a clickr ticket", isPresented: $invalid) {
                Button("OK", role: .cancel) {}
            }
        }
    }

    private func submit(_ text: String) {
        if model.pair(with: text) {
            dismiss()
        } else {
            invalid = true
        }
    }
}
