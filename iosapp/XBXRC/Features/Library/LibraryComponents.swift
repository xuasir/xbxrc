import SwiftUI

enum LibraryLayoutMetrics {
    static let heroMinimumHeight: CGFloat = 480
    static let heroMaximumHeight: CGFloat = 540
    static let heroViewportFraction: CGFloat = 0.54
    static let posterWidth: CGFloat = 112
    static let posterHeight: CGFloat = 168
    static let posterCornerRadius: CGFloat = 14
    static let recentCardWidth: CGFloat = 250
    static let recentCardHeight: CGFloat = 148
    static let collectionPageSize = 24
    static let homeShelfLimit = 8
    static let heroLimit = 5

    static func heroHeight(for viewportHeight: CGFloat) -> CGFloat {
        min(heroMaximumHeight, max(heroMinimumHeight, viewportHeight * heroViewportFraction))
    }
}

struct RecentlyPlayedHeroCarousel: View {
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    let games: [CloudLibraryGame]
    let height: CGFloat

    @State private var selection = ""

    var body: some View {
        ZStack(alignment: .bottom) {
            TabView(selection: $selection) {
                ForEach(games) { game in
                    NavigationLink {
                        GameDetailView(game: game)
                            .toolbar(.visible, for: .navigationBar)
                    } label: {
                        LibraryHeroSlide(game: game)
                    }
                    .buttonStyle(LibraryPressStyle())
                    .tag(game.id)
                }
            }
            .tabViewStyle(.page(indexDisplayMode: .never))

            LibraryHeroGlassTransition()
                .frame(height: 88)
                .offset(y: 14)

            if games.count > 1 {
                LibraryPageIndicator(
                    games: games,
                    selection: selection
                )
                .padding(.bottom, 20)
            }
        }
        .frame(height: height)
        .padding(.bottom, 14)
        .ignoresSafeArea(edges: .top)
        .onAppear(perform: normalizeSelection)
        .onChange(of: games.map(\.id)) { _, _ in
            normalizeSelection()
        }
        .task(id: autoAdvanceID) {
            await scheduleAutoAdvance()
        }
        .sensoryFeedback(.selection, trigger: selection)
        .accessibilityElement(children: .contain)
    }

    private var autoAdvanceID: String {
        "\(reduceMotion)-\(selection)-\(games.map(\.id).joined(separator: ":"))"
    }

    private func normalizeSelection() {
        guard let firstGame = games.first else {
            selection = ""
            return
        }
        if !games.contains(where: { $0.id == selection }) {
            selection = firstGame.id
            IOSRuntimeTrace.state(
                domain: "library-ui",
                event: "heroSelectionNormalized",
                payload: ["games": .integer(Int64(games.count))],
                dimension: .frontend,
                importance: .debug
            )
        }
    }

    @MainActor
    private func scheduleAutoAdvance() async {
        guard !reduceMotion, games.count > 1, !selection.isEmpty else {
            return
        }

        do {
            try await Task.sleep(for: .seconds(6))
        } catch {
            return
        }
        guard !Task.isCancelled,
              let currentIndex = games.firstIndex(where: { $0.id == selection })
        else {
            return
        }

        let nextIndex = games.index(after: currentIndex)
        let nextGame = nextIndex == games.endIndex ? games[0] : games[nextIndex]
        withAnimation(.easeInOut(duration: 0.45)) {
            selection = nextGame.id
        }
        IOSRuntimeTrace.event(
            domain: "library-ui",
            event: "heroAutoAdvanced",
            payload: [
                "games": .integer(Int64(games.count)),
                "nextIndex": .integer(Int64(nextIndex == games.endIndex ? 0 : nextIndex)),
            ],
            dimension: .frontend,
            importance: .debug
        )
    }
}

struct LibraryShelf: View {
    let collection: LibraryCollection

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            NavigationLink {
                LibraryCollectionView(collection: collection)
                    .toolbar(.visible, for: .navigationBar)
            } label: {
                HStack(spacing: 8) {
                    Text(collection.title)
                        .font(.title2.bold())
                        .foregroundStyle(.primary)
                    Spacer(minLength: 12)
                    Text("\(collection.games.count)")
                        .font(.subheadline.monospacedDigit())
                        .foregroundStyle(.secondary)
                    Image(systemName: "chevron.right")
                        .font(.subheadline.bold())
                        .foregroundStyle(.secondary)
                }
                .frame(minHeight: 44)
                .contentShape(Rectangle())
            }
            .buttonStyle(LibraryPressStyle())
            .padding(.horizontal, 16)
            .accessibilityLabel("\(collection.title)，共 \(collection.games.count) 个游戏")
            .accessibilityHint("打开完整列表")

