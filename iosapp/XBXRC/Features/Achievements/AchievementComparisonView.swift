import SwiftUI

private enum AchievementStateFilter: String, CaseIterable, Identifiable {
  case all, locked, unlocked
  var id: Self { self }
  var title: String {
    switch self {
    case .all: "全部"
    case .locked: "待解锁"
    case .unlocked: "已解锁"
    }
  }
}

struct AchievementComparisonView: View {
  let game: GameSummary
  var targetID: String? = nil
  @EnvironmentObject private var auth: AuthStore
  @EnvironmentObject private var data: XboxDataStore
  @EnvironmentObject private var settings: AppSettingsStore
  @Environment(\.appPageIsActive) private var isActive
  @Environment(\.locale) private var appLocale
  @State private var search = ""
  @State private var filter: AchievementStateFilter = .all
  @State private var revealed: Set<String> = []
  @State private var selected: AchievementComparisonRow?

  private var locale: String {
    PreferredGameLocaleNormalizer.normalize(settings.preferredGameLocale)
  }
  private var own: XboxNativeDataState<AchievementSummary> { state(target: nil) }
  private var friend: XboxNativeDataState<AchievementSummary> {
    targetID.map { state(target: $0) } ?? XboxNativeDataState()
  }
  private var rows: [AchievementComparisonRow] {
    PersonalLibraryPresentation.achievementRows(own: own.items, friend: friend.items).filter {
      row in
      guard
        search.isEmpty
          || (isSecret(row) ? "秘密成就" : row.item.name).localizedCaseInsensitiveContains(
            search)
      else { return false }
      switch filter {
      case .all: return true
      case .locked: return row.own?.isUnlocked == false
      case .unlocked: return row.own?.isUnlocked == true
      }
    }
  }

  var body: some View {
    AppInformationPage {
      Section {
        AppContentSearch(text: $search, prompt: "搜索成就")
        AppSegmentedFilter(
          selection: $filter, values: AchievementStateFilter.allCases,
          title: { $0.title },
          identifier: "achievement-state-filter", label: "我的成就状态")
        if targetID != nil {
          Text("按同一成就对比双方的解锁状态与进度。")
            .appFont(.caption).foregroundStyle(AppThemePalette.textSecondary)
        }
      }
      sourceStatus(own, title: "我的")
      if targetID != nil { sourceStatus(friend, title: "好友") }
      Section {
        ForEach(rows) { row in
          Button {
            selected = row
          } label: {
            achievementRow(row)
          }
          .buttonStyle(.plain)
          .accessibilityIdentifier("achievement-\(row.id)")
        }
        if rows.isEmpty, own.phase == .loaded || friend.phase == .loaded {
          Text(
            LocalizedStringKey(
              search.isEmpty && filter == .all ? "暂无成就记录" : "当前筛选结果为空")
          )
          .foregroundStyle(AppThemePalette.textSecondary)
        }
      } header: {
        HStack {
          Text(targetID == nil ? "游戏成就" : "成就对比")
          Spacer()
          Text("\(rows.count)").appFont(.caption).foregroundStyle(
            AppThemePalette.textSecondary)
        }
      }
      ForEach(targetID == nil ? ["我的"] : ["我的", "好友"], id: \.self) { title in
        let source = title == "我的" ? own : friend
        if source.requestAttempt == .nextPage, source.failure != nil {
          AppSectionRequestState(
            title: title == "我的" ? "我的更多成就" : "玩家更多成就", status: source.requestStatus,
            identifier: "achievements-page-\(title)"
          ) {
            await data.loadPlayerAchievements(
              titleID: game.titleID, targetID: title == "我的" ? nil : targetID,
              locale: locale,
              refresh: source.failure?.kind == .repeatedContinuation,
              nextPage: source.failure?.kind != .repeatedContinuation)
          }
        }
      }
      if own.continuation != nil || friend.continuation != nil {
        AppLoadMoreButton(title: "载入更多成就", isLoading: own.isRefreshing || friend.isRefreshing) {
          await load(nextPage: true)
        }
      }
    }
    .appThemeCanvas()
    .task(
      id:
        "\(auth.ownerGeneration):\(game.titleID):\(targetID ?? "self"):\(locale):\(isActive)"
    ) {
      guard isActive else { return }
      await load()
    }
    .refreshable { await load(refresh: true) }
    .onChange(of: auth.ownerGeneration) { _, _ in
      selected = nil
      revealed = []
    }
    .sheet(item: $selected) { row in
      NavigationStack {
        achievementDetail(row)
          .appPageToolbar(isSecret(row) ? "秘密成就" : row.item.name)
      }
      .presentationDetents([.medium, .large]).presentationDragIndicator(.visible)
    }
  }

  @ViewBuilder
  private func sourceStatus(_ source: XboxNativeDataState<AchievementSummary>, title: String)
    -> some View
  {
    if source.requestAttempt != .nextPage {
      AppSectionRequestState(
        title: title == "我的" ? "我的成就" : "玩家成就", status: source.requestStatus,
        identifier: "achievements-\(title)"
      ) {
        await data.loadPlayerAchievements(
          titleID: game.titleID, targetID: title == "我的" ? nil : targetID, locale: locale,
          refresh: true)
      }
    }
  }

