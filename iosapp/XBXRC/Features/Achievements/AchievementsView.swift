import SwiftUI

struct AchievementsView: View {
    @EnvironmentObject private var authStore: AuthStore
    @EnvironmentObject private var dataStore: XboxDataStore
    @State private var featuredGameID: String?
    @Namespace private var glassNamespace

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
            ContentUnavailableView("没有成就记录", systemImage: "trophy")
        } else {
            ScrollView {
                LazyVStack(spacing: 0) {
                    if let errorMessage = dataStore.libraryErrorMessage {
                        InlineDataErrorView(message: errorMessage)
                            .padding(.horizontal, 16)
                            .padding(.bottom, 16)
                    }

                    FeaturedAchievementsCarousel(
                        games: featuredGames,
                        selection: $featuredGameID
                    )
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
        case .failed:
            ContentUnavailableView {
                Label("无法载入成就", systemImage: "exclamationmark.triangle")
            } description: {
                Text(dataStore.libraryErrorMessage ?? "Xbox 服务暂时不可用")
            } actions: {
                Button("重新加载", systemImage: "arrow.clockwise") {
                    Task {
                        await dataStore.refreshLibrary()
                    }
                }
                .buttonStyle(.borderedProminent)
            }
        case .loaded:
            ContentUnavailableView("没有成就记录", systemImage: "trophy")
        }
    }
}

private struct FeaturedAchievementsCarousel: View {
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    let games: [GameSummary]
    @Binding var selection: String?
    @State private var dragProgress: CGFloat = 0
    private let cardWidth: CGFloat = 260
    private let cardHeight: CGFloat = 390
    private let orbitRadius: CGFloat = 3_600
    private let angleStepDegrees: CGFloat = 4.65
    private let dragStepWidth: CGFloat = 220

    var body: some View {
        GeometryReader { geometry in
            ZStack {
                ForEach(games.indices, id: \.self) { index in
                    carouselCard(game: games[index], at: index)
                }
            }
            .frame(width: geometry.size.width, height: geometry.size.height)
            .contentShape(Rectangle())
            .highPriorityGesture(carouselDragGesture)
        }
        .task(id: games.map(\.id)) {
            normalizeSelection()
        }
        .sensoryFeedback(.selection, trigger: selection)
    }

    private func carouselCard(game: GameSummary, at index: Int) -> some View {
        let position = relativePosition(for: index)
        // 卡片中心沿共享圆弧移动，卡片角度同步对齐圆周切线。
        let radians = Double(position * angleStepDegrees) * .pi / 180
        let distance = abs(position)
        let depth = min(distance, 2)

        return NavigationLink {
            GameAchievementsView(game: game)
        } label: {
            FeaturedAchievementCard(game: game)
                .frame(width: cardWidth, height: cardHeight)
                .mask(RoundedRectangle(cornerRadius: 16, style: .continuous))
        }
        .buttonStyle(LiquidGlassPressStyle())
        .overlay {
            RoundedRectangle(cornerRadius: 16, style: .continuous)
                .stroke(.white.opacity(0.2), lineWidth: 0.75)
        }
        .shadow(color: .black.opacity(0.22), radius: 14, y: 8)
        .contentShape(RoundedRectangle(cornerRadius: 16, style: .continuous))
        .scaleEffect(max(0.92, 1 - depth * 0.035))
        .opacity(cardOpacity(at: distance))
        .rotationEffect(.degrees(Double(position * angleStepDegrees)))
        .offset(
            x: orbitRadius * CGFloat(sin(radians)),
            y: orbitRadius * CGFloat(1 - cos(radians))
        )
        .zIndex(100 - Double(abs(position)))
        .allowsHitTesting(distance < 1.15)
        .accessibilityHidden(distance >= 1.15)
    }

    private func cardOpacity(at distance: CGFloat) -> Double {
        if distance <= 1.05 {
            return Double(1 - min(distance, 1) * 0.06)
        }
        if distance >= 1.7 {
            return 0
        }

        let fadeProgress = (distance - 1.05) / 0.65
        return Double(0.94 * (1 - fadeProgress))
    }

    private var carouselDragGesture: some Gesture {
        DragGesture(minimumDistance: 12)
            .onChanged { value in
                dragProgress = min(
                    max(-value.translation.width / dragStepWidth, -1.15),
                    1.15
                )
            }
            .onEnded { value in
                let projectedTranslation = abs(value.predictedEndTranslation.width)
                    > abs(value.translation.width)
                    ? value.predictedEndTranslation.width
                    : value.translation.width
                let step = abs(projectedTranslation) >= 48
                    ? (projectedTranslation < 0 ? 1 : -1)
                    : 0

                withAnimation(carouselAnimation) {
                    if step != 0 {
                        moveSelection(by: step)
                    }
                    dragProgress = 0
                }
            }
    }

    private var carouselAnimation: Animation {
        reduceMotion
            ? .easeOut(duration: 0.16)
            : .snappy(duration: 0.68, extraBounce: 0.06)
    }

    private func relativePosition(for index: Int) -> CGFloat {
        guard !games.isEmpty else {
            return 0
        }

        let selectedIndex = games.firstIndex { $0.id == selection } ?? 0
        var distance = index - selectedIndex
        let halfCount = games.count / 2

        if distance > halfCount {
            distance -= games.count
        } else if distance < -halfCount {
            distance += games.count
        }

        return CGFloat(distance) - dragProgress
    }

    private func moveSelection(by step: Int) {
        guard !games.isEmpty else {
            selection = nil
            return
        }

        let currentIndex = games.firstIndex { $0.id == selection } ?? 0
        let nextIndex = (currentIndex + step + games.count) % games.count
        selection = games[nextIndex].id
    }

    private func normalizeSelection() {
        dragProgress = 0
        guard games.contains(where: { $0.id == selection }) else {
            selection = games.first?.id
            return
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
            AsyncImage(url: url) { phase in
                switch phase {
                case let .success(image):
                    image
                        .resizable()
                        .scaledToFill()
                case .empty:
                    ZStack {
                        Rectangle().fill(.black.opacity(0.18))
                        ProgressView()
                            .tint(.white)
                    }
                case .failure:
                    placeholder
                @unknown default:
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
        AsyncImage(url: url) { phase in
            switch phase {
            case let .success(image):
                image
                    .resizable()
                    .scaledToFill()
            case .empty:
                ZStack {
                    Rectangle().fill(.regularMaterial)
                    ProgressView().controlSize(.small)
                }
            case .failure:
                placeholder
            @unknown default:
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
            await dataStore.loadAchievements(for: game)
        }
    }

    @ViewBuilder
    private var emptyContent: some View {
        switch dataStore.achievementPhase(for: game.titleID) {
        case .idle, .loading:
            GameAchievementsLoadingView()
        case .failed:
            ContentUnavailableView {
                Label("无法载入成就", systemImage: "exclamationmark.triangle")
            } description: {
                Text(dataStore.achievementError(for: game.titleID) ?? "Xbox 服务暂时不可用")
            } actions: {
                Button("重新加载", systemImage: "arrow.clockwise") {
                    Task {
                        await dataStore.loadAchievements(for: game, force: true)
                    }
                }
                .buttonStyle(.borderedProminent)
            }
        case .loaded:
            if searchText.isEmpty {
                ContentUnavailableView("没有成就", systemImage: "trophy")
            } else {
                ContentUnavailableView.search(text: searchText)
            }
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
            await dataStore.loadAchievements(for: game, force: true)
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
