import Foundation

typealias TouchLayoutAccessProvider = @MainActor @Sendable () async throws -> PreparedCloudAccess

protocol XboxTouchLayoutClient: Sendable {
    func catalog(
        game: CloudLibraryGame, serverRegion: String, refresh: Bool,
        prepareAccess: @escaping TouchLayoutAccessProvider
    ) async throws
        -> OfficialTouchCatalog
    func download(
        titleID: String, bundleID: String, game: CloudLibraryGame,
        locale: String, displayScale: Double, prepareAccess: @escaping TouchLayoutAccessProvider
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

/// 请求继续交给公共 Rust API；来源调度允许公共资源独立于 Cloud 授权完成。
struct TouchLayoutRemoteRequests: Sendable {
    var resolveTitle: @Sendable (String, String, String) async throws -> String?
    var officialCatalog:
        @Sendable (String, String, String) async throws -> [XboxOfficialTouchBundleChoice]
    var communityCatalog: @Sendable (String, String) async throws -> [XboxOfficialTouchBundleChoice]
    var download:
        @Sendable (String?, String, String, CloudLibraryGame, String, Double) async throws
            -> any OfficialTouchBundleProviding

    static let rust = Self(
        resolveTitle: { handle, productID, region in
            try await resolveCloudGame(
                accessHandle: handle, productId: productID, serverRegion: region, startup: nil
            )?.xboxTitleId
        },
        officialCatalog: { handle, titleID, productID in
            try await fetchOfficialTouchBundles(
                accessHandle: handle, xboxTitleId: titleID, productId: productID)
        },
        communityCatalog: { titleID, productID in
            try await fetchCommunityTouchBundles(xboxTitleId: titleID, productId: productID)
        },
        download: { handle, titleID, bundleID, game, locale, scale in
            let bundle: XboxOfficialTouchBundle
            if GameTouchLayoutSource.identify(bundleID) == .community {
                bundle = try await fetchCommunityTouchBundle(
                    xboxTitleId: titleID, bundleId: bundleID, productId: game.productID,
                    gameName: game.name, locale: locale, displayScale: scale)
            } else {
                guard let handle else { throw AuthStoreError.signedOut }
                bundle = try await fetchOfficialTouchBundle(
                    accessHandle: handle, xboxTitleId: titleID, bundleId: bundleID,
                    productId: game.productID, gameName: game.name, locale: locale,
                    displayScale: scale)
            }
            return RustOfficialTouchBundle(bundle: bundle)
        })
}

actor RustXboxTouchLayoutClient: XboxTouchLayoutClient {
    private let rootDirectory: URL
    private let requests: TouchLayoutRemoteRequests
    private var library: XboxTouchLayoutLibrary?
    private var communityCatalogCache: [String: [XboxOfficialTouchBundleChoice]] = [:]
    private var communityCatalogRevision: UInt64 = 0

    init(
        rootDirectory: URL = URL.applicationSupportDirectory.appendingPathComponent("TouchLayouts"),
        requests: TouchLayoutRemoteRequests = .rust
    ) {
        self.rootDirectory = rootDirectory
        self.requests = requests
    }

    private func openLibrary() throws -> XboxTouchLayoutLibrary {
        if let library { return library }
        let opened = try XboxTouchLayoutLibrary(rootDirectory: rootDirectory.path)
        library = opened
        return opened
    }

    func catalog(
        game: CloudLibraryGame, serverRegion: String, refresh: Bool = false,
        prepareAccess: @escaping TouchLayoutAccessProvider
    ) async throws
        -> OfficialTouchCatalog
    {
        let operationID = UUID().uuidString
        var stage = "titleIdentity"
        IOSRuntimeTrace.event(
            domain: "touch-layouts", event: "officialTouchCatalogStarted",
            payload: [
                "hasTitleIdentity": .bool(game.xboxTitleID?.isEmpty == false),
                "refresh": .bool(refresh),
            ],
            dimension: .core, importance: .key, operationID: operationID)
        do {
            let titleID: String
            var resolvedAccess: PreparedCloudAccess?
            if let known = game.xboxTitleID, !known.isEmpty {
                titleID = known
            } else {
                stage = "titleIdentityAccess"
                let access = try await prepareAccess()
                resolvedAccess = access
                stage = "titleIdentity"
                guard
                    let identity = try await requests.resolveTitle(
                        access.handle, game.productID, serverRegion), !identity.isEmpty
                else { throw TouchLayoutError.invalid("此游戏的 Xbox 身份暂缺") }
                titleID = identity
            }
            try Task.checkCancellation()
            stage = "catalogSources"
            if refresh {
                communityCatalogRevision &+= 1
                communityCatalogCache.removeValue(forKey: titleID)
            }
            let requests = requests
            let initialAccess = resolvedAccess
            // better-xcloud 对地平线 6 清空官方 TAB，按已知标题直接选社区来源。
            let prefersCommunity =
                UInt64(titleID) == 2_079_757_188
                || game.streamTitleID?.uppercased() == "FORZAHORIZON6"
            let officialAccess: TouchLayoutAccessProvider?
            if prefersCommunity {
                officialAccess = nil
                IOSRuntimeTrace.event(
                    domain: "touch-layouts", event: "touchCatalogSourcePolicyApplied",
                    payload: ["reason": .string("better-xcloud-horizon6-community")],
                    dimension: .core, importance: .key, operationID: operationID)
            } else {
                officialAccess = {
                    if let initialAccess { return initialAccess }
                    return try await prepareAccess()
                }
            }
            async let official = Self.catalogSource(
                .official,
                prepareAccess: officialAccess,
                operation: { handle in
                    if prefersCommunity { return [] }
                    guard let handle else { throw AuthStoreError.signedOut }
                    return try await requests.officialCatalog(handle, titleID, game.productID)
                }
            )
            async let community = communityCatalog(titleID: titleID, productID: game.productID)
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

    private func communityCatalog(
        titleID: String, productID: String
    ) async throws -> TouchCatalogSourceResult {
        try Task.checkCancellation()
        if let bundles = communityCatalogCache[titleID] {
            IOSRuntimeTrace.event(
                domain: "touch-layouts", event: "touchCatalogSourceCacheHit",
                payload: [
                    "source": .string(GameTouchLayoutSource.community.rawValue),
                    "bundleCount": .integer(Int64(bundles.count)),
                ], dimension: .core, importance: .key)
            return .init(source: .community, bundles: bundles, failure: nil)
        }
        let revision = communityCatalogRevision
        let result = try await Self.catalogSource(.community, prepareAccess: nil) { [requests] _ in
            try await requests.communityCatalog(titleID, productID)
        }
        if revision == communityCatalogRevision, result.failure == nil, !result.bundles.isEmpty {
            // 与 better-xcloud 按标题复用成功结果一致；目录保留不可变来源修订。
            if communityCatalogCache.count >= 64 { communityCatalogCache.removeAll() }
            communityCatalogCache[titleID] = result.bundles
        }
        return result
    }

    private static func catalogSource(
        _ source: GameTouchLayoutSource,
        prepareAccess: TouchLayoutAccessProvider?,
        operation: @Sendable (String?) async throws -> [XboxOfficialTouchBundleChoice]
    ) async throws -> TouchCatalogSourceResult {
        let operationID = UUID().uuidString
        var stage = "catalog"
        IOSRuntimeTrace.event(
            domain: "touch-layouts", event: "touchCatalogSourceStarted",
            payload: ["source": .string(source.rawValue)],
            dimension: .core, importance: .key, operationID: operationID)
        do {
            let handle: String?
            if let prepareAccess {
                stage = "cloudAccess"
                handle = try await prepareAccess().handle
            } else {
                handle = nil
            }
            try Task.checkCancellation()
            stage = "catalog"
            let bundles = try await operation(handle)
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
                    error, extra: ["source": .string(source.rawValue), "stage": .string(stage)]),
                dimension: .core, importance: .key, operationID: operationID)
            // 页面任务仍有效时，单个授权等待者的取消只影响该来源。
            if Task.isCancelled { throw error }
            return .init(source: source, bundles: [], failure: SectionRequestFailure(error: error))
        }
    }

    func download(
        titleID: String, bundleID: String, game: CloudLibraryGame,
        locale: String, displayScale: Double, prepareAccess: @escaping TouchLayoutAccessProvider
    ) async throws -> any OfficialTouchBundleProviding {
        let source = GameTouchLayoutSource.identify(bundleID)
        let operationID = UUID().uuidString
        var stage = "download"
        IOSRuntimeTrace.event(
            domain: "touch-layouts", event: "touchBundleDownloadStarted",
            payload: ["source": .string(source.rawValue)],
            dimension: .network, importance: .key, operationID: operationID)
        do {
            let handle: String?
            if source == .official {
                stage = "cloudAccess"
                handle = try await prepareAccess().handle
            } else {
                handle = nil
            }
            try Task.checkCancellation()
            stage = "download"
            let bundle = try await requests.download(
                handle, titleID, bundleID, game, locale, displayScale)
            try Task.checkCancellation()
            IOSRuntimeTrace.event(
                domain: "touch-layouts", event: "touchBundleDownloadSucceeded",
                payload: ["source": .string(source.rawValue)],
                dimension: .network, importance: .key, operationID: operationID)
            return bundle
        } catch {
            IOSRuntimeTrace.event(
                domain: "touch-layouts",
                event: SectionRequestFailure.isCancellation(error)
                    ? "touchBundleDownloadCancelled" : "touchBundleDownloadFailed",
                payload: CloudLibraryDiagnostics.errorPayload(
                    error, extra: ["source": .string(source.rawValue), "stage": .string(stage)]),
                dimension: .network, importance: .key, operationID: operationID)
            throw error
        }
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
