import Foundation
import QuartzCore

/// 轨道绑定独占生产租约；呈现重置继续消费同一轨道的新帧。
final class StreamingVideoIngressLease: Sendable {
    private let fence = StreamingExecutionFence()
    var isValid: Bool { fence.accepts(0) }
    func invalidate() { fence.advance() }
}

/// 图层树与配置归 MainActor；RenderQueue 仅通过有效租约获取和呈现 drawable。
struct PresentationSurfaceLease: @unchecked Sendable {
    let layer: CAMetalLayer
    let revision: UInt64
    let pixelSize: CGSize
    let fence: StreamingExecutionFence
    let fenceRevision: UInt64
    var attachedToWindow = true

    var isValid: Bool { fence.accepts(fenceRevision) }
    var isDrawableReady: Bool { isValid && attachedToWindow }
}

struct RuntimeObservationSnapshot: Sendable {
    let scope: StreamExecutionScope
    let sampledAtMs: Double
    let active: Bool
    let surfaceReady: Bool
    let health: StreamingDisplayHealthSnapshot
}

enum StreamingPresentationResetOutcome: Equatable, Sendable {
    case acknowledged, superseded, failed
}

struct StreamingFrameTraceFact: Sendable {
    let event: String
    let context: StreamingPresentationTraceContext
    let frameID: UInt64
    let atMs: Double
    var count: Int64 = 0
    var timestampSource: String = "renderQueue"
    var durationMs: Double? = nil
    var reason: String? = nil
}

/// 会话直接读取累计事实；诊断与 UI 的调度分别消费这个快照。
final class StreamingPresentationSession: Sendable {
    private let lock = NSLock()
    private nonisolated(unsafe) var observation: RuntimeObservationSnapshot?
    private nonisolated(unsafe) var resetHandler:
        (@Sendable (StreamExecutionScope, UInt64) async -> StreamingPresentationResetOutcome)?
    private nonisolated(unsafe) var facts = StreamingBoundedRing<StreamingFrameTraceFact>(
        capacity: 64)
    private nonisolated(unsafe) var droppedFacts: UInt64 = 0
    private nonisolated(unsafe) var resetActionID: UInt64?
    private nonisolated(unsafe) var nextRenderEpoch: UInt64 = 0

    var snapshot: RuntimeObservationSnapshot? { lock.withLock { observation } }

    func reserveRenderEpoch() -> UInt64 {
        lock.withLock {
            nextRenderEpoch &+= 1
            return nextRenderEpoch
        }
    }

    func bind(
        scope: StreamExecutionScope, active: Bool, surfaceReady: Bool,
        reset:
            @escaping @Sendable (StreamExecutionScope, UInt64) async ->
            StreamingPresentationResetOutcome
    ) {
        lock.withLock {
            resetHandler = reset
            observation = .init(
                scope: scope, sampledAtMs: StreamingFrameFeedbackClock.nowMilliseconds,
                active: active, surfaceReady: surfaceReady, health: observation?.health ?? .zero)
        }
    }

    func publish(scope: StreamExecutionScope, health: StreamingDisplayHealthSnapshot) {
        lock.withLock {
            guard let current = observation, current.scope == scope else { return }
            if current.health.renderEpoch == scope.renderEpoch,
                health.sampledAtMs < current.health.sampledAtMs
                    || health.submitCount < current.health.submitCount
                    || health.presentCount < current.health.presentCount
            {
                return
            }
            observation = .init(
                scope: scope, sampledAtMs: StreamingFrameFeedbackClock.nowMilliseconds,
                active: current.active, surfaceReady: current.surfaceReady, health: health)
        }
    }

    func setActive(_ active: Bool) {
        lock.withLock {
            guard let current = observation else { return }
            observation = .init(
                scope: current.scope, sampledAtMs: StreamingFrameFeedbackClock.nowMilliseconds,
                active: active, surfaceReady: current.surfaceReady, health: current.health)
        }
    }

    func setSurfaceReady(_ ready: Bool) {
        lock.withLock {
            guard let current = observation else { return }
            observation = .init(
                scope: current.scope, sampledAtMs: StreamingFrameFeedbackClock.nowMilliseconds,
                active: current.active, surfaceReady: ready, health: current.health)
        }
    }

    func reset(expected scope: StreamExecutionScope, actionID: UInt64) async
        -> StreamingPresentationResetOutcome
    {
        let handler = lock.withLock {
            guard observation?.scope == scope else {
                return nil
                    as (
                        @Sendable (StreamExecutionScope, UInt64) async ->
                            StreamingPresentationResetOutcome
                    )?
            }
            resetActionID = actionID
            return resetHandler
        }
        let result = await handler?(scope, actionID) ?? .superseded
        return lock.withLock {
            guard resetActionID == actionID else { return .superseded }
            resetActionID = nil
            return result
        }
    }

    func acceptsReset(_ actionID: UInt64) -> Bool { lock.withLock { resetActionID == actionID } }
    func cancelReset(_ actionID: UInt64) {
        lock.withLock { if resetActionID == actionID { resetActionID = nil } }
    }

    func record(_ fact: StreamingFrameTraceFact) {
        lock.withLock { if facts.append(fact) != nil { droppedFacts &+= 1 } }
    }

    func takeFacts() -> (facts: [StreamingFrameTraceFact], dropped: UInt64) {
        lock.withLock {
            let batch = facts.values()
            facts.removeAll()
            let discarded = droppedFacts
            droppedFacts = 0
            return (batch, discarded)
        }
    }
}
