import SwiftUI

struct GameLibraryView: View {
    @EnvironmentObject private var authStore: AuthStore
    @EnvironmentObject private var cloudStore: CloudLibraryStore
    @State private var didTraceAppearance = false
    let isActive: Bool

    init(isActive: Bool = true) {
        self.isActive = isActive
    }

    private var activationID: String {
        "\(authStore.ownerGeneration):\(isActive)"
    }

    private var collections: [LibraryCollection] {
        LibraryPresentation.collections(fromCloudGames: cloudStore.games)
    }

    var body: some View {
        NavigationStack {
            content
                .appThemeCanvas()
                .toolbar(.hidden, for: .navigationBar)
                .toolbarBackground(.hidden, for: .navigationBar)
        }
        .task(id: activationID) {
            guard isActive, authStore.phase == .signedIn else { return }
            await activateCloudLibrary()
        }
        .onAppear {
            guard !didTraceAppearance else {
                return
            }
            didTraceAppearance = true
            IOSRuntimeTrace.event(
                domain: "library-ui",
                event: "libraryPageAppeared",
                payload: [
                    "signedIn": .bool(authStore.isSignedIn),
                    "games": .integer(Int64(cloudStore.games.count)),
                ],
                dimension: .frontend,
                importance: .key
            )
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
        } else if cloudStore.games.isEmpty {
            emptyLibraryContent
        } else {
            libraryContent
        }
    }

    private var libraryContent: some View {
        GeometryReader { geometry in
            ScrollView {
                LazyVStack(spacing: 0) {
                    RecentlyPlayedHeroCarousel(
                        games: LibraryPresentation.heroGames(fromCloudGames: cloudStore.games),
                        height: LibraryLayoutMetrics.heroHeight(for: geometry.size.height)
                    )

                    if let errorMessage = cloudStore.errorMessage {
                        InlineDataErrorView(message: errorMessage)
                            .padding(.horizontal, 16)
                            .padding(.top, 8)
                            .padding(.bottom, 16)
                    }

                    ForEach(collections) { collection in
                        LibraryShelf(collection: collection)
                            .padding(.bottom, 30)
                    }
                }
                .padding(.bottom, 24)
            }
            .background(.clear)
            .ignoresSafeArea(edges: .top)
            .refreshable {
                await refreshCloudLibrary(reason: .pullToRefresh)
            }
        }
        .onAppear {
            IOSRuntimeTrace.state(
                domain: "library-ui",
                event: "contentPresented",
                payload: [
                    "games": .integer(Int64(cloudStore.games.count)),
                    "collections": .integer(Int64(collections.count)),
                ],
                dimension: .frontend,
                importance: .key
            )
        }
    }

    @ViewBuilder
    private var emptyLibraryContent: some View {
        switch cloudStore.phase {
        case .idle, .loading:
            GameLibraryLoadingView()
                .refreshable {
                    await refreshCloudLibrary(reason: .pullToRefresh)
                }
        case .failed:
            refreshableEmptyState {
                AppThemeEmptyState(
                    title: "无法载入游戏库",
                    systemImage: "exclamationmark.triangle",
                    description: cloudStore.errorMessage ?? "Xbox 服务暂时不可用",
                    actionTitle: "重新加载"
                ) {
                    Task {
                        await refreshCloudLibrary(reason: .manualRetry)
                    }
                }
            }
        case .loaded:
            refreshableEmptyState {
                AppThemeEmptyState(
                    title: "暂无游戏记录",
                    systemImage: "play.rectangle.on.rectangle"
                )
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
            .refreshable {
                await refreshCloudLibrary(reason: .pullToRefresh)
            }
        }
    }

    private func activateCloudLibrary() async {
        let operationID = UUID().uuidString
        IOSRuntimeTrace.event(
            domain: "library-ui",
            event: "libraryActivationRequested",
            payload: ["signedIn": .bool(authStore.isSignedIn)],
            dimension: .frontend,
            importance: .key,
            operationID: operationID
        )
        await cloudStore.activate(session: authStore.session) {
            try await authStore.prepareCloudAccess()
        }
        IOSRuntimeTrace.snapshot(
            domain: "library-ui",
            event: "libraryActivationCompleted",
            payload: [
                "phase": .string(String(describing: cloudStore.phase)),
                "cacheState": .string(cloudStore.cacheState.rawValue),
                "games": .integer(Int64(cloudStore.games.count)),
            ],
            dimension: .frontend,
            importance: .key,
            operationID: operationID
        )
    }

    private func refreshCloudLibrary(reason: CloudCatalogRefreshReason) async {
        let operationID = UUID().uuidString
        IOSRuntimeTrace.event(
            domain: "library-ui",
            event: "userRefreshRequested",
            payload: [
                "reason": .string(reason.rawValue),
                "existingGames": .integer(Int64(cloudStore.games.count)),
            ],
            dimension: .frontend,
            importance: .key,
            operationID: operationID
        )
        await cloudStore.refresh(reason: reason) {
            try await authStore.prepareCloudAccess()
        }
        IOSRuntimeTrace.snapshot(
            domain: "library-ui",
            event: "userRefreshCompleted",
            payload: [
                "reason": .string(reason.rawValue),
                "phase": .string(String(describing: cloudStore.phase)),
                "games": .integer(Int64(cloudStore.games.count)),
            ],
            dimension: .frontend,
            importance: .key,
            operationID: operationID
        )
    }

}

struct AppThemeEmptyState: View {
    let title: String
    let systemImage: String
    let description: String?
    let actionTitle: String?
    let action: (() -> Void)?

