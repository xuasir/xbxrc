import CryptoKit
import Foundation

struct CloudCatalogScope: Codable, Equatable, Hashable, Sendable {
    let accountID: String
    let regionHost: String
    let language: String
    let market: String

    var cacheKey: String {
        let catalogFingerprint = Self.fingerprint(
            [regionHost, language, market].joined(separator: "\u{1F}")
        )
        return [Self.fingerprint(accountID), catalogFingerprint]
            .joined(separator: "--")
    }

    static func sanitize(_ value: String) -> String {
        let allowed = CharacterSet.alphanumerics.union(CharacterSet(charactersIn: "-_"))
        return value.unicodeScalars.map { scalar in
            allowed.contains(scalar) ? String(scalar) : "_"
        }.joined()
    }

    static func fingerprint(_ value: String) -> String {
        SHA256.hash(data: Data(value.utf8))
            .map { String(format: "%02x", $0) }
            .joined()
    }
}

enum CloudCatalogCacheState: String, Codable, Equatable, Sendable {
    case miss
    case fresh
    case stale
    case expired
}

struct CloudCatalogSnapshot: Codable, Equatable, Sendable {
    static let schemaVersion = 1
    static let baseRenderableDuration: TimeInterval = 7 * 24 * 60 * 60
    static let overlayFreshDuration: TimeInterval = 10 * 60
    static let overlayRenderableDuration: TimeInterval = 24 * 60 * 60

    let schemaVersion: Int
    let scope: CloudCatalogScope
    let games: [CloudLibraryGame]
    let baseUpdatedAt: Date
    let overlayUpdatedAt: Date
    let successfulImageURLs: [String: URL]

    init(
        scope: CloudCatalogScope,
        games: [CloudLibraryGame],
        baseUpdatedAt: Date,
        overlayUpdatedAt: Date,
        successfulImageURLs: [String: URL] = [:]
    ) {
        schemaVersion = Self.schemaVersion
        self.scope = scope
        self.games = games
        self.baseUpdatedAt = baseUpdatedAt
        self.overlayUpdatedAt = overlayUpdatedAt
        self.successfulImageURLs = successfulImageURLs
    }

    func cacheState(at now: Date = .now) -> CloudCatalogCacheState {
        guard schemaVersion == Self.schemaVersion else {
            return .miss
        }
        let baseAge = max(0, now.timeIntervalSince(baseUpdatedAt))
        let overlayAge = max(0, now.timeIntervalSince(overlayUpdatedAt))
        guard baseAge <= Self.baseRenderableDuration,
              overlayAge <= Self.overlayRenderableDuration
        else {
            return .expired
        }
        return overlayAge <= Self.overlayFreshDuration ? .fresh : .stale
    }

    func withSuccessfulImage(productID: String, url: URL) -> CloudCatalogSnapshot {
        var urls = successfulImageURLs
        urls[productID] = url
        return CloudCatalogSnapshot(
            scope: scope,
            games: games,
            baseUpdatedAt: baseUpdatedAt,
            overlayUpdatedAt: overlayUpdatedAt,
            successfulImageURLs: urls
        )
    }
}

protocol CloudCatalogSnapshotStoring: Sendable {
    func load(scope: CloudCatalogScope) async throws -> CloudCatalogSnapshot?
    func save(_ snapshot: CloudCatalogSnapshot) async throws
    func saveSuccessfulImagePreferences(_ snapshot: CloudCatalogSnapshot) async throws
    func clearOverlay(accountID: String) async throws
}

extension CloudCatalogSnapshotStoring {
    func saveSuccessfulImagePreferences(_ snapshot: CloudCatalogSnapshot) async throws {
        try await save(snapshot)
    }
}

