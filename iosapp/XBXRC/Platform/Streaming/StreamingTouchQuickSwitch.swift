import SwiftUI

/// 串流页直接切换标准与最近自定义布局，低频读包沿同一个会话 controller。
struct StreamingTouchQuickSwitch: View {
  @ObservedObject var controller: StreamingTouchLayoutController
  let session: StreamInputSession
  @EnvironmentObject private var settings: AppSettingsStore
  @EnvironmentObject private var library: SavedTouchLayoutsStore
  @State private var operation: Task<Void, Never>?
  @State private var revision = 0

  private var nextSelection: TouchLayoutSelection? {
    controller.nextQuickSelection(preferences: settings.mobile, library: library)
  }

  private var isStandard: Bool {
    controller.effectiveSelection(preferences: settings.mobile) == .standard
  }

  var body: some View {
    Button {
      guard let selection = nextSelection else { return }
      revision &+= 1
      let request = revision
      operation?.cancel()
      session.setSuppressed(true, reason: .layoutSwitch)
      operation = Task { @MainActor in
        defer {
          if request == revision { session.setSuppressed(false, reason: .layoutSwitch) }
        }
        _ = await controller.select(
          selection, library: library, preferences: settings.mobile)
      }
    } label: {
      Group {
        if controller.pendingSelection != nil {
          ProgressView().controlSize(.small).tint(.white)
        } else {
          Image(systemName: "arrow.trianglehead.2.clockwise.rotate.90")
            .font(.system(size: 19, weight: .medium))
        }
      }
      .foregroundStyle(.white)
      .shadow(color: .black.opacity(0.6), radius: 1)
      .frame(width: 44, height: 44).contentShape(Rectangle())
    }
    .buttonStyle(.plain)
    .streamingQuickControlSurface()
    .disabled(nextSelection == nil)
    .accessibilityLabel(
      Text(LocalizedStringKey(nextSelection == .standard ? "切换标准布局" : "切换最近自定义布局"))
    )
    .accessibilityValue(
      Text(LocalizedStringKey(nextSelection == nil ? "暂无最近布局" : isStandard ? "标准" : "最近"))
    )
    .accessibilityIdentifier("stream-touch-quick-switch")
    .onDisappear {
      revision &+= 1
      operation?.cancel()
      session.setSuppressed(false, reason: .layoutSwitch)
    }
  }
}

/// 快捷入口透出游戏画面，辅助功能降低透明度时提供清晰底色。
private struct StreamingQuickControlSurface: ViewModifier {
  @Environment(\.accessibilityReduceTransparency) private var reduceTransparency

  func body(content: Content) -> some View {
    content
      .background(
        .black.opacity(reduceTransparency ? 0.82 : 0.12),
        in: RoundedRectangle(cornerRadius: 12, style: .continuous)
      )
      .overlay {
        RoundedRectangle(cornerRadius: 12, style: .continuous)
          .strokeBorder(.white.opacity(reduceTransparency ? 0.25 : 0.12), lineWidth: 0.5)
          .allowsHitTesting(false)
      }
  }
}

extension View {
  func streamingQuickControlSurface() -> some View {
    modifier(StreamingQuickControlSurface())
  }
}