            ScrollView(.horizontal) {
                LazyHStack(alignment: .top, spacing: 12) {
                    ForEach(collection.homeGames) { game in
                        NavigationLink {
                            GameDetailView(game: game)
                                .toolbar(.visible, for: .navigationBar)
                        } label: {
                            if collection.kind == .recent {
                                LibraryRecentCard(game: game)
                            } else {
                                LibraryPosterCard(
                                    game: game,
                                    kind: collection.kind
                                )
                            }
                        }
                        .buttonStyle(LibraryPressStyle())
                        .accessibilityLabel(gameAccessibilityLabel(game))
                        .accessibilityHint("查看游戏详情")
                    }
                }
            }
            .contentMargins(.horizontal, 16, for: .scrollContent)
            .scrollIndicators(.hidden)
        }
    }

    private func gameAccessibilityLabel(_ game: CloudLibraryGame) -> String {
        [
            game.name,
            LibraryPresentation.metadata(for: game, kind: collection.kind),
        ].joined(separator: "，")
    }
}

struct LibraryCollectionView: View {
    @ScaledMetric(relativeTo: .largeTitle) private var titleSize: CGFloat = 42

    let collection: LibraryCollection
    @State private var visibleGameCount = LibraryLayoutMetrics.collectionPageSize

    private var visibleGames: [CloudLibraryGame] {
        Array(collection.games.prefix(visibleGameCount))
    }

    var body: some View {
        ScrollView {
            LazyVStack(spacing: 12) {
                Text(collection.title)
                    .font(.system(size: titleSize, weight: .bold, design: .rounded))
                    .multilineTextAlignment(.center)
                    .frame(maxWidth: .infinity)
                    .padding(.horizontal, 24)
                    .padding(.top, 30)
                    .padding(.bottom, 28)
                    .accessibilityAddTraits(.isHeader)

                GlassEffectContainer(spacing: 12) {
                    LazyVStack(spacing: 12) {
                        ForEach(visibleGames) { game in
                            NavigationLink {
                                GameDetailView(game: game)
                                    .toolbar(.visible, for: .navigationBar)
                            } label: {
                                LibraryListRow(
                                    game: game,
                                    kind: collection.kind
                                )
                            }
                            .buttonStyle(LibraryPressStyle())
                            .accessibilityHint("查看游戏详情")
                            .onAppear {
                                loadMoreIfNeeded(currentGame: game)
                            }
                        }
                    }
                }
                .padding(.horizontal, 16)
            }
            .padding(.bottom, 32)
        }
        .appThemeCanvas()
        .navigationTitle("")
        .navigationBarTitleDisplayMode(.inline)
        .toolbar(.visible, for: .navigationBar)
        .toolbarBackground(.hidden, for: .navigationBar)
        .toolbar(.hidden, for: .tabBar)
        .onAppear {
            IOSRuntimeTrace.event(
                domain: "library-ui",
                event: "collectionPageAppeared",
                payload: [
                    "collection": .string(collection.kind.rawValue),
                    "totalGames": .integer(Int64(collection.games.count)),
                    "visibleGames": .integer(Int64(visibleGames.count)),
                ],
                dimension: .frontend,
                importance: .key
            )
        }
        .onChange(of: collection.games.map(\.id)) { _, _ in
            visibleGameCount = min(
                LibraryLayoutMetrics.collectionPageSize,
                collection.games.count
            )
        }
    }

    private func loadMoreIfNeeded(currentGame: CloudLibraryGame) {
        guard currentGame.id == visibleGames.last?.id,
              visibleGameCount < collection.games.count
        else {
            return
        }
        visibleGameCount = min(
            collection.games.count,
            visibleGameCount + LibraryLayoutMetrics.collectionPageSize
        )
        IOSRuntimeTrace.snapshot(
            domain: "library-ui",
            event: "collectionPageExpanded",
            payload: [
                "collection": .string(collection.kind.rawValue),
                "visibleGames": .integer(Int64(visibleGameCount)),
                "totalGames": .integer(Int64(collection.games.count)),
                "pageSize": .integer(Int64(LibraryLayoutMetrics.collectionPageSize)),
            ],
            dimension: .frontend,
            importance: .debug
        )
    }
}

private struct LibraryHeroSlide: View {
    let game: CloudLibraryGame

