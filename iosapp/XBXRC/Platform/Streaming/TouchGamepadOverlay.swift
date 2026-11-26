import SwiftUI

/// 云游戏和主机串流共用会话布局、绘制与多指输入。
struct TouchGamepadOverlay: View {
    let input: StreamInputSession
    @EnvironmentObject private var settings: AppSettingsStore
    @ObservedObject private var layout: StreamingTouchLayoutController
    @State private var isSuppressed: Bool

    init(
        input: StreamInputSession,
        layout: StreamingTouchLayoutController = StreamingTouchLayoutController()
    ) {
        self.input = input
        self.layout = layout
        _isSuppressed = State(initialValue: input.isSuppressed)
    }

    var body: some View {
        ZStack(alignment: .topLeading) {
            GeometryReader { geometry in
                let frame = StreamingTouchViewportPolicy.frame(in: geometry.size)
                TouchLayoutSurface(
                    input: layout.input, opacity: layout.opacity(preferences: settings.mobile),
                    assets: layout.assets,
                    capturesEmptySpace: false,
                    configuration: layout.configuration(
                        preferences: settings.mobile,
                        orientation: TouchLayoutOrientation(size: geometry.size)),
                    onPress: { MobileInteractionFeedback.play(settings.mobile) }
                )
                .opacity(layout.isGameHidden ? 0 : 1)
                .allowsHitTesting(!layout.isGameHidden)
                .frame(width: frame.width, height: frame.height)
                .position(x: frame.midX, y: frame.midY)
            }
            if let error = layout.loadError {
                Text(error).font(.caption).foregroundStyle(.white).padding(.top, 56)
            }
        }
        .ignoresSafeArea()
        .allowsHitTesting(!isSuppressed)
        .onAppear { layout.bind(to: input) }
        .onReceive(input.$isSuppressed.removeDuplicates()) { suppressed in
            isSuppressed = suppressed
            if suppressed { layout.input.cancelAll(reason: "inputSuppressed") }
        }
        .onDisappear { layout.surfaceClosed() }
    }
}

enum StreamingTouchViewportPolicy {
    static func frame(in size: CGSize) -> CGRect {
        let portrait = size.height > size.width
        return CGRect(
            x: 0, y: portrait ? size.height / 2 : 0,
            width: size.width, height: portrait ? size.height / 2 : size.height)
    }
}
