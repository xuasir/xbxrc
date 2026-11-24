#if DEBUG
    import SwiftUI
    import UIKit

    /// UI 验收使用冻结格式夹具；正式路径继续通过 Rust 下载与持久化。
    @MainActor
    enum OfficialTouchLayoutsUITestFixtures {
        static let templateID = "00000000-0000-4000-8000-000000000001"

        static func client() -> any XboxTouchLayoutClient {
            let png = UIGraphicsImageRenderer(size: CGSize(width: 64, height: 64)).pngData {
                context in
                UIColor(red: 0.31, green: 0.69, blue: 0.48, alpha: 1).setFill()
                context.fill(CGRect(x: 12, y: 12, width: 40, height: 40))
                UIImage(systemName: "scope")?.withTintColor(.white, renderingMode: .alwaysOriginal)
                    .draw(in: CGRect(x: 16, y: 16, width: 32, height: 32))
            }
            let mixedSources = ProcessInfo.processInfo.arguments.contains(
                "--game-touch-mixed-sources")
            var bundles = PCPageFixtures.cloudGames().map { game in
                OfficialTouchUITestBundle(game: game, image: png)
            }
            if mixedSources {
                bundles += bundles.map { source in
                    var community = source
                    community.isCommunity = true
                    return community
                }
            }
            return OfficialTouchUITestClient(
                bundles: bundles,
                seed: ProcessInfo.processInfo.arguments.contains("--seed-saved-touch-layouts"),
                failOnce: ProcessInfo.processInfo.arguments.contains(
                    "--official-touch-query-failure"),
                empty: ProcessInfo.processInfo.arguments.contains("--official-touch-empty"),
                mixedSources: mixedSources)
        }

        static func addTemplate(to settings: AppSettingsStore) {
            var template = TouchLayoutTemplate(name: "双摇杆自定义")
            template.id = templateID
            template.appearance = .init(opacity: 0.8, scale: 0.85)
            settings.mobile.touchLayoutTemplates = [template]
        }
    }

    private struct OfficialTouchUITestBundle: OfficialTouchBundleProviding {
        let game: CloudLibraryGame
        let image: Data
        var isCommunity = false
        var version: String { isCommunity ? "4416b1ab0758" : "1.0.0.0" }
        var bundleID: String {
            isCommunity
                ? "community:better-xcloud:4416b1ab07588a3244a5c02ce1fa5dfa64d8df74"
                : "fixture-\(game.xboxTitleID ?? "1")"
        }
        var choices: [XboxTouchLayoutChoice] {
            [
                .init(id: "default", name: "标准操作", isDefault: true, issues: []),
                .init(id: "alternative", name: "简化操作", isDefault: false, issues: []),
                .init(id: "unsupported", name: "待适配操作", isDefault: false, issues: []),
            ]
        }
        func preview(layoutID: String) async throws -> XboxTouchLayoutSnapshot {
            guard let choice = choices.first(where: { $0.id == layoutID }) else {
                throw TouchLayoutError.invalid("夹具布局身份无效")
            }
            let action =
                layoutID == "unsupported"
                ? "mouseLeft" : layoutID == "alternative" ? "gamepadB" : "gamepadA"
            let json =
                #"{"$schema":"https://raw.githubusercontent.com/microsoft/xbox-game-streaming-tools/main/touch-adaptation-kit/schemas/layout/4.1/layout.json","content":{"left":{"inner":[{"type":"joystick","axis":{"input":"axisXY","output":"leftJoystick"}}]},"right":{"inner":[null,{"type":"button","action":"\#(action)","styles":{"default":{"faceImage":{"type":"asset","value":"fire.png"}}}},{"type":"button","action":"gamepadX"},{"type":"joystick","axis":{"input":"axisXY","output":"rightJoystick"}}],"outer":[{"type":"button","action":"rightTrigger"},{"type":"button","action":"leftTrigger"}]},"upper":{"right":[{"type":"button","action":"menu"},{"type":"button","action":"view"}]}}}"#
            return .init(
                id: layoutID, name: choice.name, documentJson: json,
                assets: [.init(id: "fire.png", data: image)], issues: [])
        }
        func record(layoutID: String) -> XboxSavedTouchLayout {
            .init(
                id: isCommunity
                    ? "community:fixture-\(game.xboxTitleID ?? "1"):\(layoutID)"
                    : "official:\(bundleID):\(layoutID)", productId: game.productID,
                xboxTitleId: game.xboxTitleID ?? "1", gameName: game.name,
                bundleId: isCommunity ? "community:better-xcloud" : bundleID,
                bundleVersion: version, layoutId: layoutID,
                layoutName: choices.first(where: { $0.id == layoutID })?.name ?? layoutID,
                isDefault: layoutID == "default", contentHash: bundleID,
                revision: "fixture-r1-\(layoutID)",
                savedAtMs: 1, lastUsedAtMs: 0, byteCount: UInt64(image.count + 1024))
        }
    }

    private actor OfficialTouchUITestClient: XboxTouchLayoutClient {
        private let bundles: [OfficialTouchUITestBundle]
        private var items: [XboxSavedTouchLayout]
        private var failOnce: Bool
        private let empty: Bool
        private let mixedSources: Bool

        init(
            bundles: [OfficialTouchUITestBundle], seed: Bool, failOnce: Bool, empty: Bool,
            mixedSources: Bool
        ) {
            self.bundles = bundles
            items = seed ? bundles.prefix(2).map { $0.record(layoutID: "default") } : []
            self.failOnce = failOnce
            self.empty = empty
            self.mixedSources = mixedSources
        }
        func catalog(accessHandle: String, game: CloudLibraryGame, serverRegion: String)
            async throws -> OfficialTouchCatalog
        {
            if failOnce {
                failOnce = false
                throw XboxBridgeError.Query(
                    failure: XboxQueryFailure(
                        kind: .unknown, statusCode: 404, retriable: false, retryAfterSeconds: nil))
            }
            guard let source = bundles.first(where: { $0.game.productID == game.productID }) else {
                throw TouchLayoutError.invalid("夹具游戏暂缺")
            }
            if mixedSources {
                let choices = bundles.filter {
                    $0.game.productID == game.productID && $0.isCommunity
                }.map {
                    XboxOfficialTouchBundleChoice(
                        id: $0.bundleID, tabVersion: $0.version,
                        manifestVersion: "1.0", layoutVersion: "4.1")
                }
                return .init(
                    titleID: game.xboxTitleID ?? "1", bundles: choices,
                    sources: [
                        .init(
                            source: .official, bundles: [], failure: .init(kind: .invalidResponse)),
                        .init(source: .community, bundles: choices, failure: nil),
                    ])
            }
            return .init(
                titleID: game.xboxTitleID ?? "1",
                bundles: empty
                    ? []
                    : [
                        .init(
                            id: source.bundleID, tabVersion: source.version,
                            manifestVersion: "2.0.0.0", layoutVersion: "4.1.0.0")
                    ])
        }
        func download(
            accessHandle: String, titleID: String, bundleID: String, game: CloudLibraryGame,
            locale: String, displayScale: Double
        ) async throws -> any OfficialTouchBundleProviding {
            guard let bundle = bundles.first(where: { $0.bundleID == bundleID }) else {
                throw TouchLayoutError.invalid("夹具包暂缺")
            }
            return bundle
        }
        func savedLayouts() async throws -> [XboxSavedTouchLayout] { items }
        func save(bundle: any OfficialTouchBundleProviding, layoutID: String) async throws
            -> [XboxSavedTouchLayout]
        {
            guard let source = bundle as? OfficialTouchUITestBundle else {
                throw TouchLayoutError.invalid("夹具包来源无效")
            }
            let item = source.record(layoutID: layoutID)
            items.removeAll { $0.id == item.id }
            items.append(item)
            return items
        }
        func load(id: String) async throws -> XboxTouchLayoutSnapshot {
            guard let item = items.first(where: { $0.id == id }),
                let bundle = bundles.first(where: { $0.bundleID == item.bundleId })
            else { throw TouchLayoutError.invalid("夹具保存项暂缺") }
            var snapshot = try await bundle.preview(layoutID: item.layoutId)
            snapshot.id = item.id
            return snapshot
        }
        func delete(id: String) async throws -> [XboxSavedTouchLayout] {
            items.removeAll { $0.id == id }
            return items
        }
        func markUsed(id: String) async throws {}
    }

    struct StreamingTouchLayoutUITestMarker: View {
        @ObservedObject var controller: StreamingTouchLayoutController
        let session: StreamInputSession
        @State private var lastButtons: UInt16 = 0

        var body: some View {
            VStack {
                Text(verbatim: controller.input.layout?.name ?? "通用 Xbox")
                    .accessibilityIdentifier("stream-touch-active-layout")
                Text(verbatim: String(lastButtons)).accessibilityIdentifier(
                    "stream-touch-last-buttons")
                Text(verbatim: controller.selection.persistentID ?? "standard")
                    .accessibilityIdentifier("stream-touch-selected-id")
            }
            .font(.caption).foregroundStyle(.white).allowsHitTesting(false)
            .onChange(of: controller.selection) { _, _ in lastButtons = 0 }
            .task {
                while !Task.isCancelled {
                    let state = session.outputState(now: ProcessInfo.processInfo.systemUptime)
                    if state.buttonMask != 0 { lastButtons = state.buttonMask }
                    try? await Task.sleep(for: .milliseconds(30))
                }
            }
        }
    }
#endif
