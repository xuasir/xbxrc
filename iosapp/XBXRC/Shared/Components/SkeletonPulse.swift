import SwiftUI

private struct SkeletonPulseModifier: ViewModifier {
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.appIsPullRefreshing) private var isPullRefreshing
    @State private var isHighlighted = false

    let accessibilityLabel: String

    func body(content: Content) -> some View {
        content
            .opacity(reduceMotion || isPullRefreshing ? 0.62 : (isHighlighted ? 0.74 : 0.44))
            .accessibilityElement(children: .ignore)
            .accessibilityLabel(accessibilityLabel)
            .task(id: reduceMotion || isPullRefreshing) {
                guard !reduceMotion && !isPullRefreshing else {
                    return
                }

                while !Task.isCancelled {
                    withAnimation(.easeInOut(duration: 0.9)) {
                        isHighlighted.toggle()
                    }
                    try? await Task.sleep(for: .seconds(0.9))
                }
            }
    }
}

extension View {
    func skeletonPulse(accessibilityLabel: String) -> some View {
        modifier(SkeletonPulseModifier(accessibilityLabel: accessibilityLabel))
    }
}
