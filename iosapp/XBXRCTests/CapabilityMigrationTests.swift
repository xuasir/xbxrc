import Foundation
import GameController
import SwiftUI
import UIKit
import XCTest

@testable import XBXRC

final class CapabilityMigrationTests: XCTestCase {
    @MainActor
    func testPrimarySwitchGatesInactiveNavigationAndRestoresIt() async throws {
        let suite = "XBXRC.PrimaryVisibility.\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        let settings = AppSettingsStore(defaults: defaults)
        let state = MigrationNavigationState()
        let scene = try XCTUnwrap(
            UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first)
        let previousWindow = scene.windows.first { $0.isKeyWindow }
        let window = UIWindow(windowScene: scene)
        let host = UIHostingController(
            rootView: MigrationPrimaryNavigationHarness(state: state)
                .environmentObject(settings)
                .environmentObject(AuthStore(settings: settings))
                .environmentObject(XboxDataStore(client: MigrationDataClient()))
                .environmentObject(CloudLibraryStore(settingsProvider: settings))
                .environmentObject(StreamingFeatureStore(settingsProvider: settings)))
        window.frame = CGRect(x: 0, y: 0, width: 390, height: 844)
        window.rootViewController = host
        window.makeKeyAndVisible()
        defer {
            window.isHidden = true
            window.rootViewController = nil
            previousWindow?.makeKey()
        }
        try await Task.sleep(for: .milliseconds(300))
        let settingsNavigation = try XCTUnwrap(findNavigation(in: host))
        let originalDepth = settingsNavigation.viewControllers.count
        state.primarySelection = .controller
        try await Task.sleep(for: .milliseconds(600))
        XCTAssertNil(settingsNavigation.view.window, "后台原生导航应退出可见窗口及无障碍树")
        state.primarySelection = .settings
        try await Task.sleep(for: .milliseconds(600))
        XCTAssertTrue(settingsNavigation.view.window === window)
        XCTAssertTrue(settingsNavigation.view.isUserInteractionEnabled)
        XCTAssertEqual(settingsNavigation.viewControllers.count, originalDepth)
    }

    @MainActor
    func testPlainBackKeepsNativeEdgePopAndReentry() async throws {
        let suite = "XBXRC.PlainBack.\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        let settings = AppSettingsStore(defaults: defaults)
        let state = MigrationNavigationState()
        let scene = try XCTUnwrap(
            UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first)
        let previousWindow = scene.windows.first { $0.isKeyWindow }
        let window = UIWindow(windowScene: scene)
        let host = UIHostingController(
            rootView: MigrationNavigationHarness(state: state)
                .environmentObject(AuthStore(settings: settings)))
        window.frame = CGRect(x: 0, y: 0, width: 390, height: 844)
        window.rootViewController = host
        window.makeKeyAndVisible()
        defer {
            window.isHidden = true
            window.rootViewController = nil
            previousWindow?.makeKey()
        }
        try await Task.sleep(for: .milliseconds(300))
        let navigation = try XCTUnwrap(findNavigation(in: host))
        let gesture = try XCTUnwrap(navigation.interactivePopGestureRecognizer)
        let originalDelegate = gesture.delegate
        for _ in 0..<2 {
            state.selection = .streaming
            try await waitForNavigation(navigation, count: 2)
            XCTAssertEqual(navigation.viewControllers.count, 2)
            XCTAssertTrue(gesture.isEnabled)
            XCTAssertEqual(gesture.delegate?.gestureRecognizerShouldBegin?(gesture), true)
            XCTAssertNotNil(navigation.popViewController(animated: true))
            try await waitForNavigation(navigation, count: 1)
            XCTAssertNil(state.selection, "原生返回应同步更新 SwiftUI 路由")
            XCTAssertEqual(navigation.viewControllers.count, 1)
            XCTAssertTrue(gesture.delegate === originalDelegate, "退出子页应恢复原生手势代理")
        }
    }

    @MainActor
    private func waitForNavigation(_ navigation: UINavigationController, count: Int) async throws {
        let deadline = ContinuousClock.now + .seconds(5)
        while navigation.viewControllers.count != count || navigation.transitionCoordinator != nil {
            guard ContinuousClock.now < deadline else {
                throw NSError(
                    domain: "MigrationNavigation", code: 1,
                    userInfo: [NSLocalizedDescriptionKey: "原生导航未完成转场"])
            }
            try await Task.sleep(for: .milliseconds(20))
        }
        // 等待 SwiftUI 消费原生栈的完成回调。
        await Task.yield()
    }

    @MainActor
    private func findNavigation(in controller: UIViewController) -> UINavigationController? {
        if let navigation = controller as? UINavigationController { return navigation }
        for child in controller.children {
            if let navigation = findNavigation(in: child) { return navigation }
        }
        return nil
    }

