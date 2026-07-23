import SwiftUI

private struct XboxDataSyncTaskIdentity: Equatable {
    let session: StoredAuthSession?
    let ownerGeneration: UInt64
}

@main
struct XBXRCApp: App {
    @Environment(\.scenePhase) private var scenePhase
    @StateObject private var settingsStore: AppSettingsStore
    @StateObject private var authStore: AuthStore
    @StateObject private var dataStore = XboxDataStore()
    @StateObject private var cloudStore = CloudLibraryStore()
    @StateObject private var streamingStore = StreamingFeatureStore()

    init() {
        let settingsStore = AppSettingsStore()
        _settingsStore = StateObject(wrappedValue: settingsStore)
        _authStore = StateObject(wrappedValue: AuthStore(settings: settingsStore))
        IOSRuntimeTrace.event(
            domain: "ios-app",
            event: "appLaunchStarted",
            payload: [
                "buildConfiguration": .string(Self.buildConfiguration),
                "appVersion": .string(
                    Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString")
                        as? String ?? "unknown"
                ),
                "buildNumber": .string(
                    Bundle.main.object(forInfoDictionaryKey: "CFBundleVersion")
                        as? String ?? "unknown"
                ),
            ],
            dimension: .lifecycle,
            importance: .essential,
            operationID: nil
        )
    }

    var body: some Scene {
        let xboxDataSyncIdentity = XboxDataSyncTaskIdentity(
            session: authStore.session,
            ownerGeneration: authStore.ownerGeneration
        )
        WindowGroup {
            AppRootView()
                .environmentObject(authStore)
                .environmentObject(dataStore)
                .environmentObject(cloudStore)
                .environmentObject(settingsStore)
                .environmentObject(streamingStore)
                .preferredColorScheme(settingsStore.appearanceMode.colorScheme)
                .task {
                    await authStore.restore()
                    await cloudStore.restoreCached(
                        session: authStore.session,
                        source: "appRestore"
                    )
                }
                .task(id: xboxDataSyncIdentity) {
                    await dataStore.sync(
                        session: xboxDataSyncIdentity.session,
                        ownerGeneration: xboxDataSyncIdentity.ownerGeneration
                    )
                }
                .task(id: authStore.session == nil) {
                    if authStore.session == nil {
                        streamingStore.stop()
                        await cloudStore.clear()
                    }
                }
                .onChange(of: dataStore.games) { _, games in
                    cloudStore.updateActivities(games)
                }
                .onChange(of: scenePhase) { previousPhase, phase in
                    streamingStore.handleScenePhase(phase)
                    IOSRuntimeTrace.state(
                        domain: "ios-app",
                        event: "scenePhaseChanged",
                        payload: [
                            "from": .string(Self.scenePhaseName(previousPhase)),
                            "to": .string(Self.scenePhaseName(phase)),
                        ],
                        dimension: .lifecycle,
                        importance: .key,
                        operationID: nil
                    )
                    if phase != .active {
                        Task {
                            await IOSRuntimeTrace.flush()
                        }
                    }
                }
        }
    }

    private static func scenePhaseName(_ phase: ScenePhase) -> String {
        switch phase {
        case .active:
            "active"
        case .inactive:
            "inactive"
        case .background:
            "background"
        @unknown default:
            "unknown"
        }
    }

    private static var buildConfiguration: String {
        #if DEBUG
        "debug"
        #else
        "release"
        #endif
    }
}
