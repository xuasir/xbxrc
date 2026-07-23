import SwiftUI

private enum XBXProfileTokens {
    static let brand = Color(red: 16 / 255, green: 124 / 255, blue: 16 / 255)
    static let success = Color(red: 43 / 255, green: 194 / 255, blue: 74 / 255)
    static let warning = Color(red: 253 / 255, green: 185 / 255, blue: 0)
    static let danger = Color(red: 1, green: 82 / 255, blue: 82 / 255)

    static let surfaceElevated = adaptive(
        dark: UIColor(red: 26 / 255, green: 27 / 255, blue: 30 / 255, alpha: 1),
        light: .white
    )
    static let surfacePanel = adaptive(
        dark: UIColor(red: 43 / 255, green: 43 / 255, blue: 43 / 255, alpha: 1),
        light: .white
    )
    static let textPrimary = adaptive(
        dark: .white,
        light: UIColor(red: 15 / 255, green: 15 / 255, blue: 16 / 255, alpha: 1)
    )
    static let textSecondary = adaptive(
        dark: UIColor(red: 209 / 255, green: 209 / 255, blue: 214 / 255, alpha: 1),
        light: UIColor(red: 85 / 255, green: 85 / 255, blue: 94 / 255, alpha: 1)
    )
    static let textTertiary = adaptive(
        dark: UIColor(red: 161 / 255, green: 161 / 255, blue: 170 / 255, alpha: 1),
        light: UIColor(red: 124 / 255, green: 124 / 255, blue: 136 / 255, alpha: 1)
    )
    static let textDisabled = adaptive(
        dark: UIColor(red: 113 / 255, green: 113 / 255, blue: 122 / 255, alpha: 1),
        light: UIColor(red: 169 / 255, green: 169 / 255, blue: 178 / 255, alpha: 1)
    )
    static let borderSubtle = adaptive(
        dark: UIColor(white: 1, alpha: 0.08),
        light: UIColor(white: 0, alpha: 0.08)
    )
    static let divider = adaptive(
        dark: UIColor(white: 1, alpha: 0.12),
        light: UIColor(white: 0, alpha: 0.12)
    )

    static let onMediaPrimary = Color.white
    static let onMediaSecondary = Color.white.opacity(0.76)
    static let onMediaTertiary = Color.white.opacity(0.58)
    static let pageInset: CGFloat = 16
    static let cardRadius: CGFloat = 12
    static let avatarSize: CGFloat = 92

    private static func adaptive(dark: UIColor, light: UIColor) -> Color {
        Color(uiColor: UIColor { traits in
            traits.userInterfaceStyle == .dark ? dark : light
        })
    }
}

struct ProfileView: View {
    @EnvironmentObject private var authStore: AuthStore
    @EnvironmentObject private var dataStore: XboxDataStore
    @EnvironmentObject private var settingsStore: AppSettingsStore
    @State private var showsSignOutConfirmation = false
    @State private var traceProfile: IOSRuntimeTraceProfile
    @State private var isApplyingRegion = false
    let isActive: Bool

    init(isActive: Bool = true) {
        self.isActive = isActive
        _traceProfile = State(initialValue: IOSRuntimeTrace.currentProfile)
    }

    private var activationID: ProfileActivationID {
        ProfileActivationID(
            ownerGeneration: authStore.ownerGeneration,
            isActive: isActive,
            phase: authStore.phase
        )
    }

    private var showsImmersiveHero: Bool {
        authStore.isSignedIn && (authStore.profile != nil || isProfileLoading)
    }

    private var isProfileLoading: Bool {
        guard authStore.isSignedIn, authStore.profile == nil else { return false }
        return authStore.phase == .refreshing
            || (authStore.phase == .signedIn && authStore.errorMessage == nil)
    }

    var body: some View {
        NavigationStack {
            rootContent
                .appThemeCanvas()
                .navigationTitle(showsImmersiveHero ? "" : "我的")
                .navigationBarTitleDisplayMode(.inline)
                .toolbarBackground(showsImmersiveHero ? .hidden : .automatic, for: .navigationBar)
                .confirmationDialog(
                    "退出 Xbox 账户？",
                    isPresented: $showsSignOutConfirmation,
                    titleVisibility: .visible
                ) {
                    Button("退出登录", role: .destructive) {
                        Task {
                            await authStore.signOut()
                        }
                    }
                }
        }
        .task(id: activationID) {
            guard isActive, authStore.phase == .signedIn else { return }
            await dataStore.sync(
                session: authStore.session,
                ownerGeneration: authStore.ownerGeneration
            )
            async let profileActivation: Void = authStore.activateProfileOnce()
            async let libraryActivation: Void = dataStore.activateLibraryOnce()
            _ = await (profileActivation, libraryActivation)
        }
    }