    @MainActor
    func testSettingsAndControllerPagesRenderAcrossWindowAndTextSizes() async throws {
        let suite = "XBXRC.MigrationLayout.\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        let settings = AppSettingsStore(defaults: defaults)
        let auth = AuthStore(settings: settings)
        let data = XboxDataStore(client: MigrationDataClient())
        let cloud = CloudLibraryStore(settingsProvider: settings)
        let streaming = StreamingFeatureStore(settingsProvider: settings)
        let cases: [(String, CGSize, DynamicTypeSize, ColorScheme)] = [
            ("narrow", CGSize(width: 320, height: 800), .large, .light),
            ("narrow-accessibility", CGSize(width: 320, height: 800), .accessibility3, .dark),
            ("phone", CGSize(width: 390, height: 844), .large, .dark),
            ("phone-large", CGSize(width: 430, height: 932), .large, .light),
            ("landscape", CGSize(width: 844, height: 390), .large, .dark),
            ("split", CGSize(width: 744, height: 900), .large, .light),
            ("tablet-portrait", CGSize(width: 834, height: 1_194), .large, .light),
            ("wide", CGSize(width: 1_024, height: 768), .large, .light),
            ("tablet-large", CGSize(width: 1_366, height: 1_024), .large, .dark),
            ("accessibility", CGSize(width: 1_024, height: 768), .accessibility3, .dark),
        ]
        for (name, size, textSize, colorScheme) in cases {
            let pages: [(String, AnyView)] =
                [
                    ("hosts", AnyView(HostListView(isActive: false))),
                    ("cloud", AnyView(GameLibraryView(isActive: false))),
                    ("my", AnyView(ProfileView(isActive: false))),
                    ("settings", AnyView(SettingsRootView())),
                    ("controller", AnyView(ControllerView())),
                    ("settings-general", AnyView(GeneralSettingsView())),
                    ("settings-connection", AnyView(ConnectionSettingsView())),
                    (
                        "settings-picture",
                        AnyView(
                            MobilePictureSettingsView(
                                picture: .constant(MobilePicturePreferences())))
                    ),
                    ("settings-turn", AnyView(MobileTurnSettingsView())),
                    (
                        "settings-shortcuts",
                        AnyView(
                            NavigationStack {
                                ControllerMenuSettingsView().appPageToolbar("菜单操作")
                            })
                    ),
                    (
                        "settings-vibration",
                        AnyView(
                            NavigationStack {
                                ControllerVibrationView(isActive: false, input: streaming.input)
                                    .appPageToolbar("震动")
                            })
                    ),
                    (
                        "settings-shortcut-editor",
                        AnyView(NavigationStack { ControllerMenuSettingsView() })
                    ),
                    (
                        "settings-diagnostics",
                        AnyView(DiagnosticsSettingsView(traceProfile: .constant(.production)))
                    ),
                    (
                        "settings-english",
                        AnyView(
                            StreamingExperienceSettingsView().environment(
                                \.locale, Locale(identifier: "en")))
                    ),
                    (
                        "streaming",
                        AnyView(
                            AdaptiveSectionNavigation(
                                title: "设置", selection: .constant(MigrationLayoutRoute.streaming)
                            ) {
                                Text("应用偏好")
                            } detail: { _ in
                                StreamingExperienceSettingsView()
                            }
                        )
                    ),
                ]
                + AppSection.allCases.map { section in
                    ("shell-\(section)", AnyView(AppNavigationShell(selection: .constant(section))))
                }
            for (page, content) in pages {
                let root =
                    content
                    .environmentObject(settings)
                    .environmentObject(auth)
                    .environmentObject(data)
                    .environmentObject(cloud)
                    .environmentObject(CloudFavoritesStore(defaults: defaults))
                    .environmentObject(streaming)
                    .environment(\.dynamicTypeSize, textSize)
                    .environment(\.colorScheme, colorScheme)
                    .preferredColorScheme(colorScheme)
                try await attachLayoutSnapshot(
                    root: root, size: size,
                    name:
                        "\(UIDevice.current.userInterfaceIdiom == .pad ? "ipad" : "phone")-\(name)-\(page)"
                )
            }
        }
    }

