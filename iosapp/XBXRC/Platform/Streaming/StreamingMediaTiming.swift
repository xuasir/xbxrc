import Foundation

struct StreamingMediaTimingCounters: Equatable, Sendable {
    var jitterBufferDelaySeconds: Double?
    var jitterBufferEmittedCount: UInt64?
    var jitterBufferTargetDelaySeconds: Double?
    var jitterBufferMinimumDelaySeconds: Double?
    var totalDecodeTimeSeconds: Double?
    var framesDecoded: UInt64?
}

struct StreamingMediaTimingSnapshot: Equatable, Sendable {
    var totals = StreamingMediaTimingCounters()
    var windowMilliseconds: Double?
    var jitterBufferMilliseconds: Double?
    var jitterBufferTargetMilliseconds: Double?
    var jitterBufferMinimumMilliseconds: Double?
    var decodeMilliseconds: Double?

    func tracePayload(prefix: String = "") -> [String: IOSRuntimeTraceValue] {
        let fields: [String: IOSRuntimeTraceValue] = [
            "jitterBufferDelaySeconds": totals.jitterBufferDelaySeconds.map(
                IOSRuntimeTraceValue.double) ?? .null,
            "jitterBufferEmittedCount": totals.jitterBufferEmittedCount.map {
                .integer(Int64(clamping: $0))
            } ?? .null,
            "jitterBufferTargetDelaySeconds": totals.jitterBufferTargetDelaySeconds.map(
                IOSRuntimeTraceValue.double) ?? .null,
            "jitterBufferMinimumDelaySeconds": totals.jitterBufferMinimumDelaySeconds.map(
                IOSRuntimeTraceValue.double) ?? .null,
            "totalDecodeTimeSeconds": totals.totalDecodeTimeSeconds.map(IOSRuntimeTraceValue.double)
                ?? .null,
            "timingWindowMs": windowMilliseconds.map(IOSRuntimeTraceValue.double) ?? .null,
            "jitterBufferMs": jitterBufferMilliseconds.map(IOSRuntimeTraceValue.double) ?? .null,
            "jitterBufferTargetMs": jitterBufferTargetMilliseconds.map(IOSRuntimeTraceValue.double)
                ?? .null,
            "jitterBufferMinimumMs": jitterBufferMinimumMilliseconds.map(
                IOSRuntimeTraceValue.double) ?? .null,
            "decodeMs": decodeMilliseconds.map(IOSRuntimeTraceValue.double) ?? .null,
        ]
        guard !prefix.isEmpty else { return fields }
        return Dictionary(
            uniqueKeysWithValues: fields.map { key, value in
                (prefix + key.prefix(1).uppercased() + key.dropFirst(), value)
            })
    }
}

/// 每个 RTP 流独立对照累计计数，流替换、缺失和回退重新建立窗口。
struct StreamingMediaTimingTracker: Sendable {
    private var previous: (at: Double, streams: [String: StreamingMediaTimingCounters])?

    mutating func observe(
        streams: [String: StreamingMediaTimingCounters], atMilliseconds: Double
    ) -> StreamingMediaTimingSnapshot {
        guard atMilliseconds.isFinite, atMilliseconds >= 0 else {
            previous = nil
            return .init()
        }
        let totals = StreamingMediaTimingCounters(
            jitterBufferDelaySeconds: sum(streams, \.jitterBufferDelaySeconds),
            jitterBufferEmittedCount: sum(streams, \.jitterBufferEmittedCount),
            jitterBufferTargetDelaySeconds: sum(streams, \.jitterBufferTargetDelaySeconds),
            jitterBufferMinimumDelaySeconds: sum(streams, \.jitterBufferMinimumDelaySeconds),
            totalDecodeTimeSeconds: sum(streams, \.totalDecodeTimeSeconds),
            framesDecoded: sum(streams, \.framesDecoded)
        )
        let baseline = previous
        previous = (atMilliseconds, streams)
        guard let baseline, !streams.isEmpty,
            streams.keys.sorted() == baseline.streams.keys.sorted(),
            atMilliseconds > baseline.at, atMilliseconds - baseline.at <= 1_500
        else { return .init(totals: totals) }

        func mean(
            _ duration: KeyPath<StreamingMediaTimingCounters, Double?>,
            _ count: KeyPath<StreamingMediaTimingCounters, UInt64?>
        ) -> Double? {
            var durationDelta = 0.0
            var countDelta = 0.0
            for (id, current) in streams {
                guard let old = baseline.streams[id],
                    let currentDuration = current[keyPath: duration],
                    let oldDuration = old[keyPath: duration],
                    currentDuration.isFinite, oldDuration.isFinite,
                    oldDuration >= 0, currentDuration >= oldDuration,
                    let currentCount = current[keyPath: count],
                    let oldCount = old[keyPath: count], currentCount >= oldCount
                else { return nil }
                durationDelta += currentDuration - oldDuration
                countDelta += Double(currentCount - oldCount)
            }
            guard countDelta > 0 else { return nil }
            let value = durationDelta * 1_000 / countDelta
            return value.isFinite ? value : nil
        }
        return StreamingMediaTimingSnapshot(
            totals: totals,
            windowMilliseconds: atMilliseconds - baseline.at,
            jitterBufferMilliseconds: mean(\.jitterBufferDelaySeconds, \.jitterBufferEmittedCount),
            jitterBufferTargetMilliseconds: mean(
                \.jitterBufferTargetDelaySeconds, \.jitterBufferEmittedCount),
            jitterBufferMinimumMilliseconds: mean(
                \.jitterBufferMinimumDelaySeconds, \.jitterBufferEmittedCount),
            decodeMilliseconds: mean(\.totalDecodeTimeSeconds, \.framesDecoded)
        )
    }

    private func sum(
        _ streams: [String: StreamingMediaTimingCounters],
        _ key: KeyPath<StreamingMediaTimingCounters, Double?>
    ) -> Double? {
        guard !streams.isEmpty else { return nil }
        var total = 0.0
        for stream in streams.values {
            guard let value = stream[keyPath: key], value.isFinite, value >= 0 else { return nil }
            total += value
        }
        return total.isFinite ? total : nil
    }

    private func sum(
        _ streams: [String: StreamingMediaTimingCounters],
        _ key: KeyPath<StreamingMediaTimingCounters, UInt64?>
    ) -> UInt64? {
        guard !streams.isEmpty else { return nil }
        var total: UInt64 = 0
        for stream in streams.values {
            guard let value = stream[keyPath: key] else { return nil }
            let addition = total.addingReportingOverflow(value)
            guard !addition.overflow else { return nil }
            total = addition.partialValue
        }
        return total
    }
}
