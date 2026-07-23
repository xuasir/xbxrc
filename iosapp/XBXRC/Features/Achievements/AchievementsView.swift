import SwiftUI

struct AchievementsView: View {
    @EnvironmentObject private var authStore: AuthStore
    @EnvironmentObject private var dataStore: XboxDataStore
    @State private var featuredGameID: String?
    @Namespace private var glassNamespace
    let isActive: Bool

    init(isActive: Bool = true) {
        self.isActive = isActive
    }

    private var activationID: String {
        "\(authStore.ownerGeneration):\(isActive)"
    }

    private var games: [GameSummary] {
        dataStore.games.filter { $0.achievementProgress != nil }
    }

    private var featuredGames: [GameSummary] {
        Array(games.prefix(5))
    }

    var body: some View {
        NavigationStack {
            content
                .appThemeCanvas()
        }
        .task(id: activationID) {
            guard isActive, authStore.phase == .signedIn else { return }
            await dataStore.sync(
                session: authStore.session,
                ownerGeneration: authStore.ownerGeneration
            )
            await dataStore.activateLibraryOnce()
        }
    }

    @ViewBuilder
    private var content: some View {
        if !authStore.isSignedIn {
            XboxLoginView(
                isBusy: authStore.isBusy,
                errorMessage: authStore.errorMessage
            ) {
                Task {
                    await authStore.retry()
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        } else if dataStore.games.isEmpty {
            emptyLibraryContent
        } else if games.isEmpty {
            refreshableLibraryEmptyState {
                AppThemeEmptyState(
                    title: "没有成就记录",
                    systemImage: "trophy"
                )
            }
        } else {
            ScrollView {
                LazyVStack(spacing: 0) {
                    if let errorMessage = dataStore.libraryErrorMessage {
                        InlineDataErrorView(message: errorMessage)
                            .padding(.horizontal, 16)
                            .padding(.bottom, 16)
                    }

                    OrbitCardCarousel(
                        items: featuredGames,
                        selection: $featuredGameID
                    ) { game in
                        NavigationLink {
                            GameAchievementsView(game: game)
                        } label: {
                            FeaturedAchievementCard(game: game)
                        }
                    }
                    .frame(height: 390)
                    .padding(.top, 12)
                    .padding(.bottom, 20)

                    HStack {
                        Text("全部游戏")
                            .font(.headline)
                        Spacer()
                        Text("\(games.count)")
                            .font(.caption.monospacedDigit())
                            .foregroundStyle(.secondary)
                    }
                    .padding(.horizontal, 16)
                    .padding(.bottom, 12)

                    GlassEffectContainer(spacing: 12) {
                        LazyVStack(spacing: 12) {
                            ForEach(games) { game in
                                NavigationLink {
                                    GameAchievementsView(game: game)
                                } label: {
                                    AchievementGameCard(
                                        game: game,
                                        namespace: glassNamespace
                                    )
                                }
                                .buttonStyle(LiquidGlassPressStyle())
                            }
                        }
                    }
                    .padding(.horizontal, 14)
                }
                .padding(.bottom, 24)
            }
            .refreshable {
                await dataStore.refreshLibrary()
            }
        }
    }

    @ViewBuilder
    private var emptyLibraryContent: some View {
        switch dataStore.libraryPhase {
        case .idle, .loading:
            AchievementsLoadingView()
                .refreshable { await dataStore.refreshLibrary() }
        case .failed:
            refreshableLibraryEmptyState {
                AppThemeEmptyState(
                    title: "无法载入成就",
                    systemImage: "exclamationmark.triangle",
                    description: dataStore.libraryErrorMessage ?? "Xbox 服务暂时不可用",
                    actionTitle: "重新加载"
                ) {
                    Task {
                        await dataStore.refreshLibrary(reason: .manualRetry)
                    }
                }
            }
        case .loaded:
            refreshableLibraryEmptyState {
                AppThemeEmptyState(
                    title: "没有成就记录",
                    systemImage: "trophy"
                )
            }
        }
    }

    private func refreshableLibraryEmptyState<Content: View>(
        @ViewBuilder content: @escaping () -> Content
    ) -> some View {
        GeometryReader { geometry in
            ScrollView {
                content()
                    .frame(maxWidth: .infinity, minHeight: geometry.size.height)
            }
            .refreshable { await dataStore.refreshLibrary() }
        }
    }
}

private struct FeaturedAchievementCard: View {
    let game: GameSummary

    private var progress: AchievementProgress {
        game.achievementProgress ?? AchievementProgress(
            unlockedCount: 0,
            totalCount: 0,
            earnedGamerscore: 0,
            totalGamerscore: 0
        )
    }

    var body: some View {
        ZStack(alignment: .bottom) {
            FeaturedGameArtwork(url: game.heroURL ?? game.artworkURL)

            LinearGradient(
                colors: [
                    .clear,
                    .black.opacity(0.12),
                    .black.opacity(0.82),
                ],
                startPoint: .center,
                endPoint: .bottom
            )

            VStack(alignment: .leading, spacing: 9) {
                Text(game.name)
                    .font(.title2.bold())
                    .foregroundStyle(.white)
                    .lineLimit(2)

                Label(XboxPresentation.playtime(game.playtimeMinutes), systemImage: "clock")
                    .font(.subheadline)
                    .foregroundStyle(.white.opacity(0.82))

                HStack(spacing: 12) {
                    CompactAchievementMetric(
                        icon: .gamerscore,
                        value: "\(progress.earnedGamerscore)/\(progress.totalGamerscore)"
                    )
                    CompactAchievementMetric(
                        icon: .system("trophy.fill"),
                        value: "\(progress.unlockedCount)/\(progress.totalCount)"
                    )
                    CompactAchievementMetric(
                        icon: .system("chart.bar.fill"),
                        value: "\(progress.percentage)%"
                    )
                }
                .font(.caption.bold().monospacedDigit())
                .foregroundStyle(.white)
                .lineLimit(1)
                .minimumScaleFactor(0.72)

                ProgressView(value: Double(progress.percentage), total: 100)
                    .tint(.green)
            }
            .padding(18)
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(accessibilityLabel)
    }

    private var accessibilityLabel: String {
        [
            game.name,
            XboxPresentation.playtime(game.playtimeMinutes),
            "成就点数 \(progress.earnedGamerscore) 分，共 \(progress.totalGamerscore) 分",
            "已获成就 \(progress.unlockedCount) 项，共 \(progress.totalCount) 项",
            "全成就进度 \(progress.percentage)%",
        ].joined(separator: "，")
    }
}

private struct FeaturedGameArtwork: View {
    let url: URL?

    var body: some View {
        GeometryReader { geometry in
            SharedRemoteImage(url: url) { image in
                image
                    .resizable()
                    .scaledToFill()
            } placeholder: { showProgress in
                if showProgress {
                    ZStack {
                        Rectangle().fill(.black.opacity(0.18))
                        ProgressView()
                            .tint(.white)
                    }
                } else {
                    placeholder
                }
            }
            .frame(width: geometry.size.width, height: geometry.size.height)
            .clipped()
        }
        .accessibilityHidden(true)
    }

    private var placeholder: some View {
        ZStack {
            Rectangle().fill(.quaternary)
            Image(systemName: "photo.fill")
                .font(.largeTitle)
                .foregroundStyle(.secondary)
        }
    }
}

private struct LiquidGlassPressStyle: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .scaleEffect(configuration.isPressed ? 0.975 : 1)
            .brightness(configuration.isPressed ? -0.04 : 0)
            .animation(.snappy(duration: 0.22), value: configuration.isPressed)
    }
}

private struct AchievementGameCard: View {
    let game: GameSummary
    let namespace: Namespace.ID

