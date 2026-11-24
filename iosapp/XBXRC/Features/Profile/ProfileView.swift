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
    @Environment(\.locale) private var locale
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
                        libraryOverview
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
            .appRefreshable { await refresh() }
            .navigationDestination(for: MyRoute.self) { route in
                routeContent(route)
                    .appPageToolbar(route.title)
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

    private func refresh() async {
        async let profile: Void = authStore.refreshProfile(reason: .manualPull)
        async let library: Void = dataStore.refreshLibrary(reason: .manualPull)
        _ = await (profile, library)
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
        if authStore.isSignedIn {
            let identityState = authStore.profileSectionStates[.identity] ?? XboxSingleDataState()
            // 身份来源完成后展示头像与名称，其他来源可先返回数量和在线状态。
            let showsSkeleton =
                !identityState.hasSnapshot && identityState.failure == nil
                && !authStore.needsDataSignIn
                && (identityState.phase == .idle || identityState.phase == .loading)
            if showsSkeleton {
                ProfileIdentitySkeleton()
            } else if identityState.hasSnapshot, let profile = authStore.profile {
                PersonalIdentityHero(
                    person: XboxFriendSummary(
                        id: profile.xuid ?? "self", displayName: profile.displayName,
                        gamertag: profile.gamertag,
                        avatarURL: XboxImageURL.resolve(profile.displayPictureUrl),
                        gamerscore: profile.gamerScore, presenceState: profile.presenceState,
                        presenceText: profile.richPresence ?? profile.currentTitleName),
                    currentTitle: profile.currentTitleName, friendCount: profile.friendCount,
                    followingCount: profile.followingCount, followersCount: profile.followersCount)
            }
            ForEach([XboxProfileSource.identity, .presence, .friends, .social], id: \.self) {
                source in
                let status = (authStore.profileSectionStates[source] ?? XboxSingleDataState())
                    .requestStatus
                if !showsSkeleton || status.failure != nil {
                    AppSectionRequestState(
                        title: profileSourceTitle(source), status: status,
                        identifier: "profile-\(source)"
                    ) {
                        await authStore.refreshProfileSource(source)
                    }
                }
            }
        } else {
            XboxLoginView(isBusy: authStore.isBusy, errorMessage: authStore.errorMessage) {
                Task { await authStore.retry() }
            }.frame(maxWidth: .infinity, minHeight: 280)
        }
    }

    private var libraryOverview: some View {
        let state = dataStore.historyState(targetID: nil)
        let showsSkeleton =
            !state.hasSnapshot && state.failure == nil
            && !authStore.needsDataSignIn && (state.phase == .idle || state.phase == .loading)
        return VStack(alignment: .leading, spacing: 8) {
            Text("游戏与成就概况").appFont(.section, weight: .semibold)
            if state.hasSnapshot {
                PersonalLibraryOverview(games: dataStore.games)
            } else if showsSkeleton {
                AppMetricGrid {
                    ProfileMetricSkeleton()
                    ProfileMetricSkeleton()
                }
                .skeletonPulse(
                    accessibilityLabel: appLocalizedString("正在载入游戏与成就概况", locale: locale)
                )
                .accessibilityIdentifier("my-overview-skeleton")
            }
            if !showsSkeleton {
                AppSectionRequestState(
                    title: "游戏与成就概况", status: state.requestStatus,
                    identifier: "my-overview"
                ) {
                    await dataStore.refreshLibrary(reason: .manualRetry)
                }
            }
        }
    }
}

private struct ProfileIdentitySkeleton: View {
    @Environment(\.dynamicTypeSize) private var textSize
    @Environment(\.locale) private var locale

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            let layout =
                textSize.isAccessibilitySize
                ? AnyLayout(VStackLayout(alignment: .leading, spacing: 16))
                : AnyLayout(HStackLayout(alignment: .center, spacing: 16))
            layout {
                Circle().fill(.quaternary).frame(width: 72, height: 72)
                VStack(alignment: .leading, spacing: 8) {
                    Capsule().fill(.quaternary).frame(width: 160, height: 26)
                    Capsule().fill(.quaternary).frame(width: 120, height: 14)
                }
            }
            HStack(alignment: .top, spacing: 12) {
                ForEach(0..<4, id: \.self) { _ in ProfileMetricSkeleton() }
            }
        }
        .frame(maxWidth: .infinity, minHeight: 144, alignment: .bottomLeading)
        .skeletonPulse(accessibilityLabel: appLocalizedString("正在载入账户资料", locale: locale))
        .accessibilityIdentifier("my-identity-skeleton")
    }
}

private struct ProfileMetricSkeleton: View {
    var body: some View {
        VStack(alignment: .leading, spacing: AppLayoutPolicy.detailSpacing) {
            Capsule().fill(.quaternary).frame(maxWidth: 56).frame(height: 18)
            Capsule().fill(.quaternary).frame(maxWidth: 44).frame(height: 12)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}
