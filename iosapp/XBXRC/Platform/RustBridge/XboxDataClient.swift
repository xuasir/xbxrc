import Foundation

struct TitlePlaytime: Equatable, Sendable {
    let titleID: String
    let minutes: Int?
}

struct HostPowerCommandResult: Equatable, Sendable {
    let consoleID: String
    let accepted: Bool
}

protocol XboxDataClient: Sendable {
    func loadHosts(webTokenJSON: String) async throws -> [XboxHostSummary]
    func powerOn(webTokenJSON: String, consoleID: String) async throws -> HostPowerCommandResult
    func powerOff(webTokenJSON: String, consoleID: String) async throws -> HostPowerCommandResult
    func loadGameLibrary(webTokenJSON: String) async throws -> [GameSummary]
    func loadPlaytimes(webTokenJSON: String, titleIDs: [String]) async throws -> [TitlePlaytime]
    func loadAchievements(
        webTokenJSON: String,
        titleID: String,
        locale: String
    ) async throws -> [AchievementSummary]
}

struct RustXboxDataClient: XboxDataClient {
    func loadHosts(webTokenJSON: String) async throws -> [XboxHostSummary] {
        try await fetchHosts(webTokenJson: webTokenJSON).enumerated().map { index, host in
            let commandID = host.id ?? host.serverId ?? host.deviceId
            let streamTargetID = host.serverId ?? host.id ?? host.deviceId
            let stableID = streamTargetID ?? commandID ?? "host-\(index)"
            let displayName = [host.name, host.deviceName]
                .compactMap { $0?.trimmingCharacters(in: .whitespacesAndNewlines) }
                .first(where: { !$0.isEmpty }) ?? "Xbox"
            let consoleType = host.consoleType?
                .trimmingCharacters(in: .whitespacesAndNewlines)
                .nonEmpty ?? "Xbox"
            return XboxHostSummary(
                id: stableID,
                commandID: commandID,
                streamTargetID: streamTargetID,
                name: displayName,
                consoleType: consoleType,
                locale: host.locale,
                region: host.region,
                powerState: host.powerState,
                remoteManagementEnabled: host.remoteManagementEnabled,
                consoleStreamingEnabled: host.consoleStreamingEnabled,
                wirelessWarning: host.wirelessWarning,
                outOfHomeWarning: host.outOfHomeWarning,
                storageDevices: host.storageDevices.enumerated().map { storageIndex, storage in
                    XboxHostStorageSummary(
                        id: storage.id ?? "\(stableID)-storage-\(storageIndex)",
                        name: storage.name ?? "存储设备",
                        freeBytes: storage.freeBytes,
                        totalBytes: storage.totalBytes
                    )
                }
            )
        }
    }

    func powerOn(
        webTokenJSON: String,
        consoleID: String
    ) async throws -> HostPowerCommandResult {
        let result = try await powerOnConsole(
            webTokenJson: webTokenJSON,
            consoleId: consoleID
        )
        return HostPowerCommandResult(
            consoleID: result.consoleId,
            accepted: result.accepted
        )
    }

    func powerOff(
        webTokenJSON: String,
        consoleID: String
    ) async throws -> HostPowerCommandResult {
        let result = try await powerOffConsole(
            webTokenJson: webTokenJSON,
            consoleId: consoleID
        )
        return HostPowerCommandResult(
            consoleID: result.consoleId,
            accepted: result.accepted
        )
    }