    @MainActor
    func testInformationPagesRenderAtPhoneTabletAndAccessibilitySizes() async throws {
        let suite = "XBXRC.InformationLayout.\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        let settings = AppSettingsStore(defaults: defaults)
        let auth = AuthStore(
            settings: settings, client: PCPageAuthClient(),
            cloudClient: PCPageCloudClient(), keychain: PCPageSessionStore())
        await auth.restore()
        await auth.activateProfileOnce()
        let data = XboxDataStore(client: PCPageDataClient(), preferredGameLocaleProvider: settings)
        await data.sync(session: auth.session, ownerGeneration: auth.ownerGeneration)
        await data.activateHostsOnce()
        await data.activateLibraryOnce()
        let cloud = CloudLibraryStore(client: PCPageCloudClient(), settingsProvider: settings)
        let streaming = StreamingFeatureStore(settingsProvider: settings)
        let favorites = CloudFavoritesStore(defaults: defaults)
        let cases: [(String, CGSize, DynamicTypeSize, ColorScheme)] = [
            ("narrow-light", CGSize(width: 320, height: 800), .large, .light),
            ("phone-dark", CGSize(width: 390, height: 844), .large, .dark),
            ("phone-accessibility", CGSize(width: 390, height: 844), .accessibility3, .dark),
            ("phone-accessibility-max", CGSize(width: 390, height: 844), .accessibility5, .dark),
            ("window-dark", CGSize(width: 550, height: 804), .large, .dark),
            ("controller-window-dark", CGSize(width: 667, height: 809), .large, .dark),
            ("landscape-dark", CGSize(width: 844, height: 390), .large, .dark),
            ("tablet-light", CGSize(width: 1024, height: 768), .large, .light),
            ("tablet-dark", CGSize(width: 1366, height: 1024), .large, .dark),
        ]
        await data.loadInstalledLibrary(
            consoleID: PCPageFixtures.host.commandID!, market: settings.mobile.catalogMarket,
            language: settings.mobile.catalogLanguage)
        for language in ["zh-Hans", "en"] {
            for (name, size, textSize, theme) in cases {
                let pages: [(String, AnyView)] = [
                    (
                        "01-connection",
                        AnyView(
                            NavigationStack {
                                ControllerTestView(input: streaming.input, isActive: false)
                                    .appPageToolbar("连接与测试")
                            })
                    ),
                    (
                        "13-input-history",
                        AnyView(
                            NavigationStack {
                                AppSettingsForm {
                                    Section("手柄测试") {
                                        ControllerInputHistoryStrip(entries: [
                                            .a, .b, .x, .y, .leftShoulder, .rightShoulder,
                                            .view, .menu, .left, .right,
                                        ]).appSettingsInformation()
                                    }
                                }.appPageToolbar("连接与测试")
                            })
                    ),
                    (
                        "02-menu",
                        AnyView(
                            NavigationStack { ControllerMenuSettingsView().appPageToolbar("菜单操作") })
                    ),
                    (
                        "03-vibration",
                        AnyView(
                            NavigationStack {
                                ControllerVibrationView(isActive: false, input: streaming.input)
                                    .appPageToolbar("震动")
                            })
                    ),
                    (
                        "04-mapping",
                        AnyView(
                            NavigationStack {
                                ControllerMappingSettingsView().appPageToolbar("按键与轴映射")
                            }
                        )
                    ),
                    ("05-account", AnyView(ProfileView(isActive: false))),
                    ("12-controller-entries", AnyView(ControllerView(isActive: false))),
                    (
                        "06-playtime",
                        AnyView(
                            NavigationStack {
                                GameHistoryView(statistics: true).appPageToolbar("游玩统计")
                            })
                    ),
                    (
                        "07-player",
                        AnyView(
                            NavigationStack { FriendDetailView(friend: PCPageFixtures.friends[0]) })
                    ),
                    (
                        "08-game",
                        AnyView(
                            NavigationStack {
                                PersonalGameDetailView(game: PCPageFixtures.games[0])
                            })
                    ),
                    (
                        "09-album",
                        AnyView(
                            NavigationStack {
                                PersonalGameDetailView(
                                    game: PCPageFixtures.games[0], section: .album)
                            })
                    ),
                    (
                        "10-installed",
                        AnyView(
                            NavigationStack { HostInstalledGamesView(host: PCPageFixtures.host) })
                    ),
                    ("11-hosts", AnyView(HostListView(isActive: false))),
                ]
                for (page, content) in pages {
                    let root = content.environmentObject(settings).environmentObject(auth)
                        .environmentObject(data).environmentObject(cloud)
                        .environmentObject(favorites).environmentObject(streaming)
                        .environment(\.colorScheme, theme).preferredColorScheme(theme)
                        .environment(\.locale, Locale(identifier: language))
                        .appInformationTypography()
                        .environment(\.dynamicTypeSize, textSize)
                    try await attachLayoutSnapshot(
                        root: root, size: size,
                        name: "spacing-\(language)-\(name)-\(page)")
                }
            }
        }
    }

