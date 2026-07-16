import SwiftUI

struct GameDetailView: View {
    @EnvironmentObject private var cloudStore: CloudLibraryStore

    let game: CloudLibraryGame
    var onPlay: ((String) -> Void)?

    @State private var selectedCardID: GameDetailDataCard.ID?
    @State private var playAlertPresented = false

    init(
        game: CloudLibraryGame,
        onPlay: ((String) -> Void)? = nil
    ) {
        self.game = game
        self.onPlay = onPlay
    }

    init(
        gameSummary: GameSummary,
        onPlay: ((String) -> Void)? = nil
    ) {
        game = CloudLibraryGame(gameSummary: gameSummary)
        self.onPlay = onPlay
    }

    var body: some View {
        GeometryReader { geometry in
            ZStack {
                GameDetailBackdrop(
                    candidates: detailImageCandidates,
                    onSuccess: recordSuccessfulImage
                )

                ScrollView {
                    LazyVStack(spacing: 0) {
                        hero(height: GameDetailLayoutMetrics.heroHeight(for: geometry.size.height))

                        detailContent
                            .padding(.top, 22)
                    }
                    .padding(.bottom, 32)
                }
                .scrollIndicators(.hidden)
                .ignoresSafeArea(edges: .top)
            }
        }
        .safeAreaInset(edge: .bottom, spacing: 0) {
            playBar
        }
        .navigationTitle("")
        .navigationBarTitleDisplayMode(.inline)
        .toolbar(.visible, for: .navigationBar)
        .toolbarBackground(.hidden, for: .navigationBar)
        .toolbar(.hidden, for: .tabBar)
        .alert("串流运行时接入中", isPresented: $playAlertPresented) {
            Button("知道了", role: .cancel) {}
        } message: {
            Text("游戏启动身份已经准备完成，媒体串流将在 StreamingRuntime 阶段接入。")
        }
        .accessibilityElement(children: .contain)
        .onAppear {
            IOSRuntimeTrace.event(
                domain: "library-ui",
                event: "gameDetailPresented",
                payload: [
                    "dataCards": .integer(Int64(dataCards.count)),
                    "imageCandidates": .integer(Int64(detailImageCandidates.count)),
                    "canPlay": .bool(canPlay),
                    "entitlement": .string(entitlementState),
                    "streamIdentity": .string(
                        normalizedStreamTitleID == nil ? "missing" : "ready"
                    ),
                ],
                dimension: .frontend,
                importance: .key
            )
        }
    }

