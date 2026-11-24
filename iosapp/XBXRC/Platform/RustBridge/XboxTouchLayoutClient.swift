import Foundation

protocol XboxTouchLayoutClient: Sendable {
    func catalog(accessHandle: String, game: CloudLibraryGame, serverRegion: String) async throws
        -> OfficialTouchCatalog
    func download(
        accessHandle: String, titleID: String, bundleID: String, game: CloudLibraryGame,
        locale: String, displayScale: Double
    ) async throws -> any OfficialTouchBundleProviding
    func savedLayouts() async throws -> [XboxSavedTouchLayout]
    func save(bundle: any OfficialTouchBundleProviding, layoutID: String) async throws
        -> [XboxSavedTouchLayout]
    func load(id: String) async throws -> XboxTouchLayoutSnapshot
    func delete(id: String) async throws -> [XboxSavedTouchLayout]
    func markUsed(id: String) async throws
}

struct RustOfficialTouchBundle: OfficialTouchBundleProviding {
    let bundle: XboxOfficialTouchBundle
    var version: String { bundle.version() }
    var choices: [XboxTouchLayoutChoice] { bundle.choices() }
    func preview(layoutID: String) async throws -> XboxTouchLayoutSnapshot {
        try await bundle.preview(layoutId: layoutID)
    }
}

actor RustXboxTouchLayoutClient: XboxTouchLayoutClient {
    private let rootDirectory: URL
    private var library: XboxTouchLayoutLibrary?

    init(
        rootDirectory: URL = URL.applicationSupportDirectory.appendingPathComponent("TouchLayouts")
    ) {
        self.rootDirectory = rootDirectory
    }

    private func openLibrary() throws -> XboxTouchLayoutLibrary {
        if let library { return library }
        let opened = try XboxTouchLayoutLibrary(rootDirectory: rootDirectory.path)
        library = opened
        return opened
    }

    func catalog(accessHandle: String, game: CloudLibraryGame, serverRegion: String) async throws
        -> OfficialTouchCatalog
    {
        let operationID = UUID().uuidString
        var stage = "titleIdentity"
        IOSRuntimeTrace.event(
            domain: "touch-layouts", event: "officialTouchCatalogStarted",
            payload: ["hasTitleIdentity": .bool(game.xboxTitleID?.isEmpty == false)],
            dimension: .core, importance: .key, operationID: operationID)
        do {
            let titleID: String
            if let known = game.xboxTitleID, !known.isEmpty {
                titleID = known
            } else if let resolved = try await resolveCloudGame(
                accessHandle: accessHandle, productId: game.productID, serverRegion: serverRegion,
                startup: nil), let identity = resolved.xboxTitleId, !identity.isEmpty
            {
                titleID = identity
            } else {
                throw TouchLayoutError.invalid("此游戏的 Xbox 身份暂缺")
            }
            try Task.checkCancellation()
            stage = "cloudTitleDetails"
            async let official = Self.catalogSource(.official) {
                try await fetchOfficialTouchBundles(
                    accessHandle: accessHandle, xboxTitleId: titleID, productId: game.productID)
            }
            async let community = Self.catalogSource(.community) {
                try await fetchCommunityTouchBundles(
                    xboxTitleId: titleID, productId: game.productID)
            }
            let sources = try await [official, community]
            let bundles = sources.flatMap(\.bundles)
            try Task.checkCancellation()
            IOSRuntimeTrace.event(
                domain: "touch-layouts", event: "officialTouchCatalogSucceeded",
                payload: [
                    "bundleCount": .integer(Int64(bundles.count)),
                    "failedSourceCount": .integer(
                        Int64(sources.filter { $0.failure != nil }.count)),
                ],
                dimension: .core, importance: .key, operationID: operationID)
            return OfficialTouchCatalog(titleID: titleID, bundles: bundles, sources: sources)
        } catch {
            IOSRuntimeTrace.event(
                domain: "touch-layouts",
                event: SectionRequestFailure.isCancellation(error)
                    ? "officialTouchCatalogCancelled" : "officialTouchCatalogFailed",
                payload: CloudLibraryDiagnostics.errorPayload(
                    error, extra: ["stage": .string(stage)]),
                dimension: .core, importance: .key, operationID: operationID)
            throw error
        }
    }

    private static func catalogSource(
        _ source: GameTouchLayoutSource,
        operation: @Sendable () async throws -> [XboxOfficialTouchBundleChoice]
    ) async throws -> TouchCatalogSourceResult {
        let operationID = UUID().uuidString
        IOSRuntimeTrace.event(
            domain: "touch-layouts", event: "touchCatalogSourceStarted",
            payload: ["source": .string(source.rawValue)],
            dimension: .core, importance: .key, operationID: operationID)
        do {
            let bundles = try await operation()
            try Task.checkCancellation()
            IOSRuntimeTrace.event(
                domain: "touch-layouts", event: "touchCatalogSourceSucceeded",
                payload: [
                    "source": .string(source.rawValue),
                    "bundleCount": .integer(Int64(bundles.count)),
                ],
                dimension: .core, importance: .key, operationID: operationID)
            return .init(source: source, bundles: bundles, failure: nil)
        } catch {
            IOSRuntimeTrace.event(
                domain: "touch-layouts",
                event: SectionRequestFailure.isCancellation(error)
                    ? "touchCatalogSourceCancelled" : "touchCatalogSourceFailed",
                payload: CloudLibraryDiagnostics.errorPayload(
                    error, extra: ["source": .string(source.rawValue)]),
                dimension: .core, importance: .key, operationID: operationID)
            if SectionRequestFailure.isCancellation(error) { throw error }
            return .init(source: source, bundles: [], failure: SectionRequestFailure(error: error))
        }
    }

    func download(
        accessHandle: String, titleID: String, bundleID: String, game: CloudLibraryGame,
        locale: String, displayScale: Double
    ) async throws -> any OfficialTouchBundleProviding {
        let bundle: XboxOfficialTouchBundle
        if GameTouchLayoutSource.identify(bundleID) == .community {
            bundle = try await fetchCommunityTouchBundle(
                xboxTitleId: titleID, bundleId: bundleID, productId: game.productID,
                gameName: game.name, locale: locale, displayScale: displayScale)
        } else {
            bundle = try await fetchOfficialTouchBundle(
                accessHandle: accessHandle, xboxTitleId: titleID, bundleId: bundleID,
                productId: game.productID, gameName: game.name, locale: locale,
                displayScale: displayScale)
        }
        return RustOfficialTouchBundle(bundle: bundle)
    }

    func savedLayouts() async throws -> [XboxSavedTouchLayout] {
        try await openLibrary().savedLayouts()
    }

    func save(bundle: any OfficialTouchBundleProviding, layoutID: String) async throws
        -> [XboxSavedTouchLayout]
    {
        guard let source = bundle as? RustOfficialTouchBundle else {
            throw TouchLayoutError.invalid("游戏布局包来源无效")
        }
        try Task.checkCancellation()
        return try await openLibrary().save(bundle: source.bundle, layoutId: layoutID)
    }

    func load(id: String) async throws -> XboxTouchLayoutSnapshot {
        try await openLibrary().loadLayout(id: id)
    }

    func delete(id: String) async throws -> [XboxSavedTouchLayout] {
        try await openLibrary().deleteLayout(id: id)
    }

    func markUsed(id: String) async throws { try await openLibrary().markUsed(id: id) }
}
