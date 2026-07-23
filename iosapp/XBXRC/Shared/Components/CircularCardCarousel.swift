import Foundation
import SwiftUI
import UIKit

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

actor RemoteImageLoader {
    static let shared = RemoteImageLoader()

    enum Source: String, Sendable {
        case memory
        case urlCache
        case network
        case coalesced
    }

    // UIImage 作为加载完成后的不可变值跨 actor 交给主线程渲染。
    struct Result: @unchecked Sendable {
        let image: UIImage
        let source: Source
    }

    private final class MemoryEntry: NSObject {
        let image: UIImage
        let byteCount: Int

        init(image: UIImage, byteCount: Int) {
            self.image = image
            self.byteCount = byteCount
        }
    }

    private struct NetworkPayload: Sendable {
        let data: Data
        let statusCode: Int
    }

    private struct Flight {
        let task: Task<NetworkPayload, Error>
        let operationID: String
    }

    private let memoryCache = NSCache<NSURL, MemoryEntry>()
    private var inFlight: [URL: Flight] = [:]

    private init() {
        memoryCache.countLimit = 256
        memoryCache.totalCostLimit = 64 * 1024 * 1024
    }

    func image(for url: URL) async throws -> Result {
        let normalizedURL = Self.normalize(url)
        let request = URLRequest(url: normalizedURL, cachePolicy: .useProtocolCachePolicy)
        let key = normalizedURL as NSURL

        if let entry = memoryCache.object(forKey: key) {
            traceCacheHit(url: normalizedURL, source: Source.memory.rawValue, byteCount: entry.byteCount)
            return Result(image: entry.image, source: .memory)
        }

        if let cached = URLCache.shared.cachedResponse(for: request) {
            if let image = UIImage(data: cached.data) {
                let entry = MemoryEntry(image: image, byteCount: cached.data.count)
                memoryCache.setObject(entry, forKey: key, cost: cached.data.count)
                traceCacheHit(
                    url: normalizedURL,
                    source: Source.urlCache.rawValue,
                    byteCount: cached.data.count
                )
                return Result(image: image, source: .urlCache)
            }
            URLCache.shared.removeCachedResponse(for: request)
        }

        if let flight = inFlight[normalizedURL] {
            IOSRuntimeTrace.event(
                domain: "image",
                event: "imageNetworkCoalesced",
                payload: ["scheme": .string(normalizedURL.scheme ?? "unknown")],
                dimension: .network,
                importance: .debug,
                operationID: flight.operationID
            )
            let payload = try await flight.task.value
            if let entry = memoryCache.object(forKey: key) {
                return Result(image: entry.image, source: .coalesced)
            }
            let image = try Self.decode(payload.data)
            let entry = MemoryEntry(image: image, byteCount: payload.data.count)
            memoryCache.setObject(entry, forKey: key, cost: payload.data.count)
            return Result(image: image, source: .coalesced)
        }

        let operationID = UUID().uuidString
        IOSRuntimeTrace.event(
            domain: "image",
            event: "imageNetworkStarted",
            payload: ["scheme": .string(normalizedURL.scheme ?? "unknown")],
            dimension: .network,
            importance: .debug,
            operationID: operationID
        )
        let task = Task(priority: .utility) {
            let (data, response) = try await URLSession.shared.data(for: request)
            let statusCode = (response as? HTTPURLResponse)?.statusCode ?? 0
            guard (200..<300).contains(statusCode), !data.isEmpty else {
                throw RemoteImageLoaderError.httpStatus(statusCode)
            }
            return NetworkPayload(data: data, statusCode: statusCode)
        }
        inFlight[normalizedURL] = Flight(task: task, operationID: operationID)

        do {
            let payload = try await task.value
            defer {
                if inFlight[normalizedURL]?.operationID == operationID {
                    inFlight.removeValue(forKey: normalizedURL)
                }
            }
            let image = try Self.decode(payload.data)
            let entry = MemoryEntry(image: image, byteCount: payload.data.count)
            memoryCache.setObject(entry, forKey: key, cost: payload.data.count)
            IOSRuntimeTrace.event(
                domain: "image",
                event: "imageNetworkCompleted",
                payload: [
                    "success": .bool(true),
                    "statusCode": .integer(Int64(payload.statusCode)),
                    "byteCount": .integer(Int64(payload.data.count)),
                ],
                dimension: .network,
                importance: .debug,
                operationID: operationID
            )
            return Result(image: image, source: .network)
        } catch {
            if inFlight[normalizedURL]?.operationID == operationID {
                inFlight.removeValue(forKey: normalizedURL)
            }
            IOSRuntimeTrace.event(
                domain: "image",
                event: "imageNetworkCompleted",
                payload: [
                    "success": .bool(false),
                    "errorDomain": .string(Self.errorDomain(error)),
                ],
                dimension: .network,
                importance: .debug,
                operationID: operationID
            )
            throw error
        }
    }

    private static func decode(_ data: Data) throws -> UIImage {
        guard let image = UIImage(data: data) else {
            throw RemoteImageLoaderError.invalidImage
        }
        return image
    }

    private static func normalize(_ url: URL) -> URL {
        guard var components = URLComponents(url: url, resolvingAgainstBaseURL: true) else {
            return url.absoluteURL.standardized
        }
        components.scheme = components.scheme?.lowercased()
        components.host = components.host?.lowercased()
        components.fragment = nil
        if (components.scheme == "https" && components.port == 443)
            || (components.scheme == "http" && components.port == 80) {
            components.port = nil
        }
        return components.url?.standardized ?? url.absoluteURL.standardized
    }

    private func traceCacheHit(url: URL, source: String, byteCount: Int) {
        IOSRuntimeTrace.event(
            domain: "image",
            event: "imageCacheHit",
            payload: [
                "source": .string(source),
                "scheme": .string(url.scheme ?? "unknown"),
                "byteCount": .integer(Int64(byteCount)),
            ],
            dimension: .network,
            importance: .debug
        )
    }

    fileprivate static func errorDomain(_ error: Error) -> String {
        if let urlError = error as? URLError {
            return "url." + String(urlError.code.rawValue)
        }
        if let loaderError = error as? RemoteImageLoaderError {
            return loaderError.errorDomain
        }
        return String(reflecting: type(of: error))
    }
}

