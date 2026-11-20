import Foundation

/// 固定容量记录；只在展示或导出时生成按时间排列的数组。
struct TouchRingBuffer<Element> {
    let capacity: Int
    private var storage: [Element?]
    private var head = 0
    private(set) var count = 0
    private(set) var evictedCount = 0

    init(capacity: Int) {
        precondition(capacity > 0)
        self.capacity = capacity
        storage = Array(repeating: nil, count: capacity)
    }

    mutating func append(_ element: Element) {
        if count == capacity {
            storage[head] = element
            head = (head + 1) % capacity
            evictedCount += 1
        } else {
            storage[(head + count) % capacity] = element
            count += 1
        }
    }

    var elements: [Element] {
        (0..<count).compactMap { storage[(head + $0) % capacity] }
    }

    mutating func removeAll() { self = .init(capacity: capacity) }
}

struct TouchTimingSummary: Codable {
    let totalCount: Int
    let windowCount: Int
    let meanMilliseconds: Double
    let p95Milliseconds: Double
    let p99Milliseconds: Double
    let maximumMilliseconds: Double

    var frequencyHz: Double { meanMilliseconds > 0 ? 1000 / meanMilliseconds : 0 }
}

struct TouchTimingWindow {
    static let capacity = 4096
    private var values = TouchRingBuffer<Double>(capacity: Self.capacity)
    private(set) var totalCount = 0

    mutating func record(_ seconds: TimeInterval) {
        guard seconds.isFinite, seconds >= 0 else { return }
        totalCount += 1
        values.append(seconds * 1000)
    }

    var summary: TouchTimingSummary {
        let sorted = values.elements.sorted()
        func percentile(_ fraction: Double) -> Double {
            guard !sorted.isEmpty else { return 0 }
            return sorted[max(0, Int(ceil(Double(sorted.count) * fraction)) - 1)]
        }
        return .init(
            totalCount: totalCount, windowCount: sorted.count,
            meanMilliseconds: sorted.isEmpty ? 0 : sorted.reduce(0, +) / Double(sorted.count),
            p95Milliseconds: percentile(0.95), p99Milliseconds: percentile(0.99),
            maximumMilliseconds: sorted.last ?? 0)
    }
}

struct TouchPointerTimingSummary: Codable {
    let pointerID: Int
    let sampleCount: Int
    let active: Bool
    let movementIntervals: TouchTimingSummary
}

struct TouchSamplingSummary: Codable {
    let callbackCount: Int
    let offeredSamples: Int
    let acceptedSamples: Int
    let coalescedSamples: Int
    let beganCount: Int
    let endedCount: Int
    let cancelledCount: Int
    let synthesizedCancelCount: Int
    let duplicateSamples: Int
    let outOfOrderSamples: Int
    let invalidSamples: Int
    let orphanSamples: Int
    let pointerLimitSamples: Int
    let stateChangeCount: Int
    let renderCount: Int
    let callbackIntervals: TouchTimingSummary
    let movementIntervals: TouchTimingSummary
    let stateChangeIntervals: TouchTimingSummary
    let renderIntervals: TouchTimingSummary
    let sourceToReceipt: TouchTimingSummary
    let sourceToState: TouchTimingSummary
    let processing: TouchTimingSummary
    let pointers: [TouchPointerTimingSummary]
}

/// 输入热路径只更新计数与有界窗口，分位数排序留给诊断刷新。
struct TouchInputDiagnostics {
    private final class Pointer {
        var sampleCount = 0
        var active = true
        var lastSource: TimeInterval?
        var intervals = TouchTimingWindow()
        let order: Int
        init(order: Int) { self.order = order }
    }