    @ViewBuilder
    private var rootContent: some View {
        if authStore.isSignedIn {
            signedInView(profile: authStore.profile)
        } else {
            signedOutView
        }
    }

    private func signedInView(profile: XboxProfile?) -> some View {
        ScrollView {
            LazyVStack(spacing: 0) {
                if let profile {
                    ProfileHero(
                        profile: profile,
                        recentGame: mostRecentGame
                    )
                } else {
                    profileStatusView
                }

                VStack(spacing: 16) {
                    if profile != nil || !dataStore.games.isEmpty {
                        AchievementOverviewCard(
                            overview: ProfileAchievementOverview(games: dataStore.games)
                        )
                    }

                    if profile != nil, let errorMessage = authStore.errorMessage {
                        ProfileErrorCard(message: errorMessage)
                    }

                    if let errorMessage = dataStore.libraryErrorMessage {
                        ProfileErrorCard(message: errorMessage)
                    }

                    settingsAndSupport
                    signOutRow
                }
                .frame(maxWidth: 720)
                .padding(.horizontal, XBXProfileTokens.pageInset)
                .padding(.top, showsImmersiveHero ? 16 : 24)
                .padding(.bottom, 32)
                .frame(maxWidth: .infinity)
            }
        }
        .background(.clear)
        .ignoresSafeArea(edges: showsImmersiveHero ? .top : Edge.Set())
        .refreshable {
            await refreshProfileData()
        }
    }

    private var signedOutView: some View {
        ScrollView {
            VStack(spacing: 24) {
                XboxLoginView(
                    isBusy: authStore.isBusy,
                    errorMessage: authStore.errorMessage
                ) {
                    Task {
                        await authStore.retry()
                    }
                }

                settingsAndSupport
            }
            .frame(maxWidth: 720)
            .padding(.horizontal, XBXProfileTokens.pageInset)
            .padding(.top, 24)
            .padding(.bottom, 32)
            .frame(maxWidth: .infinity)
        }
    }

    @ViewBuilder
    private var profileStatusView: some View {
        if isProfileLoading {
            ProfileLoadingView()
        } else {
            AppThemeEmptyState(
                title: "无法载入账户资料",
                systemImage: "person.crop.circle.badge.exclamationmark",
                description: authStore.errorMessage ?? "Xbox 服务暂时不可用",
                actionTitle: "重新加载"
            ) {
                Task {
                    await authStore.refreshProfile(reason: .manualRetry)
                }
            }
            .frame(maxWidth: .infinity, minHeight: 300)
        }
    }

    private var settingsAndSupport: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("设置与支持")
                .font(.headline.weight(.bold))
                .foregroundStyle(XBXProfileTokens.textPrimary)