    private var progress: AchievementProgress {
        game.achievementProgress ?? AchievementProgress(
            unlockedCount: 0,
            totalCount: 0,
            earnedGamerscore: 0,
            totalGamerscore: 0
        )
    }

    var body: some View {
        HStack(alignment: .top, spacing: 12) {
            GameArtworkView(url: game.artworkURL, size: 76)

            VStack(alignment: .leading, spacing: 6) {
                Text(game.name)
                    .font(.headline)
                    .lineLimit(2)
                    .frame(maxWidth: .infinity, alignment: .leading)

                HStack(spacing: 10) {
                    CompactAchievementMetric(
                        icon: .system("clock"),
                        value: XboxPresentation.playtime(game.playtimeMinutes),
                        foregroundStyle: .secondary,
                        font: .caption.weight(.semibold).monospacedDigit()
                    )
                    CompactAchievementMetric(
                        icon: .gamerscore,
                        value: "\(progress.earnedGamerscore) / \(progress.totalGamerscore)",
                        foregroundStyle: .secondary,
                        font: .caption.weight(.semibold).monospacedDigit()
                    )
                }

                HStack(spacing: 10) {
                    CompactAchievementMetric(
                        icon: .system("trophy.fill"),
                        value: "\(progress.unlockedCount) / \(progress.totalCount)",
                        foregroundStyle: .secondary,
                        font: .caption.weight(.semibold).monospacedDigit()
                    )
                    CompactAchievementMetric(
                        icon: .system("chart.bar.fill"),
                        value: "\(progress.percentage)%",
                        foregroundStyle: .secondary,
                        font: .caption.weight(.semibold).monospacedDigit()
                    )
                }

                ProgressView(value: Double(progress.percentage), total: 100)
                    .tint(.green)
                    .accessibilityLabel("全成就进度")
                    .accessibilityValue("\(progress.percentage)%")
            }
        }
        .padding(14)
        .overlay {
            RoundedRectangle(cornerRadius: 16, style: .continuous)
                .stroke(.separator.opacity(0.35), lineWidth: 0.5)
        }
        .glassEffect(
            .regular.interactive(),
            in: RoundedRectangle(cornerRadius: 16, style: .continuous)
        )
        .glassEffectID("list-card-\(game.id)", in: namespace)
        .glassEffectTransition(.matchedGeometry)
        .contentShape(RoundedRectangle(cornerRadius: 16, style: .continuous))
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(accessibilityLabel)
    }

