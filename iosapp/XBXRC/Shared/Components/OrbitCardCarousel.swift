import SwiftUI

struct OrbitCardCarousel<Item: Identifiable, Card: View>: View where Item.ID: Hashable {
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    let items: [Item]
    @Binding var selection: Item.ID?
    let cardWidth: CGFloat
    let cardHeight: CGFloat
    let showsCardChrome: Bool
    @ViewBuilder let card: (Item) -> Card

    @State private var dragProgress: CGFloat = 0

    private let orbitRadius: CGFloat = 3_600
    private let angleStepDegrees: CGFloat = 4.65
    private let dragStepWidth: CGFloat = 220

    init(
        items: [Item],
        selection: Binding<Item.ID?>,
        cardWidth: CGFloat = 260,
        cardHeight: CGFloat = 390,
        showsCardChrome: Bool = true,
        @ViewBuilder card: @escaping (Item) -> Card
    ) {
        self.items = items
        _selection = selection
        self.cardWidth = cardWidth
        self.cardHeight = cardHeight
        self.showsCardChrome = showsCardChrome
        self.card = card
    }

    var body: some View {
        GeometryReader { geometry in
            ZStack {
                ForEach(items.indices, id: \.self) { index in
                    carouselCard(item: items[index], at: index)
                }
            }
            .frame(width: geometry.size.width, height: geometry.size.height)
            .contentShape(Rectangle())
            .highPriorityGesture(carouselDragGesture)
        }
        .task(id: items.map(\.id)) {
            normalizeSelection()
        }
        .sensoryFeedback(.selection, trigger: selection)
    }

    private func carouselCard(item: Item, at index: Int) -> some View {
        let position = relativePosition(for: index)
        // 卡片中心沿共享圆弧移动，卡片角度同步对齐圆周切线。
        let radians = Double(position * angleStepDegrees) * .pi / 180
        let distance = abs(position)
        let depth = min(distance, 2)

        return cardSurface(item)
            .buttonStyle(OrbitCardPressStyle())
            .contentShape(RoundedRectangle(cornerRadius: 16, style: .continuous))
            .scaleEffect(max(0.92, 1 - depth * 0.035))
            .opacity(cardOpacity(at: distance))
            .rotationEffect(.degrees(Double(position * angleStepDegrees)))
            .offset(
                x: orbitRadius * CGFloat(sin(radians)),
                y: orbitRadius * CGFloat(1 - cos(radians))
            )
            .zIndex(100 - Double(abs(position)))
            .allowsHitTesting(distance < 1.15)
            .accessibilityHidden(distance >= 1.15)
    }

    @ViewBuilder
    private func cardSurface(_ item: Item) -> some View {
        if showsCardChrome {
            card(item)
                .frame(width: cardWidth, height: cardHeight)
                .mask(RoundedRectangle(cornerRadius: 16, style: .continuous))
                .overlay {
                    RoundedRectangle(cornerRadius: 16, style: .continuous)
                        .stroke(.white.opacity(0.2), lineWidth: 0.75)
                }
                .shadow(color: .black.opacity(0.22), radius: 14, y: 8)
        } else {
            card(item)
                .frame(width: cardWidth, height: cardHeight)
        }
    }

    private func cardOpacity(at distance: CGFloat) -> Double {
        if distance <= 1.05 {
            return Double(1 - min(distance, 1) * 0.06)
        }
        if distance >= 1.7 {
            return 0
        }

        let fadeProgress = (distance - 1.05) / 0.65
        return Double(0.94 * (1 - fadeProgress))
    }

    private var carouselDragGesture: some Gesture {
        DragGesture(minimumDistance: 12)
            .onChanged { value in
                dragProgress = min(
                    max(-value.translation.width / dragStepWidth, -1.15),
                    1.15
                )
            }
            .onEnded { value in
                let projectedTranslation = abs(value.predictedEndTranslation.width)
                    > abs(value.translation.width)
                    ? value.predictedEndTranslation.width
                    : value.translation.width
                let step = abs(projectedTranslation) >= 48
                    ? (projectedTranslation < 0 ? 1 : -1)
                    : 0

                withAnimation(carouselAnimation) {
                    if step != 0 {
                        moveSelection(by: step)
                    }
                    dragProgress = 0
                }
            }
    }

    private var carouselAnimation: Animation {
        reduceMotion
            ? .easeOut(duration: 0.16)
            : .snappy(duration: 0.68, extraBounce: 0.06)
    }

    private func relativePosition(for index: Int) -> CGFloat {
        guard !items.isEmpty else {
            return 0
        }

        let selectedIndex = items.firstIndex { $0.id == selection } ?? 0
        var distance = index - selectedIndex
        let halfCount = items.count / 2

        if distance > halfCount {
            distance -= items.count
        } else if distance < -halfCount {
            distance += items.count
        }

        return CGFloat(distance) - dragProgress
    }

    private func moveSelection(by step: Int) {
        guard !items.isEmpty else {
            selection = nil
            return
        }

        let currentIndex = items.firstIndex { $0.id == selection } ?? 0
        let nextIndex = (currentIndex + step + items.count) % items.count
        selection = items[nextIndex].id
    }

    private func normalizeSelection() {
        dragProgress = 0
        guard items.contains(where: { $0.id == selection }) else {
            selection = items.first?.id
            return
        }
    }
}

private struct OrbitCardPressStyle: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .scaleEffect(configuration.isPressed ? 0.975 : 1)
            .brightness(configuration.isPressed ? -0.04 : 0)
            .animation(.snappy(duration: 0.22), value: configuration.isPressed)
    }
}
