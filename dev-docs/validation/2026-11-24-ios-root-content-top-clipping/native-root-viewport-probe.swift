// BEGIN IOS ROOT VIEWPORT PROBE
@MainActor
@Observable
private final class RootViewportProbeState {
    var selection: AppSection = .hosts
}

@MainActor
private struct RootViewportProbe: View {
    @Bindable var state: RootViewportProbeState
    var body: some View { AppNavigationShell(selection: $state.selection) }
}

extension CapabilityMigrationTests {
    @MainActor
    func testScopedRootViewportGeometry() async throws {
        let suite = "XBXRC.RootViewport.\(UUID().uuidString)"
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
        let state = RootViewportProbeState()
        let root = RootViewportProbe(state: state)
            .environmentObject(settings).environmentObject(auth).environmentObject(data)
            .environmentObject(cloud).environmentObject(streaming).environmentObject(favorites)
            .environmentObject(SavedTouchLayoutsStore())
            .environment(\.locale, Locale(identifier: "zh-Hans"))
            .environment(\.dynamicTypeSize, .large)
            .preferredColorScheme(.light)
        let scene = try XCTUnwrap(
            UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first)
        let previousWindow = scene.windows.first { $0.isKeyWindow }
        let window = UIWindow(windowScene: scene)
        let host = UIHostingController(rootView: root)
        let size = CGSize(width: 390, height: 844)
        window.frame = CGRect(origin: .zero, size: size)
        window.rootViewController = host
        window.makeKeyAndVisible()
        defer {
            window.isHidden = true
            window.rootViewController = nil
            previousWindow?.makeKey()
        }

        func rect(_ value: CGRect) -> [String: Double] {
            ["x": value.minX, "y": value.minY, "width": value.width, "height": value.height]
        }
        func insets(_ value: UIEdgeInsets) -> [String: Double] {
            ["top": value.top, "left": value.left, "bottom": value.bottom, "right": value.right]
        }
        func activeNavigation(in controller: UIViewController) -> UINavigationController? {
            if let navigation = controller as? UINavigationController,
                navigation.viewIfLoaded?.window === window
            {
                return navigation
            }
            return controller.children.lazy.compactMap { activeNavigation(in: $0) }.first
        }
        func scrollViews(in view: UIView) -> [UIScrollView] {
            let own = (view as? UIScrollView).map { [$0] } ?? []
            return own + view.subviews.flatMap { scrollViews(in: $0) }
        }
        func clippingAncestors(of view: UIView) -> [[String: Any]] {
            var records: [[String: Any]] = []
            var current: UIView? = view
            while let ancestor = current {
                if ancestor.clipsToBounds || ancestor.layer.masksToBounds {
                    records.append([
                        "type": String(reflecting: type(of: ancestor)),
                        "frame": rect(ancestor.convert(ancestor.bounds, to: window)),
                        "safeArea": insets(ancestor.safeAreaInsets),
                    ])
                }
                current = ancestor.superview
            }
            return records
        }
        func describeBarView(_ view: UIView, depth: Int = 0) -> [String: Any] {
            [
                "type": String(reflecting: type(of: view)),
                "frame": rect(view.convert(view.bounds, to: window)),
                "bounds": rect(view.bounds),
                "safeArea": insets(view.safeAreaInsets),
                "isHidden": view.isHidden,
                "alpha": view.alpha,
                "clipsToBounds": view.clipsToBounds,
                "zPosition": view.layer.zPosition,
                "autoresizingMask": view.autoresizingMask.rawValue,
                "layerMask": view.layer.mask.map {
                    ["type": String(reflecting: type(of: $0)), "frame": rect($0.frame)]
                } ?? [:],
                "subviews": depth < 5
                    ? view.subviews.map { describeBarView($0, depth: depth + 1) } : [],
            ]
        }

        var records: [[String: Any]] = []
        for section in AppSection.allCases {
            var transaction = Transaction()
            transaction.animation = nil
            withTransaction(transaction) { state.selection = section }
            try await Task.sleep(for: .milliseconds(650))
            host.view.frame = CGRect(origin: .zero, size: size)
            host.view.layoutIfNeeded()
            let navigation = try XCTUnwrap(activeNavigation(in: host))
            let page = try XCTUnwrap(navigation.topViewController?.view)
            let bar = navigation.navigationBar
            let toolbarRegion = bar.convert(bar.bounds, to: window)
            let cover = bar.subviews.first { $0.layer.zPosition == -1 }
            if section == .controller || section == .settings { XCTAssertNotNil(cover) }
            if let cover {
                let coverRegion = cover.convert(cover.bounds, to: window)
                XCTAssertEqual(coverRegion.minY, 0, accuracy: 1,
                    "\(section.title)标题背景应从窗口顶部开始")
                XCTAssertEqual(coverRegion.maxY, toolbarRegion.maxY, accuracy: 1,
                    "\(section.title)标题背景应止于导航真实底边")
            }
            let scrollRecords: [[String: Any]] = scrollViews(in: page).map { scroll in
                [
                    "type": String(reflecting: type(of: scroll)),
                    "viewHierarchy": describeBarView(scroll),
                    "topEdgeHidden": scroll.topEdgeEffect.isHidden,
                    "identifier": scroll.accessibilityIdentifier ?? "",
                    "viewport": rect(scroll.convert(scroll.bounds, to: window)),
                    "contentOriginY": scroll.convert(.zero, to: window).y,
                    "contentSize": [
                        "width": scroll.contentSize.width, "height": scroll.contentSize.height,
                    ],
                    "contentOffsetY": scroll.contentOffset.y,
                    "contentInset": insets(scroll.contentInset),
                    "adjustedContentInset": insets(scroll.adjustedContentInset),
                    "safeArea": insets(scroll.safeAreaInsets),
                    "clipAncestors": clippingAncestors(of: scroll),
                ]
            }
            records.append([
                "section": section.title,
                "window": rect(window.bounds),
                "navigation": rect(navigation.view.convert(navigation.view.bounds, to: window)),
                "toolbar": rect(bar.convert(bar.bounds, to: window)),
                "page": rect(page.convert(page.bounds, to: window)),
                "pageSafeArea": insets(page.safeAreaInsets),
                "scrolls": scrollRecords,
                "barSubviews": describeBarView(bar),
            ])
            let format = UIGraphicsImageRendererFormat()
            format.scale = 1
            var rendered = false
            let image = UIGraphicsImageRenderer(size: size, format: format).image { _ in
                rendered = host.view.drawHierarchy(in: host.view.bounds, afterScreenUpdates: true)
            }
            XCTAssertTrue(rendered)
            if section == .controller || section == .settings {
                let headerRegion = CGRect(
                    x: AppLayoutPolicy.pageInset,
                    y: bar.convert(bar.bounds, to: window).maxY + AppLayoutPolicy.pageInset,
                    width: 120, height: 26)
                let crop = try XCTUnwrap(image.cgImage?.cropping(to: headerRegion))
                var pixels = [UInt8](repeating: 0, count: crop.width * crop.height * 4)
                pixels.withUnsafeMutableBytes { buffer in
                    let context = CGContext(
                        data: buffer.baseAddress, width: crop.width, height: crop.height,
                        bitsPerComponent: 8, bytesPerRow: crop.width * 4,
                        space: CGColorSpaceCreateDeviceRGB(),
                        bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)
                    context?.draw(
                        crop, in: CGRect(x: 0, y: 0, width: crop.width, height: crop.height))
                }
                let values = stride(from: 0, to: pixels.count, by: 4).map { Int(pixels[$0]) }
                let contrast = (values.max() ?? 0) - (values.min() ?? 0)
                XCTAssertGreaterThan(contrast, 30, "\(section.title)首次切换时首行标题应完整绘制")
            }
            let attachment = XCTAttachment(image: image)
            attachment.name = "root-viewport-\(section.rawValue)"
            attachment.lifetime = .keepAlways
            add(attachment)
        }
        let geometryJSON = try JSONSerialization.data(
            withJSONObject: records, options: [.prettyPrinted, .sortedKeys])
        let attachment = XCTAttachment(data: geometryJSON, uniformTypeIdentifier: "public.json")
        attachment.name = "root-viewport-geometry"
        attachment.lifetime = .keepAlways
        add(attachment)
    }
}
// END IOS ROOT VIEWPORT PROBE
