import SwiftUI

private struct XboxDataSyncTaskIdentity: Equatable {
    let session: StoredAuthSession?
    let ownerGeneration: UInt64
}

@main
struct XBXRCApp: App {
    @Environment(\.scenePhase) private var scenePhase
    @UIApplicationDelegateAdaptor(XBXRCAppDelegate.self) private var appDelegate
    @StateObject private var settingsStore: AppSettingsStore
    @StateObject private var authStore: AuthStore
    @StateObject private var dataStore: XboxDataStore
    @StateObject private var cloudStore: CloudLibraryStore
    @StateObject private var favorites: CloudFavoritesStore
    @StateObject private var streamingStore: StreamingFeatureStore

    init() {
        let settingsStore: AppSettingsStore
        #if DEBUG
            if ProcessInfo.processInfo.arguments.contains("--settings-ui-test")
                || ProcessInfo.processInfo.arguments.contains("--pc-pages-ui-test"),
                let defaults = UserDefaults(suiteName: "XBXRC.SettingsUITests")
            {
                if !ProcessInfo.processInfo.arguments.contains("--preserve-settings-ui-test") {
                    defaults.removePersistentDomain(forName: "XBXRC.SettingsUITests")
                }
                settingsStore = AppSettingsStore(defaults: defaults)
                settingsStore.mobile.appLanguage = .chinese
            } else {
                settingsStore = AppSettingsStore()
            }
        #else
            settingsStore = AppSettingsStore()
        #endif
        _settingsStore = StateObject(wrappedValue: settingsStore)
        #if DEBUG
            if ProcessInfo.processInfo.arguments.contains("--pc-pages-ui-test") {
                let dataClient = PCPageDataClient()
                let cloudClient = PCPageCloudClient()
                _authStore = StateObject(
                    wrappedValue: AuthStore(
                        settings: settingsStore, client: PCPageAuthClient(),
                        cloudClient: cloudClient,
                        keychain: PCPageSessionStore()))
                _dataStore = StateObject(
                    wrappedValue: XboxDataStore(
                        client: dataClient, preferredGameLocaleProvider: settingsStore))
                _cloudStore = StateObject(
                    wrappedValue: CloudLibraryStore(
                        client: cloudClient,
                        repository: CloudCatalogSnapshotRepository(
                            rootDirectory: FileManager.default.temporaryDirectory
                                .appendingPathComponent(
                                    "pc-page-ui-cache-\(UUID().uuidString)")),
                        settingsProvider: settingsStore))
                _favorites = StateObject(
                    wrappedValue: CloudFavoritesStore(
                        defaults: UserDefaults(suiteName: "XBXRC.SettingsUITests")!))
            } else {
                _cloudStore = StateObject(
                    wrappedValue: CloudLibraryStore(settingsProvider: settingsStore))
                if ProcessInfo.processInfo.arguments.contains("--signed-out-ui-test") {
                    _authStore = StateObject(
                        wrappedValue: AuthStore(
                            settings: settingsStore, keychain: SignedOutUITestSessionStore()))
                } else {
                    _authStore = StateObject(wrappedValue: AuthStore(settings: settingsStore))
                }
                _dataStore = StateObject(
                    wrappedValue: XboxDataStore(preferredGameLocaleProvider: settingsStore))
                _favorites = StateObject(wrappedValue: CloudFavoritesStore())
            }
        #else
            _cloudStore = StateObject(
                wrappedValue: CloudLibraryStore(settingsProvider: settingsStore))
            _authStore = StateObject(wrappedValue: AuthStore(settings: settingsStore))
            _dataStore = StateObject(
                wrappedValue: XboxDataStore(preferredGameLocaleProvider: settingsStore)
            )
            _favorites = StateObject(wrappedValue: CloudFavoritesStore())
        #endif
        _streamingStore = StateObject(
            wrappedValue: StreamingFeatureStore(settingsProvider: settingsStore)
        )
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

    @ViewBuilder
    private var rootView: some View {
        #if DEBUG
            if ProcessInfo.processInfo.arguments.contains("--streaming-panels-ui-test") {
                StreamingPanelsUITestView()
            } else if ProcessInfo.processInfo.arguments.contains("--touch-playground-ui-test") {
                TouchLayoutPlaygroundView()
            } else {
                AppRootView(streamingStore: streamingStore)
            }
        #else
            AppRootView(streamingStore: streamingStore)
        #endif
    }

    var body: some Scene {
        let xboxDataSyncIdentity = XboxDataSyncTaskIdentity(
            session: authStore.session,
            ownerGeneration: authStore.ownerGeneration
        )
        WindowGroup {
            rootView
                .environmentObject(authStore)
                .environmentObject(dataStore)
                .environmentObject(cloudStore)
                .environmentObject(settingsStore)
                .environmentObject(streamingStore)
                .environmentObject(favorites)
                .environment(\.locale, settingsStore.mobile.appLanguage.locale)
                .preferredColorScheme(settingsStore.appearanceMode.colorScheme)
                .onChange(of: settingsStore.mobile) { _, preferences in
                    MobileInteractionFeedback.play(preferences)
                }
                .task {
                    #if DEBUG
                        if ProcessInfo.processInfo.arguments.contains("--settings-ui-test")
                            && !ProcessInfo.processInfo.arguments.contains("--signed-out-ui-test")
                        {
                            return
                        }
                    #endif
                    await authStore.restore()
                    await cloudStore.restoreCached(
                        session: authStore.session,
                        source: "appRestore"
                    )
                }
                .task(id: xboxDataSyncIdentity) {
                    streamingStore.authentication = authStore
                    dataStore.authentication = authStore
                    cloudStore.authentication = authStore
                    favorites.bind(
                        accountID: xboxDataSyncIdentity.session.flatMap { Self.accountID($0) })
                    await dataStore.sync(
                        session: xboxDataSyncIdentity.session,
                        ownerGeneration: xboxDataSyncIdentity.ownerGeneration
                    )
                }
                .task(id: authStore.session == nil) {
                    if authStore.session == nil {
                        streamingStore.stopForAuthRevocation()
                        await cloudStore.clear()
                    }
                }
                .onChange(of: dataStore.games) { _, games in
                    cloudStore.updateActivities(games)
                }
                .onChange(of: scenePhase) { previousPhase, phase in
                    streamingStore.handleScenePhase(phase)
                    if phase == .active {
                        #if DEBUG
                            let preview = ProcessInfo.processInfo.arguments.contains(
                                "--streaming-panels-ui-test")
                        #else
                            let preview = false
                        #endif
                        AppOrientationCoordinator.shared.sync(
                            streamingPresented: streamingStore.isPresentingPlayer && !preview
                        )
                    }
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

    private static func accountID(_ session: StoredAuthSession) -> String? {
        guard let data = session.webTokenJSON.data(using: .utf8),
            let token = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else { return session.cloudAccountID }
        let root = token["data"] as? [String: Any] ?? token
        guard let claims = root["DisplayClaims"] as? [String: Any],
            let users = claims["xui"] as? [[String: Any]], let user = users.first
        else { return session.cloudAccountID }
        return user["xid"] as? String ?? session.cloudAccountID ?? user["uhs"] as? String
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

#if DEBUG
    private actor SignedOutUITestSessionStore: AuthSessionStoring {
        private var session: StoredAuthSession?
        func load() async throws -> StoredAuthSession? { session }
        func save(_ session: StoredAuthSession) async throws { self.session = session }
        func delete() async throws { session = nil }
    }
#endif