            VStack(spacing: 0) {
                NavigationLink {
                    CloudGamingSettingsView(isApplyingRegion: $isApplyingRegion)
                } label: {
                    MySettingsRow(
                        title: "云游戏",
                        value: isApplyingRegion ? "正在应用" : settingsPresentation.cloudGamingSummary,
                        systemImage: "network",
                        tint: XBXProfileTokens.brand
                    )
                }

                settingsDivider

                NavigationLink {
                    LoginPreferencesView()
                } label: {
                    MySettingsRow(
                        title: "登录偏好",
                        value: settingsPresentation.loginMode,
                        systemImage: "person.badge.key",
                        tint: .blue
                    )
                }

                settingsDivider

                NavigationLink {
                    AppearanceSettingsView()
                } label: {
                    MySettingsRow(
                        title: "外观与图标",
                        value: "\(settingsStore.appearanceMode.title) · \(settingsStore.appIconPreset.title)",
                        systemImage: "paintbrush.pointed",
                        tint: .purple
                    )
                }

                settingsDivider

                NavigationLink {
                    DiagnosticsSettingsView(traceProfile: $traceProfile)
                } label: {
                    MySettingsRow(
                        title: "诊断",
                        value: settingsPresentation.traceSummary,
                        systemImage: "waveform.path.ecg",
                        tint: .orange
                    )
                }

                settingsDivider

                NavigationLink {
                    AboutSettingsView()
                } label: {
                    MySettingsRow(
                        title: "关于",
                        value: settingsPresentation.version,
                        systemImage: "info.circle",
                        tint: .gray
                    )
                }
            }
            .buttonStyle(.plain)
            .glassEffect(
                .regular,
                in: RoundedRectangle(cornerRadius: 16, style: .continuous)
            )
        }
    }

    private var settingsDivider: some View {
        Divider()
            .padding(.leading, 54)
    }

    private var signOutRow: some View {
        Button(role: .destructive) {
            showsSignOutConfirmation = true
        } label: {
            HStack(spacing: 12) {
                Image(systemName: "rectangle.portrait.and.arrow.right")
                    .font(.body.weight(.semibold))
                Text("退出登录")
                    .font(.body.weight(.semibold))
                Spacer(minLength: 8)
            }
            .frame(maxWidth: .infinity, minHeight: 54, alignment: .leading)
            .padding(.horizontal, 16)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .foregroundStyle(XBXProfileTokens.danger)
        .disabled(isApplyingRegion)
        .opacity(isApplyingRegion ? 0.45 : 1)
        .glassEffect(
            .regular,
            in: RoundedRectangle(cornerRadius: 16, style: .continuous)
        )
        .accessibilityHint(
            isApplyingRegion ? "地区设置应用完成后可退出账户" : "退出当前 Xbox 账户"
        )
    }

    private var settingsPresentation: MySettingsPresentation {
        MySettingsPresentation(
            appLevel: authStore.session.map { Int($0.appLevel) },
            cloudRegionTitle: settingsStore.cloudRegionPreset.title,
            usesEphemeralLoginSession: settingsStore.usesEphemeralLoginSession,
            traceProfileTitle: traceProfile.title,
            version: versionText
        )
    }

    private var versionText: String {
        let version = Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString")
            as? String ?? "unknown"
        let build = Bundle.main.object(forInfoDictionaryKey: "CFBundleVersion")
            as? String ?? "unknown"
        return "\(version) (\(build))"
    }

    private var mostRecentGame: GameSummary? {
        let datedGames = dataStore.games.compactMap { game -> (GameSummary, Date)? in
            guard let lastPlayedAt = game.lastPlayedAt else {
                return nil
            }
            return (game, lastPlayedAt)
        }
        return datedGames.max { $0.1 < $1.1 }?.0 ?? dataStore.games.first
    }

    private func refreshProfileData() async {
        guard !isApplyingRegion else { return }
        async let profileRefresh: Void = authStore.refreshProfile(reason: .manualPull)
        async let libraryRefresh: Void = dataStore.refreshLibrary(reason: .manualPull)
        _ = await (profileRefresh, libraryRefresh)
    }
}

private struct ProfileLoadingView: View {
    var body: some View {
        ZStack(alignment: .bottomLeading) {
            AppThemeBackground()

            LinearGradient(
                colors: [
                    Color.primary.opacity(0.04),
                    Color.primary.opacity(0.16),
                ],
                startPoint: .top,
                endPoint: .bottom
            )
            .accessibilityHidden(true)

            VStack(alignment: .leading, spacing: 22) {
                ProfileIdentityLoadingView()

                Divider()
                    .overlay(Color.white.opacity(0.16))

                ProfileActivityLoadingView()
                ProfileSocialStatsLoadingView()
            }
            .padding(.horizontal, 20)
            .padding(.top, 132)
            .padding(.bottom, 26)
            .frame(maxWidth: 720, alignment: .leading)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .frame(maxWidth: .infinity)
        .frame(minHeight: 540, alignment: .bottomLeading)
        .clipped()
        .skeletonPulse(accessibilityLabel: "正在载入账户资料")
    }
}

private struct ProfileIdentityLoadingView: View {
    @Environment(\.dynamicTypeSize) private var dynamicTypeSize

    var body: some View {
        Group {
            if dynamicTypeSize.isAccessibilitySize {
                VStack(alignment: .leading, spacing: 16) {
                    avatar
                    details
                }
            } else {
                ViewThatFits(in: .horizontal) {
                    HStack(alignment: .center, spacing: 16) {
                        avatar
                        details
                    }

                    VStack(alignment: .leading, spacing: 16) {
                        avatar
                        details
                    }
                }
            }
        }
    }

