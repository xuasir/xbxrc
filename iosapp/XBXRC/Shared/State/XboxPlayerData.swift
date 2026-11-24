import Foundation

extension XboxDataStore {
    func historyState(targetID: String?) -> XboxNativeDataState<GameSummary> {
        if let targetID { return playerHistoryStates[targetID] ?? XboxNativeDataState() }
        return XboxNativeDataState(
            items: games, phase: libraryPhase, isRefreshing: isRefreshingLibrary,
            errorMessage: libraryErrorMessage, continuation: libraryContinuation,
            updatedAt: libraryUpdatedAt, failure: libraryFailure, requestAttempt: libraryAttempt)
    }

    func loadPlayerProfile(targetID: String, refresh: Bool = false) async {
        let key = "profile-\(targetID)"
        var state = playerProfiles[targetID] ?? XboxSingleDataState()
        guard let context = nativeQueryContext(), nativeRequestIDs[key] == nil,
            refresh || state.needsActivation
        else { return }
        let request = UUID()
        nativeRequestIDs[key] = request
        state.beginRequest()
        playerProfiles[targetID] = state
        do {
            let profile = try await performNativeQuery(context: context) { token in
                try await client.loadPlayerProfile(
                    webTokenJSON: token, targetID: targetID)
            }
            guard ownsNativeQuery(context), nativeRequestIDs[key] == request else { return }
            guard profile.id == targetID else { throw XboxNativeQueryError.unavailable }
            state.value = profile
            state.phase = .loaded
            state.updatedAt = .now
        } catch {
            guard ownsNativeQuery(context), nativeRequestIDs[key] == request else { return }
            state.failRequest(error)
        }
        state.isRefreshing = false
        playerProfiles[targetID] = state
        nativeRequestIDs[key] = nil
    }

    func loadHistory(targetID: String? = nil, refresh: Bool = false, nextPage: Bool = false) async {
        guard let targetID else {
            if nextPage {
                await loadMoreLibrary()
            } else if refresh {
                await refreshLibrary()
            } else {
                await activateLibraryOnce()
            }
            return
        }
        let key = "history-\(targetID)"
        var state = playerHistoryStates[targetID] ?? XboxNativeDataState()
        guard let context = nativeQueryContext(), nativeRequestIDs[key] == nil else { return }
        if nextPage && state.continuation == nil { return }
        if !refresh && !nextPage && !state.needsActivation { return }
        let continuation = nextPage ? state.continuation : nil
        let request = UUID()
        if !nextPage { invalidateHistoryPlaytimes(owner: targetID) }
        nativeRequestIDs[key] = request
        state.beginRequest(nextPage: nextPage)
        playerHistoryStates[targetID] = state
        do {
            let page = try await performNativeQuery(context: context) { token in
                try await client.loadHistoryPage(
                    webTokenJSON: token, targetID: targetID, continuation: continuation)
            }
            guard ownsNativeQuery(context), nativeRequestIDs[key] == request else { return }
            if !nextPage { state.seenContinuations = [] }
            if let next = page.continuation,
                next == continuation || state.seenContinuations.contains(next)
            {
                throw XboxNativeQueryError.repeatedContinuation
            }
            if let continuation { state.seenContinuations.insert(continuation) }
            var seen = Set<String>()
            state.items = ((nextPage ? state.items : []) + page.items).filter {
                seen.insert($0.titleID).inserted
            }.map { game in
                let time = playerPlaytimeStates[
                    XboxPersonalQueryKey.game(game.titleID, targetID: targetID)]?.value
                return game.replacingPlaytime(time ?? game.playtimeMinutes)
            }
            state.continuation = page.continuation
            state.phase = .loaded
            state.updatedAt = .now
        } catch {
            guard ownsNativeQuery(context), nativeRequestIDs[key] == request else { return }
            state.failRequest(error)
        }
        state.isRefreshing = false
        playerHistoryStates[targetID] = state
        nativeRequestIDs[key] = nil
    }

