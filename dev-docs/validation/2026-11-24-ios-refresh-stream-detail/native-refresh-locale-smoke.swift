
extension PCDataParityTests {
    @MainActor
    func testNativePullRefreshAndInstalledLocaleSmoke() async throws {
        let suite = "XBXRC.RefreshLocaleSmoke.\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        let settings = AppSettingsStore(defaults: defaults)
        let scene = try XCTUnwrap(
            UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first)
        let previous = scene.windows.first { $0.isKeyWindow }
        let window = UIWindow(windowScene: scene)
        let state = RefreshLocaleSmokeState()
        let gate = PCResponseGate<Bool>()
        let hosting = UIHostingController(
            rootView: AnyView(
                RefreshLocaleSmokeHarness(state: state, gate: gate)
                    .environmentObject(settings).preferredColorScheme(.dark)))
        window.frame = CGRect(x: 0, y: 0, width: 390, height: 844)
        window.rootViewController = hosting
        window.makeKeyAndVisible()
        defer {
            window.isHidden = true
            window.rootViewController = nil
            previous?.makeKey()
        }
        try await Task.sleep(for: .milliseconds(350))
        print("RefreshLocaleSmoke: native window ready")
        let before = smokeSpinners(hosting.view)
        print("RefreshLocaleSmoke: before spinners=\(before)")
        XCTAssertGreaterThanOrEqual(before, 3)
        let scroll = try XCTUnwrap(smokeScroll(hosting.view))
        print("RefreshLocaleSmoke: scroll=\(type(of: scroll)), refreshControl=\(String(describing: scroll.refreshControl))")
        let control = try XCTUnwrap(scroll.refreshControl)
        control.beginRefreshing()
        scroll.setContentOffset(CGPoint(x: 0, y: -100), animated: false)
        control.sendActions(for: .valueChanged)
        try await waitFor { state.isPullRefreshing }
        XCTAssertTrue(control.isRefreshing)
        try await Task.sleep(for: .milliseconds(180))
        let during = smokeSpinners(hosting.view)
        XCTAssertEqual(during, 0)
        smokeSnapshot(hosting.view, name: "refresh-content-waiting")
        await gate.resolve(true)
        try await waitFor { !state.isPullRefreshing && !control.isRefreshing }
        try await Task.sleep(for: .milliseconds(180))
        let after = smokeSpinners(hosting.view)
        XCTAssertEqual(after, before)

        let auth = AuthStore(
            settings: settings, client: PCPageAuthClient(), cloudClient: PCPageCloudClient(),
            keychain: PCPageSessionStore())
        await auth.restore()
        let data = XboxDataStore(client: PCPageDataClient(), preferredGameLocaleProvider: settings)
        await data.sync(session: auth.session, ownerGeneration: auth.ownerGeneration)
        let requests = RefreshLocaleSmokeRequests()
        var client = ScriptedPCCloudClient()
        client.detail = { product, market, language in
            await requests.record("\(market):\(language):\(product)")
            let base = try await PCPageCloudClient().loadProductDetail(
                productID: product, market: market, language: language)
            return CloudProductDetails(
                productID: product,
                name: language == "en-US" ? "Forza Horizon 5" : "極限競速：地平線 5",
                publisherName: base.publisherName,
                description: language == "en-US"
                    ? "Explore Mexico with friends." : "駕駛各式賽車探索墨西哥的海岸、雨林與山地，與好友共同完成挑戰。",
                tileURL: base.tileURL, posterURL: base.posterURL, heroURL: base.heroURL,
                categories: base.categories, developer: base.developer,
                releaseDate: base.releaseDate,
                rating: base.rating, attributes: base.attributes, screenshots: [],
                platforms: base.platforms, languages: base.languages,
                accessibility: base.accessibility)
        }
        settings.mobile.catalogMarket = "TW"
        settings.mobile.catalogLanguage = "zh-TW"
        let cloud = CloudLibraryStore(client: client, settingsProvider: settings)
        let streaming = StreamingFeatureStore(settingsProvider: settings)
        let installed = XboxInstalledGameSummary(
            id: "installed-smoke", titleID: "2", productID: "FORZA0000001", name: "Forza Horizon 5",
            artworkURL: nil, storageDeviceID: "internal", sizeBytes: 100_000_000_000,
            installedAt: nil, lastPlayedAt: nil)
        hosting.rootView = AnyView(
            NavigationStack {
                PersonalGameDetailView(
                    game: PCPageFixtures.games[1], installedGame: installed,
                    host: PCPageFixtures.host)
            }
            .environmentObject(settings).environmentObject(auth).environmentObject(data)
            .environmentObject(cloud).environmentObject(streaming)
            .environment(\.locale, Locale(identifier: "zh-Hans"))
            .preferredColorScheme(.dark))
        try await waitFor { cloud.productDetailState("FORZA0000001").value?.name == "極限競速：地平線 5" }
        try await Task.sleep(for: .milliseconds(200))
        XCTAssertEqual(
            data.libraryGameStates["self:2"]?.value?.achievementProgress?.unlockedCount, 10)
        smokeSnapshot(hosting.view, name: "installed-detail-configured-chinese")
        settings.mobile.catalogMarket = "US"
        settings.mobile.catalogLanguage = "en-US"
        try await waitFor {
            cloud.productDetailState("FORZA0000001").value?.name == "Forza Horizon 5"
        }
        let observed = await requests.values
        XCTAssertEqual(observed, ["TW:zh-TW:FORZA0000001", "US:en-US:FORZA0000001"])
        let summary = XCTAttachment(
            string:
                "refreshSpinnersBefore=\(before)\nrefreshSpinnersDuring=\(during)\nrefreshSpinnersAfter=\(after)\nrequests=\(observed)"
        )
        summary.name = "native-refresh-locale-summary"
        summary.lifetime = .keepAlways
        add(summary)
    }

