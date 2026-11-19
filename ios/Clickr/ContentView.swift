import SwiftUI

struct ContentView: View {
    @EnvironmentObject private var model: RemoteModel

    var body: some View {
        if model.selected != nil {
            RemoteView()
        } else {
            HostListView()
        }
    }
}
