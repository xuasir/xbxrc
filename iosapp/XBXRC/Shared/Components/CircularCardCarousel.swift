import SwiftUI

struct CircularCardCarouselMetrics: Hashable, Sendable {
    let cardWidth: CGFloat
    let cardHeight: CGFloat
    let cardSpacing: CGFloat

    static let gameDetail = CircularCardCarouselMetrics(
        cardWidth: 248,
        cardHeight: 190,
        cardSpacing: 14
    )
}

struct CircularCardCarousel<Item: Identifiable, Card: View>: View where Item.ID: Hashable {
    let items: [Item]
    @Binding var selection: Item.ID?
    let metrics: CircularCardCarouselMetrics
    @ViewBuilder let card: (Item) -> Card

    var body: some View {
        GeometryReader { geometry in
            GlassEffectContainer(spacing: 0) {
                ScrollView(.horizontal) {
                    LazyHStack(spacing: metrics.cardSpacing) {
                        ForEach(items) { item in
                            card(item)
                                .frame(width: metrics.cardWidth, height: metrics.cardHeight)
                                .id(item.id)
                        }
                    }
                    .scrollTargetLayout()
                }
                .scrollIndicators(.hidden)
                .scrollTargetBehavior(.viewAligned(limitBehavior: .alwaysByOne))
                .scrollPosition(id: $selection, anchor: .center)
                .contentMargins(
                    .horizontal,
                    horizontalContentMargin(for: geometry.size.width),
                    for: .scrollContent
                )
                .scrollDisabled(items.count <= 1)
            }
        }
        .task(id: items.map(\.id)) {
            normalizeSelection()
        }
        .onChange(of: selection) { _, newSelection in
            IOSRuntimeTrace.state(
                domain: "library-ui",
                event: "detailCarouselSelectionChanged",
                payload: [
                    "items": .integer(Int64(items.count)),
                    "selectedIndex": .integer(Int64(
                        items.firstIndex { $0.id == newSelection } ?? 0
                    )),
                ],
                dimension: .frontend,
                importance: .debug
            )
        }
        .sensoryFeedback(.selection, trigger: selection)
    }

    private func horizontalContentMargin(for viewportWidth: CGFloat) -> CGFloat {
        max(metrics.cardSpacing, (viewportWidth - metrics.cardWidth) / 2)
    }

    private func normalizeSelection() {
        guard items.contains(where: { $0.id == selection }) else {
            selection = items.first?.id
            return
        }
    }
}

struct CloudGameRemoteImage: View {
    let candidates: [URL]
    let contentMode: ContentMode
    var onSuccess: ((URL) -> Void)?

    @State private var candidateIndex = 0
    @State private var traceOperationID = UUID().uuidString
    @State private var lastStartedURL: URL?
    @State private var lastFailedURL: URL?
    @State private var lastSuccessfulURL: URL?
    @State private var reportedEmptyCandidateSet = false

    init(
        candidates: [URL],
        contentMode: ContentMode,
        onSuccess: ((URL) -> Void)? = nil
    ) {
        self.candidates = candidates
        self.contentMode = contentMode
        self.onSuccess = onSuccess
    }

    private var currentURL: URL? {
        candidates.indices.contains(candidateIndex) ? candidates[candidateIndex] : nil
    }

    var body: some View {
        GeometryReader { geometry in
            if let currentURL {
                AsyncImage(url: currentURL) { phase in
                    switch phase {
                    case let .success(image):
                        image
                            .resizable()
                            .aspectRatio(contentMode: contentMode)
                            .onAppear {
                                reportSuccess(currentURL)
                            }
                    case .empty:
                        placeholder(showProgress: true)
                    case .failure:
                        placeholder(showProgress: false)
                            .onAppear {
                                advanceCandidate(afterFailureOf: currentURL)
                            }
                    @unknown default:
                        placeholder(showProgress: false)
                    }
                }
                .id(currentURL)
                .onAppear {
                    reportCandidateStarted(currentURL)
                }
                .frame(width: geometry.size.width, height: geometry.size.height)
                .clipped()
            } else {
                placeholder(showProgress: false)
                    .onAppear {
                        reportEmptyCandidateSet()
                    }
            }
        }
        .onChange(of: candidates) { _, _ in
            candidateIndex = 0
            traceOperationID = UUID().uuidString
            lastStartedURL = nil
            lastFailedURL = nil
            lastSuccessfulURL = nil
            reportedEmptyCandidateSet = false
        }
        .accessibilityHidden(true)
    }

    private func placeholder(showProgress: Bool) -> some View {
        ZStack {
            LinearGradient(
                colors: [
                    AppThemePalette.canvasTop,
                    AppThemePalette.canvasBottom,
                ],
                startPoint: .topLeading,
                endPoint: .bottomTrailing
            )
            Image(systemName: "gamecontroller.fill")
                .font(.system(size: 54, weight: .semibold))
                .foregroundStyle(.white.opacity(0.3))
            if showProgress {
                ProgressView()
                    .tint(.white)
                    .offset(y: 52)
            }
        }
    }

    private func advanceCandidate(afterFailureOf url: URL) {
        guard lastFailedURL != url else {
            return
        }
        lastFailedURL = url
        let hasNext = candidateIndex + 1 < candidates.count
        IOSRuntimeTrace.event(
            domain: "image",
            event: "imageCandidateFailed",
            payload: [
                "candidateIndex": .integer(Int64(candidateIndex)),
                "candidateCount": .integer(Int64(candidates.count)),
                "hasNext": .bool(hasNext),
            ],
            dimension: .presentation,
            importance: hasNext ? .debug : .key,
            operationID: traceOperationID
        )
        guard candidateIndex + 1 < candidates.count else {
            IOSRuntimeTrace.decision(
                domain: "image",
                event: "imageCandidatesExhausted",
                payload: ["candidateCount": .integer(Int64(candidates.count))],
                dimension: .presentation,
                importance: .key,
                operationID: traceOperationID
            )
            return
        }
        DispatchQueue.main.async {
            candidateIndex += 1
        }
    }

    private func reportSuccess(_ url: URL) {
        guard lastSuccessfulURL != url else {
            return
        }
        lastSuccessfulURL = url
        IOSRuntimeTrace.event(
            domain: "image",
            event: "imageCandidateSucceeded",
            payload: [
                "candidateIndex": .integer(Int64(candidateIndex)),
                "candidateCount": .integer(Int64(candidates.count)),
                "scheme": .string(url.scheme ?? "unknown"),
            ],
            dimension: .presentation,
            importance: .debug,
            operationID: traceOperationID
        )
        onSuccess?(url)
    }

    private func reportCandidateStarted(_ url: URL) {
        guard lastStartedURL != url else {
            return
        }
        lastStartedURL = url
        IOSRuntimeTrace.event(
            domain: "image",
            event: "imageCandidateStarted",
            payload: [
                "candidateIndex": .integer(Int64(candidateIndex)),
                "candidateCount": .integer(Int64(candidates.count)),
                "scheme": .string(url.scheme ?? "unknown"),
            ],
            dimension: .presentation,
            importance: .debug,
            operationID: traceOperationID
        )
    }

    private func reportEmptyCandidateSet() {
        guard !reportedEmptyCandidateSet else {
            return
        }
        reportedEmptyCandidateSet = true
        IOSRuntimeTrace.decision(
            domain: "image",
            event: "imageCandidatesExhausted",
            payload: ["candidateCount": 0, "reason": "emptyCandidateSet"],
            dimension: .presentation,
            importance: .key,
            operationID: traceOperationID
        )
    }
}