    private var accessibilityLabel: String {
        [
            game.name,
            "游戏时长 \(XboxPresentation.playtime(game.playtimeMinutes))",
            "成就点数 \(progress.earnedGamerscore) 分，共 \(progress.totalGamerscore) 分",
            "已获成就 \(progress.unlockedCount) 项，共 \(progress.totalCount) 项",
            "全成就进度 \(progress.percentage)%",
        ].joined(separator: "，")
    }
}

private enum AchievementMetricIcon {
    case system(String)
    case gamerscore
}

private struct CompactAchievementMetric: View {
    let icon: AchievementMetricIcon
    let value: String
    var foregroundStyle: Color = .white
    var font: Font = .subheadline.weight(.semibold).monospacedDigit()

    var body: some View {
        HStack(spacing: 6) {
            metricIcon
            Text(value)
                .font(font)
                .lineLimit(1)
                .minimumScaleFactor(0.72)
        }
        .foregroundStyle(foregroundStyle)
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    @ViewBuilder
    private var metricIcon: some View {
        switch icon {
        case let .system(name):
            Image(systemName: name)
                .frame(width: 18, height: 18)
        case .gamerscore:
            GamerscoreCoinIcon()
        }
    }
}

struct GameArtworkView: View {
    let url: URL?
    let size: CGFloat

