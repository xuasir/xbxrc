import SwiftUI

private struct SkeletonPulseModifier: ViewModifier {
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var isHighlighted = false

    let accessibilityLabel: String

    func body(content: Content) -> some View {
        content
            .opacity(reduceMotion ? 0.62 : (isHighlighted ? 0.74 : 0.44))
            .accessibilityElement(children: .ignore)
            .accessibilityLabel(accessibilityLabel)
            .task {
                guard !reduceMotion else {
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