    private var avatar: some View {
        Circle()
            .fill(.quaternary)
            .frame(width: XBXProfileTokens.avatarSize, height: XBXProfileTokens.avatarSize)
    }

    private var details: some View {
        VStack(alignment: .leading, spacing: 7) {
            Text("玩家名称")
                .font(.system(.largeTitle, design: .default, weight: .black))
                .tracking(-0.6)
                .lineLimit(2)

            Text("@gamertag")
                .font(.subheadline.weight(.medium))
                .lineLimit(1)

            ViewThatFits(in: .horizontal) {
                HStack(spacing: 16) {
                    gamerscore
                    presence
                }

                VStack(alignment: .leading, spacing: 8) {
                    gamerscore
                    presence
                }
            }
            .padding(.top, 4)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .redacted(reason: .placeholder)
    }

    private var gamerscore: some View {
        HStack(spacing: 7) {
            Circle()
                .frame(width: 18, height: 18)
            Text("12345")
                .font(.subheadline.weight(.bold).monospacedDigit())
        }
    }

    private var presence: some View {
        HStack(spacing: 7) {
            Circle()
                .frame(width: 8, height: 8)
            Text("在线 · Xbox")
                .font(.caption.weight(.semibold))
                .lineLimit(2)
        }
    }
}

private struct ProfileActivityLoadingView: View {
    var body: some View {
        VStack(alignment: .leading, spacing: 7) {
            Text("当前活动")
                .font(.caption.weight(.bold))
                .tracking(1.2)
            Text("正在游玩的游戏")
                .font(.title3.weight(.bold))
                .lineLimit(2)
            Text("最近游玩 · 游戏名称")
                .font(.caption)
                .lineLimit(2)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .redacted(reason: .placeholder)
    }
}

private struct ProfileSocialStatsLoadingView: View {
    @Environment(\.dynamicTypeSize) private var dynamicTypeSize

