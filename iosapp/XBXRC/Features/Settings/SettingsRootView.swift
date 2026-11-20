import SwiftUI

private enum SettingsRoute: AppSecondaryRoute {
  case general, streaming, connection, diagnostics, about
  var title: String {
    switch self {
    case .general: "通用"
    case .streaming: "串流体验"
    case .connection: "连接与主机"
    case .diagnostics: "高级诊断"
    case .about: "关于"
    }
  }
  var icon: AppIcon {
    switch self {
    case .general: .paintbrush
    case .streaming: .video
    case .connection: .network
    case .diagnostics: .pulse
    case .about: .info
    }
  }
  var subtitle: String {
    switch self {
    case .general: "外观 · 语言 · 操作反馈"
    case .streaming: "画面 · 音频 · 码率"
    case .connection: "地区 · 服务器 · 网络"
    case .diagnostics: "录制 · 导出 · 清理"
    case .about: "版本与许可"
    }
  }
}

struct SettingsRootView: View {
  @State private var selection: SettingsRoute?
  @State private var traceProfile = IOSRuntimeTrace.currentProfile

  var body: some View {
    AdaptiveSectionNavigation(title: "设置", selection: $selection, compactRows: true) {
      Text("应用偏好").appFont(.section, weight: .semibold)
        .frame(maxWidth: .infinity, alignment: .leading)
    } detail: { route in
      switch route {
      case .general: GeneralSettingsView()
      case .streaming: StreamingExperienceSettingsView()
      case .connection: ConnectionSettingsView()
      case .diagnostics: DiagnosticsSettingsView(traceProfile: $traceProfile)
      case .about: AboutSettingsView()
      }
    }
  }
}

struct GeneralSettingsView: View {
  @EnvironmentObject private var settings: AppSettingsStore
  var body: some View {
    AppSettingsForm {
      Section("外观") {
        AppSettingsPicker(
          "应用外观", selection: $settings.appearanceMode,
          selectedTitle: settings.appearanceMode.title
        ) {
          ForEach(AppAppearanceMode.allCases) {
            Text(LocalizedStringKey($0.title)).tag($0)
          }
        }
        AppSettingsNavigationLink("应用图标", destination: AppearanceSettingsView())
        AppSettingsPicker(
          "应用语言", selection: $settings.mobile.appLanguage,
          selectedTitle: settings.mobile.appLanguage.title
        ) {
          ForEach(MobileAppLanguage.allCases) {
            Text(LocalizedStringKey($0.title)).tag($0)
          }
        }

      }
      Section("操作反馈") {
        AppSettingsToggle("操作触觉", isOn: $settings.mobile.uiHaptics)
        AppSettingsToggle("操作音效", isOn: $settings.mobile.uiAudio)
        AppSettingsActionButton("试听操作反馈") {
          MobileInteractionFeedback.play(settings.mobile)
        }
      }
      Section("登录") {
        AppSettingsToggle("使用临时登录会话", isOn: $settings.usesEphemeralLoginSession)
          .appSettingsHint("下次登录生效", identifier: "general-login")
      }
      Section("屏幕") {
        AppSettingsToggle("串流时保持屏幕常亮", isOn: $settings.mobile.keepScreenAwake)
          .appSettingsHint("退出串流后恢复自动锁定", identifier: "general-screen")
      }
    }
    .scrollContentBackground(.hidden).appThemeCanvas()
  }
}

struct StreamingExperienceSettingsView: View {
  @EnvironmentObject private var settings: AppSettingsStore

