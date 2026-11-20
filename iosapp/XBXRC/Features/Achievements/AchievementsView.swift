import SwiftUI

struct AchievementsView: View {
  @EnvironmentObject private var authStore: AuthStore
  @EnvironmentObject private var dataStore: XboxDataStore
  @State private var featuredGameID: String?
  let isActive: Bool
  let embedded: Bool

  init(isActive: Bool = true, embedded: Bool = false) {
    self.isActive = isActive
    self.embedded = embedded
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
    Group {
      if embedded {
        content.appThemeCanvas()
      } else {
        NavigationStack { content.appThemeCanvas() }
      }
    }
    .navigationTitle("成就")
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
      AppPageScroll {
        LazyVStack(spacing: 0) {
          AppAccountRequestNotice()
          libraryRequestState.padding(.horizontal, AppLayoutPolicy.pageInset)
            .padding(.bottom, AppLayoutPolicy.pageInset)

          HStack {
            Text("全部游戏")
              .appFont(.section, weight: .semibold)
            Spacer()
            Text("\(games.count)")
              .appFont(.caption, weight: .regular).monospacedDigit()
              .foregroundStyle(AppThemePalette.textSecondary)
          }
          .padding(.horizontal, AppLayoutPolicy.pageInset)
          .padding(.bottom, 12)

          Group {
            LazyVStack(spacing: 12) {
              ForEach(games) { game in
                NavigationLink {
                  GameAchievementsView(game: game)
                } label: {
                  AchievementGameCard(
                    game: game
                  )
                }
                .buttonStyle(AppPressStyle())
              }
            }
          }
          .padding(.horizontal, 14)
        }
        .padding(.bottom, 24)
        .frame(maxWidth: 960).frame(maxWidth: .infinity)
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
      refreshableLibraryEmptyState {
        ProgressView("正在载入成就列表")
      }
    case .failed:
      refreshableLibraryEmptyState { libraryRequestState.padding(AppLayoutPolicy.pageInset) }
    case .loaded:
      refreshableLibraryEmptyState {
        VStack(spacing: 16) {
          libraryRequestState
          AppThemeEmptyState(
            title: "没有成就记录",
            systemImage: "trophy"
          )
        }
      }
    }
  }

  private var libraryRequestState: some View {
    AppSectionRequestState(
      title: "游玩记录", status: dataStore.historyState(targetID: nil).requestStatus,
      identifier: "achievement-library"
    ) { await dataStore.refreshLibrary(reason: .manualRetry) }
  }

  private func refreshableLibraryEmptyState<Content: View>(
    @ViewBuilder content: @escaping () -> Content
  ) -> some View {
    GeometryReader { geometry in
      AppPageScroll {
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
    game.achievementProgress
      ?? AchievementProgress(
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
          .appFont(.title, weight: .bold)
          .foregroundStyle(.white)
          .lineLimit(2)

        Label(XboxPresentation.playtime(game.playtimeMinutes), systemImage: "clock")
          .appFont(.bodySmall, weight: .regular)
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
        .appFont(.caption, weight: .bold).monospacedDigit()
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
        .appFont(.hero, weight: .regular)
        .foregroundStyle(AppThemePalette.textSecondary)
    }
  }
}

private struct AppPressStyle: ButtonStyle {
  func makeBody(configuration: Configuration) -> some View {
    configuration.label
      .scaleEffect(configuration.isPressed ? 0.975 : 1)
      .brightness(configuration.isPressed ? -0.04 : 0)
      .animation(.snappy(duration: 0.22), value: configuration.isPressed)
  }
}

private struct AchievementGameCard: View {
  let game: GameSummary

  private var progress: AchievementProgress {
    game.achievementProgress
      ?? AchievementProgress(
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
          .appFont(.body, weight: .semibold)
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
      RoundedRectangle(cornerRadius: 8, style: .continuous)
        .stroke(.separator.opacity(0.35), lineWidth: 0.5)
    }
    .appSurface(radius: 8)

    .contentShape(RoundedRectangle(cornerRadius: 8, style: .continuous))
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
    case .system(let name):
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
          Rectangle().fill(AppThemePalette.panel.opacity(0.42))
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
        .foregroundStyle(AppThemePalette.textSecondary)
    }
  }
}

struct InlineDataErrorView: View {
  let message: String

  var body: some View {
    VStack(alignment: .leading, spacing: 8) {
      Label("暂时无法获取内容，请重试。", systemImage: "exclamationmark.triangle")
        .appFont(.body).foregroundStyle(AppThemePalette.textPrimary)
      DisclosureGroup("查看错误详情") {
        Text(verbatim: message).appFont(.caption)
          .foregroundStyle(AppThemePalette.textSecondary)
          .textSelection(.enabled)
      }
      .appFont(.caption).tint(AppThemePalette.brand).frame(minHeight: 44)
    }
    .frame(maxWidth: .infinity, alignment: .leading)
    .padding(12).appSurface(radius: 8)
  }
}

enum XboxPresentation {
  static func playtime(_ minutes: Int?, locale: Locale? = nil) -> String {
    if let locale {
      guard let minutes else { return appLocalizedString("时长未知", locale: locale) }
      if minutes < 60 {
        return appLocalizedFormat(
          "%@ 分钟", locale: locale, minutes.formatted(.number.locale(locale)))
      }
      let hours = Double(minutes) / 60
      let value =
        hours < 10
        ? hours.formatted(.number.locale(locale).precision(.fractionLength(1)))
        : Int(hours.rounded()).formatted(.number.locale(locale))
      return appLocalizedFormat("%@ 小时", locale: locale, value)
    }
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
  let game: GameSummary
  var body: some View { PersonalGameDetailView(game: game, section: .achievements) }
}