    @MainActor
    private func smokeSpinners(_ view: UIView) -> Int {
        if view is UIRefreshControl { return 0 }
        let own = (view as? UIActivityIndicatorView)?.isAnimating == true ? 1 : 0
        return own + view.subviews.reduce(0) { $0 + smokeSpinners($1) }
    }

    @MainActor
    private func smokeScroll(_ view: UIView) -> UIScrollView? {
        if let scroll = view as? UIScrollView { return scroll }
        for child in view.subviews {
            if let scroll = smokeScroll(child) { return scroll }
        }
        return nil
    }

    @MainActor
    private func smokeSnapshot(_ view: UIView, name: String) {
        view.layoutIfNeeded()
        let format = UIGraphicsImageRendererFormat()
        format.scale = 1
        let image = UIGraphicsImageRenderer(size: view.bounds.size, format: format).image { _ in
            XCTAssertTrue(view.drawHierarchy(in: view.bounds, afterScreenUpdates: true))
        }
        let attachment = XCTAttachment(image: image)
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }
}

@MainActor
private final class RefreshLocaleSmokeState: ObservableObject {
    @Published var isPullRefreshing = false
    var action: RefreshAction?
}

private actor RefreshLocaleSmokeRequests {
    private(set) var values: [String] = []
    func record(_ value: String) { values.append(value) }
}

private struct RefreshLocaleSmokeHarness: View {
    @ObservedObject var state: RefreshLocaleSmokeState
    let gate: PCResponseGate<Bool>
    var body: some View {
        AppPageScroll {
            VStack(spacing: 20) {
                AppSectionRequestState(
                    title: "游戏资料",
                    status: SectionRequestStatus(phase: .loaded, isBusy: true, hasSnapshot: true),
                    identifier: "refresh-smoke")
                AppRefreshButton("刷新", isRefreshing: true) {}
                AppLoadMoreButton(isLoading: true) {}
                RefreshLocaleSmokeCapture(state: state)
            }.padding(24)
        }
        .appThemeCanvas()
        .appRefreshable { _ = await gate.wait() }
    }
}

private struct RefreshLocaleSmokeCapture: View {
    @Environment(\.refresh) private var action
    @Environment(\.appIsPullRefreshing) private var isPullRefreshing
    @ObservedObject var state: RefreshLocaleSmokeState
    var body: some View {
        Text(isPullRefreshing ? "原生刷新处理中" : "原生刷新待命")
            .onChange(of: isPullRefreshing, initial: true) { _, value in
                print("RefreshLocaleSmoke: pullRefreshing=\(value)")
                state.isPullRefreshing = value
            }
    }
}