  private func achievementRow(_ row: AchievementComparisonRow) -> some View {
    VStack(alignment: .leading, spacing: 12) {
      HStack(alignment: .top, spacing: 12) {
        if isSecret(row) {
          Image(systemName: "lock.fill").foregroundStyle(AppThemePalette.textSecondary)
            .frame(width: 52, height: 52).appSurface(radius: 8)
        } else {
          PersonalArtwork(url: row.item.imageURL, size: 52)
        }
        VStack(alignment: .leading, spacing: 5) {
          Text(LocalizedStringKey(isSecret(row) ? "秘密成就" : row.item.name))
            .appFont(.body, weight: .semibold).foregroundStyle(.primary)
          Text(LocalizedStringKey(isSecret(row) ? "打开详情可主动揭示" : description(row.item)))
            .appFont(.caption).foregroundStyle(AppThemePalette.textSecondary).lineLimit(
              3)
          Text("\(row.item.gamerscore) G").appFont(.caption).foregroundStyle(
            AppThemePalette.brand)
        }
        Spacer(minLength: 0)
        AppIconView(.chevron, size: 16).foregroundStyle(AppThemePalette.textSecondary)
      }
      ViewThatFits(in: .horizontal) {
        HStack(spacing: 24) {
          status(row.own, title: "我的")
          if targetID != nil { status(row.friend, title: "好友") }
        }
        VStack(alignment: .leading, spacing: 8) {
          status(row.own, title: "我的")
          if targetID != nil { status(row.friend, title: "好友") }
        }
      }
    }
    .padding(12).frame(maxWidth: .infinity, alignment: .leading)
    .appSurface(radius: AppLayoutPolicy.cardRadius).contentShape(
      Rectangle())
  }

  private func status(_ achievement: AchievementSummary?, title: String) -> some View {
    HStack(spacing: 8) {
      Text(LocalizedStringKey(title)).foregroundStyle(AppThemePalette.textSecondary)
      Text(statusText(achievement)).foregroundStyle(
        achievement?.isUnlocked == true
          ? AppThemePalette.brand : AppThemePalette.textPrimary)
    }.appFont(.caption)
  }

  private func achievementDetail(_ row: AchievementComparisonRow) -> some View {
    AppInformationPage {
      Section {
        if isSecret(row) {
          Text("这是一项秘密成就，揭示后可查看名称、说明与完成条件。")
            .appFont(.body).foregroundStyle(AppThemePalette.textSecondary)
          Button("揭示秘密成就") { revealed.insert(row.id) }
            .accessibilityIdentifier("achievement-reveal-secret")
        } else {
          HStack(spacing: 16) {
            PersonalArtwork(url: row.item.imageURL, size: 72)
            VStack(alignment: .leading, spacing: 8) {
              Text(row.item.name).appFont(.section, weight: .semibold)
              Text("\(row.item.gamerscore) G").appFont(.bodySmall).foregroundStyle(
                AppThemePalette.brand)
            }
          }
          Text(description(row.item)).appFont(.body).fixedSize(
            horizontal: false, vertical: true)
        }
      }
      detailStatus(row.own, title: "我的", showRequirements: !isSecret(row))
      if targetID != nil {
        detailStatus(row.friend, title: "好友", showRequirements: !isSecret(row))
      }
    }.appThemeCanvas()
  }

  @ViewBuilder
  private func detailStatus(
    _ achievement: AchievementSummary?, title: String, showRequirements: Bool
  ) -> some View {
    Section {
      LabeledContent("状态", value: statusText(achievement))
      if let date = achievement?.unlockedAt {
        LabeledContent("解锁时间", value: date.formatted(date: .abbreviated, time: .shortened))
      }
      if showRequirements, let achievement {
        ForEach(Array(achievement.requirements.enumerated()), id: \.offset) {
          index, requirement in
          LabeledContent(
            "条件 \(index + 1)",
            value: "\(requirement.current ?? "—") / \(requirement.target ?? "—")"
          )
          .accessibilityElement(children: .ignore)
          .accessibilityLabel("条件 \(index + 1)")
          .accessibilityValue(
            "\(requirement.current ?? "—") / \(requirement.target ?? "—")"
          )
          .accessibilityIdentifier("achievement-requirement-\(index)")
        }
      }
    } header: {
      Text(LocalizedStringKey(title))
    }
  }

  private func isSecret(_ row: AchievementComparisonRow) -> Bool {
    row.item.isSecret && row.own?.isUnlocked != true && !revealed.contains(row.id)
  }
  private func description(_ item: AchievementSummary) -> String {
    item.isUnlocked
      ? item.description
      : item.lockedDescription.flatMap { $0.isEmpty ? nil : $0 } ?? item.description
  }
  private func statusText(_ achievement: AchievementSummary?) -> String {
    guard let achievement else { return appLocalizedString("状态待载入", locale: appLocale) }
    if achievement.isUnlocked { return appLocalizedString("已解锁", locale: appLocale) }
    return achievement.progressPercentage.map {
      appLocalizedFormat("待解锁 · %lld%%", locale: appLocale, $0)
    } ?? appLocalizedString("待解锁", locale: appLocale)
  }
  private func state(target: String?) -> XboxNativeDataState<AchievementSummary> {
    data.playerAchievementStates[
      XboxPersonalQueryKey.achievements(game.titleID, targetID: target, locale: locale)]
      ?? XboxNativeDataState()
  }
  private func load(refresh: Bool = false, nextPage: Bool = false) async {
    async let mine: Void = data.loadPlayerAchievements(
      titleID: game.titleID, locale: locale, refresh: refresh, nextPage: nextPage)
    if let targetID {
      async let theirs: Void = data.loadPlayerAchievements(
        titleID: game.titleID, targetID: targetID, locale: locale, refresh: refresh,
        nextPage: nextPage)
      _ = await (mine, theirs)
    } else {
      await mine
    }
  }
}