    @MainActor
    func testNavigationBackgroundAndReadingPagesRenderAcrossLayouts() async throws {
        let suite = "XBXRC.NavigationReading.\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        let settings = AppSettingsStore(defaults: defaults)
        let auth = AuthStore(
            settings: settings, client: PCPageAuthClient(), cloudClient: PCPageCloudClient(),
            keychain: PCPageSessionStore())
        await auth.restore()
        await auth.activateProfileOnce()
        let data = XboxDataStore(client: PCPageDataClient(), preferredGameLocaleProvider: settings)
        await data.sync(session: auth.session, ownerGeneration: auth.ownerGeneration)
        await data.activateHostsOnce()
        await data.activateLibraryOnce()
        let cloud = CloudLibraryStore(client: PCPageCloudClient(), settingsProvider: settings)
        let streaming = StreamingFeatureStore(settingsProvider: settings)
        let favorites = CloudFavoritesStore(defaults: defaults)
        let cases: [(String, CGSize, DynamicTypeSize, ColorScheme)] = [
            ("narrow-light", CGSize(width: 320, height: 800), .large, .light),
            ("phone-dark", CGSize(width: 390, height: 844), .large, .dark),
            ("phone-large-light", CGSize(width: 430, height: 932), .large, .light),
            ("phone-accessibility", CGSize(width: 390, height: 844), .accessibility3, .dark),
            ("split-dark", CGSize(width: 744, height: 900), .large, .dark),
            ("tablet-portrait-light", CGSize(width: 834, height: 1194), .large, .light),
            ("tablet-dark", CGSize(width: 1024, height: 768), .large, .dark),
            ("tablet-large-dark", CGSize(width: 1366, height: 1024), .large, .dark),
        ]
        for language in ["zh-Hans", "en"] {
            for (name, size, textSize, theme) in cases {
                let pages: [(String, AnyView)] = [
                    ("my", AnyView(ProfileView(isActive: false))),
                    ("settings", AnyView(AppNavigationShell(selection: .constant(.settings)))),
                    (
                        "general",
                        AnyView(NavigationStack { GeneralSettingsView().appPageToolbar("通用") })
                    ),
                    (
                        "streaming",
                        AnyView(
                            NavigationStack {
                                StreamingExperienceSettingsView().appPageToolbar("串流体验")
                            })
                    ),
                    (
                        "diagnostics",
                        AnyView(
                            NavigationStack {
                                DiagnosticsSettingsView(traceProfile: .constant(.production))
                                    .appPageToolbar("高级诊断")
                            })
                    ),
                    (
                        "menu",
                        AnyView(
                            NavigationStack { ControllerMenuSettingsView().appPageToolbar("菜单操作") })
                    ),
                    (
                        "picture",
                        AnyView(
                            NavigationStack {
                                MobilePictureSettingsView(
                                    picture: .constant(MobilePicturePreferences()))
                            })
                    ),
                    (
                        "cloud-browser",
                        AnyView(
                            NavigationStack { CloudLibraryBrowserView(page: .all) }
                        )
                    ),
                    (
                        "cloud-detail",
                        AnyView(
                            NavigationStack { GameDetailView(game: PCPageFixtures.cloudGames()[1]) }
                        )
                    ),
                ]
                for (page, content) in pages {
                    let root = content.environmentObject(settings).environmentObject(auth)
                        .environmentObject(data).environmentObject(cloud)
                        .environmentObject(streaming).environmentObject(favorites)
                        .environment(\.locale, Locale(identifier: language))
                        .environment(\.dynamicTypeSize, textSize)
                        .environment(\.colorScheme, theme).preferredColorScheme(theme)
                    try await attachLayoutSnapshot(
                        root: root, size: size, name: "reading-\(language)-\(name)-\(page)")
                }
            }
        }
    }

    @MainActor
    func testHostOverviewRendersAtPhoneWindowTabletAndAccessibilitySizes() async throws {
        let suite = "XBXRC.HostOverviewLayout.\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        let settings = AppSettingsStore(defaults: defaults)
        let auth = AuthStore(
            settings: settings, client: PCPageAuthClient(),
            cloudClient: PCPageCloudClient(), keychain: PCPageSessionStore())
        await auth.restore()
        let data = XboxDataStore(
            client: PCPageDataClient(includesAllHosts: true), preferredGameLocaleProvider: settings)
        await data.sync(session: auth.session, ownerGeneration: auth.ownerGeneration)
        await data.activateHostsOnce()
        let streaming = StreamingFeatureStore(settingsProvider: settings)
        let cases: [(String, CGSize, DynamicTypeSize, ColorScheme)] = [
            ("narrow-light", CGSize(width: 320, height: 800), .large, .light),
            ("phone-dark", CGSize(width: 390, height: 844), .large, .dark),
            ("phone-accessibility", CGSize(width: 390, height: 844), .accessibility3, .dark),
            ("window-dark", CGSize(width: 550, height: 804), .large, .dark),
            ("landscape-dark", CGSize(width: 844, height: 390), .large, .dark),
            ("tablet-light", CGSize(width: 1024, height: 768), .large, .light),
            ("tablet-dark", CGSize(width: 1366, height: 1024), .large, .dark),
        ]
        for language in ["zh-Hans", "en"] {
            for (name, size, textSize, theme) in cases {
                let root = HostListView(isActive: false)
                    .environmentObject(settings).environmentObject(auth)
                    .environmentObject(data).environmentObject(streaming)
                    .environment(\.colorScheme, theme).preferredColorScheme(theme)
                    .environment(\.locale, Locale(identifier: language))
                    .environment(\.dynamicTypeSize, textSize)
                try await attachLayoutSnapshot(
                    root: root, size: size, name: "host-overview-\(language)-\(name)")
            }
        }
    }