    func loadGameLibrary(webTokenJSON: String) async throws -> [GameSummary] {
        let operationID = UUID().uuidString
        let startedAt = Date()
        IOSRuntimeTrace.event(
            domain: "xbox-data",
            event: "gameLibraryBoundaryStarted",
            payload: [:],
            dimension: .network,
            importance: .key,
            operationID: operationID
        )
        do {
            let games = try await fetchGameLibrary(webTokenJson: webTokenJSON).map { game in
            GameSummary(
                id: game.titleId,
                titleID: game.titleId,
                name: game.name,
                artworkURL: XboxImageURL.resolve(game.artworkUrl),
                heroURL: XboxImageURL.resolve(game.heroUrl),
                lastPlayedAt: parseISO8601(game.lastPlayedAt),
                playtimeMinutes: nil,
                achievementProgress: game.achievementProgress.map { progress in
                    AchievementProgress(
                        unlockedCount: Int(progress.unlockedCount),
                        totalCount: Int(progress.totalCount),
                        earnedGamerscore: Int(progress.earnedGamerscore),
                        totalGamerscore: Int(progress.totalGamerscore),
                        percentage: Int(progress.percentage)
                    )
                }
            )
            }
            IOSRuntimeTrace.event(
                domain: "xbox-data",
                event: "gameLibraryBoundarySucceeded",
                payload: [
                    "games": .integer(Int64(games.count)),
                    "elapsedMs": .integer(Int64(CloudLibraryDiagnostics.elapsedMilliseconds(since: startedAt))),
                ],
                dimension: .network,
                importance: .key,
                operationID: operationID
            )
            return games
        } catch {
            IOSRuntimeTrace.event(
                domain: "xbox-data",
                event: "gameLibraryBoundaryFailed",
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

    func loadPlaytimes(
        webTokenJSON: String,
        titleIDs: [String]
    ) async throws -> [TitlePlaytime] {
        let operationID = UUID().uuidString
        let startedAt = Date()
        IOSRuntimeTrace.event(
            domain: "xbox-data",
            event: "playtimesBoundaryStarted",
            payload: ["requested": .integer(Int64(titleIDs.count))],
            dimension: .network,
            importance: .debug,
            operationID: operationID
        )
        do {
            let playtimes = try await fetchPlaytimes(
                webTokenJson: webTokenJSON,
                titleIds: titleIDs
            ).map { playtime in
                TitlePlaytime(
                    titleID: playtime.titleId,
                    minutes: playtime.minutes.flatMap(Int.init(exactly:))
                )
            }
            IOSRuntimeTrace.event(
                domain: "xbox-data",
                event: "playtimesBoundarySucceeded",
                payload: [
                    "requested": .integer(Int64(titleIDs.count)),
                    "received": .integer(Int64(playtimes.count)),
                    "elapsedMs": .integer(Int64(CloudLibraryDiagnostics.elapsedMilliseconds(since: startedAt))),
                ],
                dimension: .network,
                importance: .debug,
                operationID: operationID
            )
            return playtimes
        } catch {
            IOSRuntimeTrace.event(
                domain: "xbox-data",
                event: "playtimesBoundaryFailed",
                payload: CloudLibraryDiagnostics.errorPayload(
                    error,
                    extra: [
                        "requested": .integer(Int64(titleIDs.count)),
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

    func loadAchievements(
        webTokenJSON: String,
        titleID: String,
        locale: String
    ) async throws -> [AchievementSummary] {
        let operationID = UUID().uuidString
        let startedAt = Date()
        IOSRuntimeTrace.event(
            domain: "xbox-data",
            event: "achievementsBoundaryStarted",
            payload: ["locale": .string(locale)],
            dimension: .network,
            importance: .debug,
            operationID: operationID
        )
        do {
            let achievements = try await fetchAchievements(
                webTokenJson: webTokenJSON,
                titleId: titleID,
                locale: locale
            ).map { achievement in
                let visibleDescription: String
                if achievement.isUnlocked || !achievement.isSecret {
                    visibleDescription = achievement.description.isEmpty
                        ? achievement.lockedDescription
                        : achievement.description
                } else {
                    visibleDescription = achievement.lockedDescription
                }

                return AchievementSummary(
                    id: achievement.id,
                    titleID: achievement.titleId,
                    name: achievement.name,
                    description: visibleDescription,
                    imageURL: XboxImageURL.resolve(achievement.imageUrl),
                    isSecret: achievement.isSecret,
                    isUnlocked: achievement.isUnlocked,
                    gamerscore: Int(achievement.gamerscore),
                    progressPercentage: achievement.progressPercentage.map(Int.init),
                    unlockedAt: parseISO8601(achievement.unlockedAt)
                )
            }
            IOSRuntimeTrace.event(
                domain: "xbox-data",
                event: "achievementsBoundarySucceeded",
                payload: [
                    "received": .integer(Int64(achievements.count)),
                    "elapsedMs": .integer(Int64(CloudLibraryDiagnostics.elapsedMilliseconds(since: startedAt))),
                ],
                dimension: .network,
                importance: .debug,
                operationID: operationID
            )
            return achievements
        } catch {
            IOSRuntimeTrace.event(
                domain: "xbox-data",
                event: "achievementsBoundaryFailed",
                payload: CloudLibraryDiagnostics.errorPayload(
                    error,
                    extra: [
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

    private func parseISO8601(_ value: String?) -> Date? {
        guard let value else {
            return nil
        }
        return try? Date(value, strategy: .iso8601)
    }
}

enum XboxImageURL {
    static func resolve(_ value: String?) -> URL? {
        guard let value else {
            return nil
        }
        let normalized = value.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !normalized.isEmpty else {
            return nil
        }
        if normalized.hasPrefix("//") {
            return URL(string: "https:\(normalized)")
        }
        guard var components = URLComponents(string: normalized) else {
            return nil
        }
        if components.scheme?.lowercased() == "http" {
            components.scheme = "https"
        }
        return components.url
    }
}

private extension String {
    var nonEmpty: String? {
        isEmpty ? nil : self
    }
}
