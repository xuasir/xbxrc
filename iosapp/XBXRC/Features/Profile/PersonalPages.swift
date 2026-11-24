import SwiftUI

private enum FriendFilter: String, CaseIterable, Identifiable {
  case all, online, playing
  var id: Self { self }
  var title: String {
    switch self {
    case .all: "全部"
    case .online: "在线"
    case .playing: "游戏中"
    }
  }
}

struct FriendsView: View {
  @Environment(\.locale) private var locale
  @EnvironmentObject private var auth: AuthStore
  @EnvironmentObject private var data: XboxDataStore
  @Environment(\.appPageIsActive) private var isActive
  @State private var search = ""
  @State private var filter: FriendFilter = .all
  private var friends: [XboxFriendSummary] {
    data.friendsState.items.filter { friend in
      let matches =
        search.isEmpty || friend.gamertag.localizedCaseInsensitiveContains(search)
        || friend.displayName.localizedCaseInsensitiveContains(search)
      let online = friend.presenceState?.lowercased() == "online"
      return matches
        && (filter == .all || online && (filter == .online || friend.currentTitleID != nil))
    }.sorted { lhs, rhs in
      let leftOnline = lhs.presenceState?.lowercased() == "online"
      let rightOnline = rhs.presenceState?.lowercased() == "online"
      if leftOnline != rightOnline { return leftOnline }
      if (lhs.currentTitleID != nil) != (rhs.currentTitleID != nil) {
        return lhs.currentTitleID != nil
      }
      return lhs.gamertag.localizedStandardCompare(rhs.gamertag) == .orderedAscending
    }
  }

  var body: some View {
    AppInformationPage {
      Section {
        AppContentSearch(text: $search, prompt: "搜索好友")
        AppSegmentedFilter(
          selection: $filter, values: FriendFilter.allCases, title: { $0.title },
          identifier: "friends-state-filter", label: "好友状态")
      }

      AppSectionRequestState(
        title: "好友列表", status: data.friendsState.requestStatus, identifier: "friends"
      ) { await data.loadFriends(refresh: true) }
      Section {
        ForEach(friends) { friend in
          NavigationLink {
            FriendDetailView(friend: friend)
          } label: {
            PersonalProfileRow(person: friend)
          }
          .buttonStyle(.plain)
          .accessibilityIdentifier("friend-\(friend.id)")
        }
      }
      if friends.isEmpty, data.friendsState.phase == .loaded {
        Text(LocalizedStringKey(search.isEmpty && filter == .all ? "好友列表为空" : "当前筛选结果为空"))
          .foregroundStyle(AppThemePalette.textSecondary)
      }
    }
    .appThemeCanvas()
    .task(id: "\(auth.ownerGeneration):\(isActive)") {
      guard isActive else { return }
      await data.loadFriends()
    }
    .appRefreshable { await data.loadFriends(refresh: true) }
  }
}

struct PersonalProfileRow: View {
  let person: XboxFriendSummary
  var body: some View {
    HStack(spacing: 12) {
      AsyncImage(url: person.avatarURL) { image in
        image.resizable().scaledToFill()
      } placeholder: {
        Image(systemName: "person.crop.circle.fill").resizable().foregroundStyle(
          AppThemePalette.textSecondary)
      }
      .frame(width: 40, height: 40).clipShape(Circle()).accessibilityHidden(true)
      VStack(alignment: .leading, spacing: 4) {
        Text(person.gamertag.isEmpty ? person.displayName : person.gamertag)
          .appFont(.body, weight: .semibold).foregroundStyle(.primary)
        HStack(spacing: 4) {
          if !person.displayName.isEmpty, person.displayName != person.gamertag {
            Text(person.displayName)
            Text("·")
          }
          Text(LocalizedStringKey(Self.presenceTitle(person.presenceState)))
        }.appFont(.caption).foregroundStyle(AppThemePalette.textSecondary)
        if let activity = person.presenceText, !activity.isEmpty,
          activity != Self.presenceTitle(person.presenceState)
        {
          Text(LocalizedStringKey(activity)).appFont(.caption)
            .foregroundStyle(AppThemePalette.textSecondary)
        }
      }
      Spacer(minLength: 8)
      if let score = person.gamerscore {
        Text("\(score) G").appFont(.caption).monospacedDigit()
          .foregroundStyle(AppThemePalette.textPrimary)
          .fixedSize(horizontal: true, vertical: false)
      }
      AppIconView(.chevron, size: 16).foregroundStyle(AppThemePalette.textSecondary)
    }
    .padding(12).frame(maxWidth: .infinity, minHeight: 44, alignment: .leading)
    .appSurface(radius: AppLayoutPolicy.cardRadius)
    .contentShape(Rectangle())
  }