    func loadHistoryPlaytimes(targetID: String? = nil, refresh: Bool = false) async {
        let owner = targetID ?? "self"
        let key = "history-times-\(owner)"
        guard let context = nativeQueryContext(), nativeRequestIDs[key] == nil else { return }
        let titles = historyState(targetID: targetID).items.map(\.titleID).filter {
            let state = playerPlaytimeStates[XboxPersonalQueryKey.game($0, targetID: targetID)]
            return refresh || state?.phase != .loaded || state?.failure != nil
                || state?.errorMessage != nil
        }
        guard !titles.isEmpty else { return }
        let request = UUID()
        nativeRequestIDs[key] = request
        var summary = historyPlaytimeStates[owner] ?? XboxSingleDataState<Bool>()
        summary.beginRequest()
        historyPlaytimeStates[owner] = summary
        defer { if nativeRequestIDs[key] == request { nativeRequestIDs[key] = nil } }
        var failed: SectionRequestFailure?
        var succeeded = false
        var cancelled = false
        for start in stride(from: 0, to: titles.count, by: 100) {
            let batch = Array(titles[start..<min(start + 100, titles.count)])
            do {
                let result = try await performNativeQuery(context: context) { token in
                    try await client.loadPlayerPlaytimes(
                        webTokenJSON: token, titleIDs: batch, targetID: targetID)
                }
                guard ownsNativeQuery(context), nativeRequestIDs[key] == request else { return }
                let values = Dictionary(
                    result.items.map { ($0.titleID, $0.minutes) },
                    uniquingKeysWith: { first, _ in first })
                for title in batch {
                    let identity = XboxPersonalQueryKey.game(title, targetID: targetID)
                    var state = playerPlaytimeStates[identity] ?? XboxSingleDataState()
                    if result.failedTitleIDs.contains(title) {
                        let failure =
                            result.failures[title] ?? SectionRequestFailure(kind: .unknown)
                        state.failure = failure
                        state.errorMessage = failure.message
                        state.phase = state.hasSnapshot ? .loaded : .failed
                        failed = failure
                    } else {
                        state.value = values[title] ?? nil
                        state.phase = .loaded
                        state.failure = nil
                        state.errorMessage = nil
                        state.updatedAt = .now
                        succeeded = true
                    }
                    playerPlaytimeStates[identity] = state
                }
                let updated = historyState(targetID: targetID).items.map { game in
                    let state = playerPlaytimeStates[
                        XboxPersonalQueryKey.game(game.titleID, targetID: targetID)]
                    return state?.phase == .loaded && state?.failure == nil
                        ? game.replacingPlaytime(state?.value) : game
                }
                if let targetID {
                    playerHistoryStates[targetID]?.items = updated
                } else {
                    updateLibraryPlaytimes(updated)
                }
            } catch {
                guard ownsNativeQuery(context), nativeRequestIDs[key] == request else { return }
                if SectionRequestFailure.isCancellation(error) {
                    summary.failRequest(error)
                    cancelled = true
                    break
                }
                let failure = SectionRequestFailure(error: error)
                failed = failure
                for title in batch {
                    let identity = XboxPersonalQueryKey.game(title, targetID: targetID)
                    var state = playerPlaytimeStates[identity] ?? XboxSingleDataState()
                    state.failRequest(error)
                    playerPlaytimeStates[identity] = state
                }
            }
        }
        guard ownsNativeQuery(context), nativeRequestIDs[key] == request else { return }
        if cancelled || Task.isCancelled {
            summary.failRequest(CancellationError())
        } else if let failed {
            summary.failure = failed
            summary.errorMessage = failed.message
            if succeeded { summary.value = true }
            summary.phase = summary.hasSnapshot ? .loaded : .failed
        } else if !Task.isCancelled {
            summary.value = true
            summary.phase = .loaded
            summary.updatedAt = .now
        }
        summary.isRefreshing = false
        historyPlaytimeStates[owner] = summary
    }