actor CloudCatalogSnapshotRepository: CloudCatalogSnapshotStoring {
    private let rootDirectory: URL
    private let encoder: JSONEncoder
    private let decoder: JSONDecoder

    init(rootDirectory: URL? = nil) {
        self.rootDirectory = rootDirectory ?? Self.defaultRootDirectory()
        encoder = JSONEncoder()
        encoder.dateEncodingStrategy = .millisecondsSince1970
        encoder.outputFormatting = [.sortedKeys]
        decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .millisecondsSince1970
    }

    func load(scope: CloudCatalogScope) async throws -> CloudCatalogSnapshot? {
        let operationID = UUID().uuidString
        let startedAt = Date()
        IOSRuntimeTrace.event(
            domain: "cloud-library",
            event: "cacheDiskLoadStarted",
            payload: [
                "market": .string(scope.market),
                "language": .string(scope.language),
            ],
            dimension: .core,
            importance: .debug,
            operationID: operationID
        )
        let currentOverlayURL = overlaySnapshotURL(scope: scope)
        let legacyOverlayURL = legacyOverlaySnapshotURL(scope: scope)
        let overlayURL: URL
        if FileManager.default.fileExists(atPath: currentOverlayURL.path) {
            overlayURL = currentOverlayURL
        } else if FileManager.default.fileExists(atPath: legacyOverlayURL.path) {
            overlayURL = legacyOverlayURL
        } else {
            IOSRuntimeTrace.snapshot(
                domain: "cloud-library",
                event: "cacheDiskOverlayMissing",
                payload: [
                    "elapsedMs": .integer(Int64(CloudLibraryDiagnostics.elapsedMilliseconds(since: startedAt))),
                ],
                dimension: .core,
                importance: .debug,
                operationID: operationID
            )
            return nil
        }
        let overlay = try decoder.decode(
            CachedCloudCatalogOverlay.self,
            from: Data(contentsOf: overlayURL)
        )
        guard overlay.schemaVersion == CloudCatalogSnapshot.schemaVersion,
              overlay.scope == scope
        else {
            IOSRuntimeTrace.decision(
                domain: "cloud-library",
                event: "cacheDiskOverlayRejected",
                payload: [
                    "schema": .integer(Int64(overlay.schemaVersion)),
                    "expectedSchema": .integer(Int64(CloudCatalogSnapshot.schemaVersion)),
                    "scopeMatches": .bool(overlay.scope == scope),
                    "elapsedMs": .integer(Int64(CloudLibraryDiagnostics.elapsedMilliseconds(since: startedAt))),
                ],
                dimension: .core,
                importance: .key,
                operationID: operationID
            )
            return nil
        }
        let currentBaseURL = baseSnapshotURL(scope: scope)
        let legacyBaseURL = legacyBaseSnapshotURL(scope: scope)
        let baseURL = FileManager.default.fileExists(atPath: currentBaseURL.path)
            ? currentBaseURL
            : legacyBaseURL
        let base: CachedCloudCatalogBase? = if FileManager.default.fileExists(atPath: baseURL.path) {
            try? decoder.decode(
                CachedCloudCatalogBase.self,
                from: Data(contentsOf: baseURL)
            )
        } else {
            nil
        }
        let baseByProductID = Dictionary(
            uniqueKeysWithValues: (base?.games ?? []).map { ($0.productID, $0) }
        )
        let games = overlay.games.map { overlayGame in
            let baseGame = baseByProductID[overlayGame.productID]
            return CloudLibraryGame(
                productID: overlayGame.productID,
                streamTitleID: overlayGame.streamTitleID,
                xboxTitleID: overlayGame.xboxTitleID,
                name: baseGame?.name.isEmpty == false
                    ? baseGame?.name ?? overlayGame.fallbackName
                    : overlayGame.fallbackName,
                publisherName: baseGame?.publisherName ?? "",
                description: baseGame?.description ?? "",
                tileURL: baseGame?.tileURL,
                posterURL: baseGame?.posterURL,
                heroURL: baseGame?.heroURL,
                artworkURL: overlayGame.artworkURL,
                categories: baseGame?.categories ?? [],
                supportedInputTypes: overlayGame.supportedInputTypes,
                hasEntitlement: overlayGame.hasEntitlement,
                isRecentlyPlayed: overlayGame.isRecentlyPlayed,
                isNew: overlayGame.isNew,
                lastPlayedAt: overlayGame.lastPlayedAt,
                playtimeMinutes: overlayGame.playtimeMinutes,
                achievementProgress: overlayGame.achievementProgress
            )
        }
        let snapshot = CloudCatalogSnapshot(
            scope: scope,
            games: games,
            baseUpdatedAt: base?.updatedAt ?? overlay.updatedAt,
            overlayUpdatedAt: overlay.updatedAt,
            successfulImageURLs: base?.successfulImageURLs ?? [:]
        )
        if overlayURL == legacyOverlayURL || baseURL == legacyBaseURL {
            do {
                try prepareRootDirectory()
                try writeBase(snapshot)
                try writeOverlay(snapshot)
                removeLegacyFiles(scope: scope)
            } catch {
                // 保留旧缓存，下次读取时继续尝试迁移。
            }
        }
        IOSRuntimeTrace.snapshot(
            domain: "cloud-library",
            event: "cacheDiskLoadSucceeded",
            payload: [
                "games": .integer(Int64(snapshot.games.count)),
                "hasBase": .bool(base != nil),
                "successfulImages": .integer(Int64(snapshot.successfulImageURLs.count)),
                "cacheState": .string(snapshot.cacheState().rawValue),
                "elapsedMs": .integer(Int64(CloudLibraryDiagnostics.elapsedMilliseconds(since: startedAt))),
            ],
            dimension: .core,
            importance: .key,
            operationID: operationID
        )
        return snapshot
    }

    func save(_ snapshot: CloudCatalogSnapshot) async throws {
        let operationID = UUID().uuidString
        let startedAt = Date()
        IOSRuntimeTrace.event(
            domain: "cloud-library",
            event: "cacheDiskSaveStarted",
            payload: [
                "games": .integer(Int64(snapshot.games.count)),
                "successfulImages": .integer(Int64(snapshot.successfulImageURLs.count)),
            ],
            dimension: .core,
            importance: .debug,
            operationID: operationID
        )
        do {
            try prepareRootDirectory()
            try writeBase(snapshot)
            try writeOverlay(snapshot)
            removeLegacyFiles(scope: snapshot.scope)
        } catch {
            IOSRuntimeTrace.event(
                domain: "cloud-library",
                event: "cacheDiskSaveFailed",
                payload: CloudLibraryDiagnostics.errorPayload(
                    error,
                    extra: [
                        "games": .integer(Int64(snapshot.games.count)),
                        "elapsedMs": .integer(Int64(CloudLibraryDiagnostics.elapsedMilliseconds(since: startedAt))),
                    ]
                ),
                dimension: .core,
                importance: .key,
                operationID: operationID
            )
            throw error
        }
        IOSRuntimeTrace.event(
            domain: "cloud-library",
            event: "cacheDiskSaveSucceeded",
            payload: [
                "games": .integer(Int64(snapshot.games.count)),
                "elapsedMs": .integer(Int64(CloudLibraryDiagnostics.elapsedMilliseconds(since: startedAt))),
            ],
            dimension: .core,
            importance: .debug,
            operationID: operationID
        )
    }

    func saveSuccessfulImagePreferences(_ snapshot: CloudCatalogSnapshot) async throws {
        try prepareRootDirectory()
        let url = baseSnapshotURL(scope: snapshot.scope)
        let existingBase = try? decoder.decode(
            CachedCloudCatalogBase.self,
            from: Data(contentsOf: url)
        )
        let base: CachedCloudCatalogBase
        if let existingBase,
           existingBase.schemaVersion == snapshot.schemaVersion
        {
            base = CachedCloudCatalogBase(
                schemaVersion: existingBase.schemaVersion,
                updatedAt: existingBase.updatedAt,
                games: existingBase.games,
                successfulImageURLs: snapshot.successfulImageURLs
            )
        } else {
            base = makeBase(snapshot)
        }
        try encoder.encode(base).write(to: url, options: .atomic)
        try applyStorageProtection(to: url)
        try? FileManager.default.removeItem(at: legacyBaseSnapshotURL(scope: snapshot.scope))
    }

    func clearOverlay(accountID: String) async throws {
        guard FileManager.default.fileExists(atPath: rootDirectory.path) else {
            return
        }
        let prefixes = [
            "overlay-v1-" + CloudCatalogScope.fingerprint(accountID) + "--",
            "overlay-v1-" + CloudCatalogScope.sanitize(accountID) + "--",
        ]
        var removedCount = 0
        for url in try FileManager.default.contentsOfDirectory(
            at: rootDirectory,
            includingPropertiesForKeys: nil
        ) where prefixes.contains(where: url.lastPathComponent.hasPrefix) {
            try FileManager.default.removeItem(at: url)
            removedCount += 1
        }
        IOSRuntimeTrace.event(
            domain: "cloud-library",
            event: "cacheOverlayCleared",
            payload: ["removed": .integer(Int64(removedCount))],
            dimension: .core,
            importance: .key
        )
    }

    private func baseSnapshotURL(scope: CloudCatalogScope) -> URL {
        let key = CloudCatalogScope.fingerprint(
            [scope.regionHost, scope.language, scope.market].joined(separator: "\u{1F}")
        )
        return rootDirectory.appendingPathComponent("base-v1-\(key).json")
    }

    private func overlaySnapshotURL(scope: CloudCatalogScope) -> URL {
        rootDirectory.appendingPathComponent("overlay-v1-\(scope.cacheKey).json")
    }

    private func legacyBaseSnapshotURL(scope: CloudCatalogScope) -> URL {
        let key = [scope.regionHost, scope.language, scope.market]
            .map(CloudCatalogScope.sanitize)
            .joined(separator: "--")
        return rootDirectory.appendingPathComponent("base-v1-\(key).json")
    }

    private func legacyOverlaySnapshotURL(scope: CloudCatalogScope) -> URL {
        let key = [scope.accountID, scope.regionHost, scope.language, scope.market]
            .map(CloudCatalogScope.sanitize)
            .joined(separator: "--")
        return rootDirectory.appendingPathComponent("overlay-v1-\(key).json")
    }

    private func prepareRootDirectory() throws {
        try FileManager.default.createDirectory(
            at: rootDirectory,
            withIntermediateDirectories: true
        )
        try applyStorageProtection(to: rootDirectory)
    }

    private func writeBase(_ snapshot: CloudCatalogSnapshot) throws {
        let url = baseSnapshotURL(scope: snapshot.scope)
        try encoder.encode(makeBase(snapshot)).write(to: url, options: .atomic)
        try applyStorageProtection(to: url)
    }

    private func makeBase(_ snapshot: CloudCatalogSnapshot) -> CachedCloudCatalogBase {
        CachedCloudCatalogBase(
            schemaVersion: snapshot.schemaVersion,
            updatedAt: snapshot.baseUpdatedAt,
            games: snapshot.games.map(CachedCloudCatalogBaseGame.init),
            successfulImageURLs: snapshot.successfulImageURLs
        )
    }

    private func writeOverlay(_ snapshot: CloudCatalogSnapshot) throws {
        let overlay = CachedCloudCatalogOverlay(
            schemaVersion: snapshot.schemaVersion,
            scope: snapshot.scope,
            updatedAt: snapshot.overlayUpdatedAt,
            games: snapshot.games.map(CachedCloudCatalogOverlayGame.init)
        )
        let url = overlaySnapshotURL(scope: snapshot.scope)
        try encoder.encode(overlay).write(to: url, options: .atomic)
        try applyStorageProtection(to: url)
    }

    private func applyStorageProtection(to url: URL) throws {
        try FileManager.default.setAttributes(
            [.protectionKey: FileProtectionType.completeUntilFirstUserAuthentication],
            ofItemAtPath: url.path
        )
        var protectedURL = url
        var resourceValues = URLResourceValues()
        resourceValues.isExcludedFromBackup = true
        try protectedURL.setResourceValues(resourceValues)
    }

    private func removeLegacyFiles(scope: CloudCatalogScope) {
        try? FileManager.default.removeItem(at: legacyBaseSnapshotURL(scope: scope))
        try? FileManager.default.removeItem(at: legacyOverlaySnapshotURL(scope: scope))
    }

    private static func defaultRootDirectory() -> URL {
        let applicationSupport = FileManager.default.urls(
            for: .applicationSupportDirectory,
            in: .userDomainMask
        ).first ?? FileManager.default.temporaryDirectory
        return applicationSupport
            .appendingPathComponent("XBXRC", isDirectory: true)
            .appendingPathComponent("CloudCatalog", isDirectory: true)
    }
}