    private(set) var callbackCount = 0
    private(set) var offeredSamples = 0
    private(set) var acceptedSamples = 0
    private(set) var coalescedSamples = 0
    private(set) var beganCount = 0
    private(set) var endedCount = 0
    private(set) var cancelledCount = 0
    private(set) var synthesizedCancelCount = 0
    var duplicateSamples = 0
    var outOfOrderSamples = 0
    var invalidSamples = 0
    var orphanSamples = 0
    var pointerLimitSamples = 0
    private(set) var stateChangeCount = 0
    private(set) var renderCount = 0
    private var callbackIntervals = TouchTimingWindow()
    private var movementIntervals = TouchTimingWindow()
    private var stateChangeIntervals = TouchTimingWindow()
    private var renderIntervals = TouchTimingWindow()
    private var sourceToReceipt = TouchTimingWindow()
    private var sourceToState = TouchTimingWindow()
    private var processing = TouchTimingWindow()
    private var lastCallback: TimeInterval?
    private var lastStateChange: TimeInterval?
    private var lastRender: TimeInterval?
    private var pointers: [Int: Pointer] = [:]

    mutating func receive(sampleCount: Int, at timestamp: TimeInterval) {
        callbackCount += 1
        offeredSamples += sampleCount
        if let lastCallback { callbackIntervals.record(timestamp - lastCallback) }
        lastCallback = timestamp
    }

    mutating func observe(
        _ event: TouchPointerEvent, receivedAt: TimeInterval, stateCompletedAt: TimeInterval,
        synthesized: Bool = false
    ) {
        if synthesized { synthesizedCancelCount += 1 } else { acceptedSamples += 1 }
        if event.isCoalesced { coalescedSamples += 1 }
        switch event.phase {
        case .began:
            beganCount += 1
            // 只保留最近十二个操作指针的诊断，活动指针始终保留。
            if pointers[event.pointerID] == nil,
                pointers.count >= TouchInputAccumulator.maximumPointers,
                let oldest = pointers.filter({ !$0.value.active }).min(by: {
                    $0.value.order < $1.value.order
                })?.key
            {
                pointers.removeValue(forKey: oldest)
            }
            pointers[event.pointerID] = Pointer(order: acceptedSamples)
        case .ended: endedCount += 1
        case .cancelled: cancelledCount += 1
        case .moved: break
        }
        let pointer = pointers[event.pointerID] ?? Pointer(order: acceptedSamples)
        pointer.sampleCount += 1
        if let source = event.sourceTimestamp {
            sourceToReceipt.record(receivedAt - source)
            sourceToState.record(stateCompletedAt - source)
            if event.phase == .moved, let previous = pointer.lastSource, source > previous {
                pointer.intervals.record(source - previous)
                movementIntervals.record(source - previous)
            }
            pointer.lastSource = source
        }
        if event.phase == .ended || event.phase == .cancelled {
            pointer.active = false
            pointer.lastSource = nil
        }
        pointers[event.pointerID] = pointer
    }

    mutating func stateChanged(at timestamp: TimeInterval) {
        stateChangeCount += 1
        if let lastStateChange { stateChangeIntervals.record(timestamp - lastStateChange) }
        lastStateChange = timestamp
    }

    mutating func processed(duration: TimeInterval) { processing.record(duration) }

    mutating func rendered(at timestamp: TimeInterval) {
        renderCount += 1
        if let lastRender { renderIntervals.record(timestamp - lastRender) }
        lastRender = timestamp
    }

    var summary: TouchSamplingSummary {
        .init(
            callbackCount: callbackCount, offeredSamples: offeredSamples,
            acceptedSamples: acceptedSamples, coalescedSamples: coalescedSamples,
            beganCount: beganCount, endedCount: endedCount, cancelledCount: cancelledCount,
            synthesizedCancelCount: synthesizedCancelCount,
            duplicateSamples: duplicateSamples, outOfOrderSamples: outOfOrderSamples,
            invalidSamples: invalidSamples, orphanSamples: orphanSamples,
            pointerLimitSamples: pointerLimitSamples, stateChangeCount: stateChangeCount,
            renderCount: renderCount, callbackIntervals: callbackIntervals.summary,
            movementIntervals: movementIntervals.summary,
            stateChangeIntervals: stateChangeIntervals.summary,
            renderIntervals: renderIntervals.summary,
            sourceToReceipt: sourceToReceipt.summary, sourceToState: sourceToState.summary,
            processing: processing.summary,
            pointers: pointers.keys.sorted().compactMap { id in
                pointers[id].map {
                    .init(
                        pointerID: id, sampleCount: $0.sampleCount, active: $0.active,
                        movementIntervals: $0.intervals.summary)
                }
            })
    }
}
