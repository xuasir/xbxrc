import Foundation

enum TouchSampleBatch {
    /// UIKit 的历史副本统一复用原始触点身份；终止边沿始终位于该指针末尾。
    static func coalescing(
        current: TouchPointerSample, history: [TouchPointerSample]
    ) -> [TouchPointerSample] {
        guard current.phase != .began else { return [current] }
        let earlier = history.filter { point in
            guard let source = point.sourceTimestamp, source.isFinite,
                let latest = current.sourceTimestamp, latest.isFinite
            else { return false }
            return source < latest
        }.map { point in
            TouchPointerSample(
                id: current.id, phase: .moved, position: point.position,
                pressure: point.pressure, sourceTimestamp: point.sourceTimestamp,
                isCoalesced: true)
        }
        return ordered(earlier) + [current]
    }

    static func ordered(_ samples: [TouchPointerSample]) -> [TouchPointerSample] {
        samples.enumerated().sorted { left, right in
            let lhs = left.element.sourceTimestamp.flatMap { $0.isFinite ? $0 : nil } ?? .infinity
            let rhs = right.element.sourceTimestamp.flatMap { $0.isFinite ? $0 : nil } ?? .infinity
            return lhs == rhs ? left.offset < right.offset : lhs < rhs
        }.map(\.element)
    }
}
