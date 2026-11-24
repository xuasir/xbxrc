// 临时追加到 StreamingControlsStateTests.swift，定向验收后恢复测试源。
extension StreamingControlsStateTests {
    @MainActor
    func testNativeHiddenControlsKeepTouchHitRouting() async throws {
        let suite = "XBXRC.QuickHitSmoke.\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        let settings = AppSettingsStore(defaults: defaults)
        let store = StreamingFeatureStore(settingsProvider: settings)
        store.prepareControlsUITestPreview()
        let root = ZStack {
            Color.black
            TouchGamepadOverlay(input: store.input, layout: store.touchLayout)
            StreamingControlsOverlay(
                store: store, revealToken: 0, revealSource: .blankTap,
                autoHideAfter: .milliseconds(100))
        }
        .background { StreamingControlsGestureLayer(onBlankTap: {}) {} }
        .environmentObject(settings).environmentObject(SavedTouchLayoutsStore())
        let scene = try XCTUnwrap(
            UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first)
        let previous = scene.windows.first { $0.isKeyWindow }
        let window = UIWindow(windowScene: scene)
        let host = UIHostingController(rootView: root)
        window.frame = CGRect(x: 0, y: 0, width: 390, height: 844)
        window.rootViewController = host
        window.makeKeyAndVisible()
        defer {
            window.isHidden = true
            window.rootViewController = nil
            previous?.makeKey()
        }
        try await Task.sleep(for: .milliseconds(500))
        host.view.layoutIfNeeded()
        func findSurface(_ view: UIView) -> TouchLayoutSurfaceView? {
            if let surface = view as? TouchLayoutSurfaceView { return surface }
            for child in view.subviews {
                if let surface = findSurface(child) { return surface }
            }
            return nil
        }
        let surface = try XCTUnwrap(findSurface(host.view))
        let socket = try XCTUnwrap(surface.input.sockets.first { $0.id == "right/inner/1" })
        let point = surface.convert(socket.center, to: window)
        let hit = window.hitTest(point, with: nil)
        var ancestors: [[String: Any]] = []
        var target: UIView? = surface
        while let view = target {
            ancestors.append([
                "type": String(describing: type(of: view)),
                "interactive": view.isUserInteractionEnabled,
                "hidden": view.isHidden, "alpha": view.alpha,
            ])
            target = view.superview
        }
        let diagnostic: [String: Any] = [
            "point": [point.x, point.y],
            "hitType": hit.map { String(describing: type(of: $0)) } ?? "nil",
            "surfaceHit": hit === surface, "inputSuppressed": store.input.isSuppressed,
            "ancestors": ancestors,
        ]
        let data = try JSONSerialization.data(
            withJSONObject: diagnostic, options: [.prettyPrinted, .sortedKeys])
        let attachment = XCTAttachment(data: data, uniformTypeIdentifier: "public.json")
        attachment.name = "hidden-controls-hit-routing"
        attachment.lifetime = .keepAlways
        add(attachment)
        print("QuickHitSmoke: \(String(decoding: data, as: UTF8.self))")
        XCTAssertFalse(store.input.isSuppressed)
        XCTAssertTrue(hit === surface)
    }
}