    var body: some View {
        SharedRemoteImage(url: url) { image in
            image
                .resizable()
                .scaledToFill()
        } placeholder: { showProgress in
            if showProgress {
                ZStack {
                    Rectangle().fill(.regularMaterial)
                    ProgressView().controlSize(.small)
                }
            } else {
                placeholder
            }
        }
        .frame(width: size, height: size)
        .clipShape(RoundedRectangle(cornerRadius: 8, style: .continuous))
        .accessibilityHidden(true)
    }

    private var placeholder: some View {
        ZStack {
            Rectangle().fill(.quaternary)
            Image(systemName: "photo")
                .foregroundStyle(.secondary)
        }
    }
}

struct InlineDataErrorView: View {
    let message: String

    var body: some View {
        Label(message, systemImage: "exclamationmark.triangle.fill")
            .font(.footnote)
            .foregroundStyle(.red)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(12)
            .glassEffect(
                .regular.tint(.red.opacity(0.12)),
                in: RoundedRectangle(cornerRadius: 8, style: .continuous)
            )
    }
}

enum XboxPresentation {
    static func playtime(_ minutes: Int?) -> String {
        guard let minutes else {
            return "时长未知"
        }
        if minutes < 60 {
            return "\(minutes) 分钟"
        }
        let hours = Double(minutes) / 60
        if hours < 10 {
            return "\(hours.formatted(.number.precision(.fractionLength(1)))) 小时"
        }
        return "\(Int(hours.rounded())) 小时"
    }
}

struct GameAchievementsView: View {
    @EnvironmentObject private var dataStore: XboxDataStore
    let game: GameSummary
    @State private var searchText = ""
    @Namespace private var glassNamespace

    private var achievements: [AchievementSummary] {
        let values = dataStore.achievements(for: game.titleID)
        let filtered = searchText.isEmpty
            ? values
            : values.filter { $0.name.localizedStandardContains(searchText) }
        return filtered.sorted(by: achievementOrder)
    }

    var body: some View {
        Group {
            if achievements.isEmpty {
                emptyContent
            } else {
                achievementList
            }
        }
        .appThemeCanvas()
        .navigationTitle(game.name)
        .navigationBarTitleDisplayMode(.inline)
        .searchable(text: $searchText, prompt: "搜索成就")
        .task(id: game.titleID) {
            await dataStore.activateAchievementsOnce(for: game)
        }
    }

    @ViewBuilder
    private var emptyContent: some View {
        switch dataStore.achievementPhase(for: game.titleID) {
        case .idle, .loading:
            GameAchievementsLoadingView()
                .refreshable { await dataStore.refreshAchievements(for: game) }
        case .failed:
            refreshableEmptyState {
                AppThemeEmptyState(
                    title: "无法载入成就",
                    systemImage: "exclamationmark.triangle",
                    description: dataStore.achievementError(for: game.titleID)
                        ?? "Xbox 服务暂时不可用",
                    actionTitle: "重新加载"
                ) {
                    Task {
                        await dataStore.refreshAchievements(for: game)
                    }
                }
            }
        case .loaded:
            refreshableEmptyState {
                if searchText.isEmpty {
                    AppThemeEmptyState(
                        title: "没有成就",
                        systemImage: "trophy"
                    )
                } else {
                    AppThemeEmptyState(
                        title: "未找到结果",
                        systemImage: "magnifyingglass",
                        description: "没有与“\(searchText)”匹配的成就"
                    )
                }
            }
        }
    }

    private func refreshableEmptyState<Content: View>(
        @ViewBuilder content: @escaping () -> Content
    ) -> some View {
        GeometryReader { geometry in
            ScrollView {
                content()
                    .frame(maxWidth: .infinity, minHeight: geometry.size.height)
            }
            .refreshable { await dataStore.refreshAchievements(for: game) }
        }
    }

