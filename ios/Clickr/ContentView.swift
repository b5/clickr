import SwiftUI

struct ContentView: View {
    @EnvironmentObject private var model: RemoteModel

    var body: some View {
        if model.ticket == nil {
            PairingView()
        } else {
            RemoteView()
        }
    }
}