    func loadLibraryGame(titleID: String, targetID: String? = nil, refresh: Bool = false) async {
        let identity = XboxPersonalQueryKey.game(titleID, targetID: targetID)
        let key = "game-\(identity)"
        var state = libraryGameStates[identity] ?? XboxSingleDataState()
        guard let context = nativeQueryContext(), nativeRequestIDs[key] == nil,
            refresh || state.needsActivation
        else { return }
        let request = UUID()
        nativeRequestIDs[key] = request
        state.beginRequest()
        libraryGameStates[identity] = state
        do {
            let game = try await performNativeQuery(context: context) { token in
                try await client.loadLibraryGame(
                    webTokenJSON: token, titleID: titleID, targetID: targetID)
            }
            guard ownsNativeQuery(context), nativeRequestIDs[key] == request else { return }
            if let game, game.titleID != titleID { throw XboxNativeQueryError.unavailable }
            state.value = game
            state.phase = .loaded
            state.updatedAt = .now
        } catch {
            guard ownsNativeQuery(context), nativeRequestIDs[key] == request else { return }
            state.failRequest(error)
        }
        state.isRefreshing = false
        libraryGameStates[identity] = state
        nativeRequestIDs[key] = nil
    }

    func loadPlayerPlaytime(titleID: String, targetID: String? = nil, refresh: Bool = false) async {
        let identity = XboxPersonalQueryKey.game(titleID, targetID: targetID)
        let key = "time-\(identity)"
        var state = playerPlaytimeStates[identity] ?? XboxSingleDataState()
        guard let context = nativeQueryContext(), nativeRequestIDs[key] == nil,
            refresh || state.needsActivation
        else { return }
        let request = UUID()
        nativeRequestIDs[key] = request
        state.beginRequest()
        playerPlaytimeStates[identity] = state
        do {
            let result = try await performNativeQuery(context: context) { token in
                try await client.loadPlayerPlaytimes(
                    webTokenJSON: token, titleIDs: [titleID], targetID: targetID)
            }
            guard ownsNativeQuery(context), nativeRequestIDs[key] == request else { return }
            if result.failedTitleIDs.contains(titleID) {
                state.failure = result.failures[titleID] ?? SectionRequestFailure(kind: .unknown)
                state.errorMessage = state.failure?.message
                state.phase = state.hasSnapshot ? .loaded : .failed
            } else {
                state.value = result.items.first { $0.titleID == titleID }?.minutes
                state.phase = .loaded
                state.updatedAt = .now
            }
        } catch {
            guard ownsNativeQuery(context), nativeRequestIDs[key] == request else { return }
            state.failRequest(error)
        }
        state.isRefreshing = false
        playerPlaytimeStates[identity] = state
        nativeRequestIDs[key] = nil
    }

    func loadPlayerAchievements(
        titleID: String, targetID: String? = nil, locale: String,
        refresh: Bool = false, nextPage: Bool = false
    ) async {
        let identity = XboxPersonalQueryKey.achievements(
            titleID, targetID: targetID, locale: locale)
        let key = "achievements-\(identity)"
        var state = playerAchievementStates[identity] ?? XboxNativeDataState()
        guard let context = nativeQueryContext(), nativeRequestIDs[key] == nil else { return }
        if nextPage && state.continuation == nil { return }
        if !refresh && !nextPage && !state.needsActivation { return }
        let continuation = nextPage ? state.continuation : nil
        let request = UUID()
        nativeRequestIDs[key] = request
        state.beginRequest(nextPage: nextPage)
        playerAchievementStates[identity] = state
        do {
            let page = try await performNativeQuery(context: context) { token in
                try await client.loadAchievementPage(
                    webTokenJSON: token, titleID: titleID, targetID: targetID, locale: locale,
                    continuation: continuation)
            }
            guard ownsNativeQuery(context), nativeRequestIDs[key] == request else { return }
            if !nextPage { state.seenContinuations = [] }
            if let next = page.continuation,
                next == continuation || state.seenContinuations.contains(next)
            {
                throw XboxNativeQueryError.repeatedContinuation
            }
            if let continuation { state.seenContinuations.insert(continuation) }
            var seen = Set<String>()
            state.items = ((nextPage ? state.items : []) + page.items).filter {
                seen.insert($0.queryIdentity).inserted
            }
            state.continuation = page.continuation
            state.phase = .loaded
            state.updatedAt = .now
        } catch {
            guard ownsNativeQuery(context), nativeRequestIDs[key] == request else { return }
            state.failRequest(error)
        }
        state.isRefreshing = false
        playerAchievementStates[identity] = state
        nativeRequestIDs[key] = nil
    }
}