    var body: some View {
        ZStack(alignment: .bottomLeading) {
            LibraryArtwork(game: game)

            LinearGradient(
                colors: [
                    .black.opacity(0.4),
                    .clear,
                    .black.opacity(0.14),
                    .black.opacity(0.9),
                    AppThemePalette.canvasTop.opacity(0.46),
                ],
                startPoint: .top,
                endPoint: .bottom
            )

            VStack(alignment: .leading, spacing: 8) {
                Text(game.name)
                    .font(.system(.title, design: .rounded, weight: .bold))
                    .foregroundStyle(.white)
                    .lineLimit(2)
                    .shadow(color: .black.opacity(0.35), radius: 5, y: 2)

                Text(recentMetadata)
                    .font(.subheadline.weight(.medium))
                    .foregroundStyle(.white.opacity(0.82))
                    .lineLimit(2)

                if let progress = game.achievementProgress {
                    HStack(spacing: 8) {
                        Label(
                            "\(progress.percentage)%",
                            systemImage: "chart.bar.fill"
                        )
                        HStack(spacing: 6) {
                            GamerscoreCoinIcon()
                            Text("\(progress.earnedGamerscore)")
                        }
                    }
                    .font(.caption.bold().monospacedDigit())
                    .foregroundStyle(.white.opacity(0.9))
                }
            }
            .padding(.horizontal, 20)
            .padding(.bottom, 86)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .contentShape(Rectangle())
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(accessibilityLabel)
        .accessibilityHint("查看游戏详情")
    }

    private var accessibilityLabel: String {
        var values = [
            game.name,
            recentMetadata,
        ]
        if let progress = game.achievementProgress {
            values.append("成就进度 \(progress.percentage)%")
            values.append("已获得 \(progress.earnedGamerscore) 点成就分数")
        }
        return values.joined(separator: "，")
    }

    private var recentMetadata: String {
        [
            LibraryPresentation.metadata(for: game, kind: .recent),
            XboxPresentation.playtime(game.playtimeMinutes),
        ].joined(separator: " · ")
    }
}

private struct LibraryHeroGlassTransition: View {
    var body: some View {
        Rectangle()
            .fill(AppThemePalette.canvasTop.opacity(0.035))
            .glassEffect(
                .regular.tint(AppThemePalette.canvasTop.opacity(0.08)),
                in: Rectangle()
            )
            .opacity(0.16)
            .mask {
                LinearGradient(
                    stops: [
                        .init(color: .clear, location: 0),
                        .init(color: .white.opacity(0.12), location: 0.24),
                        .init(color: .white.opacity(0.24), location: 0.58),
                        .init(color: .white.opacity(0.1), location: 0.82),
                        .init(color: .clear, location: 1),
                    ],
                    startPoint: .top,
                    endPoint: .bottom
                )
            }
            .allowsHitTesting(false)
            .accessibilityHidden(true)
    }
}

private struct LibraryPageIndicator: View {
    let games: [CloudLibraryGame]
    let selection: String

    var body: some View {
        HStack(spacing: 7) {
            ForEach(games) { game in
                Capsule()
                    .fill(
                        game.id == selection
                            ? Color.white
                            : Color.white.opacity(0.35)
                    )
                    .frame(
                        width: game.id == selection ? 18 : 6,
                        height: 6
                    )
            }
        }
        .padding(.horizontal, 11)
        .padding(.vertical, 8)
        .glassEffect(.regular, in: Capsule())
        .animation(.easeInOut(duration: 0.2), value: selection)
        .accessibilityHidden(true)
    }
}

private struct LibraryRecentCard: View {
    let game: CloudLibraryGame

    var body: some View {
        ZStack(alignment: .bottomLeading) {
            LibraryArtwork(game: game)

            LinearGradient(
                colors: [.clear, .black.opacity(0.84)],
                startPoint: .center,
                endPoint: .bottom
            )

            VStack(alignment: .leading, spacing: 3) {
                Text(game.name)
                    .font(.headline)
                    .foregroundStyle(.white)
                    .lineLimit(1)
                Text(recentMetadata)
                    .font(.caption)
                    .foregroundStyle(.white.opacity(0.78))
                    .lineLimit(1)
            }
            .padding(12)
        }
        .frame(
            width: LibraryLayoutMetrics.recentCardWidth,
            height: LibraryLayoutMetrics.recentCardHeight
        )
        .clipShape(RoundedRectangle(cornerRadius: 16, style: .continuous))
        .overlay {
            RoundedRectangle(cornerRadius: 16, style: .continuous)
                .stroke(.white.opacity(0.18), lineWidth: 0.5)
        }
        .shadow(color: .black.opacity(0.14), radius: 8, y: 4)
        .contentShape(RoundedRectangle(cornerRadius: 16, style: .continuous))
    }