  static func presenceTitle(_ state: String?) -> String {
    switch state?.lowercased() {
    case "online": "在线"
    case "away": "离开"
    case "offline": "离线"
    default: "状态暂缺"
    }
  }
}

struct FriendDetailView: View {
  let friend: XboxFriendSummary
  @EnvironmentObject private var auth: AuthStore
  @EnvironmentObject private var data: XboxDataStore
  @Environment(\.appPageIsActive) private var isActive
  private var state: XboxSingleDataState<XboxFriendSummary> {
    data.playerProfiles[friend.id] ?? XboxSingleDataState()
  }
  private var presence: XboxFriendSummary {
    data.friendsState.items.first { $0.id == friend.id } ?? friend
  }
  private var person: XboxFriendSummary {
    guard let profile = state.value else { return presence }
    return XboxFriendSummary(
      id: profile.id, displayName: profile.displayName, gamertag: profile.gamertag,
      avatarURL: profile.avatarURL ?? presence.avatarURL,
      gamerscore: profile.gamerscore ?? presence.gamerscore,
      presenceState: presence.presenceState, presenceText: presence.presenceText,
      currentTitleID: presence.currentTitleID)
  }

  var body: some View {
    AppInformationPage {
      Section {
        PersonalIdentityHero(person: person)
        AppSectionRequestState(
          title: "玩家资料", status: state.requestStatus, identifier: "player-profile"
        ) { await data.loadPlayerProfile(targetID: friend.id, refresh: true) }
      }
      Section {
        AppContentToolbar {
          if let titleID = presence.currentTitleID {
            NavigationLink("当前游戏") {
              PersonalGameDetailView(
                game: data.historyState(targetID: friend.id).items.first {
                  $0.titleID == titleID
                }
                  ?? GameSummary(
                    id: titleID, titleID: titleID,
                    name: presence.presenceText ?? "正在玩的游戏", artworkURL: nil,
                    playtimeMinutes: nil, achievementProgress: nil),
                targetID: friend.id, section: .achievements)
            }
            .accessibilityLabel("正在玩的游戏 · 成就对比")
            .accessibilityIdentifier("friend-current-game")
          }
          NavigationLink("游戏历史") {
            GameHistoryView(targetID: friend.id).appPageToolbar("玩家游戏历史")
          }.accessibilityIdentifier("friend-game-history")
          NavigationLink("游玩统计") {
            GameHistoryView(targetID: friend.id, statistics: true).appPageToolbar(
              "玩家游玩统计")
          }.accessibilityIdentifier("friend-game-statistics")
        }
      }
      Section("最近游玩") {
        let history = data.historyState(targetID: friend.id)
        AppSectionRequestState(
          title: "最近游玩", status: history.requestStatus, identifier: "player-history"
        ) { await data.loadHistory(targetID: friend.id, refresh: true) }
        AppMediaGrid {
          ForEach(Array(history.items.prefix(5))) { game in
            NavigationLink {
              PersonalGameDetailView(
                game: game, targetID: friend.id, section: .achievements)
            } label: {
              PersonalGameCard(game: game)
            }.buttonStyle(.plain)
          }
        }
        if history.items.isEmpty, history.phase == .loaded {
          Text("暂无可见游戏记录").foregroundStyle(AppThemePalette.textSecondary)
        }
      }
    }
    .appThemeCanvas().appPageToolbar(
      person.gamertag.isEmpty ? person.displayName : person.gamertag,
      artwork: .avatar(person.avatarURL)
    )
    .appInformationTypography()
    .task(id: "\(auth.ownerGeneration):\(friend.id):\(isActive)") {
      guard isActive else { return }
      await load(refresh: false)
    }
    .appRefreshable { await load(refresh: true) }
  }

  private func load(refresh: Bool) async {
    async let profile: Void = data.loadPlayerProfile(targetID: friend.id, refresh: refresh)
    async let history: Void = data.loadHistory(targetID: friend.id, refresh: refresh)
    _ = await (profile, history)
  }
}

