// BEGIN IOS BANNER HEADER SMOKE PROBE
private enum BannerHeaderProbePage: Hashable {
    case personal, cloud, console
}

@MainActor
@Observable
private final class BannerHeaderProbeState {
    var rootURL: URL
    var path: [BannerHeaderProbePage] = []

    init(rootURL: URL) { self.rootURL = rootURL }
}

@MainActor
private struct BannerHeaderProbe: View {
    @Bindable var state: BannerHeaderProbeState
    let game: GameSummary

    var body: some View {
        NavigationStack(path: $state.path) {
            AppInformationPage {
                Section {
                    Text("玩家身份").appFont(.hero, weight: .semibold)
                        .frame(minHeight: 160, alignment: .bottomLeading)
                    Text("通顶背景与标题栏共享窗口坐标")
                }
            }
            .appPageToolbar("我的", showsBrand: true, artwork: .avatar(state.rootURL))
            .navigationDestination(for: BannerHeaderProbePage.self) { page in
                switch page {
                case .personal:
                    PersonalGameDetailView(game: game)
                case .cloud:
                    GameDetailView(gameSummary: game)
                case .console:
                    AppInformationPage {
                        Section {
                            Text("Xbox Series X").appFont(.hero, weight: .semibold)
                                .frame(minHeight: 216, alignment: .bottomLeading)
                            Text("主机图片沿相同透出与渐隐规则呈现")
                        }
                    }
                    .appPageToolbar("主机", artwork: .console("ConsoleSeriesX"))
                }
            }
        }
    }
}