    @MainActor
    func testSectionErrorsRenderAcrossWidthLanguageAndTextSizes() async throws {
        let suite = "XBXRC.SectionErrors.\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        let settings = AppSettingsStore(defaults: defaults)
        let auth = AuthStore(
            settings: settings, client: PCPageAuthClient(), cloudClient: PCPageCloudClient(),
            keychain: PCPageSessionStore())
        await auth.restore()
        let data = XboxDataStore(client: PCPageDataClient(), preferredGameLocaleProvider: settings)
        await data.sync(session: auth.session, ownerGeneration: auth.ownerGeneration)
        await data.loadInstalledLibrary(
            consoleID: PCPageFixtures.host.commandID!, market: settings.mobile.catalogMarket,
            language: settings.mobile.catalogLanguage)
        data.installedLibraryStates[PCPageFixtures.host.commandID!]?.sections[.storage]?.failure =
            SectionRequestFailure(kind: .timeout)
        data.installedLibraryStates[PCPageFixtures.host.commandID!]?.sections[.storage]?
            .requestAttempt = .refresh
        let cloud = CloudLibraryStore(client: PCPageCloudClient(), settingsProvider: settings)
        let streaming = StreamingFeatureStore(settingsProvider: settings)
        let favorites = CloudFavoritesStore(defaults: defaults)
        // .large 对应系统标准字号；无障碍字号使用独立样例。
        let cases: [(String, CGSize, String, DynamicTypeSize, ColorScheme)] = [
            ("phone-narrow-zh-light", CGSize(width: 320, height: 800), "zh-Hans", .large, .light),
            ("phone-narrow-en-light", CGSize(width: 320, height: 800), "en", .large, .light),
            ("phone-zh-dark", CGSize(width: 390, height: 844), "zh-Hans", .large, .dark),
            (
                "phone-en-accessibility", CGSize(width: 390, height: 844), "en", .accessibility3,
                .dark
            ),
            ("ipad-en-light", CGSize(width: 1024, height: 1024), "en", .large, .light),
        ]
        for (name, size, language, textSize, theme) in cases {
            let root = NavigationStack { HostInstalledGamesView(host: PCPageFixtures.host) }
                .environmentObject(auth).environmentObject(data).environmentObject(settings)
                .environmentObject(cloud).environmentObject(streaming).environmentObject(favorites)
                .environment(\.appPageIsActive, false)
                .environment(\.locale, Locale(identifier: language))
                .environment(\.dynamicTypeSize, textSize).preferredColorScheme(theme)
            try await attachLayoutSnapshot(root: root, size: size, name: "section-errors-\(name)")
        }
        // 大字号时让失败状态单独占据可见槽位，完整验证全部文案的折行。
        let status = SectionRequestStatus(
            phase: .loaded, hasSnapshot: true, updatedAt: .now,
            attempt: .nextPage, failure: SectionRequestFailure(kind: .serviceUnavailable))
        let panel = AppPageScroll {
            AppSectionRequestState(title: "游玩记录", status: status, identifier: "layout-page") {}
                .padding(AppLayoutPolicy.pageInset)
        }.appThemeCanvas().environment(\.locale, Locale(identifier: "en"))
            .environment(\.dynamicTypeSize, .accessibility3).preferredColorScheme(.dark)
        try await attachLayoutSnapshot(
            root: panel, size: CGSize(width: 320, height: 800),
            name: "section-errors-page-en-accessibility")
    }

    @MainActor
    private func attachLayoutSnapshot<Content: View>(root: Content, size: CGSize, name: String)
        async throws
    {
        let scene = try XCTUnwrap(
            UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first)
        let previousWindow = scene.windows.first { $0.isKeyWindow }
        let window = UIWindow(windowScene: scene)
        // 独立截图窗口复用壳层的连续画布，覆盖透明导航容器的全部区域。
        let host = UIHostingController(
            rootView: ZStack {
                AppThemeBackground()
                root
            })
        window.frame = CGRect(origin: .zero, size: size)
        window.rootViewController = host
        window.makeKeyAndVisible()
        defer {
            window.isHidden = true
            window.rootViewController = nil
            previousWindow?.makeKey()
        }
        try await Task.sleep(for: .milliseconds(350))
        host.view.frame = CGRect(origin: .zero, size: size)
        host.view.layoutIfNeeded()
        let format = UIGraphicsImageRendererFormat()
        format.scale = 1
        var rendered = false
        let image = UIGraphicsImageRenderer(size: size, format: format).image { _ in
            rendered = host.view.drawHierarchy(in: host.view.bounds, afterScreenUpdates: true)
        }
        XCTAssertTrue(rendered, "\(name) 页面应完成实际原生绘制")
        let attachment = XCTAttachment(image: image)
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }

    @MainActor
    func testLatePersonalResponseCannotPublishIntoAnotherAccount() async {
        let client = MigrationDataClient()
        let store = XboxDataStore(client: client)
        await store.sync(session: migrationSession("first"), ownerGeneration: 1)
        let request = Task { await store.loadFriends() }
        await client.waitForFriendRequest()
        await store.sync(session: migrationSession("second"), ownerGeneration: 2)
        await client.finishFriendRequest()
        await request.value
        XCTAssertTrue(store.friendsState.items.isEmpty)
        XCTAssertEqual(store.friendsState.phase, .idle)
        XCTAssertFalse(store.friendsState.isRefreshing)
    }

    @MainActor
    func testAlbumRepeatedContinuationKeepsLastSuccessfulPage() async {
        let store = XboxDataStore(client: MigrationDataClient())
        await store.sync(session: migrationSession("first"), ownerGeneration: 1)
        await store.loadAlbum(kind: .screenshot)
        XCTAssertEqual(store.albumStates[.screenshot]?.items.map(\.id), ["first"])
        await store.loadAlbum(kind: .screenshot, nextPage: true)
        XCTAssertEqual(store.albumStates[.screenshot]?.items.map(\.id), ["first"])
        XCTAssertNotNil(store.albumStates[.screenshot]?.errorMessage)
        XCTAssertFalse(store.albumStates[.screenshot]?.isRefreshing ?? true)
    }

    @MainActor
    func testPartialInstalledRefreshPreservesGamesStorageAndSourceFailures() async {
        let store = XboxDataStore(client: MigrationDataClient())
        await store.sync(session: migrationSession("first"), ownerGeneration: 1)
        await store.loadInstalledLibrary(consoleID: "console", market: "US", language: "en-US")
        await store.loadInstalledLibrary(
            consoleID: "console", market: "US", language: "en-US", refresh: true)
        let snapshot = store.installedLibraryStates["console"]?.snapshot
        XCTAssertEqual(snapshot?.games?.first?.name, "Installed")
        XCTAssertEqual(snapshot?.storage?.first?.id, "internal")
        XCTAssertEqual(
            store.installedLibraryStates["console"]?.sections[.games]?.failure?.kind, .timeout)
        XCTAssertEqual(
            store.installedLibraryStates["console"]?.sections[.storage]?.failure?.kind, .timeout)
        XCTAssertEqual(snapshot?.powerState, "On")
        XCTAssertNil(store.installedLibraryStates["console"]?.sections[.status]?.failure)
    }

    @MainActor
    func testLifecycleAndMenuSuppressionReleaseIndependently() {
        let input = StreamInputSession(controller: { nil })
        input.setSuppressed(true, reason: .menu)
        input.setSuppressed(true, reason: .lifecycle)
        input.setSuppressed(false, reason: .lifecycle)
        XCTAssertTrue(input.isSuppressed)
        input.setSuppressed(false, reason: .menu)
        XCTAssertFalse(input.isSuppressed)
        input.setSuppressed(true, reason: .terminal)
        input.setSuppressed(false, reason: .menu)
        XCTAssertTrue(input.isSuppressed)
    }

    private func migrationSession(_ owner: String) -> StoredAuthSession {
        StoredAuthSession(refreshToken: "test", seedJSON: owner, webTokenJSON: "test", appLevel: 2)
    }
    @MainActor
    func testMobilePreferencesPersistAndProjectServerIndependentlyFromAuthorization() throws {
        let suite = "XBXRC.CapabilityMigration.\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        let settings = AppSettingsStore(defaults: defaults)
        settings.setCloudRegionPreset(.japan)
        settings.mobile.cloudServerRegion = "WESTEUROPE"
        settings.mobile.catalogMarket = "KR"
        settings.mobile.catalogLanguage = "ko-KR"
        settings.mobile.videoFit = "fill"
        settings.mobile.startMuted = true
        settings.mobile.leftStickDeadzone = 0.2
        let restored = AppSettingsStore(defaults: defaults)
        XCTAssertEqual(restored.cloudRegionPreset, .japan)
        XCTAssertEqual(restored.mobile.catalogMarket, "KR")
        XCTAssertEqual(restored.mobile.catalogLanguage, "ko-KR")
        XCTAssertEqual(restored.streamingSessionSettings.cloudServerRegion, "WESTEUROPE")
        XCTAssertEqual(restored.streamingSessionSettings.videoFit, "fill")
        XCTAssertTrue(restored.streamingSessionSettings.startMuted)
        XCTAssertEqual(restored.mobile.leftStickDeadzone, 0.2)
    }