private enum RemoteImageLoaderError: Error {
    case httpStatus(Int)
    case invalidImage

    var errorDomain: String {
        switch self {
        case let .httpStatus(code): "http." + String(code)
        case .invalidImage: "invalidImage"
        }
    }
}

@MainActor
struct SharedRemoteImage<Content: View, Placeholder: View>: View {
    let candidates: [URL]
    let content: (Image) -> Content
    let placeholder: (Bool) -> Placeholder
    var onSuccess: ((URL) -> Void)?

    @State private var renderedImage: UIImage?
    @State private var isLoading = false

    init(
        candidates: [URL],
        onSuccess: ((URL) -> Void)? = nil,
        @ViewBuilder content: @escaping (Image) -> Content,
        @ViewBuilder placeholder: @escaping (Bool) -> Placeholder
    ) {
        self.candidates = candidates
        self.onSuccess = onSuccess
        self.content = content
        self.placeholder = placeholder
    }

    init(
        url: URL?,
        onSuccess: ((URL) -> Void)? = nil,
        @ViewBuilder content: @escaping (Image) -> Content,
        @ViewBuilder placeholder: @escaping (Bool) -> Placeholder
    ) {
        self.init(
            candidates: url.map { [$0] } ?? [],
            onSuccess: onSuccess,
            content: content,
            placeholder: placeholder
        )
    }

    var body: some View {
        ZStack {
            if let renderedImage {
                content(Image(uiImage: renderedImage))
            } else {
                placeholder(isLoading)
            }
        }
        .task(id: candidates) {
            await loadCandidates()
        }
    }