    private var achievementList: some View {
        ScrollView {
            GlassEffectContainer(spacing: 12) {
                LazyVStack(spacing: 12) {
                    gameHeader

                    if let error = dataStore.achievementError(for: game.titleID) {
                        InlineDataErrorView(message: error)
                    }

                    ForEach(achievements) { achievement in
                        NavigationLink {
                            AchievementDetailView(achievement: achievement)
                        } label: {
                            AchievementRow(
                                achievement: achievement,
                                namespace: glassNamespace
                            )
                        }
                        .buttonStyle(LiquidGlassPressStyle())
                    }
                }
            }
            .padding(.horizontal, 16)
            .padding(.top, 12)
            .padding(.bottom, 24)
        }
        .refreshable {
            await dataStore.refreshAchievements(for: game)
        }
    }

    private var gameHeader: some View {
        HStack(spacing: 14) {
            GameArtworkView(url: game.artworkURL, size: 72)
            VStack(alignment: .leading, spacing: 6) {
                Text(game.name)
                    .font(.headline)
                if let progress = game.achievementProgress {
                    Text("\(progress.unlockedCount)/\(progress.totalCount) 成就")
                    Text("\(progress.earnedGamerscore)/\(progress.totalGamerscore) 分")
                }
            }
            .font(.subheadline.monospacedDigit())
            .foregroundStyle(.secondary)
        }
        .padding(16)
        .frame(maxWidth: .infinity, alignment: .leading)
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

    private func achievementOrder(_ lhs: AchievementSummary, _ rhs: AchievementSummary) -> Bool {
        if lhs.isUnlocked != rhs.isUnlocked {
            return !lhs.isUnlocked
        }
        if lhs.isUnlocked {
            return (lhs.unlockedAt ?? .distantPast) > (rhs.unlockedAt ?? .distantPast)
        }
        return (lhs.progressPercentage ?? 0) > (rhs.progressPercentage ?? 0)
    }
}

private struct AchievementsLoadingView: View {
    var body: some View {
        ScrollView {
            LazyVStack(spacing: 0) {
                featuredCard
                    .padding(.top, 12)
                    .padding(.bottom, 20)

                HStack {
                    Capsule()
                        .fill(.quaternary)
                        .frame(width: 96, height: 18)
                    Spacer()
                    Capsule()
                        .fill(.quaternary)
                        .frame(width: 22, height: 12)
                }
                .padding(.horizontal, 16)
                .padding(.bottom, 12)

                LazyVStack(spacing: 12) {
                    ForEach(0..<4, id: \.self) { _ in
                        AchievementGameSkeletonRow()
                    }
                }
                .padding(.horizontal, 14)
            }
            .padding(.bottom, 24)
        }
        .skeletonPulse(accessibilityLabel: "正在载入成就列表")
    }

    private var featuredCard: some View {
        RoundedRectangle(cornerRadius: 16, style: .continuous)
            .fill(.quaternary)
            .frame(width: 260, height: 390)
            .overlay(alignment: .bottomLeading) {
                VStack(alignment: .leading, spacing: 10) {
                    Capsule()
                        .fill(.tertiary)
                        .frame(width: 170, height: 24)
                    Capsule()
                        .fill(.tertiary)
                        .frame(width: 116, height: 14)
                    Capsule()
                        .fill(.tertiary)
                        .frame(height: 6)
                }
                .padding(18)
            }
    }
}

private struct AchievementGameSkeletonRow: View {
    var body: some View {
        HStack(alignment: .top, spacing: 12) {
            RoundedRectangle(cornerRadius: 8, style: .continuous)
                .fill(.quaternary)
                .frame(width: 76, height: 76)

            VStack(alignment: .leading, spacing: 8) {
                Capsule()
                    .fill(.quaternary)
                    .frame(width: 164, height: 16)

                HStack(spacing: 10) {
                    metricPlaceholder(width: 78)
                    metricPlaceholder(width: 92)
                }

                HStack(spacing: 10) {
                    metricPlaceholder(width: 68)
                    metricPlaceholder(width: 54)
                }

                Capsule()
                    .fill(.quaternary)
                    .frame(height: 5)
            }
        }
        .padding(14)
        .frame(maxWidth: .infinity, alignment: .leading)
        .overlay {
            RoundedRectangle(cornerRadius: 16, style: .continuous)
                .stroke(.separator.opacity(0.24), lineWidth: 0.5)
        }
        .glassEffect(
            .regular,
            in: RoundedRectangle(cornerRadius: 16, style: .continuous)
        )
    }

