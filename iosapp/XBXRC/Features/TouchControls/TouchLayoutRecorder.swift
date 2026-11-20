import Combine
import Foundation

@MainActor
final class TouchLayoutRecorder: ObservableObject {
    let input = TouchInputAccumulator()
    let choices: [TouchLayoutChoice]
    let loadError: String?
    @Published private(set) var selectedID: String
    @Published private(set) var latest: TouchInputFrame?
    @Published private(set) var recentEvents: [String] = []
    @Published private(set) var sampling: TouchSamplingSummary?
    @Published private(set) var retainedFrameCount = 0
    @Published private(set) var evictedFrameCount = 0
    private var buffer = TouchRingBuffer<TouchInputFrame>(capacity: maximumFrames)
    private var eventBuffer = TouchRingBuffer<TouchPointerEvent>(capacity: 8)
    private var pendingLatest: TouchInputFrame?
    private var pendingRefresh: DispatchWorkItem?
    private var lastRefresh: TimeInterval = 0
    var stateSink: ((TouchInputFrame) -> Void)?
    private(set) var presentationCount = 0
    static let maximumFrames = 2048
    static let presentationInterval: TimeInterval = 0.1

    init() {
        do {
            choices = try TouchLayoutCatalog.builtIn()
            loadError = nil
        } catch {
            choices = []
            loadError = error.localizedDescription
        }
        selectedID = choices.first?.id ?? ""
        input.onInput = { [weak self] frame in self?.record(frame) }
        if let choice = choices.first { input.load(choice.layout) }
    }

    var selected: TouchLayoutChoice? { choices.first { $0.id == selectedID } }
    var frames: [TouchInputFrame] { buffer.elements }

    func select(_ id: String) {
        guard id != selectedID, let choice = choices.first(where: { $0.id == id }) else { return }
        input.load(choice.layout)
        selectedID = id
        clear()
    }

    func clear() {
        pendingRefresh?.cancel()
        pendingRefresh = nil
        buffer.removeAll()
        eventBuffer.removeAll()
        input.cancelAll(reason: "recorderCleared")
        input.resetDiagnostics()
        presentationCount = 0
        refreshPresentation()
    }

    func export() throws -> Data {
        struct Export: Encodable {
            let version: Int
            let clock: String
            let layoutID: String
            let capability: TouchCapabilityReport?
            let evictedFrames: Int
            let frameCapacity: Int
            let metricWindowCapacity: Int
            let presentationInterval: TimeInterval
            let presentationCount: Int
            let sampling: TouchSamplingSummary
            let frames: [TouchInputFrame]
        }
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
        return try encoder.encode(
            Export(
                version: 2, clock: "systemUptimeSeconds", layoutID: selectedID,
                capability: selected?.layout.report, evictedFrames: buffer.evictedCount,
                frameCapacity: Self.maximumFrames, metricWindowCapacity: TouchTimingWindow.capacity,
                presentationInterval: Self.presentationInterval,
                presentationCount: presentationCount,
                sampling: input.diagnostics.summary, frames: frames))
    }

    /// 面板和字符串最多每 100ms 刷新一次，完整事件记录随输入即时入环。
    func refreshPresentation(now: TimeInterval = ProcessInfo.processInfo.systemUptime) {
        pendingRefresh?.cancel()
        pendingRefresh = nil
        lastRefresh = now
        presentationCount += 1
        latest = pendingLatest
        sampling = input.diagnostics.summary
        retainedFrameCount = buffer.count
        evictedFrameCount = buffer.evictedCount
        let titles = Dictionary(
            input.sockets.map { ($0.id, $0.control.title) },
            uniquingKeysWith: { first, _ in first })
        recentEvents = eventBuffer.elements.map { event in
            let title = event.controlID.flatMap { titles[$0] } ?? event.controlID ?? "空白"
            return "#\(event.pointerID) \(title) \(event.phase.rawValue)"
        }
        if pendingLatest?.events.isEmpty == true {
            recentEvents.append(pendingLatest?.reason ?? "")
            recentEvents = Array(recentEvents.suffix(8))
        }
    }

    private func record(_ frame: TouchInputFrame) {
        stateSink?(frame)
        buffer.append(frame)
        for event in frame.events { eventBuffer.append(event) }
        pendingLatest = frame
        guard pendingRefresh == nil else { return }
        let now = ProcessInfo.processInfo.systemUptime
        let delay = max(0, Self.presentationInterval - (now - lastRefresh))
        let work = DispatchWorkItem { [weak self] in
            MainActor.assumeIsolated { self?.refreshPresentation() }
        }
        pendingRefresh = work
        DispatchQueue.main.asyncAfter(deadline: .now() + delay, execute: work)
    }
}
