import SwiftUI

/// Pairs with a computer by scanning or pasting the ticket from `clickr accept`.
struct PairingView: View {
    @EnvironmentObject private var model: RemoteModel
    @State private var pasted = ""
    @State private var invalid = false
    @State private var scanning = false

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    Text("Run `clickr accept` on your computer, then scan the code it prints.")
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
            .navigationTitle("Pair Clickr")
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
        if !model.pair(with: text) {
            invalid = true
        }
    }
}