    private func hero(height: CGFloat) -> some View {
        ZStack(alignment: .bottomLeading) {
            CloudGameRemoteImage(
                candidates: detailImageCandidates,
                contentMode: .fill,
                onSuccess: recordSuccessfulImage
            )
            .frame(height: height)

            LinearGradient(
                colors: [
                    .black.opacity(0.34),
                    .clear,
                    .black.opacity(0.2),
                    .black.opacity(0.9),
                    AppThemePalette.canvasTop.opacity(0.92),
                ],
                startPoint: .top,
                endPoint: .bottom
            )

            VStack(alignment: .leading, spacing: 10) {
                if game.isNew == true || game.isRecentlyPlayed == true {
                    HStack(spacing: 8) {
                        if game.isNew == true {
                            GameDetailBadge(title: "新入库", systemImage: "sparkles")
                        }
                        if game.isRecentlyPlayed == true {
                            GameDetailBadge(title: "最近游玩", systemImage: "clock.arrow.circlepath")
                        }
                    }
                }

                Text(game.name)
                    .font(.system(.largeTitle, design: .rounded, weight: .bold))
                    .foregroundStyle(.white)
                    .lineLimit(3)
                    .minimumScaleFactor(0.76)
                    .shadow(color: .black.opacity(0.36), radius: 8, y: 3)

                if !game.publisherName.isEmpty {
                    Text(game.publisherName)
                        .font(.headline)
                        .foregroundStyle(.white.opacity(0.82))
                        .lineLimit(1)
                }

                if !game.categories.isEmpty {
                    Text(game.categories.prefix(3).joined(separator: " · "))
                        .font(.subheadline.weight(.medium))
                        .foregroundStyle(.white.opacity(0.72))
                        .lineLimit(2)
                }
            }
            .padding(.horizontal, GameDetailLayoutMetrics.horizontalInset)
            .padding(.bottom, 34)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .frame(height: height)
        .clipped()
        .accessibilityElement(children: .combine)
        .accessibilityLabel(heroAccessibilityLabel)
    }

    private var detailContent: some View {
        VStack(alignment: .leading, spacing: 26) {
            VStack(alignment: .leading, spacing: 5) {
                Text("游戏数据")
                    .font(.title2.bold())
                Text("左右滑动查看游玩、成就与云游戏信息")
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
            }
            .padding(.horizontal, GameDetailLayoutMetrics.horizontalInset)

            CircularCardCarousel(
                items: dataCards,
                selection: $selectedCardID,
                metrics: .gameDetail
            ) { card in
                GameDetailMetricCard(card: card)
            }
            .frame(height: CircularCardCarouselMetrics.gameDetail.cardHeight + 16)

            if !game.description.isEmpty {
                VStack(alignment: .leading, spacing: 10) {
                    Text("关于游戏")
                        .font(.headline)
                    Text(game.description)
                        .font(.body)
                        .foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
                .padding(18)
                .frame(maxWidth: .infinity, alignment: .leading)
                .overlay {
                    RoundedRectangle(cornerRadius: 18, style: .continuous)
                        .stroke(.separator.opacity(0.32), lineWidth: 0.5)
                }
                .glassEffect(
                    .regular.tint(AppThemePalette.canvasTop.opacity(0.08)),
                    in: RoundedRectangle(cornerRadius: 18, style: .continuous)
                )
                .padding(.horizontal, GameDetailLayoutMetrics.horizontalInset)
            }
        }
    }

    private var playBar: some View {
        VStack(spacing: 7) {
            Button(action: handlePlay) {
                HStack(spacing: 10) {
                    Image(systemName: "play.fill")
                    Text("游玩")
                        .fontWeight(.bold)
                    Spacer(minLength: 12)
                    Image(systemName: "cloud.fill")
                        .foregroundStyle(.white.opacity(0.72))
                }
                .frame(maxWidth: .infinity)
                .padding(.horizontal, 18)
                .frame(minHeight: 54)
            }
            .buttonStyle(.borderedProminent)
            .buttonBorderShape(.roundedRectangle(radius: 16))
            .tint(AppThemePalette.brand)
            .disabled(!canPlay)
            .accessibilityLabel("游玩 \(game.name)")
            .accessibilityHint(playAvailabilityText)

            Text(playAvailabilityText)
                .font(.caption)
                .foregroundStyle(.secondary)
                .lineLimit(2)
                .multilineTextAlignment(.center)
        }
        .padding(.horizontal, GameDetailLayoutMetrics.horizontalInset)
        .padding(.top, 12)
        .padding(.bottom, 8)
        .background(.ultraThinMaterial)
        .overlay(alignment: .top) {
            Divider().opacity(0.42)
        }
    }

    private var dataCards: [GameDetailDataCard] {
        var cards: [GameDetailDataCard] = []

        if let progress = game.achievementProgress {
            cards.append(
                GameDetailDataCard(
                    id: "achievements",
                    eyebrow: "成就进度",
                    value: "\(progress.percentage)%",
                    caption: "\(progress.unlockedCount)/\(progress.totalCount) 项 · \(progress.earnedGamerscore)/\(progress.totalGamerscore) G",
                    systemImage: "trophy.fill",
                    progress: Double(progress.percentage) / 100
                )
            )
        }

        cards.append(
            GameDetailDataCard(
                id: "activity",
                eyebrow: "游玩记录",
                value: XboxPresentation.playtime(game.playtimeMinutes),
                caption: lastPlayedText,
                systemImage: "clock.fill"
            )
        )

        cards.append(
            GameDetailDataCard(
                id: "input",
                eyebrow: "输入支持",
                value: inputSupportTitle,
                caption: inputSupportCaption,
                systemImage: "gamecontroller.fill"
            )
        )

        cards.append(
            GameDetailDataCard(
                id: "catalog",
                eyebrow: "游戏分类",
                value: categoryTitle,
                caption: game.publisherName.isEmpty ? "Xbox Cloud Gaming" : game.publisherName,
                systemImage: "square.grid.2x2.fill"
            )
        )

        cards.append(
            GameDetailDataCard(
                id: "cloud",
                eyebrow: "云游戏",
                value: cloudAccessTitle,
                caption: cloudAccessCaption,
                systemImage: "cloud.fill"
            )
        )

        return cards
    }

    private var normalizedStreamTitleID: String? {
        guard let value = game.streamTitleID?.trimmingCharacters(in: .whitespacesAndNewlines),
              !value.isEmpty
        else {
            return nil
        }
        return value
    }

    private var detailImageCandidates: [URL] {
        game.imageCandidates(preferredURL: cloudStore.preferredImageURL(for: game))
    }

    private var canPlay: Bool {
        normalizedStreamTitleID != nil && game.hasEntitlement != false
    }

    private var entitlementState: String {
        switch game.hasEntitlement {
        case true:
            "granted"
        case false:
            "denied"
        case nil:
            "unknown"
        }
    }

    private var playAvailabilityText: String {
        if game.hasEntitlement == false {
            return "当前账号需要对应的云游戏访问权限"
        }
        if normalizedStreamTitleID == nil {
            return "云游戏启动标识同步中"
        }
        if onPlay == nil {
            return "启动入口已就绪 · 串流运行时接入中"
        }
        return "通过 Xbox Cloud Gaming 启动"
    }

    private var cloudAccessTitle: String {
        if game.hasEntitlement == false {
            return "需要权限"
        }
        return normalizedStreamTitleID == nil ? "同步中" : "可游玩"
    }

    private var cloudAccessCaption: String {
        if game.isNew == true {
            return "Game Pass 新入库"
        }
        if game.isRecentlyPlayed == true {
            return "最近使用过云游戏"
        }
        return normalizedStreamTitleID == nil ? "等待目录补充启动身份" : "云端目录已就绪"
    }

    private var lastPlayedText: String {
        guard let lastPlayedAt = game.lastPlayedAt else {
            return "最近游玩时间同步中"
        }
        return "上次游玩 · \(lastPlayedAt.formatted(.relative(presentation: .named, unitsStyle: .wide)))"
    }

    private var inputSupportTitle: String {
        guard let first = game.supportedInputTypes.first else {
            return "信息同步中"
        }
        return localizedInputType(first)
    }

    private var inputSupportCaption: String {
        guard !game.supportedInputTypes.isEmpty else {
            return "目录更新后显示支持的控制方式"
        }
        return game.supportedInputTypes
            .prefix(3)
            .map(localizedInputType)
            .joined(separator: " · ")
    }

    private var categoryTitle: String {
        game.categories.first ?? "Xbox 游戏"
    }

    private var heroAccessibilityLabel: String {
        [game.name, game.publisherName, game.categories.prefix(3).joined(separator: "，")]
            .filter { !$0.isEmpty }
            .joined(separator: "，")
    }

    private func localizedInputType(_ value: String) -> String {
        switch value.lowercased() {
        case "controller", "gamepad":
            return "手柄"
        case "mouselkeyboard", "mousekeyboard", "mouseandkeyboard":
            return "键盘鼠标"
        case "touch":
            return "触控"
        default:
            return value
        }
    }

    private func handlePlay() {
        guard let streamTitleID = normalizedStreamTitleID else {
            IOSRuntimeTrace.decision(
                domain: "library-ui",
                event: "playUnavailable",
                payload: ["reason": "missingStreamIdentity"],
                dimension: .frontend,
                importance: .key
            )
            return
        }
        guard game.hasEntitlement != false else {
            IOSRuntimeTrace.decision(
                domain: "library-ui",
                event: "playUnavailable",
                payload: ["reason": "missingEntitlement"],
                dimension: .frontend,
                importance: .key
            )
            return
        }
        guard let onPlay else {
            IOSRuntimeTrace.decision(
                domain: "library-ui",
                event: "playUnavailable",
                payload: ["reason": "streamingRuntimePending"],
                dimension: .frontend,
                importance: .key
            )
            playAlertPresented = true
            return
        }
        IOSRuntimeTrace.event(
            domain: "library-ui",
            event: "playRequested",
            payload: [
                "entitlement": .string(entitlementState),
                "streamIdentity": "ready",
            ],
            dimension: .frontend,
            importance: .essential
        )
        onPlay(streamTitleID)
    }

    private func recordSuccessfulImage(_ url: URL) {
        cloudStore.recordSuccessfulImage(productID: game.productID, url: url)
    }
}

private enum GameDetailLayoutMetrics {
    static let horizontalInset: CGFloat = 16
    static let heroMinimumHeight: CGFloat = 470
    static let heroMaximumHeight: CGFloat = 620
    static let heroViewportFraction: CGFloat = 0.64

    static func heroHeight(for viewportHeight: CGFloat) -> CGFloat {
        min(heroMaximumHeight, max(heroMinimumHeight, viewportHeight * heroViewportFraction))
    }
}

private struct GameDetailDataCard: Identifiable, Hashable {
    let id: String
    let eyebrow: String
    let value: String
    let caption: String
    let systemImage: String
    var progress: Double?
}

private struct GameDetailMetricCard: View {
    let card: GameDetailDataCard

    var body: some View {
        VStack(alignment: .leading, spacing: 13) {
            HStack {
                Image(systemName: card.systemImage)
                    .font(.title3.bold())
                    .foregroundStyle(AppThemePalette.brand)
                    .frame(width: 36, height: 36)
                    .background(AppThemePalette.brand.opacity(0.14), in: Circle())

                Spacer()

                Text(card.eyebrow)
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(.secondary)
                    .textCase(.uppercase)
            }

            Text(card.value)
                .font(.system(.title2, design: .rounded, weight: .bold))
                .lineLimit(2)
                .minimumScaleFactor(0.72)

            Text(card.caption)
                .font(.subheadline)
                .foregroundStyle(.secondary)
                .lineLimit(2)
                .frame(maxWidth: .infinity, alignment: .leading)

            if let progress = card.progress {
                ProgressView(value: progress)
                    .tint(AppThemePalette.brand)
            } else {
                Spacer(minLength: 4)
            }
        }
        .padding(18)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .overlay {
            RoundedRectangle(cornerRadius: 18, style: .continuous)
                .stroke(.separator.opacity(0.35), lineWidth: 0.5)
        }
        .glassEffect(
            .regular
                .tint(AppThemePalette.canvasTop.opacity(0.08))
                .interactive(),
            in: RoundedRectangle(cornerRadius: 18, style: .continuous)
        )
        .contentShape(RoundedRectangle(cornerRadius: 18, style: .continuous))
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("\(card.eyebrow)，\(card.value)，\(card.caption)")
    }
}

private struct GameDetailBadge: View {
    let title: String
    let systemImage: String

    var body: some View {
        Label(title, systemImage: systemImage)
            .font(.caption.bold())
            .foregroundStyle(.white)
            .padding(.horizontal, 10)
            .padding(.vertical, 7)
            .glassEffect(.regular.tint(.black.opacity(0.24)), in: Capsule())
    }
}

private struct GameDetailBackdrop: View {
    let candidates: [URL]
    let onSuccess: (URL) -> Void

    var body: some View {
        GeometryReader { geometry in
            CloudGameRemoteImage(
                candidates: candidates,
                contentMode: .fill,
                onSuccess: onSuccess
            )
                .frame(width: geometry.size.width, height: geometry.size.height)
                .scaleEffect(1.12)
                .blur(radius: 24)
                .overlay {
                    LinearGradient(
                        colors: [
                            .black.opacity(0.34),
                            AppThemePalette.canvasTop.opacity(0.88),
                            AppThemePalette.canvas,
                        ],
                        startPoint: .top,
                        endPoint: .bottom
                    )
                }
        }
        .ignoresSafeArea()
        .allowsHitTesting(false)
        .accessibilityHidden(true)
    }
}
