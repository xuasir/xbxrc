import SwiftUI

private enum MyRoute: AppSecondaryRoute {
    case friends, achievements, playtime, album

    var title: String {
        switch self {
        case .friends: "好友"
        case .achievements: "成就"
        case .playtime: "游玩统计"
        case .album: "Xbox 相册"
        }
    }
    var icon: AppIcon {
        switch self {
        case .friends: .people
        case .achievements: .trophy
        case .playtime: .chart
        case .album: .photos
        }
    }
    var subtitle: String {
        switch self {
        case .friends: "在线状态与玩家资料"
        case .achievements: "进度与解锁条件"
        case .playtime: "时长与游戏排行"
        case .album: "截图与录制片段"
        }
    }
}

struct ProfileView: View {
    @EnvironmentObject private var authStore: AuthStore
    @EnvironmentObject private var dataStore: XboxDataStore
    @State private var path: [MyRoute] = []
    @State private var showsSignOutConfirmation = false
    let isActive: Bool
    init(isActive: Bool = true) { self.isActive = isActive }

    var body: some View {
        NavigationStack(path: $path) {
            AppPageScroll {
                VStack(alignment: .leading, spacing: AppLayoutPolicy.sectionSpacing) {
                    AppAccountRequestNotice()
                    overview
                    if authStore.isSignedIn {
                        VStack(alignment: .leading, spacing: 8) {
                            Text("游戏与成就概况").appFont(.section, weight: .semibold)
                            if dataStore.historyState(targetID: nil).hasSnapshot {
                                PersonalLibraryOverview(games: dataStore.games)
                            }
                            AppSectionRequestState(
                                title: "游戏与成就概况",
                                status: dataStore.historyState(targetID: nil).requestStatus,
                                identifier: "my-overview"
                            ) {
                                await dataStore.refreshLibrary(reason: .manualRetry)
                            }
                        }
                        VStack(spacing: 12) {
                            ForEach(MyRoute.allCases) { route in
                                NavigationLink(value: route) {
                                    AppNavigationRow(
                                        title: route.title, subtitle: route.subtitle,
                                        icon: route.icon, compact: true)
                                }
                                .buttonStyle(.plain)
                                .accessibilityIdentifier("section-\(route.id)")
                            }
                        }
                    }
                }
                .padding(AppLayoutPolicy.pageInset)
                .padding(.bottom, AppLayoutPolicy.pageInset)
                .frame(maxWidth: AppLayoutPolicy.contentMaximumWidth, alignment: .leading)
                .frame(maxWidth: .infinity)
            }
            .accessibilityIdentifier("my-information-scroll")
            .appThemeCanvas()
            .appPageToolbar(
                "我的", showsBrand: true, showsBack: false,
                artwork: authStore.isSignedIn
                    ? .avatar(XboxImageURL.resolve(authStore.profile?.displayPictureUrl)) : nil
            )
            .toolbar {
                if authStore.isSignedIn {
                    ToolbarItem(placement: .topBarTrailing) {
                        AppRefreshButton("刷新账户资料", isRefreshing: authStore.isBusy) {
                            await refresh()
                        }
                    }.sharedBackgroundVisibility(.hidden)
                    ToolbarItem(placement: .topBarTrailing) {
                        Button {
                            showsSignOutConfirmation = true
                        } label: {
                            AppIconView(.signOut, size: 22)
                                .frame(width: 44, height: 44)
                                .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                        .foregroundStyle(AppThemePalette.textPrimary)
                        .accessibilityLabel("退出登录")
                        .accessibilityIdentifier("my-sign-out")
                    }.sharedBackgroundVisibility(.hidden)
                }
            }
            .refreshable { await refresh() }
            .navigationDestination(for: MyRoute.self) { route in
                routeContent(route)
                    .appPageToolbar(route.title)
                    .toolbar {
                        ToolbarItem(placement: .topBarTrailing) {
                            AppRefreshButton("刷新", isRefreshing: routeIsRefreshing(route)) {
                                await refresh(route)
                            }
                        }.sharedBackgroundVisibility(.hidden)
                    }
            }
        }
        .environment(\.appPageIsActive, isActive)
        .appInformationTypography()
        .confirmationDialog(
            "退出 Xbox 账户？", isPresented: $showsSignOutConfirmation, titleVisibility: .visible
        ) {
            Button("退出登录", role: .destructive) { Task { await authStore.signOut() } }
        }
        .onChange(of: authStore.ownerGeneration) { _, _ in path = [] }
        .task(id: "\(authStore.ownerGeneration):\(isActive)") {
            guard isActive, authStore.phase == .signedIn else { return }
            await dataStore.sync(
                session: authStore.session, ownerGeneration: authStore.ownerGeneration)
            async let profile: Void = authStore.activateProfileOnce()
            async let library: Void = dataStore.activateLibraryOnce()
            _ = await (profile, library)
        }
    }

    @ViewBuilder
    private func routeContent(_ route: MyRoute) -> some View {
        switch route {
        case .friends: FriendsView()
        case .achievements: GameHistoryView()
        case .playtime: PlaytimeView()
        case .album: XboxAlbumView()
        }
    }

    private func routeIsRefreshing(_ route: MyRoute) -> Bool {
        switch route {
        case .friends: dataStore.friendsState.isRefreshing
        case .achievements, .playtime: dataStore.historyState(targetID: nil).isRefreshing
        case .album: dataStore.albumStates.values.contains { $0.isRefreshing }
        }
    }

    private func refresh(_ route: MyRoute? = nil) async {
        switch route {
        case nil:
            async let profile: Void = authStore.refreshProfile(reason: .manualPull)
            async let library: Void = dataStore.refreshLibrary(reason: .manualPull)
            _ = await (profile, library)
        case .friends: await dataStore.loadFriends(refresh: true)
        case .achievements, .playtime: await dataStore.loadHistory(refresh: true)
        case .album:
            async let screenshots: Void = dataStore.loadAlbum(kind: .screenshot, refresh: true)
            async let clips: Void = dataStore.loadAlbum(kind: .clip, refresh: true)
            _ = await (screenshots, clips)
        }
    }

    private func profileSourceTitle(_ source: XboxProfileSource) -> String {
        switch source {
        case .identity: "账户资料"
        case .presence: "在线状态"
        case .friends: "好友数量"
        case .social: "关注与粉丝"
        }
    }

    @ViewBuilder
    private var overview: some View {
        if let profile = authStore.profile, authStore.isSignedIn {
            PersonalIdentityHero(
                person: XboxFriendSummary(
                    id: profile.xuid ?? "self", displayName: profile.displayName,
                    gamertag: profile.gamertag,
                    avatarURL: XboxImageURL.resolve(profile.displayPictureUrl),
                    gamerscore: profile.gamerScore, presenceState: profile.presenceState,
                    presenceText: profile.richPresence ?? profile.currentTitleName),
                currentTitle: profile.currentTitleName, friendCount: profile.friendCount,
                followingCount: profile.followingCount, followersCount: profile.followersCount)
            ForEach([XboxProfileSource.identity, .presence, .friends, .social], id: \.self) {
                source in
                AppSectionRequestState(
                    title: profileSourceTitle(source),
                    status: (authStore.profileSectionStates[source] ?? XboxSingleDataState())
                        .requestStatus,
                    identifier: "profile-\(source)"
                ) {
                    await authStore.refreshProfileSource(source)
                }
            }
        } else if authStore.isSignedIn {
            VStack(alignment: .leading, spacing: 8) {
                AppSectionRequestState(
                    title: "账户资料",
                    status: (authStore.profileSectionStates[.identity]
                        ?? XboxSingleDataState(phase: .loading, isRefreshing: true)).requestStatus,
                    identifier: "profile-identity"
                ) {
                    await authStore.refreshProfileSource(.identity)
                }
            }
        } else {
            XboxLoginView(isBusy: authStore.isBusy, errorMessage: authStore.errorMessage) {
                Task { await authStore.retry() }
            }.frame(maxWidth: .infinity, minHeight: 280)
        }
    }
}