  var body: some View {
    AppSettingsForm {
      Section {
        AppSettingsPicker(
          "云游戏分辨率", selection: $settings.cloudResolution,
          selectedTitle: settings.cloudResolution.title
        ) {
          ForEach(StreamingResolutionOption.cloudChoices) {
            Text(LocalizedStringKey($0.title)).tag($0)
          }
        }
        AppSettingsPicker(
          "主机串流分辨率", selection: $settings.homeResolution,
          selectedTitle: settings.homeResolution.title
        ) {
          ForEach(StreamingResolutionOption.homeChoices) {
            Text(LocalizedStringKey($0.title)).tag($0)
          }
        }
        AppSettingsPicker(
          "视频质量档位", selection: $settings.codecPreference,
          selectedTitle: settings.codecPreference.title
        ) {
          ForEach(StreamingCodecOption.allCases) {
            Text(LocalizedStringKey($0.title)).tag($0)
          }
        }
        .appSettingsHint("分辨率与编码在下次连接生效", identifier: "stream-quality")
        AppSettingsToggle("防止分辨率下降", isOn: $settings.mobile.preventResolutionDrops)
          .appSettingsHint(
            "下次连接生效，设备压力可能增加卡顿",
            identifier: "stream-adaptation")
      } header: {
        Text("画质")
      }
      Section {
        AppSettingsPicker(
          "初始画面", selection: $settings.mobile.videoFit,
          selectedTitle: MobileVideoFormat(rawValue: settings.mobile.videoFit)?.title
            ?? "完整显示"
        ) {
          ForEach(MobileVideoFormat.allCases) {
            Text(LocalizedStringKey($0.title)).tag($0.rawValue)
          }
        }
        .appSettingsHint("游玩时可在控制菜单调整", identifier: "stream-playback")
        AppSettingsNavigationLink(
          "画面调节",
          destination: MobilePictureSettingsView(picture: $settings.mobile.picture))
        AppSettingsToggle("显示音量控制", isOn: $settings.mobile.enableAudioControl)
        AppSettingsToggle("以静音开始", isOn: $settings.mobile.startMuted)
        AppSettingsToggle("紧凑性能显示", isOn: $settings.mobile.compactPerformanceOverlay)
      } header: {
        Text("播放")
      }
      Section("视频码率上限") {
        AppSettingsPicker(
          "主机模式", selection: $settings.homeBitrateMode,
          selectedTitle: settings.homeBitrateMode.title
        ) {
          ForEach(StreamingBitrateMode.allCases) {
            Text(LocalizedStringKey($0.title)).tag($0)
          }
        }
        if settings.homeBitrateMode == .custom {
          MobileIntegerSetting(
            title: "主机码率", value: $settings.homeBitrateMbps,
            range: 0...200, unit: "Mb/s", presets: [10, 20, 40, 80]
          )
          .appSettingsHint("下次连接生效；0 为自动协商")
        }
        AppSettingsPicker(
          "云游戏模式", selection: $settings.cloudBitrateMode,
          selectedTitle: settings.cloudBitrateMode.title
        ) {
          ForEach(StreamingBitrateMode.allCases) {
            Text(LocalizedStringKey($0.title)).tag($0)
          }
        }
        if settings.cloudBitrateMode == .custom {
          MobileIntegerSetting(
            title: "云游戏码率", value: $settings.cloudBitrateMbps,
            range: 0...200, unit: "Mb/s", presets: [10, 20, 40, 80]
          )
          .appSettingsHint("下次连接生效；0 为自动协商")
        }
      }
      Section {
        AppSettingsPicker(
          "音频模式", selection: $settings.audioBitrateMode,
          selectedTitle: settings.audioBitrateMode.title
        ) {
          ForEach(StreamingBitrateMode.allCases) {
            Text(LocalizedStringKey($0.title)).tag($0)
          }
        }
        .appSettingsHint("下次连接生效；0 为自动协商", identifier: "stream-bitrate")
        if settings.audioBitrateMode == .custom {
          MobileIntegerSetting(
            title: "音频码率", value: $settings.audioBitrateKbps,
            range: 0...200, unit: "kb/s", presets: [0, 64, 128, 200])
        }
      } header: {
        Text("音频码率上限")
      }
    }
    .scrollContentBackground(.hidden).appThemeCanvas()
  }
}

struct ConnectionSettingsView: View {
  @EnvironmentObject private var settings: AppSettingsStore
  @EnvironmentObject private var auth: AuthStore
  @EnvironmentObject private var cloud: CloudLibraryStore
  @State private var regions: [String] = []
  @State private var isApplying = false
  @State private var selectedRegion: CloudRegionPreset = .default
  @State private var message: String?

