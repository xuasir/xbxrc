import SwiftUI

struct LaunchExperienceView: View {
    let isRestoring: Bool
    let onFinished: () -> Void

    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var iconScale: CGFloat = 1
    @State private var overlayOpacity = 1.0
    @State private var exitStarted = false

    var body: some View {
        GeometryReader { geometry in
            ZStack {
                Color.white

                Image("LaunchIcon")
                    .resizable()
                    .scaledToFit()
                    .frame(width: iconSize(in: geometry.size))
                    .scaleEffect(iconScale)
                    .accessibilityHidden(true)
            }
            .frame(width: geometry.size.width, height: geometry.size.height)
            .opacity(overlayOpacity)
            .accessibilityElement(children: .ignore)
            .accessibilityLabel("正在启动 XBXRC")
            .accessibilityAddTraits(.isStaticText)
        }
        .ignoresSafeArea()
        .zIndex(100)
        .onChange(of: isRestoring) { _, restoring in
            if !restoring {
                startExit()
            }
        }
        .task {
            if !isRestoring {
                startExit()
            }
        }
    }

    private func iconSize(in size: CGSize) -> CGFloat {
        let shortSide = min(size.width, size.height)
        return min(176, max(132, shortSide * 0.30))
    }

    private func startExit() {
        guard !exitStarted else {
            return
        }
        exitStarted = true

        let duration = reduceMotion ? 0.22 : 0.42
        withAnimation(.easeInOut(duration: duration)) {
            overlayOpacity = 0
            if !reduceMotion {
                iconScale = 7
            }
        }

        Task { @MainActor in
            try? await Task.sleep(for: .milliseconds(Int(duration * 1_000) + 60))
            onFinished()
        }
    }
}