    private func metricPlaceholder(width: CGFloat) -> some View {
        HStack(spacing: 6) {
            Circle()
                .fill(.quaternary)
                .frame(width: 18, height: 18)
            Capsule()
                .fill(.quaternary)
                .frame(width: width, height: 10)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

private struct GameAchievementsLoadingView: View {
    var body: some View {
        ScrollView {
            LazyVStack(spacing: 12) {
                headerPlaceholder

                ForEach(0..<5, id: \.self) { _ in
                    achievementPlaceholder
                }
            }
            .padding(.horizontal, 16)
            .padding(.top, 12)
            .padding(.bottom, 24)
        }
        .skeletonPulse(accessibilityLabel: "正在载入游戏成就列表")
    }

    private var headerPlaceholder: some View {
        HStack(spacing: 14) {
            RoundedRectangle(cornerRadius: 8, style: .continuous)
                .fill(.quaternary)
                .frame(width: 72, height: 72)

            VStack(alignment: .leading, spacing: 8) {
                Capsule()
                    .fill(.quaternary)
                    .frame(width: 170, height: 16)
                Capsule()
                    .fill(.quaternary)
                    .frame(width: 112, height: 12)
                Capsule()
                    .fill(.quaternary)
                    .frame(width: 132, height: 12)
            }
        }
        .skeletonCardPadding(16)
    }

    private var achievementPlaceholder: some View {
        HStack(alignment: .top, spacing: 14) {
            RoundedRectangle(cornerRadius: 8, style: .continuous)
                .fill(.quaternary)
                .frame(width: 56, height: 56)

            VStack(alignment: .leading, spacing: 8) {
                HStack {
                    Capsule()
                        .fill(.quaternary)
                        .frame(width: 146, height: 16)
                    Spacer()
                    Capsule()
                        .fill(.quaternary)
                        .frame(width: 38, height: 11)
                }

                Capsule()
                    .fill(.quaternary)
                    .frame(maxWidth: .infinity)
                    .frame(height: 11)
                Capsule()
                    .fill(.quaternary)
                    .frame(width: 132, height: 11)
                Capsule()
                    .fill(.quaternary)
                    .frame(width: 88, height: 11)
            }
        }
        .skeletonCardPadding(14)
    }
}

private extension View {
    func skeletonCardPadding(_ length: CGFloat) -> some View {
        padding(length)
            .frame(maxWidth: .infinity, alignment: .leading)
            .overlay {
                RoundedRectangle(cornerRadius: 16, style: .continuous)
                    .stroke(.separator.opacity(0.24), lineWidth: 0.5)
            }
            .glassEffect(
                .regular,
                in: RoundedRectangle(cornerRadius: 16, style: .continuous)
            )
    }
}

private struct AchievementRow: View {
    let achievement: AchievementSummary
    let namespace: Namespace.ID

    var body: some View {
        HStack(alignment: .top, spacing: 14) {
            GameArtworkView(url: achievement.imageURL, size: 56)
                .opacity(achievement.isUnlocked ? 1 : 0.65)

            VStack(alignment: .leading, spacing: 5) {
                HStack(alignment: .firstTextBaseline, spacing: 8) {
                    Text(achievement.name)
                        .font(.headline)
                        .lineLimit(2)
                    Spacer(minLength: 8)
                    Label("\(achievement.gamerscore)", systemImage: "diamond.fill")
                        .font(.caption.bold().monospacedDigit())
                        .foregroundStyle(achievement.isUnlocked ? .green : .secondary)
                }

                if !achievement.description.isEmpty {
                    Text(achievement.description)
                        .font(.subheadline)
                        .foregroundStyle(.secondary)
                        .lineLimit(2)
                }

                if achievement.isUnlocked {
                    Label("已解锁", systemImage: "checkmark.circle.fill")
                        .foregroundStyle(.green)
                } else if let progress = achievement.progressPercentage, progress > 0 {
                    HStack(spacing: 8) {
                        ProgressView(value: Double(progress), total: 100)
                            .tint(.green)
                        Text("\(progress)%")
                            .monospacedDigit()
                    }
                } else {
                    Label("未解锁", systemImage: "lock.fill")
                        .foregroundStyle(.secondary)
                }
            }
            .font(.caption)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .padding(14)
        .frame(maxWidth: .infinity, alignment: .leading)
        .overlay {
            RoundedRectangle(cornerRadius: 16, style: .continuous)
                .stroke(.separator.opacity(0.35), lineWidth: 0.5)
        }
        .glassEffect(
            .regular.interactive(),
            in: RoundedRectangle(cornerRadius: 16, style: .continuous)
        )
        .glassEffectID("achievement-\(achievement.id)", in: namespace)
        .glassEffectTransition(.matchedGeometry)
        .contentShape(RoundedRectangle(cornerRadius: 16, style: .continuous))
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(accessibilityLabel)
    }

    private var accessibilityLabel: String {
        let state = achievement.isUnlocked ? "已解锁" : "未解锁"
        return "\(achievement.name)，\(state)，\(achievement.gamerscore) 分，\(achievement.description)"
    }
}

private struct AchievementDetailView: View {
    let achievement: AchievementSummary

    var body: some View {
        ScrollView {
            VStack(spacing: 22) {
                GameArtworkView(url: achievement.imageURL, size: 132)

                VStack(spacing: 8) {
                    Text(achievement.name)
                        .font(.title2.bold())
                        .multilineTextAlignment(.center)
                    HStack(spacing: 7) {
                        GamerscoreCoinIcon()
                        Text("\(achievement.gamerscore)")
                    }
                    .font(.headline.monospacedDigit())
                    .foregroundStyle(achievement.isUnlocked ? .green : .secondary)
                }

                VStack(alignment: .leading, spacing: 16) {
                    LabeledContent("状态") {
                        Label(
                            achievement.isUnlocked ? "已解锁" : "未解锁",
                            systemImage: achievement.isUnlocked ? "checkmark.circle.fill" : "lock.fill"
                        )
                        .foregroundStyle(achievement.isUnlocked ? .green : .secondary)
                    }

                    if let progress = achievement.progressPercentage, !achievement.isUnlocked {
                        VStack(alignment: .leading, spacing: 8) {
                            LabeledContent("进度", value: "\(progress)%")
                                .monospacedDigit()
                            ProgressView(value: Double(progress), total: 100)
                                .tint(.green)
                        }
                    }

                    if let unlockedAt = achievement.unlockedAt {
                        LabeledContent("解锁时间") {
                            Text(unlockedAt, format: .dateTime.year().month().day().hour().minute())
                        }
                    }

                    if !achievement.description.isEmpty {
                        Text(achievement.description)
                            .font(.body)
                            .foregroundStyle(.secondary)
                            .frame(maxWidth: .infinity, alignment: .leading)
                    }
                }
                .padding(16)
                .frame(maxWidth: .infinity, alignment: .leading)
                .glassEffect(
                    .regular,
                    in: RoundedRectangle(cornerRadius: 16, style: .continuous)
                )
            }
            .padding(24)
            .frame(maxWidth: 620)
            .frame(maxWidth: .infinity)
        }
        .appThemeCanvas()
        .navigationTitle("成就详情")
        .navigationBarTitleDisplayMode(.inline)
    }
}