    private var recentMetadata: String {
        [
            LibraryPresentation.metadata(for: game, kind: .recent),
            XboxPresentation.playtime(game.playtimeMinutes),
        ].joined(separator: " · ")
    }
}

private struct LibraryPosterCard: View {
    let game: CloudLibraryGame
    let kind: LibraryCollectionKind

    var body: some View {
        VStack(alignment: .leading, spacing: 7) {
            LibraryArtwork(game: game)
                .frame(
                    width: LibraryLayoutMetrics.posterWidth,
                    height: LibraryLayoutMetrics.posterHeight
                )
                .clipShape(
                    RoundedRectangle(
                        cornerRadius: LibraryLayoutMetrics.posterCornerRadius,
                        style: .continuous
                    )
                )
                .overlay {
                    RoundedRectangle(
                        cornerRadius: LibraryLayoutMetrics.posterCornerRadius,
                        style: .continuous
                    )
                        .stroke(.white.opacity(0.16), lineWidth: 0.5)
                }
                .shadow(color: .black.opacity(0.13), radius: 7, y: 3)

            Text(game.name)
                .font(.subheadline.weight(.semibold))
                .foregroundStyle(.primary)
                .lineLimit(2)
                .frame(maxWidth: .infinity, alignment: .leading)

            Text(LibraryPresentation.metadata(for: game, kind: kind))
                .font(.caption2)
                .foregroundStyle(.secondary)
                .lineLimit(1)
        }
        .frame(width: LibraryLayoutMetrics.posterWidth, alignment: .leading)
        .contentShape(Rectangle())
    }
}

private struct LibraryListRow: View {
    @Environment(\.dynamicTypeSize) private var dynamicTypeSize

    let game: CloudLibraryGame
    let kind: LibraryCollectionKind

    var body: some View {
        Group {
            if dynamicTypeSize.isAccessibilitySize {
                VStack(alignment: .leading, spacing: 14) {
                    artwork
                        .frame(maxWidth: .infinity)
                        .frame(height: 170)
                    details
                }
            } else {
                HStack(alignment: .center, spacing: 14) {
                    artwork
                        .frame(width: 128, height: 80)
                    details
                }
            }
        }
        .padding(14)
        .frame(maxWidth: .infinity, minHeight: 108, alignment: .leading)
        .overlay {
            RoundedRectangle(cornerRadius: 18, style: .continuous)
                .stroke(.separator.opacity(0.35), lineWidth: 0.5)
        }
        .glassEffect(
            .regular.interactive(),
            in: RoundedRectangle(cornerRadius: 18, style: .continuous)
        )
        .contentShape(RoundedRectangle(cornerRadius: 18, style: .continuous))
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(accessibilityLabel)
    }

    private var artwork: some View {
        LibraryArtwork(game: game)
            .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
            .overlay {
                RoundedRectangle(cornerRadius: 12, style: .continuous)
                    .stroke(.white.opacity(0.15), lineWidth: 0.5)
            }
    }

    private var details: some View {
        VStack(alignment: .leading, spacing: 7) {
            Text(game.name)
                .font(.headline)
                .foregroundStyle(.primary)
                .lineLimit(dynamicTypeSize.isAccessibilitySize ? nil : 2)
                .frame(maxWidth: .infinity, alignment: .leading)

            Text(LibraryPresentation.metadata(for: game, kind: kind))
                .font(.subheadline)
                .foregroundStyle(.secondary)
                .lineLimit(dynamicTypeSize.isAccessibilitySize ? nil : 2)
                .frame(maxWidth: .infinity, alignment: .leading)

        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private var accessibilityLabel: String {
        [
            game.name,
            LibraryPresentation.metadata(for: game, kind: kind),
        ].joined(separator: "，")
    }
}

private struct LibraryArtwork: View {
    @EnvironmentObject private var cloudStore: CloudLibraryStore

    let game: CloudLibraryGame

    var body: some View {
        CloudGameRemoteImage(
            candidates: game.imageCandidates(
                preferredURL: cloudStore.preferredImageURL(for: game)
            ),
            contentMode: .fill
        ) { url in
            cloudStore.recordSuccessfulImage(productID: game.productID, url: url)
        }
    }
}

private struct LibraryPressStyle: ButtonStyle {
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .scaleEffect(configuration.isPressed ? 0.975 : 1)
            .brightness(configuration.isPressed ? -0.04 : 0)
            .animation(
                reduceMotion ? nil : .snappy(duration: 0.2),
                value: configuration.isPressed
            )
    }
}