private struct CachedCloudCatalogBase: Codable, Sendable {
    let schemaVersion: Int
    let updatedAt: Date
    let games: [CachedCloudCatalogBaseGame]
    let successfulImageURLs: [String: URL]
}

private struct CachedCloudCatalogBaseGame: Codable, Sendable {
    let productID: String
    let name: String
    let publisherName: String
    let description: String
    let tileURL: URL?
    let posterURL: URL?
    let heroURL: URL?
    let categories: [String]

    init(game: CloudLibraryGame) {
        productID = game.productID
        name = game.name
        publisherName = game.publisherName
        description = game.description
        tileURL = game.tileURL
        posterURL = game.posterURL
        heroURL = game.heroURL
        categories = game.categories
    }
}

private struct CachedCloudCatalogOverlay: Codable, Sendable {
    let schemaVersion: Int
    let scope: CloudCatalogScope
    let updatedAt: Date
    let games: [CachedCloudCatalogOverlayGame]
}

private struct CachedCloudCatalogOverlayGame: Codable, Sendable {
    let productID: String
    let fallbackName: String
    let streamTitleID: String?
    let xboxTitleID: String?
    let artworkURL: URL?
    let supportedInputTypes: [String]
    let hasEntitlement: Bool?
    let isRecentlyPlayed: Bool?
    let isNew: Bool?
    let lastPlayedAt: Date?
    let playtimeMinutes: Int?
    let achievementProgress: AchievementProgress?

    init(game: CloudLibraryGame) {
        productID = game.productID
        fallbackName = game.name
        streamTitleID = game.streamTitleID
        xboxTitleID = game.xboxTitleID
        artworkURL = game.artworkURL
        supportedInputTypes = game.supportedInputTypes
        hasEntitlement = game.hasEntitlement
        isRecentlyPlayed = game.isRecentlyPlayed
        isNew = game.isNew
        lastPlayedAt = game.lastPlayedAt
        playtimeMinutes = game.playtimeMinutes
        achievementProgress = game.achievementProgress
    }
}