    func testRadialDeadzoneAndIndependentYInversionKeepNeutralAndFullTravel() {
        let neutral = IOSStreamStickTransform.apply(x: 0.05, y: 0.05, deadzone: 0.1, invertY: false)
        XCTAssertEqual(neutral.0, 0)
        XCTAssertEqual(neutral.1, 0)
        var preferences = MobilePreferences()
        preferences.invertRightY = true
        let state = IOSStreamStickTransform.apply(
            IOSStreamGamepadState(leftThumbY: 1, rightThumbY: 1), preferences: preferences)
        XCTAssertEqual(state.leftThumbY, 1)
        XCTAssertEqual(state.rightThumbY, -1)
        let diagonal = IOSStreamStickTransform.apply(x: 1, y: 1, deadzone: 0.1, invertY: false)
        XCTAssertEqual(hypot(diagonal.0, diagonal.1), 1, accuracy: 0.0001)
    }

    @MainActor
    func testTouchInputsReleaseOnMenuAndRequireNeutralBeforeResuming() {
        let input = StreamInputSession(controller: { nil })
        input.setTouchButton(16, pressed: true, id: "a")
        input.setTouchTrigger(left: true, pressed: true)
        let before = input.outputState(now: 0)
        XCTAssertEqual(before.buttonMask & 16, 16)
        XCTAssertEqual(before.leftTrigger, 1)
        input.setSuppressed(true)
        XCTAssertEqual(input.outputState(now: 8), .neutral)
        input.setSuppressed(false)
        XCTAssertEqual(input.outputState(now: 16), .neutral)
        input.setTouchButton(32, pressed: true, id: "b")
        XCTAssertEqual(input.outputState(now: 24).buttonMask, 32)
        input.reset()
        XCTAssertEqual(input.outputState(now: 32), .neutral)
    }

    @MainActor
    func testControllerSwitchReleasesTouchesAndWaitsForPhysicalNeutral() throws {
        let devices = MigrationControllerSource()
        let input = StreamInputSession(controller: { devices.current })
        input.setTouchButton(16, pressed: true, id: "a")
        XCTAssertEqual(input.outputState(now: 0).buttonMask, 16)

        let controller = GCController.withExtendedGamepad()
        let gamepad = try XCTUnwrap(controller.extendedGamepad)
        gamepad.buttonA.setValue(1)
        devices.current = controller
        XCTAssertEqual(input.outputState(now: 8), .neutral)
        gamepad.buttonA.setValue(0)
        XCTAssertEqual(input.outputState(now: 16), .neutral)
        gamepad.buttonA.setValue(1)
        XCTAssertEqual(input.outputState(now: 24).buttonMask, 16)

        devices.current = nil
        XCTAssertEqual(input.outputState(now: 32), .neutral)
        input.setTouchButton(32, pressed: true, id: "b")
        XCTAssertEqual(input.outputState(now: 40).buttonMask, 32)
    }

    func testTwelveAndThirteenByteRumblePreserveDelayRepeatAndWeakShortPulse() throws {
        let payload: [UInt8] = [128, 0, 0, 1, 2, 3, 4, 200, 0, 100, 0, 2]
        let effect = try XCTUnwrap(IOSStreamRumblePacketDecoder.decode(Data(payload)).first)
        XCTAssertEqual(effect.durationMilliseconds, 200)
        XCTAssertEqual(effect.delayMilliseconds, 100)
        XCTAssertEqual(effect.repeatCount, 2)
        let pulse = try XCTUnwrap(
            IOSStreamHapticsRouter.pulses(
                for: effect,
                supportedRoutes: [.leftHandle, .rightHandle, .leftTrigger, .rightTrigger]
            ).first)
        XCTAssertEqual(pulse.startTimesMilliseconds, [100, 400, 700])
        let extended: [UInt8] = [128, 0] + Array(payload.dropFirst())
        XCTAssertEqual(IOSStreamRumblePacketDecoder.decode(Data(extended)).first, effect)
        let short = IOSStreamRumbleEffect(
            gamepadIndex: 0, strongMagnitude: 0.01, weakMagnitude: 0, leftTrigger: 0,
            rightTrigger: 0,
            durationMilliseconds: 4, format: .betterXcloud)
        let shortPulse = try XCTUnwrap(
            IOSStreamHapticsRouter.pulses(for: short, supportedRoutes: [.leftHandle]).first)
        XCTAssertEqual(shortPulse.durationMilliseconds, 4)
        XCTAssertEqual(shortPulse.intensity, 0.01)
    }

    func testCloudFavoritesAreIsolatedBetweenAccountsAndRetainedOnRebind() async throws {
        await MainActor.run {
            let suite = "XBXRC.Favorites.\(UUID().uuidString)"
            let defaults = UserDefaults(suiteName: suite)!
            defer { defaults.removePersistentDomain(forName: suite) }
            let favorites = CloudFavoritesStore(defaults: defaults)
            favorites.bind(accountID: "first")
            favorites.toggle("product")
            XCTAssertTrue(favorites.productIDs.contains("product"))
            favorites.bind(accountID: "second")
            XCTAssertTrue(favorites.productIDs.isEmpty)
            favorites.bind(accountID: "first")
            XCTAssertTrue(favorites.productIDs.contains("product"))
            favorites.bind(accountID: nil)
            XCTAssertTrue(favorites.productIDs.isEmpty)
        }
    }
}

