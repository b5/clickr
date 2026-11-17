import ClickrKit
import SwiftUI
import UIKit

@main
struct ClickrApp: App {
    @StateObject private var model: RemoteModel
    @Environment(\.scenePhase) private var scenePhase

    init() {
        // Set up logging before the model starts connecting.
        initLogging(level: .warn)
        _model = StateObject(wrappedValue: RemoteModel())
    }

    var body: some Scene {
        WindowGroup {
            ContentView()
                .environmentObject(model)
                .onAppear { keepAwake() }
        }
        .onChange(of: scenePhase) { _, phase in
            switch phase {
            case .active:
                keepAwake()
                model.resume()
            case .background:
                model.suspend()
            default:
                break
            }
        }
    }

    /// Stops iOS from dimming and locking the screen while Clickr is in front.
    ///
    /// iOS resets this flag in some situations (for example after the app returns from
    /// the background), so it is reapplied every time the scene becomes active.
    private func keepAwake() {
        UIApplication.shared.isIdleTimerDisabled = true
    }
}
