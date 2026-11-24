import Foundation

extension XboxDataStore {
    func loadFriends(refresh: Bool = false) async {
        let key = "friends"
        guard let context = nativeQueryContext(), nativeRequestIDs[key] == nil,
            refresh || friendsState.needsActivation
        else { return }
        let request = UUID()
        nativeRequestIDs[key] = request
        friendsState.beginRequest()
        do {
            let friends = try await performNativeQuery(context: context) { token in
                try await client.loadFriends(webTokenJSON: token)
            }
            guard ownsNativeQuery(context), nativeRequestIDs[key] == request else { return }
            friendsState.items = friends.sorted {
                if ($0.presenceState == "Online") != ($1.presenceState == "Online") {
                    return $0.presenceState == "Online"
                }
                return $0.gamertag.localizedStandardCompare($1.gamertag) == .orderedAscending
            }
            friendsState.phase = .loaded
            friendsState.updatedAt = .now
        } catch {
            guard ownsNativeQuery(context), nativeRequestIDs[key] == request else { return }
            friendsState.failRequest(error)
        }
        if nativeRequestIDs[key] == request {
            nativeRequestIDs[key] = nil
            friendsState.isRefreshing = false
        }
    }

    func loadAlbum(kind: XboxAlbumKind, refresh: Bool = false, nextPage: Bool = false) async {
        let key = "album-\(kind.rawValue)"
        var state = albumStates[kind] ?? XboxNativeDataState()
        guard let context = nativeQueryContext(), nativeRequestIDs[key] == nil else { return }
        if nextPage && state.continuation == nil { return }
        if !refresh && !nextPage && !state.needsActivation { return }
        let continuation = nextPage ? state.continuation : nil
        let request = UUID()
        nativeRequestIDs[key] = request
        state.beginRequest(nextPage: nextPage)
        albumStates[kind] = state
        do {
            let page = try await performNativeQuery(context: context) { token in
                try await client.loadMedia(
                    webTokenJSON: token, kind: kind, continuation: continuation)
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
            let items = page.items.map { item in
                var media = item
                media.pageContinuation = continuation
                return media
            }
            state.items = ((nextPage ? state.items : []) + items).filter {
                seen.insert($0.queryIdentity).inserted
            }
            state.continuation = page.continuation
            state.phase = .loaded
            state.updatedAt = .now
        } catch {
            guard ownsNativeQuery(context), nativeRequestIDs[key] == request else { return }
            state.failRequest(error)
        }
        if nativeRequestIDs[key] == request {
            state.isRefreshing = false
            albumStates[kind] = state
            nativeRequestIDs[key] = nil
        }
    }

    func renewAlbumMedia(_ item: XboxAlbumMedia) async throws -> XboxAlbumMedia {
        guard let context = nativeQueryContext() else { throw XboxNativeQueryError.unavailable }
        let page = try await performNativeQuery(context: context) { token in
            try await client.loadMedia(
                webTokenJSON: token, kind: item.kind, continuation: item.pageContinuation)
        }
        guard ownsNativeQuery(context), var fresh = page.items.first(where: { $0.id == item.id })
        else {
            throw XboxNativeQueryError.unavailable
        }
        fresh.pageContinuation = item.pageContinuation
        if let index = albumStates[item.kind]?.items.firstIndex(where: { $0.id == item.id }) {
            albumStates[item.kind]?.items[index] = fresh
        }
        return fresh
    }

    func loadInstalledLibrary(
        consoleID: String, market: String, language: String, refresh: Bool = false
    ) async {
        let scope = "\(market.uppercased()):\(language.lowercased())"
        if installedLibraryStates[consoleID]?.scopeID != scope {
            installedLibraryStates[consoleID] = XboxInstalledLibraryState(scopeID: scope)
        }
        await withTaskGroup(of: Void.self) { group in
            for source in [XboxInstalledSource.games, .storage, .status] {
                group.addTask {
                    await self.loadInstalledSection(
                        consoleID: consoleID, market: market, language: language, source: source,
                        refresh: refresh)
                }
            }
        }
    }

    func loadInstalledSection(
        consoleID: String, market: String, language: String, source: XboxInstalledSource,
        refresh: Bool = true
    ) async {
        let scope = "\(market.uppercased()):\(language.lowercased())"
        let key = "installed-\(consoleID)-\(scope)-\(source)"
        guard let context = nativeQueryContext(), nativeRequestIDs[key] == nil else { return }
        var section =
            installedLibraryStates[consoleID]?.sections[source] ?? XboxSingleDataState<Bool>()
        guard refresh || section.needsActivation else { return }
        let request = UUID()
        nativeRequestIDs[key] = request
        section.beginRequest()
        defer { if nativeRequestIDs[key] == request { nativeRequestIDs[key] = nil } }
        installedLibraryStates[consoleID, default: XboxInstalledLibraryState(scopeID: scope)]
            .sections[
                source] = section
        installedLibraryStates[consoleID]?.isRefreshing = true
        var productIDs =
            installedLibraryStates[consoleID]?.snapshot?.games?.compactMap(\.productID) ?? []
        productIDs = Array(Set(productIDs))
        do {
            let result = try await performNativeQuery(context: context) { token in
                try await client.loadInstalledSection(
                    webTokenJSON: token, consoleID: consoleID, market: market, language: language,
                    source: source, productIDs: productIDs)
            }
            guard ownsNativeQuery(context), nativeRequestIDs[key] == request,
                installedLibraryStates[consoleID]?.scopeID == scope
            else { return }
            guard result.snapshot.consoleID == consoleID else {
                throw XboxNativeQueryError.unavailable
            }
            var aggregate =
                installedLibraryStates[consoleID] ?? XboxInstalledLibraryState(scopeID: scope)
            var snapshot =
                aggregate.snapshot
                ?? XboxInstalledLibrarySnapshot(consoleID: consoleID, notices: [])
            if source == .games, let games = result.snapshot.games { snapshot.games = games }
            if source == .storage, let storage = result.snapshot.storage {
                snapshot.storage = storage
            }
            if source == .status { snapshot.powerState = result.snapshot.powerState }
            if source == .artwork {
                let metadata = Dictionary(
                    result.artwork.map { ($0.productID, $0) },
                    uniquingKeysWith: { first, _ in first })
                snapshot.games = snapshot.games?.map { game in
                    let cover = game.productID.flatMap { metadata[$0] }
                    return XboxInstalledGameSummary(
                        id: game.id, titleID: game.titleID, productID: game.productID,
                        name: game.name,
                        artworkURL: cover?.posterURL ?? cover?.tileURL ?? cover?.heroURL
                            ?? game.artworkURL,
                        storageDeviceID: game.storageDeviceID, sizeBytes: game.sizeBytes,
                        installedAt: game.installedAt, lastPlayedAt: game.lastPlayedAt)
                }
            }
            if let failure = result.failure {
                section.failure = failure
                section.errorMessage = failure.message
                section.phase = section.hasSnapshot ? .loaded : .failed
            } else {
                section.value = true
                section.phase = .loaded
                section.updatedAt = .now
            }
            section.isRefreshing = false
            aggregate.sections[source] = section
            aggregate.snapshot = snapshot
            aggregate.phase = .loaded
            aggregate.updatedAt = source == .games ? section.updatedAt : aggregate.updatedAt
            aggregate.isRefreshing = aggregate.sections.values.contains { $0.isRefreshing }
            installedLibraryStates[consoleID] = aggregate
        } catch {
            guard ownsNativeQuery(context), nativeRequestIDs[key] == request,
                installedLibraryStates[consoleID]?.scopeID == scope
            else { return }
            section.failRequest(error)
            installedLibraryStates[consoleID]?.sections[source] = section
            installedLibraryStates[consoleID]?.isRefreshing =
                installedLibraryStates[consoleID]?.sections.values.contains { $0.isRefreshing }
                ?? false
        }
        if nativeRequestIDs[key] == request { nativeRequestIDs[key] = nil }
        if source == .games, section.failure == nil,
            installedLibraryStates[consoleID]?.scopeID == scope
        {
            await loadInstalledSection(
                consoleID: consoleID, market: market, language: language, source: .artwork,
                refresh: true)
        }
    }
}