extension CapabilityMigrationTests {
    @MainActor
    func testScopedBannerHeaderSmoke() async throws {
        let suite = "XBXRC.BannerHeader.\(UUID().uuidString)"
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
        await data.activateLibraryOnce()
        let cloud = CloudLibraryStore(client: PCPageCloudClient(), settingsProvider: settings)
        let streaming = StreamingFeatureStore(settingsProvider: settings)
        let favorites = CloudFavoritesStore(defaults: defaults)

        // 固定图片直接写入本轮 URL 缓存，覆盖实际远程图片组件的成功绘制路径。
        let colors: [UIColor] = [
            UIColor(red: 0.78, green: 0.18, blue: 0.24, alpha: 1),
            UIColor(red: 0.10, green: 0.34, blue: 0.78, alpha: 1),
            UIColor(red: 0.14, green: 0.64, blue: 0.28, alpha: 1),
        ]
        var urls: [URL] = []
        for (index, color) in colors.enumerated() {
            let url = try XCTUnwrap(
                URL(string: "https://banner-smoke.invalid/\(suite)/\(index).png"))
            let format = UIGraphicsImageRendererFormat()
            format.scale = 1
            let image = UIGraphicsImageRenderer(
                size: CGSize(width: 1280, height: 720), format: format
            ).image { context in
                color.setFill()
                context.fill(CGRect(x: 0, y: 0, width: 1280, height: 720))
                UIColor.white.withAlphaComponent(0.28).setFill()
                context.fill(CGRect(x: 0, y: 0, width: 280, height: 720))
                context.fill(CGRect(x: 0, y: 320, width: 1280, height: 110))
                UIColor.black.withAlphaComponent(0.24).setFill()
                context.fill(CGRect(x: 960, y: 0, width: 320, height: 720))
            }
            let response = try XCTUnwrap(
                HTTPURLResponse(
                    url: url, statusCode: 200, httpVersion: "HTTP/1.1",
                    headerFields: ["Content-Type": "image/png", "Cache-Control": "max-age=600"]))
            URLCache.shared.storeCachedResponse(
                CachedURLResponse(response: response, data: try XCTUnwrap(image.pngData())),
                for: URLRequest(url: url))
            _ = try await RemoteImageLoader.shared.image(for: url)
            urls.append(url)
        }
        defer {
            for url in urls { URLCache.shared.removeCachedResponse(for: URLRequest(url: url)) }
        }
        let fixture = PCPageFixtures.games[1]
        let game = GameSummary(
            id: fixture.id, titleID: fixture.titleID, name: fixture.name,
            artworkURL: urls[0], heroURL: urls[1], lastPlayedAt: fixture.lastPlayedAt,
            playtimeMinutes: fixture.playtimeMinutes,
            achievementProgress: fixture.achievementProgress,
            description: fixture.description, productID: fixture.productID)
        let state = BannerHeaderProbeState(rootURL: urls[0])
        let size = CGSize(width: 390, height: 844)
        let root = BannerHeaderProbe(state: state, game: game)
            .environmentObject(settings).environmentObject(auth).environmentObject(data)
            .environmentObject(cloud).environmentObject(streaming).environmentObject(favorites)
            .environmentObject(SavedTouchLayoutsStore())
            .environment(\.locale, Locale(identifier: "zh-Hans"))
            .environment(\.appCanvasBounds, CGRect(origin: .zero, size: size))
            .environment(\.appFloatingNavigationInset, AppLayoutPolicy.floatingNavigationInset)
            .environment(\.colorScheme, .dark).preferredColorScheme(.dark)
        let scene = try XCTUnwrap(
            UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first)
        let previousWindow = scene.windows.first { $0.isKeyWindow }
        let window = UIWindow(windowScene: scene)
        let host = UIHostingController(rootView: root)
        window.frame = CGRect(origin: .zero, size: size)
        window.rootViewController = host
        window.makeKeyAndVisible()
        defer {
            window.isHidden = true
            window.rootViewController = nil
            previousWindow?.makeKey()
        }

        func snapshot(_ name: String?) async throws -> UIImage {
            try await Task.sleep(for: .milliseconds(500))
            for _ in 0..<20 {
                guard navigation(in: host)?.transitionCoordinator != nil else { break }
                try await Task.sleep(for: .milliseconds(50))
            }
            // 原生栈完成进入或返回后，再等待标题与图片提交同一帧。
            try await Task.sleep(for: .milliseconds(50))
            host.view.frame = CGRect(origin: .zero, size: size)
            host.view.layoutIfNeeded()
            let format = UIGraphicsImageRendererFormat()
            format.scale = 1
            var rendered = false
            let image = UIGraphicsImageRenderer(size: size, format: format).image { _ in
                rendered = host.view.drawHierarchy(in: host.view.bounds, afterScreenUpdates: true)
            }
            XCTAssertTrue(rendered)
            if let name {
                let attachment = XCTAttachment(image: image)
                attachment.name = name
                attachment.lifetime = .keepAlways
                add(attachment)
            }
            return image
        }

        func navigation(in controller: UIViewController) -> UINavigationController? {
            if let navigation = controller as? UINavigationController { return navigation }
            return controller.children.lazy.compactMap { navigation(in: $0) }.first
        }

        func headerPixels(_ image: UIImage) throws -> Data {
            let bar = try XCTUnwrap(navigation(in: host)?.navigationBar)
            let region = bar.convert(bar.bounds, to: host.view)
            let sample = CGRect(x: size.width * 0.78, y: region.maxY - 6, width: 4, height: 3)
            let crop = try XCTUnwrap(image.cgImage?.cropping(to: sample))
            return try XCTUnwrap(UIImage(cgImage: crop).pngData())
        }

        let profilePixels = try headerPixels(await snapshot("banner-profile-dark"))
        state.path = [.personal]
        let gamePixels = try headerPixels(await snapshot("banner-game-phone-dark"))
        XCTAssertNotEqual(profilePixels, gamePixels, "进入游戏后标题栏应使用游戏背景")
        state.rootURL = urls[2]
        let updatedBehindGame = try headerPixels(await snapshot(nil))
        XCTAssertEqual(gamePixels, updatedBehindGame, "后台玩家资料更新应保留当前游戏的标题背景")
        state.path = []
        let returnedPixels = try headerPixels(await snapshot("banner-profile-returned-dark"))
        XCTAssertNotEqual(gamePixels, returnedPixels, "返回后标题栏应使用当前玩家的背景")
        XCTAssertNotEqual(profilePixels, returnedPixels, "返回后应使用玩家资料更新后的图片")
        state.path = [.cloud]
        _ = try await snapshot("banner-cloud-dark")
        state.path = [.console]
        _ = try await snapshot("banner-console-dark")

        for (name, layout, theme) in [
            ("phone-light", CGSize(width: 390, height: 844), ColorScheme.light),
            ("landscape-dark", CGSize(width: 844, height: 390), ColorScheme.dark),
        ] {
            let root = NavigationStack { PersonalGameDetailView(game: game) }
                .environmentObject(settings).environmentObject(auth).environmentObject(data)
                .environmentObject(streaming)
                .environment(\.locale, Locale(identifier: "zh-Hans"))
                .environment(\.appCanvasBounds, CGRect(origin: .zero, size: layout))
                .environment(\.appFloatingNavigationInset, AppLayoutPolicy.floatingNavigationInset)
                .environment(\.colorScheme, theme).preferredColorScheme(theme)
            try await attachLayoutSnapshot(root: root, size: layout, name: "banner-game-\(name)")
        }
    }
}
// END IOS BANNER HEADER SMOKE PROBE