  var body: some View {
    AppSettingsForm {
      Section {
        AppSettingsPicker(
          "授权地区", selection: $selectedRegion, selectedTitle: selectedRegion.title
        ) {
          ForEach(CloudRegionPreset.allCases) {
            Text(LocalizedStringKey($0.title)).tag($0)
          }
        }
        LabeledContent("云访问") {
          Text(
            LocalizedStringKey(
              MySettingsPresentation.cloudAccessStatus(
                for: auth.session.map { Int($0.appLevel) })))
        }.appSettingsInformation()
        AppSettingsActionButton(isApplying ? "正在应用…" : "应用授权地区") { applyRegion() }
          .disabled(isApplying)
          .appSettingsHint("下次打开云游戏时生效", identifier: "connection-auth")
      } header: {
        Text("云游戏授权")
      }
      Section {
        AppSettingsPicker(
          "串流服务器", selection: $settings.mobile.cloudServerRegion,
          selectedTitle: settings.mobile.cloudServerRegion == "auto"
            ? "自动选择" : settings.mobile.cloudServerRegion
        ) {
          Text("自动选择").tag("auto")
          ForEach(regionChoices, id: \.self) { Text($0).tag($0) }
        }
        .appSettingsHint("当前账号可用区域；下次连接生效", identifier: "connection-server")
        AppSettingsActionButton("刷新可用服务器") { Task { await loadRegions() } }
          .disabled(isApplying || !auth.isSignedIn)
        AppSettingsPicker(
          "游戏语言", selection: $settings.preferredGameLocale,
          selectedTitle: settings.preferredGameLanguageTitle
        ) {
          ForEach(PreferredGameLanguageCatalog.all) {
            Text(LocalizedStringKey($0.title)).tag($0.code)
          }
        }
        .appSettingsHint("下次连接生效")
      } header: {
        Text("串流连接")
      }
      Section {
        AppSettingsPicker(
          "目录市场", selection: $settings.mobile.catalogMarket,
          selectedTitle: settings.mobile.catalogMarket
        ) {
          ForEach(MobilePreferences.markets, id: \.self) { Text($0).tag($0) }
        }
        AppSettingsPicker(
          "目录语言", selection: $settings.mobile.catalogLanguage,
          selectedTitle: PreferredGameLanguageCatalog.title(
            for: settings.mobile.catalogLanguage)
        ) {
          ForEach(PreferredGameLanguageCatalog.all.filter { !$0.code.isEmpty }) {
            Text(LocalizedStringKey($0.title)).tag($0.code)
          }
        }
        .appSettingsHint("下次打开云游戏时生效", identifier: "connection-catalog")
      } header: {
        Text("云游戏目录")
      }
      Section {
        AppSettingsToggle("连接时唤醒主机", isOn: $settings.mobile.powerOn)
          .appSettingsHint("下次连接生效；主机需启用远程功能", identifier: "connection-power-on")
        AppSettingsToggle("优先 IPv6", isOn: $settings.preferIPv6)
          .appSettingsHint("下次连接生效")
        AppSettingsToggle("主机串流允许 TURN 中继", isOn: $settings.homeTurnFallbackEnabled)
          .appSettingsHint("直接连接失败时尝试中继；下次连接生效", identifier: "connection-network")
      } header: {
        Text("网络与主机")
      }
      Section("高级连接") {
        AppSettingsNavigationLink("自定义 TURN 中继", destination: MobileTurnSettingsView())
      }
      if let message {
        Section { Text(message).foregroundStyle(AppThemePalette.textSecondary) }
      }
    }
    .disabled(isApplying)
    .scrollContentBackground(.hidden).appThemeCanvas()
    .task {
      selectedRegion = settings.cloudRegionPreset
      await loadRegions()
    }
  }

  private var regionChoices: [String] {
    Array(
      Set(
        regions
          + (settings.mobile.cloudServerRegion == "auto"
            ? [] : [settings.mobile.cloudServerRegion]))
    )
    .sorted()
  }

  private func loadRegions() async {
    guard auth.isSignedIn else { return }
    let owner = auth.ownerGeneration
    do {
      let access = try await auth.prepareCloudAccess()
      defer { try? releaseStreamAccess(accessHandle: access.handle) }
      let result = try streamCloudRegions(accessHandle: access.handle)
      guard owner == auth.ownerGeneration, !Task.isCancelled else { return }
      regions = result
    } catch {
      guard owner == auth.ownerGeneration, !Task.isCancelled else { return }
      message = CloudLibraryDiagnostics.safeError(error)
    }
  }

  private func applyRegion() {
    guard !isApplying else { return }
    isApplying = true
    message = nil
    settings.setCloudRegionPreset(selectedRegion)
    Task {
      await cloud.clear()
      if auth.isSignedIn {
        let renewed = await auth.refreshForRegionChange()
        if renewed {
          await loadRegions()
        } else {
          message = auth.errorMessage ?? "授权刷新失败，请重试"
        }
      }
      isApplying = false
    }
  }
}