    @MainActor
    private func loadCandidates() async {
        let operationID = UUID().uuidString
        guard !candidates.isEmpty else {
            renderedImage = nil
            isLoading = false
            tracePhase("empty", operationID: operationID, candidateIndex: nil)
            IOSRuntimeTrace.decision(
                domain: "image",
                event: "imageCandidatesExhausted",
                payload: ["candidateCount": 0, "reason": "emptyCandidateSet"],
                dimension: .presentation,
                importance: .key,
                operationID: operationID
            )
            return
        }

        isLoading = true
        tracePhase(
            "loading",
            operationID: operationID,
            candidateIndex: 0,
            hasPreviousImage: renderedImage != nil
        )

        for (candidateIndex, url) in candidates.enumerated() {
            guard !Task.isCancelled else { return }
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
                operationID: operationID
            )
            do {
                let result = try await RemoteImageLoader.shared.image(for: url)
                guard !Task.isCancelled else { return }
                renderedImage = result.image
                isLoading = false
                tracePhase(
                    "success",
                    operationID: operationID,
                    candidateIndex: candidateIndex,
                    source: result.source.rawValue
                )
                IOSRuntimeTrace.event(
                    domain: "image",
                    event: "imageCandidateSucceeded",
                    payload: [
                        "candidateIndex": .integer(Int64(candidateIndex)),
                        "candidateCount": .integer(Int64(candidates.count)),
                        "scheme": .string(url.scheme ?? "unknown"),
                        "source": .string(result.source.rawValue),
                    ],
                    dimension: .presentation,
                    importance: .debug,
                    operationID: operationID
                )
                onSuccess?(url)
                return
            } catch {
                IOSRuntimeTrace.event(
                    domain: "image",
                    event: "imageCandidateFailed",
                    payload: [
                        "candidateIndex": .integer(Int64(candidateIndex)),
                        "candidateCount": .integer(Int64(candidates.count)),
                        "hasNext": .bool(candidateIndex + 1 < candidates.count),
                        "errorDomain": .string(RemoteImageLoader.errorDomain(error)),
                    ],
                    dimension: .presentation,
                    importance: candidateIndex + 1 < candidates.count ? .debug : .key,
                    operationID: operationID
                )
            }
        }

        isLoading = false
        tracePhase(
            "failure",
            operationID: operationID,
            candidateIndex: candidates.count - 1,
            hasPreviousImage: renderedImage != nil
        )
        IOSRuntimeTrace.decision(
            domain: "image",
            event: "imageCandidatesExhausted",
            payload: ["candidateCount": .integer(Int64(candidates.count))],
            dimension: .presentation,
            importance: .key,
            operationID: operationID
        )
    }

    private func tracePhase(
        _ phase: String,
        operationID: String,
        candidateIndex: Int?,
        source: String? = nil,
        hasPreviousImage: Bool? = nil
    ) {
        var payload: [String: IOSRuntimeTraceValue] = ["phase": .string(phase)]
        if let candidateIndex {
            payload["candidateIndex"] = .integer(Int64(candidateIndex))
        }
        if let source {
            payload["source"] = .string(source)
        }
        if let hasPreviousImage {
            payload["hasPreviousImage"] = .bool(hasPreviousImage)
        }
        IOSRuntimeTrace.event(
            domain: "image",
            event: "imageViewPhase",
            payload: payload,
            dimension: .presentation,
            importance: .debug,
            operationID: operationID
        )
    }
}

@MainActor
struct CloudGameRemoteImage: View {
    let candidates: [URL]
    let contentMode: ContentMode
    var onSuccess: ((URL) -> Void)?

    init(
        candidates: [URL],
        contentMode: ContentMode,
        onSuccess: ((URL) -> Void)? = nil
    ) {
        self.candidates = candidates
        self.contentMode = contentMode
        self.onSuccess = onSuccess
    }

    var body: some View {
        GeometryReader { geometry in
            SharedRemoteImage(candidates: candidates, onSuccess: onSuccess) { image in
                image
                    .resizable()
                    .aspectRatio(contentMode: contentMode)
            } placeholder: { showProgress in
                placeholder(showProgress: showProgress)
            }
            .frame(width: geometry.size.width, height: geometry.size.height)
            .clipped()
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
}
