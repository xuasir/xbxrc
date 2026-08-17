import Foundation
import XCTest
@testable import XBXRC

final class SettingsAndPersistenceTests: XCTestCase {
    @MainActor
    func testSettingsMigrationNormalizesAndWritesBackInvalidLegacyValues() throws {
        let suiteName = "XBXRC.SettingsTests.\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suiteName))
        defer { defaults.removePersistentDomain(forName: suiteName) }

        defaults.set("unknown", forKey: AppSettingsStore.appearanceModeKey)
        defaults.set("zh_CN", forKey: AppSettingsStore.preferredGameLocaleKey)
        defaults.set(999, forKey: AppSettingsStore.cloudBitrateMbpsKey)
        defaults.set(-10, forKey: AppSettingsStore.audioBitrateKbpsKey)

        let store = AppSettingsStore(defaults: defaults)

        XCTAssertEqual(store.appearanceMode, .system)
        XCTAssertEqual(store.preferredGameLocale, "zh-CN")
        XCTAssertEqual(store.cloudBitrateMbps, 200)
        XCTAssertEqual(store.audioBitrateKbps, 1)
        XCTAssertEqual(defaults.string(forKey: AppSettingsStore.appearanceModeKey), "system")
        XCTAssertEqual(defaults.string(forKey: AppSettingsStore.preferredGameLocaleKey), "zh-CN")
        XCTAssertEqual(defaults.integer(forKey: AppSettingsStore.cloudBitrateMbpsKey), 200)
        XCTAssertNotNil(defaults.data(forKey: AppSettingsPersistence.snapshotKey))
    }

    func testLocaleNormalizerProducesStableAchievementCacheLocales() {
        XCTAssertEqual(PreferredGameLocaleNormalizer.normalize("zh_CN"), "zh-CN")
        XCTAssertEqual(PreferredGameLocaleNormalizer.normalize("  "), "en-US")
        XCTAssertEqual(PreferredGameLocaleNormalizer.normalize("EN-us"), "en-US")
    }

    func testCloudCatalogCacheKeyDoesNotExposeAccountOrRegion() {
        let scope = CloudCatalogScope(
            accountID: "sensitive-account-id",
            regionHost: "westus.example.com",
            language: "zh-CN",
            market: "CN"
        )

        XCTAssertEqual(scope.cacheKey.count, 130)
        XCTAssertFalse(scope.cacheKey.contains(scope.accountID))
        XCTAssertFalse(scope.cacheKey.contains(scope.regionHost))
    }

    func testImagePreferenceSaveUpdatesBaseWithoutCreatingOverlay() async throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("XBXRC-CloudCatalogTests-\(UUID().uuidString)")
        let repository = CloudCatalogSnapshotRepository(rootDirectory: root)
        let scope = CloudCatalogScope(
            accountID: "account",
            regionHost: "region.example.com",
            language: "en-US",
            market: "US"
        )
        let snapshot = CloudCatalogSnapshot(
            scope: scope,
            games: [],
            baseUpdatedAt: .now,
            overlayUpdatedAt: .now,
            successfulImageURLs: ["product": try XCTUnwrap(URL(string: "https://example.com/image.jpg"))]
        )

        try await repository.saveSuccessfulImagePreferences(snapshot)

        let files = try FileManager.default.contentsOfDirectory(
            at: root,
            includingPropertiesForKeys: [.isExcludedFromBackupKey]
        )
        XCTAssertEqual(files.count, 1)
        let file = try XCTUnwrap(files.first)
        XCTAssertTrue(file.lastPathComponent.hasPrefix("base-v1-"))
        XCTAssertEqual(
            try file.resourceValues(forKeys: [.isExcludedFromBackupKey]).isExcludedFromBackup,
            true
        )
    }

    func testImagePreferenceSavePreservesExistingOverlayAndBaseTimestamp() async throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("XBXRC-CloudCatalogTests-\(UUID().uuidString)")
        let repository = CloudCatalogSnapshotRepository(rootDirectory: root)
        let scope = CloudCatalogScope(
            accountID: "account",
            regionHost: "region.example.com",
            language: "en-US",
            market: "US"
        )
        let baseUpdatedAt = Date(timeIntervalSince1970: 1_700_000_000)
        let overlayUpdatedAt = Date(timeIntervalSince1970: 1_700_000_100)
        let initial = CloudCatalogSnapshot(
            scope: scope,
            games: [],
            baseUpdatedAt: baseUpdatedAt,
            overlayUpdatedAt: overlayUpdatedAt
        )
        try await repository.save(initial)

        let imageUpdate = CloudCatalogSnapshot(
            scope: scope,
            games: [],
            baseUpdatedAt: .now,
            overlayUpdatedAt: .now,
            successfulImageURLs: ["product": try XCTUnwrap(URL(string: "https://example.com/image.jpg"))]
        )
        try await repository.saveSuccessfulImagePreferences(imageUpdate)
        let loaded = try await repository.load(scope: scope)
        let restored = try XCTUnwrap(loaded)

        XCTAssertEqual(restored.baseUpdatedAt, baseUpdatedAt)
        XCTAssertEqual(restored.overlayUpdatedAt, overlayUpdatedAt)
        XCTAssertEqual(restored.successfulImageURLs, imageUpdate.successfulImageURLs)
    }
}
