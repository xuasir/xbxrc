import SwiftUI

/// 云游戏和主机串流共用通用 Xbox 布局、绘制与多指输入。
struct TouchGamepadOverlay: View {
    let input: StreamInputSession
    @EnvironmentObject private var settings: AppSettingsStore
    @StateObject private var layout = StreamingTouchLayoutController()
    @State private var isSuppressed: Bool

    init(input: StreamInputSession) {
        self.input = input
        _isSuppressed = State(initialValue: input.isSuppressed)
    }

    var body: some View {
        ZStack(alignment: .topLeading) {
            GeometryReader { geometry in
                let frame = StreamingTouchViewportPolicy.frame(in: geometry.size)
                TouchLayoutSurface(
                    input: layout.input, opacity: settings.mobile.selectedTouchAppearance.opacity,
                    capturesEmptySpace: false,
                    configuration: settings.mobile.touchConfiguration(
                        for: TouchLayoutOrientation(size: geometry.size)),
                    onPress: { MobileInteractionFeedback.play(settings.mobile) }
                )
                .frame(width: frame.width, height: frame.height)
                .position(x: frame.midX, y: frame.midY)
            }
            if let error = layout.loadError {
                Text(error).font(.caption).foregroundStyle(.white).padding(.top, 56)
            }
        }
        .allowsHitTesting(!isSuppressed)
        .onAppear { layout.bind(to: input) }
        .onReceive(input.$isSuppressed.removeDuplicates()) { suppressed in
            isSuppressed = suppressed
            if suppressed { layout.input.cancelAll(reason: "inputSuppressed") }
        }
        .onDisappear { layout.unbind(from: input) }
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

@MainActor
final class StreamingTouchLayoutController: ObservableObject {
    let input = TouchInputAccumulator()
    let loadError: String?

    init(bundle: Bundle = .main) {
        do {
            input.load(try TouchLayoutCatalog.standard(bundle: bundle))
            loadError = nil
        } catch {
            loadError = error.localizedDescription
        }
    }

    func bind(to session: StreamInputSession) {
        input.onInput = { [weak session] frame in
            session?.setTouchLayoutState(frame.gamepad, active: frame.pointerCount > 0)
        }
    }

    func unbind(from session: StreamInputSession) {
        input.cancelAll(reason: "streamLayoutClosed")
        input.onInput = nil
        session.releaseTouches()
    }
}
