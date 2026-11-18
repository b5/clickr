import ClickrKit
import SwiftUI

/// The known computers, with a way to pair a new one and to rename this phone.
struct HostListView: View {
    @EnvironmentObject private var model: RemoteModel
    @State private var pairing = false
    @State private var settings = false

    var body: some View {
        NavigationStack {
            List {
                if model.hosts.isEmpty {
                    Section {
                        Text("Open Clickr on your Mac, or run `clickr accept`, then pair with the code it shows.")
                            .foregroundStyle(.secondary)
                    }
                } else {
                    Section("Computers") {
                        ForEach(model.hosts) { host in
                            Button {
                                model.open(id: host.id)
                            } label: {
                                HostRow(host: host)
                            }
                            .foregroundStyle(.primary)
                        }
                        .onDelete { offsets in
                            for index in offsets {
                                model.forget(id: model.hosts[index].id)
                            }
                        }
                    }
                }
                Section {
                    Button {
                        pairing = true
                    } label: {
                        Label("Pair new computer", systemImage: "plus.circle")
                    }
                }
            }
            .navigationTitle("Clickr")
            .toolbar {
                ToolbarItem(placement: .topBarTrailing) {
                    Button {
                        settings = true
                    } label: {
                        Image(systemName: "gearshape")
                    }
                    .accessibilityLabel("Settings")
                }
            }
            .sheet(isPresented: $pairing) {
                PairingView()
            }
            .sheet(isPresented: $settings) {
                SettingsView()
            }
        }
    }
}

private struct HostRow: View {
    let host: KnownHost

    var body: some View {
        HStack {
            Image(systemName: "desktopcomputer")
                .foregroundStyle(.tint)
            VStack(alignment: .leading) {
                Text(host.nickname)
                Text(shortEndpointId(id: host.id))
                    .font(.caption.monospaced())
                    .foregroundStyle(.secondary)
            }
            Spacer()
            if host.token == nil {
                Text("Not approved yet")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            Image(systemName: "chevron.right")
                .foregroundStyle(.tertiary)
        }
    }
}

/// Lets the user rename this phone.
private struct SettingsView: View {
    @EnvironmentObject private var model: RemoteModel
    @Environment(\.dismiss) private var dismiss
    @State private var nickname = ""

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    TextField("Nickname", text: $nickname)
                        .autocorrectionDisabled()
                } header: {
                    Text("This phone's name")
                } footer: {
                    Text("Computers see this name when you pair. They learn a new name the next time you connect.")
                }
            }
            .navigationTitle("Settings")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Save") {
                        model.setNickname(nickname)
                        dismiss()
                    }
                    .disabled(nickname.trimmingCharacters(in: .whitespaces).isEmpty)
                }
            }
            .onAppear { nickname = model.nickname }
        }
    }
}