private enum MigrationLayoutRoute: AppSecondaryRoute {
    case streaming
    var title: String { "串流体验" }
    var subtitle: String { "画面、音频、分辨率与码率" }
    var icon: AppIcon { .video }
}

@MainActor
private final class MigrationNavigationState: ObservableObject {
    @Published var selection: MigrationLayoutRoute?
    @Published var primarySelection: AppSection = .settings
}

private struct MigrationPrimaryNavigationHarness: View {
    @ObservedObject var state: MigrationNavigationState
    var body: some View { AppNavigationShell(selection: $state.primarySelection) }
}

private struct MigrationNavigationHarness: View {
    @ObservedObject var state: MigrationNavigationState

    var body: some View {
        AdaptiveSectionNavigation(title: "设置", selection: $state.selection) {
            Text("应用偏好")
        } detail: { _ in
            Text("串流体验").appThemeCanvas()
        }
    }
}

@MainActor
private final class MigrationControllerSource {
    var current: GCController?
}

private actor MigrationDataClient: XboxDataClient {
    private var friendStarted = false
    private var friendWaiters: [CheckedContinuation<Void, Never>] = []
    private var friendContinuation: CheckedContinuation<[XboxFriendSummary], Never>?
    private var installedCalls: [XboxInstalledSource: Int] = [:]

    func waitForFriendRequest() async {
        if friendStarted { return }
        await withCheckedContinuation { friendWaiters.append($0) }
    }
    func finishFriendRequest() {
        friendContinuation?.resume(returning: [
            XboxFriendSummary(
                id: "old", displayName: "Old", gamertag: "Old", avatarURL: nil, gamerscore: nil,
                presenceState: nil,
                presenceText: nil)
        ])
        friendContinuation = nil
    }
    func loadFriends(webTokenJSON: String) async throws -> [XboxFriendSummary] {
        await withCheckedContinuation {
            friendContinuation = $0
            friendStarted = true
            for waiter in friendWaiters { waiter.resume() }
            friendWaiters = []
        }
    }
    func loadMedia(webTokenJSON: String, kind: XboxAlbumKind, continuation: String?) async throws
        -> XboxAlbumPage
    {
        XboxAlbumPage(
            items: [
                XboxAlbumMedia(
                    id: continuation == nil ? "first" : "second", kind: kind, titleID: nil,
                    titleName: "Game",
                    capturedAt: nil, durationSeconds: nil, thumbnailURL: nil, url: nil,
                    expiresAt: nil)
            ], continuation: "next")
    }
    func loadInstalledLibrary(
        webTokenJSON: String, consoleID: String, market: String, language: String
    ) async throws
        -> XboxInstalledLibrarySnapshot
    {
        return XboxInstalledLibrarySnapshot(
            consoleID: consoleID,
            games: [
                XboxInstalledGameSummary(
                    id: "game", titleID: nil, productID: nil, name: "Installed",
                    artworkURL: nil,
                    storageDeviceID: "internal", sizeBytes: nil, installedAt: nil,
                    lastPlayedAt: nil)
            ],
            storage: [
                XboxHostStorageSummary(
                    id: "internal", name: "Internal", freeBytes: nil, totalBytes: nil)
            ],
            powerState: "On", notices: [])
    }
    func loadInstalledSection(
        webTokenJSON: String, consoleID: String, market: String, language: String,
        source: XboxInstalledSource, productIDs: [String]
    ) async throws -> InstalledSectionResult {
        installedCalls[source, default: 0] += 1
        if installedCalls[source, default: 0] > 1, source == .games || source == .storage {
            throw SectionRequestFailure(kind: .timeout)
        }
        return InstalledSectionResult(
            snapshot: try await loadInstalledLibrary(
                webTokenJSON: webTokenJSON, consoleID: consoleID, market: market, language: language
            ))
    }
    func loadHosts(webTokenJSON: String) async throws -> [XboxHostSummary] { [] }
    func powerOn(webTokenJSON: String, consoleID: String) async throws -> HostPowerCommandResult {
        HostPowerCommandResult(consoleID: consoleID, accepted: true)
    }
    func powerOff(webTokenJSON: String, consoleID: String) async throws -> HostPowerCommandResult {
        HostPowerCommandResult(consoleID: consoleID, accepted: true)
    }
    func loadGameLibrary(webTokenJSON: String) async throws -> [GameSummary] { [] }
    func loadPlaytimes(webTokenJSON: String, titleIDs: [String]) async throws -> [TitlePlaytime] {
        []
    }
    func loadAchievements(webTokenJSON: String, titleID: String, locale: String) async throws
        -> [AchievementSummary]
    {
        []
    }
}