    init(
        title: String,
        systemImage: String,
        description: String? = nil,
        actionTitle: String? = nil,
        action: (() -> Void)? = nil
    ) {
        self.title = title
        self.systemImage = systemImage
        self.description = description
        self.actionTitle = actionTitle
        self.action = action
    }

    var body: some View {
        VStack(spacing: 20) {
            Image(systemName: systemImage)
                .font(.system(size: 38, weight: .medium))
                .symbolRenderingMode(.hierarchical)
                .foregroundStyle(.secondary)
                .accessibilityHidden(true)

            VStack(spacing: 8) {
                Text(title)
                    .font(.title3.weight(.bold))
                    .foregroundStyle(.primary)

                if let description {
                    Text(description)
                        .font(.subheadline)
                        .foregroundStyle(.secondary)
                        .multilineTextAlignment(.center)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            .accessibilityElement(children: .combine)

            if let actionTitle, let action {
                Button(action: action) {
                    Label(actionTitle, systemImage: "arrow.clockwise")
                }
                .buttonStyle(AppThemeGlassActionButtonStyle())
            }
        }
        .frame(maxWidth: 420)
        .padding(.horizontal, 32)
        .padding(.vertical, 24)
    }
}

private struct AppThemeGlassActionButtonStyle: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(.body.weight(.semibold))
            .foregroundStyle(.primary)
            .padding(.horizontal, 20)
            .frame(minHeight: 48)
            .contentShape(Capsule())
            .glassEffect(
                .regular
                    .tint(AppThemePalette.canvasTop.opacity(0.08))
                    .interactive(),
                in: Capsule()
            )
            .scaleEffect(configuration.isPressed ? 0.97 : 1)
            .opacity(configuration.isPressed ? 0.82 : 1)
            .animation(.easeOut(duration: 0.16), value: configuration.isPressed)
    }
}

private struct GameLibraryLoadingView: View {
    var body: some View {
        GeometryReader { geometry in
            ScrollView {
                VStack(spacing: 28) {
                    RoundedRectangle(cornerRadius: 0)
                        .fill(.quaternary)
                        .frame(height: LibraryLayoutMetrics.heroHeight(for: geometry.size.height))
                        .overlay(alignment: .bottomLeading) {
                            VStack(alignment: .leading, spacing: 10) {
                                placeholderLine(width: 210, height: 26)
                                placeholderLine(width: 168, height: 14)
                            }
                            .padding(20)
                            .padding(.bottom, 30)
                        }

                    ForEach(LibraryCollectionKind.allCases) { kind in
                        VStack(alignment: .leading, spacing: 12) {
                            placeholderLine(width: 132, height: 22)
                                .padding(.horizontal, 16)

                            ScrollView(.horizontal) {
                                HStack(spacing: 12) {
                                    ForEach(0..<3, id: \.self) { _ in
                                        RoundedRectangle(
                                            cornerRadius: LibraryLayoutMetrics.posterCornerRadius,
                                            style: .continuous
                                        )
                                        .fill(.quaternary)
                                        .frame(
                                            width: kind == .recent
                                                ? LibraryLayoutMetrics.recentCardWidth
                                                : LibraryLayoutMetrics.posterWidth,
                                            height: kind == .recent
                                                ? LibraryLayoutMetrics.recentCardHeight
                                                : LibraryLayoutMetrics.posterHeight
                                        )
                                    }
                                }
                            }
                            .contentMargins(.horizontal, 16, for: .scrollContent)
                            .scrollDisabled(true)
                            .scrollIndicators(.hidden)
                        }
                    }
                }
                .padding(.bottom, 24)
            }
            .ignoresSafeArea(edges: .top)
        }
        .skeletonPulse(accessibilityLabel: "正在载入游戏库")
        .onAppear {
            IOSRuntimeTrace.state(
                domain: "library-ui",
                event: "skeletonPresented",
                payload: [
                    "heroSections": 1,
                    "shelfSections": .integer(Int64(LibraryCollectionKind.allCases.count)),
                    "cardsPerShelf": 3,
                ],
                dimension: .frontend,
                importance: .key
            )
        }
    }

    private func placeholderLine(width: CGFloat, height: CGFloat) -> some View {
        Capsule()
            .fill(.quaternary)
            .frame(width: width, height: height)
    }
}
