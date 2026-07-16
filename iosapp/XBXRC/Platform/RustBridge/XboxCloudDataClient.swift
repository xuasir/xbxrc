import Foundation

struct PreparedCloudAccess: Sendable {
    let authSession: AuthSession
    let handle: String
    let accountID: String
    let regionHost: String
}

struct RemoteCloudCatalogSnapshot: Sendable {
    let games: [CloudLibraryGame]
    let scope: CloudCatalogScope
    let fetchedAt: Date
    let failedHydrationChunks: Int
    let pendingHydrationProductIDs: [String]
}

struct CloudCatalogMetadata: Sendable {
    let productID: String
    let name: String
    let publisherName: String
    let description: String
    let tileURL: URL?
    let posterURL: URL?
    let heroURL: URL?
    let categories: [String]
}

protocol XboxCloudDataClient: Sendable {
    func prepareAccess(
        refreshToken: String,
        seedJSON: String,
        forceRegionIP: String
    ) async throws -> PreparedCloudAccess
    func loadCatalog(
        accessHandle: String,
        market: String,
        language: String
    ) async throws -> RemoteCloudCatalogSnapshot
    func loadMetadataPage(
        accessHandle: String,
        market: String,
        language: String,
        productIDs: [String]
    ) async throws -> [CloudCatalogMetadata]
    func releaseAccess(handle: String) async
}

struct RustXboxCloudDataClient: XboxCloudDataClient {
    func prepareAccess(
        refreshToken: String,
        seedJSON: String,
        forceRegionIP: String
    ) async throws -> PreparedCloudAccess {
        let operationID = UUID().uuidString
        let startedAt = Date()
        IOSRuntimeTrace.event(
            domain: "auth",
            event: "cloudAccessUniFFIBoundaryStarted",
            payload: [:],
            dimension: .network,
            importance: .key,
            operationID: operationID
        )
        do {
            let result = try await prepareCloudAccess(
                refreshToken: refreshToken,
                seedJson: seedJSON,
                forceRegionIp: forceRegionIP
            )
            IOSRuntimeTrace.event(
                domain: "auth",
                event: "cloudAccessUniFFIBoundarySucceeded",
                payload: [
                    "elapsedMs": .integer(Int64(CloudLibraryDiagnostics.elapsedMilliseconds(since: startedAt))),
                    "appLevel": .integer(Int64(result.authSession.appLevel)),
                ],
                dimension: .network,
                importance: .key,
                operationID: operationID
            )
            return PreparedCloudAccess(
                authSession: result.authSession,
                handle: result.accessHandle,
                accountID: result.accountId,
                regionHost: result.regionHost
            )
        } catch {
            IOSRuntimeTrace.event(
                domain: "auth",
                event: "cloudAccessUniFFIBoundaryFailed",
                payload: CloudLibraryDiagnostics.errorPayload(
                    error,
                    extra: [
                        "elapsedMs": .integer(Int64(CloudLibraryDiagnostics.elapsedMilliseconds(since: startedAt))),
                    ]
                ),
                dimension: .network,
                importance: .essential,
                operationID: operationID
            )
            throw error
        }
    }