struct PlaytimeView: View {
  var body: some View { GameHistoryView(statistics: true) }
}

struct PersonalIdentityHero: View {
  @Environment(\.appInformationIsSidebar) private var isSidebar
  @Environment(\.dynamicTypeSize) private var textSize
  let person: XboxFriendSummary
  var currentTitle: String? = nil
  var friendCount: UInt32? = nil
  var followingCount: UInt32? = nil
  var followersCount: UInt32? = nil
  var body: some View {
    VStack(alignment: .leading, spacing: 16) {
      let identityLayout =
        textSize.isAccessibilitySize && !isSidebar
        ? AnyLayout(VStackLayout(alignment: .leading, spacing: 16))
        : AnyLayout(HStackLayout(alignment: .center, spacing: 16))
      identityLayout {
        AsyncImage(url: person.avatarURL) { image in
          image.resizable().scaledToFill()
        } placeholder: {
          Image(systemName: "person.crop.circle.fill").resizable()
            .foregroundStyle(AppThemePalette.textSecondary)
        }
        .frame(width: isSidebar ? 48 : 72, height: isSidebar ? 48 : 72).clipShape(Circle())
        .accessibilityHidden(true)
        VStack(alignment: .leading, spacing: 8) {
          Text(person.gamertag.isEmpty ? person.displayName : person.gamertag)
            .appFont(isSidebar ? .body : .hero, weight: .semibold)
          AppWrappingLayout(spacing: 8, lineSpacing: 4) {
            if !person.displayName.isEmpty, person.displayName != person.gamertag {
              Text(person.displayName)
            }
            HStack(spacing: 5) {
              Circle()
                .fill(
                  person.presenceState?.lowercased() == "online"
                    ? AppThemePalette.brand : AppThemePalette.icon
                )
                .frame(width: 6, height: 6)
                .accessibilityHidden(true)
              Text(
                LocalizedStringKey(
                  PersonalProfileRow.presenceTitle(person.presenceState)))
            }
          }
          .appFont(.bodySmall).foregroundStyle(AppThemePalette.textSecondary)
          if let activity {
            Label {
              Text(activity).fixedSize(horizontal: false, vertical: true)
            } icon: {
              AppIconView(.controller, size: 14)
            }
            .labelStyle(AppIconLabelStyle())
            .appFont(.bodySmall).foregroundStyle(AppThemePalette.textSecondary)
          }
        }
      }
      if !isSidebar {
        ViewThatFits(in: .horizontal) {
          identityMetrics
          ScrollView(.horizontal) { identityMetrics }
            .scrollIndicators(.hidden).fixedSize(horizontal: false, vertical: true)
        }
      } else if let score = person.gamerscore {
        Text("\(score) G").appFont(.caption).foregroundStyle(AppThemePalette.textPrimary)
      }
    }
    .frame(maxWidth: .infinity, minHeight: isSidebar ? 0 : 144, alignment: .bottomLeading)
    .accessibilityIdentifier("personal-identity-hero")
  }

  private var activity: String? {
    let activity = person.presenceText.flatMap { $0.isEmpty ? nil : $0 }
    guard let currentTitle, !currentTitle.isEmpty else { return activity }
    guard activity?.localizedCaseInsensitiveContains(currentTitle) != true else {
      return activity
    }
    return [activity, currentTitle].compactMap { $0 }.joined(separator: " · ")
  }

  private var identityMetrics: some View {
    HStack(alignment: .top, spacing: 0) {
      if let score = person.gamerscore {
        AppMetric(title: "成就点数", value: "\(score) G", compact: true)
          .fixedSize(horizontal: true, vertical: false)
      }
      if friendCount != nil || followingCount != nil || followersCount != nil {
        Spacer(minLength: 12)
        AppMetric(title: "好友", value: friendCount.map(String.init) ?? "—", compact: true)
          .fixedSize(horizontal: true, vertical: false)
        Spacer(minLength: 12)
        AppMetric(title: "关注", value: followingCount.map(String.init) ?? "—", compact: true)
          .fixedSize(horizontal: true, vertical: false)
        Spacer(minLength: 12)
        AppMetric(title: "粉丝", value: followersCount.map(String.init) ?? "—", compact: true)
          .fixedSize(horizontal: true, vertical: false)
      }
    }
  }
}
