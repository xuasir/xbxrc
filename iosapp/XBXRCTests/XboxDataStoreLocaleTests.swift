import Foundation
import XCTest
@testable import XBXRC

final class XboxDataStoreLocaleTests: XCTestCase {
    @MainActor
    func testAchievementCacheAndSingleFlightAreScopedByNormalizedLocale() async {
        let client = LocaleTestXboxDataClient()
        let locale = LocaleTestProvider(value: "en-US")
        let store = XboxDataStore(client: client, preferredGameLocaleProvider: locale)
        await store.sync(
            session: StoredAuthSession(
                refreshToken: "refresh",
                seedJSON: "seed",
                webTokenJSON: "web",
                appLevel: 1
            ),
            ownerGeneration: 1
        )
        let game = GameSummary(
            id: "game",
            titleID: "42",
            name: "Game",
            artworkURL: nil,
            playtimeMinutes: nil,
            achievementProgress: nil
        )

        await store.loadAchievements(for: game)
        XCTAssertEqual(store.achievements(for: game.titleID).first?.name, "en-US")

        locale.preferredGameLocale = "zh_CN"
        await store.loadAchievements(for: game)
        XCTAssertEqual(store.achievements(for: game.titleID).first?.name, "zh-CN")

        locale.preferredGameLocale = "en-US"
        await store.loadAchievements(for: game)
        XCTAssertEqual(store.achievements(for: game.titleID).first?.name, "en-US")
        let requestedLocales = await client.requestedLocales()
        XCTAssertEqual(requestedLocales, ["en-US", "zh-CN"])
    }
}

@MainActor
private final class LocaleTestProvider: PreferredGameLocaleProviding {
    var preferredGameLocale: String

    init(value: String) {
        preferredGameLocale = value
    }
}

private actor LocaleTestXboxDataClient: XboxDataClient {
    private var locales: [String] = []

    func loadHosts(webTokenJSON _: String) async throws -> [XboxHostSummary] { [] }

    func powerOn(
        webTokenJSON _: String,
        consoleID: String
    ) async throws -> HostPowerCommandResult {
        HostPowerCommandResult(consoleID: consoleID, accepted: true)
    }

    func powerOff(
        webTokenJSON _: String,
        consoleID: String
    ) async throws -> HostPowerCommandResult {
        HostPowerCommandResult(consoleID: consoleID, accepted: true)
    }

    func loadGameLibrary(webTokenJSON _: String) async throws -> [GameSummary] { [] }

    func loadPlaytimes(
        webTokenJSON _: String,
        titleIDs _: [String]
    ) async throws -> [TitlePlaytime] { [] }

    func loadAchievements(
        webTokenJSON _: String,
        titleID: String,
        locale: String
    ) async throws -> [AchievementSummary] {
        locales.append(locale)
        return [
            AchievementSummary(
                id: "\(titleID)-\(locale)",
                titleID: titleID,
                name: locale,
                description: "",
                imageURL: nil,
                isSecret: false,
                isUnlocked: false,
                gamerscore: 0,
                progressPercentage: 0,
                unlockedAt: nil
            ),
        ]
    }

    func requestedLocales() -> [String] { locales }
}