    var body: some View {
        Group {
            if dynamicTypeSize.isAccessibilitySize {
                VStack(spacing: 12) {
                    ForEach(0 ..< 3, id: \.self) { index in
                        if index > 0 {
                            Divider().overlay(Color.white.opacity(0.12))
                        }
                        metric
                    }
                }
            } else {
                HStack(spacing: 16) {
                    ForEach(0 ..< 3, id: \.self) { index in
                        if index > 0 {
                            Rectangle()
                                .fill(Color.white.opacity(0.14))
                                .frame(width: 1, height: 42)
                        }
                        metric
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
            }
        }
    }

    private var metric: some View {
        VStack(alignment: .leading, spacing: 3) {
            Text("123")
                .font(.title3.weight(.bold).monospacedDigit())
            Text("社交数据")
                .font(.caption)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .redacted(reason: .placeholder)
    }
}

private struct ProfileActivationID: Equatable {
    let ownerGeneration: UInt64
    let isActive: Bool
    let phase: AuthPhase
}

private struct MySettingsRow: View {
    @Environment(\.dynamicTypeSize) private var dynamicTypeSize

    let title: String
    let value: String
    let systemImage: String
    let tint: Color

    var body: some View {
        Group {
            if dynamicTypeSize.isAccessibilitySize {
                VStack(alignment: .leading, spacing: 6) {
                    rowHeader
                    Text(value)
                        .font(.subheadline)
                        .foregroundStyle(XBXProfileTokens.textTertiary)
                        .lineLimit(2)
                        .padding(.leading, 42)
                }
                .padding(.vertical, 10)
            } else {
                HStack(spacing: 12) {
                    rowIcon
                    rowTitle
                    Spacer(minLength: 8)
                    Text(value)
                        .font(.subheadline)
                        .foregroundStyle(XBXProfileTokens.textTertiary)
                        .lineLimit(1)
                    chevron
                }
            }
        }
        .frame(maxWidth: .infinity, minHeight: 56, alignment: .leading)
        .padding(.horizontal, 12)
        .contentShape(Rectangle())
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(title)
        .accessibilityValue(value)
    }

    private var rowHeader: some View {
        HStack(spacing: 12) {
            rowIcon
            rowTitle
            Spacer(minLength: 8)
            chevron
        }
    }

    private var rowIcon: some View {
        Image(systemName: systemImage)
            .font(.system(size: 15, weight: .semibold))
            .foregroundStyle(.white)
            .frame(width: 30, height: 30)
            .background(tint, in: RoundedRectangle(cornerRadius: 7, style: .continuous))
            .accessibilityHidden(true)
    }

    private var rowTitle: some View {
        Text(title)
            .font(.body.weight(.medium))
            .foregroundStyle(XBXProfileTokens.textPrimary)
    }

    private var chevron: some View {
        Image(systemName: "chevron.right")
            .font(.caption.weight(.semibold))
            .foregroundStyle(XBXProfileTokens.textTertiary)
            .accessibilityHidden(true)
    }
}

private struct ProfileHero: View {
    @Environment(\.dynamicTypeSize) private var dynamicTypeSize

    let profile: XboxProfile
    let recentGame: GameSummary?

    private var avatarURL: URL? {
        nonEmpty(profile.displayPictureUrl).flatMap(URL.init(string:))
    }

    private var displayName: String {
        nonEmpty(profile.displayName) ?? profile.gamertag
    }

    private var presence: ProfilePresencePresentation {
        ProfilePresencePresentation(
            state: profile.presenceState,
            device: profile.presenceDevice
        )
    }

    var body: some View {
        ZStack(alignment: .bottomLeading) {
            ProfileHeroBackdrop(avatarURL: avatarURL)

            LinearGradient(
                colors: [
                    .black.opacity(0.42),
                    .black.opacity(0.10),
                    .black.opacity(0.80),
                ],
                startPoint: .top,
                endPoint: .bottom
            )
            .accessibilityHidden(true)

            VStack(alignment: .leading, spacing: 22) {
                identity

                Divider()
                    .overlay(Color.white.opacity(0.16))

                ProfileActivitySummary(
                    presenceState: profile.presenceState,
                    currentTitleName: profile.currentTitleName,
                    richPresence: profile.richPresence,
                    recentGame: recentGame
                )

                ProfileSocialStats(
                    friendCount: profile.friendCount,
                    followingCount: profile.followingCount,
                    followersCount: profile.followersCount
                )
            }
            .padding(.horizontal, 20)
            .padding(.top, 132)
            .padding(.bottom, 26)
            .frame(maxWidth: 720, alignment: .leading)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .frame(maxWidth: .infinity)
        .frame(minHeight: 540, alignment: .bottomLeading)
        .clipped()
    }

    @ViewBuilder
    private var identity: some View {
        if dynamicTypeSize.isAccessibilitySize {
            VStack(alignment: .leading, spacing: 16) {
                avatar
                identityDetails
            }
        } else {
            ViewThatFits(in: .horizontal) {
                HStack(alignment: .center, spacing: 16) {
                    avatar
                    identityDetails
                }

                VStack(alignment: .leading, spacing: 16) {
                    avatar
                    identityDetails
                }
            }
        }
    }

    private var avatar: some View {
        SharedRemoteImage(url: avatarURL) { image in
            image
                .resizable()
                .scaledToFill()
        } placeholder: { showProgress in
            if showProgress {
                ZStack {
                    Color.white.opacity(0.10)
                    ProgressView()
                        .tint(.white)
                }
            } else {
                avatarPlaceholder
            }
        }
        .frame(width: XBXProfileTokens.avatarSize, height: XBXProfileTokens.avatarSize)
        .clipShape(Circle())
        .overlay {
            Circle()
                .stroke(Color.white.opacity(0.28), lineWidth: 2)
        }
        .accessibilityHidden(true)
    }

    private var avatarPlaceholder: some View {
        ZStack {
            XBXProfileTokens.surfacePanel
            Image(systemName: "person.fill")
                .font(.system(size: 34, weight: .semibold))
                .foregroundStyle(XBXProfileTokens.onMediaSecondary)
        }
    }

    private var identityDetails: some View {
        VStack(alignment: .leading, spacing: 7) {
            Text(displayName)
                .font(.system(.largeTitle, design: .default, weight: .black))
                .tracking(-0.6)
                .foregroundStyle(XBXProfileTokens.onMediaPrimary)
                .lineLimit(2)
                .accessibilityAddTraits(.isHeader)

            Text("@\(profile.gamertag)")
                .font(.subheadline.weight(.medium))
                .foregroundStyle(XBXProfileTokens.onMediaSecondary)
                .lineLimit(1)
                .minimumScaleFactor(0.8)
                .accessibilityLabel("玩家代号 \(profile.gamertag)")

            ViewThatFits(in: .horizontal) {
                HStack(spacing: 16) {
                    gamerscore
                    presenceStatus
                }

                VStack(alignment: .leading, spacing: 8) {
                    gamerscore
                    presenceStatus
                }
            }
            .padding(.top, 4)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private var gamerscore: some View {
        HStack(spacing: 7) {
            Text("G")
                .font(.system(size: 11, weight: .black))
                .foregroundStyle(Color.white)
                .frame(width: 18, height: 18)
                .background(XBXProfileTokens.brand, in: Circle())
                .accessibilityHidden(true)

            Text(nonEmpty(profile.gamerScore) ?? "0")
                .font(.subheadline.weight(.bold).monospacedDigit())
                .foregroundStyle(XBXProfileTokens.onMediaPrimary)
                .lineLimit(1)
                .minimumScaleFactor(0.8)
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("Gamerscore \(nonEmpty(profile.gamerScore) ?? "0")")
    }

    private var presenceStatus: some View {
        HStack(spacing: 7) {
            Circle()
                .fill(presence.color)
                .frame(width: 8, height: 8)
                .accessibilityHidden(true)

            Text(presence.text)
                .font(.caption.weight(.semibold))
                .foregroundStyle(XBXProfileTokens.onMediaSecondary)
                .lineLimit(2)
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(presence.accessibilityLabel)
    }
}

private struct ProfileHeroBackdrop: View {
    let avatarURL: URL?

    var body: some View {
        GeometryReader { geometry in
            ZStack {
                AppThemeBackground()
                AvatarBackdrop(url: avatarURL)
            }
            .frame(width: geometry.size.width, height: geometry.size.height)
            .clipped()
        }
        .accessibilityHidden(true)
    }
}

private struct AvatarBackdrop: View {
    let url: URL?

    var body: some View {
        SharedRemoteImage(url: url) { image in
            image
                .resizable()
                .scaledToFill()
                .scaleEffect(1.24)
                .blur(radius: 14)
                .saturation(1.04)
                .contrast(1.06)
        } placeholder: { _ in
            EmptyView()
        }
    }
}

private struct ProfileActivitySummary: View {
    let presenceState: String?
    let currentTitleName: String?
    let richPresence: String?
    let recentGame: GameSummary?

    private var hasActivePresence: Bool {
        switch nonEmpty(presenceState)?.lowercased() {
        case "online", "away", "idle":
            return true
        default:
            return false
        }
    }

    private var currentTitle: String? {
        hasActivePresence ? nonEmpty(currentTitleName) : nil
    }

    private var activityDetail: String? {
        guard hasActivePresence, let richPresence = nonEmpty(richPresence) else {
            return nil
        }
        guard let currentTitle else {
            return richPresence
        }
        return richPresence.localizedCaseInsensitiveCompare(currentTitle) == .orderedSame
            ? nil
            : richPresence
    }

    private var hasCurrentActivity: Bool {
        currentTitle != nil || activityDetail != nil
    }

    private var headline: String {
        currentTitle ?? activityDetail ?? recentGame?.name ?? "暂无游戏活动"
    }

    private var sectionTitle: String {
        if hasCurrentActivity {
            return "当前活动"
        }
        return recentGame == nil ? "游戏活动" : "最近游玩"
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 7) {
            Text(sectionTitle)
                .font(.caption.weight(.bold))
                .tracking(1.2)
                .foregroundStyle(XBXProfileTokens.onMediaTertiary)

            Text(headline)
                .font(.title3.weight(.bold))
                .foregroundStyle(XBXProfileTokens.onMediaPrimary)
                .lineLimit(2)

            if currentTitle != nil, let activityDetail {
                Text(activityDetail)
                    .font(.subheadline)
                    .foregroundStyle(XBXProfileTokens.onMediaSecondary)
                    .lineLimit(2)
            }

            if hasCurrentActivity, let recentGame {
                Text(recentGameDescription(recentGame))
                    .font(.caption)
                    .foregroundStyle(XBXProfileTokens.onMediaTertiary)
                    .lineLimit(2)
            } else if let lastPlayedAt = recentGame?.lastPlayedAt {
                Text(
                    "上次游玩 · \(lastPlayedAt.formatted(.relative(presentation: .named, unitsStyle: .wide)))"
                )
                .font(.caption)
                .foregroundStyle(XBXProfileTokens.onMediaTertiary)
                .lineLimit(1)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .accessibilityElement(children: .combine)
    }

    private func recentGameDescription(_ game: GameSummary) -> String {
        guard let lastPlayedAt = game.lastPlayedAt else {
            return "最近游玩 · \(game.name)"
        }
        let relativeDate = lastPlayedAt.formatted(
            .relative(presentation: .named, unitsStyle: .wide)
        )
        return "最近游玩 · \(game.name) · \(relativeDate)"
    }
}

private struct ProfileSocialStats: View {
    @Environment(\.dynamicTypeSize) private var dynamicTypeSize

    let friendCount: UInt32?
    let followingCount: UInt32?
    let followersCount: UInt32?

    private var metrics: [ProfileSocialMetricValue] {
        [
            friendCount.map {
                ProfileSocialMetricValue(id: "friends", title: "好友", value: $0)
            },
            followingCount.map {
                ProfileSocialMetricValue(id: "following", title: "正在关注", value: $0)
            },
            followersCount.map {
                ProfileSocialMetricValue(id: "followers", title: "关注者", value: $0)
            },
        ].compactMap { $0 }
    }

    @ViewBuilder
    var body: some View {
        if !metrics.isEmpty {
            if dynamicTypeSize.isAccessibilitySize {
                verticalStats
            } else {
                ViewThatFits(in: .horizontal) {
                    horizontalStats
                    verticalStats
                }
            }
        }
    }

    private var horizontalStats: some View {
        HStack(spacing: 16) {
            ForEach(metrics) { metric in
                if metric.id != metrics.first?.id {
                    socialDivider
                }
                ProfileSocialMetric(title: metric.title, value: metric.value)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private var verticalStats: some View {
        VStack(spacing: 12) {
            ForEach(metrics) { metric in
                if metric.id != metrics.first?.id {
                    Divider().overlay(Color.white.opacity(0.12))
                }
                ProfileSocialMetric(title: metric.title, value: metric.value)
            }
        }
    }

    private var socialDivider: some View {
        Rectangle()
            .fill(Color.white.opacity(0.14))
            .frame(width: 1, height: 42)
            .accessibilityHidden(true)
    }
}

private struct ProfileSocialMetricValue: Identifiable {
    let id: String
    let title: String
    let value: UInt32
}

private struct ProfileSocialMetric: View {
    let title: String
    let value: UInt32

    private var displayValue: String {
        Int(value).formatted()
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 3) {
            Text(displayValue)
                .font(.title3.weight(.bold).monospacedDigit())
                .foregroundStyle(XBXProfileTokens.onMediaPrimary)
                .lineLimit(1)
                .minimumScaleFactor(0.74)
            Text(title)
                .font(.caption)
                .foregroundStyle(XBXProfileTokens.onMediaTertiary)
                .lineLimit(1)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(title)
        .accessibilityValue(displayValue)
    }
}

private struct ProfilePresencePresentation {
    let text: String
    let color: Color

    var accessibilityLabel: String {
        "在线状态 \(text)"
    }

    init(state: String?, device: String?) {
        let normalizedState = nonEmpty(state)
        let deviceName = nonEmpty(device)
        let stateText: String

        switch normalizedState?.lowercased() {
        case "online":
            stateText = "在线"
            color = XBXProfileTokens.success
        case "away", "idle":
            stateText = "暂离"
            color = XBXProfileTokens.warning
        case "offline":
            stateText = "离线"
            color = XBXProfileTokens.textDisabled
        case let rawState?:
            stateText = rawState
            color = XBXProfileTokens.textTertiary
        case nil:
            stateText = "状态暂不可用"
            color = XBXProfileTokens.textDisabled
        }

        if let deviceName {
            text = "\(stateText) · \(deviceName)"
        } else {
            text = stateText
        }
    }
}

private struct ProfileAchievementOverview {
    let gameCount: Int
    let unlockedCount: Int
    let totalCount: Int
    let earnedGamerscore: Int
    let totalGamerscore: Int

    init(games: [GameSummary]) {
        let progress = games.compactMap(\.achievementProgress)
        gameCount = progress.count
        unlockedCount = progress.reduce(0) { $0 + $1.unlockedCount }
        totalCount = progress.reduce(0) { $0 + $1.totalCount }
        earnedGamerscore = progress.reduce(0) { $0 + $1.earnedGamerscore }
        totalGamerscore = progress.reduce(0) { $0 + $1.totalGamerscore }
    }

    var percentage: Int {
        if totalGamerscore > 0 {
            return boundedPercentage(earned: earnedGamerscore, total: totalGamerscore)
        }
        return boundedPercentage(earned: unlockedCount, total: totalCount)
    }

    private func boundedPercentage(earned: Int, total: Int) -> Int {
        guard total > 0 else {
            return 0
        }
        let value = Int((Double(earned) / Double(total) * 100).rounded())
        return min(100, max(0, value))
    }
}

private struct AchievementOverviewCard: View {
    let overview: ProfileAchievementOverview

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack(alignment: .firstTextBaseline, spacing: 12) {
                Text("成就概况")
                    .font(.title3.weight(.bold))
                    .foregroundStyle(XBXProfileTokens.textPrimary)
                Spacer(minLength: 8)
                if overview.gameCount > 0 {
                    Text("\(overview.gameCount) 款游戏")
                        .font(.caption.monospacedDigit())
                        .foregroundStyle(XBXProfileTokens.textTertiary)
                }
            }

            if overview.gameCount > 0 {
                ViewThatFits(in: .horizontal) {
                    HStack(alignment: .bottom, spacing: 24) {
                        completionMetric
                        secondaryMetrics
                    }

                    VStack(alignment: .leading, spacing: 16) {
                        completionMetric
                        secondaryMetrics
                    }
                }

                ProgressView(value: Double(overview.percentage), total: 100)
                    .tint(XBXProfileTokens.brand)
                    .accessibilityLabel("总成就完成度")
                    .accessibilityValue("\(overview.percentage)%")
            } else {
                VStack(alignment: .leading, spacing: 5) {
                    Text("暂无成就概况")
                        .font(.headline.weight(.semibold))
                        .foregroundStyle(XBXProfileTokens.textSecondary)
                    Text("游戏库同步完成后会显示汇总数据")
                        .font(.caption)
                        .foregroundStyle(XBXProfileTokens.textTertiary)
                }
                .frame(maxWidth: .infinity, alignment: .leading)
            }
        }
        .padding(14)
        .overlay {
            RoundedRectangle(cornerRadius: 16, style: .continuous)
                .stroke(.separator.opacity(0.35), lineWidth: 0.5)
        }
        .glassEffect(
            .regular,
            in: RoundedRectangle(cornerRadius: 16, style: .continuous)
        )
        .contentShape(RoundedRectangle(cornerRadius: 16, style: .continuous))
    }

    private var completionMetric: some View {
        VStack(alignment: .leading, spacing: 3) {
            Text("\(overview.percentage)%")
                .font(.system(.largeTitle, design: .default, weight: .black))
                .foregroundStyle(XBXProfileTokens.textPrimary)
                .monospacedDigit()
            Text("整体完成度")
                .font(.caption)
                .foregroundStyle(XBXProfileTokens.textTertiary)
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("整体完成度")
        .accessibilityValue("\(overview.percentage)%")
    }

    private var secondaryMetrics: some View {
        HStack(spacing: 20) {
            ProfileAchievementMetric(
                title: "已解锁",
                value: "\(overview.unlockedCount)/\(overview.totalCount)"
            )
            ProfileAchievementMetric(
                title: "成就点数",
                value: "\(overview.earnedGamerscore)/\(overview.totalGamerscore)"
            )
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

private struct ProfileAchievementMetric: View {
    let title: String
    let value: String

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(value)
                .font(.headline.weight(.bold).monospacedDigit())
                .foregroundStyle(XBXProfileTokens.textSecondary)
                .lineLimit(1)
                .minimumScaleFactor(0.68)
            Text(title)
                .font(.caption)
                .foregroundStyle(XBXProfileTokens.textTertiary)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(title)
        .accessibilityValue(value)
    }
}

private struct ProfileErrorCard: View {
    let message: String

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 10) {
            Image(systemName: "exclamationmark.triangle.fill")
                .foregroundStyle(XBXProfileTokens.danger)
                .accessibilityHidden(true)
            Text(message)
                .foregroundStyle(XBXProfileTokens.textSecondary)
        }
        .font(.footnote)
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(14)
        .glassEffect(
            .regular,
            in: RoundedRectangle(
                cornerRadius: XBXProfileTokens.cardRadius,
                style: .continuous
            )
        )
        .accessibilityElement(children: .combine)
    }
}

private func nonEmpty(_ value: String?) -> String? {
    guard let value else {
        return nil
    }
    let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
    return trimmed.isEmpty ? nil : trimmed
}
