// BEGIN 20261123 Scoped full viewport render probe
extension CapabilityMigrationTests {
    @MainActor
    func testScopedTouchWorkspaceRealViewportSnapshots() async throws {
        let suite = "XBXRC.RealTouchViewport.\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        let settings = AppSettingsStore(defaults: defaults)
        let scene = try XCTUnwrap(
            UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first)
        let previousWindow = scene.windows.first { $0.isKeyWindow }
        func findSurface(_ view: UIView) -> TouchLayoutSurfaceView? {
            if let surface = view as? TouchLayoutSurfaceView { return surface }
            return view.subviews.lazy.compactMap { findSurface($0) }.first
        }
        for editor in [true, false] {
            for (name, size, theme, language, textSize) in [
                ("phone-dark", CGSize(width: 390, height: 844), ColorScheme.dark, "zh-Hans", DynamicTypeSize.large),
                ("landscape-dark", CGSize(width: 844, height: 390), ColorScheme.dark, "en", DynamicTypeSize.large),
                ("narrow-accessibility", CGSize(width: 320, height: 800), ColorScheme.light, "en", DynamicTypeSize.accessibility3),
                ("tablet-light", CGSize(width: 1024, height: 768), ColorScheme.light, "zh-Hans", DynamicTypeSize.large),
            ] {
                let content = editor
                    ? AnyView(TouchLayoutEditorView(template: nil, appearance: .init()))
                    : AnyView(TouchLayoutAdjustmentView(template: nil, appearance: .init()))
                let host = UIHostingController(rootView: NavigationStack { content }
                    .environmentObject(settings)
                    .environment(\.locale, Locale(identifier: language))
                    .environment(\.dynamicTypeSize, textSize)
                    .environment(\.appFloatingNavigationInset, 0)
                    .preferredColorScheme(theme)
                    .transaction { $0.disablesAnimations = true })
                let window = UIWindow(windowScene: scene)
                window.frame = CGRect(origin: .zero, size: size)
                window.rootViewController = host
                window.makeKeyAndVisible()
                try await Task.sleep(for: .milliseconds(400))
                host.view.frame = CGRect(origin: .zero, size: size)
                host.view.layoutIfNeeded()
                try await Task.sleep(for: .milliseconds(160))
                let surface = try XCTUnwrap(findSurface(host.view))
                let expected = StreamingTouchViewportPolicy.frame(in: size)
                XCTAssertEqual(surface.bounds.width, expected.width, accuracy: 1)
                XCTAssertEqual(surface.bounds.height, expected.height, accuracy: 1)
                let actual = surface.convert(surface.bounds, to: host.view)
                XCTAssertEqual(actual.minX, expected.minX, accuracy: 1)
                XCTAssertEqual(actual.minY, expected.minY, accuracy: 1)
                XCTAssertTrue(surface.transform.isIdentity)
                XCTAssertFalse(surface.input.sockets.isEmpty)
                let format = UIGraphicsImageRendererFormat()
                format.scale = 1
                var rendered = false
                let image = UIGraphicsImageRenderer(size: size, format: format).image { _ in
                    rendered = host.view.drawHierarchy(in: host.view.bounds, afterScreenUpdates: true)
                }
                XCTAssertTrue(rendered)
                let attachment = XCTAttachment(image: image)
                attachment.name = "touch-\(editor ? "editor" : "adjustment")-\(name)"
                attachment.lifetime = .keepAlways
                add(attachment)
                window.isHidden = true
                window.rootViewController = nil
                previousWindow?.makeKey()
            }
        }
    }
}
// END 20261123 Scoped full viewport render probe