    func loadCatalog(
        accessHandle: String,
        market: String,
        language: String
    ) async throws -> RemoteCloudCatalogSnapshot {
        let operationID = UUID().uuidString
        let startedAt = Date()
        IOSRuntimeTrace.event(
            domain: "cloud-library",
            event: "catalogRequestBoundaryStarted",
            payload: [
                "market": .string(market),
                "language": .string(language),
            ],
            dimension: .network,
            importance: .key,
            operationID: operationID
        )
        let snapshot: XboxCloudCatalogSnapshot
        do {
            snapshot = try await fetchCloudCatalog(
                accessHandle: accessHandle,
                market: market,
                language: language
            )
        } catch {
            IOSRuntimeTrace.event(
                domain: "cloud-library",
                event: "catalogRequestBoundaryFailed",
                payload: CloudLibraryDiagnostics.errorPayload(
                    error,
                    extra: [
                        "market": .string(market),
                        "language": .string(language),
                        "elapsedMs": .integer(Int64(CloudLibraryDiagnostics.elapsedMilliseconds(since: startedAt))),
                    ]
                ),
                dimension: .network,
                importance: .essential,
                operationID: operationID
            )
            throw error
        }
        let fetchedAt = Date(timeIntervalSince1970: TimeInterval(snapshot.fetchedAtMs) / 1_000)
        let scope = CloudCatalogScope(
            accountID: snapshot.accountId,
            regionHost: snapshot.regionHost,
            language: snapshot.language,
            market: snapshot.market
        )
        let result = RemoteCloudCatalogSnapshot(
            games: snapshot.games.map { game in
                CloudLibraryGame(
                    productID: game.productId,
                    streamTitleID: game.streamTitleId,
                    xboxTitleID: game.xboxTitleId,
                    name: game.name,
                    publisherName: game.publisherName,
                    description: game.description,
                    tileURL: XboxImageURL.resolve(game.tileImageUrl),
                    posterURL: XboxImageURL.resolve(game.posterImageUrl),
                    heroURL: XboxImageURL.resolve(game.heroImageUrl),
                    artworkURL: nil,
                    categories: game.categories,
                    supportedInputTypes: game.supportedInputTypes,
                    hasEntitlement: game.hasEntitlement,
                    isRecentlyPlayed: game.isRecentlyPlayed,
                    isNew: game.isNew,
                    lastPlayedAt: nil,
                    playtimeMinutes: nil,
                    achievementProgress: nil
                )
            },
            scope: scope,
            fetchedAt: fetchedAt,
            failedHydrationChunks: Int(snapshot.failedHydrationChunks),
            pendingHydrationProductIDs: snapshot.pendingHydrationProductIds
        )
        IOSRuntimeTrace.snapshot(
            domain: "cloud-library",
            event: "catalogRequestBoundarySucceeded",
            payload: [
                "games": .integer(Int64(result.games.count)),
                "pendingHydration": .integer(Int64(result.pendingHydrationProductIDs.count)),
                "failedHydrationChunks": .integer(Int64(result.failedHydrationChunks)),
                "elapsedMs": .integer(Int64(CloudLibraryDiagnostics.elapsedMilliseconds(since: startedAt))),
            ],
            dimension: .network,
            importance: .key,
            operationID: operationID
        )
        return result
    }

    func loadMetadataPage(
        accessHandle: String,
        market: String,
        language: String,
        productIDs: [String]
    ) async throws -> [CloudCatalogMetadata] {
        let operationID = UUID().uuidString
        let startedAt = Date()
        IOSRuntimeTrace.event(
            domain: "cloud-library",
            event: "metadataPageBoundaryStarted",
            payload: [
                "products": .integer(Int64(productIDs.count)),
                "market": .string(market),
                "language": .string(language),
            ],
            dimension: .network,
            importance: .debug,
            operationID: operationID
        )
        do {
            let result = try await hydrateCloudCatalogPage(
                accessHandle: accessHandle,
                market: market,
                language: language,
                productIds: productIDs
            ).map { metadata in
                CloudCatalogMetadata(
                    productID: metadata.productId,
                    name: metadata.name,
                    publisherName: metadata.publisherName,
                    description: metadata.description,
                    tileURL: XboxImageURL.resolve(metadata.tileImageUrl),
                    posterURL: XboxImageURL.resolve(metadata.posterImageUrl),
                    heroURL: XboxImageURL.resolve(metadata.heroImageUrl),
                    categories: metadata.categories
                )
            }
            IOSRuntimeTrace.snapshot(
                domain: "cloud-library",
                event: "metadataPageBoundarySucceeded",
                payload: [
                    "requested": .integer(Int64(productIDs.count)),
                    "received": .integer(Int64(result.count)),
                    "elapsedMs": .integer(Int64(CloudLibraryDiagnostics.elapsedMilliseconds(since: startedAt))),
                ],
                dimension: .network,
                importance: .debug,
                operationID: operationID
            )
            return result
        } catch {
            IOSRuntimeTrace.event(
                domain: "cloud-library",
                event: "metadataPageBoundaryFailed",
                payload: CloudLibraryDiagnostics.errorPayload(
                    error,
                    extra: [
                        "products": .integer(Int64(productIDs.count)),
                        "elapsedMs": .integer(Int64(CloudLibraryDiagnostics.elapsedMilliseconds(since: startedAt))),
                    ]
                ),
                dimension: .network,
                importance: .key,
                operationID: operationID
            )
            throw error
        }
    }

    func releaseAccess(handle: String) async {
        let operationID = UUID().uuidString
        let startedAt = Date()
        IOSRuntimeTrace.event(
            domain: "auth",
            event: "cloudAccessReleaseStarted",
            payload: [:],
            dimension: .network,
            importance: .debug,
            operationID: operationID
        )
        try? releaseCloudAccess(accessHandle: handle)
        IOSRuntimeTrace.event(
            domain: "auth",
            event: "cloudAccessReleaseCompleted",
            payload: [
                "elapsedMs": .integer(Int64(CloudLibraryDiagnostics.elapsedMilliseconds(since: startedAt))),
            ],
            dimension: .network,
            importance: .debug,
            operationID: operationID
        )
    }
}
