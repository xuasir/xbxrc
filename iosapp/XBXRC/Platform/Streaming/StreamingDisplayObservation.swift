import Foundation

struct StreamingDisplayObservation: Encodable, Sendable {
    let source: String
    let active: Bool
    let surfaceReady: Bool
    let renderEpoch: UInt64
    let surfaceRevision: UInt64
    let sampledAtMs: UInt64
    let submitted: UInt64
    let gpuCompleted: UInt64
    let displayed: UInt64
    let lastSubmitAtMs: Double?
    let lastCompletedAtMs: Double?
    let lastDisplayedAtMs: Double?
    let firstDisplayedAtMs: Double?
    let lastFrameId: UInt64?
    let drawableAcquireAgeMs: Double?
    let inflightAgeMs: Double?
    let frameIntervalMs: Double?
    let submitToPresentP95Ms: Double?
    let submitToPresentP99Ms: Double?

    init(_ snapshot: RuntimeObservationSnapshot, now: Double) {
        let health = snapshot.health
        source = health.presentationTimestampSource
        active = snapshot.active
        surfaceReady = snapshot.surfaceReady
        renderEpoch = snapshot.scope.renderEpoch
        surfaceRevision = snapshot.scope.surfaceRevision
        sampledAtMs = UInt64(max(0, snapshot.sampledAtMs))
        submitted = UInt64(max(0, health.submitCount))
        gpuCompleted = UInt64(max(0, health.presentCount))
        displayed = UInt64(max(0, health.displayedCount))
        lastSubmitAtMs = health.lastSubmitAtMilliseconds
        lastCompletedAtMs = health.lastPresentAtMilliseconds
        lastDisplayedAtMs = health.lastDisplayedAtMs
        firstDisplayedAtMs = health.firstDisplayedAtMs
        lastFrameId = health.lastDisplayedFrameID
        drawableAcquireAgeMs = health.drawableAcquireStartedAtMs.map { max(0, now - $0) }
        inflightAgeMs = health.oldestInflightAtMs.map { max(0, now - $0) }
        frameIntervalMs = health.frameIntervalMilliseconds
        submitToPresentP95Ms =
            source == "drawablePresented"
            ? health.submitToDrawableP95Ms : health.submitToPresentP95Milliseconds
        submitToPresentP99Ms =
            source == "drawablePresented"
            ? health.submitToDrawableP99Ms : health.submitToPresentP99Milliseconds
    }
}

struct StreamingDisplayRecoveryAction: Codable, Equatable, Sendable {
    let id: UInt64
    let renderEpoch: UInt64
    let surfaceRevision: UInt64
    let observedAtMs: UInt64
    let reason: String
}

struct StreamingDisplayRecoveryCompletion: Encodable, Sendable {
    let id: UInt64
    let result: String
    let completedAtMs: UInt64
    let renderEpoch: UInt64
}

struct StreamingDisplayRecoveryDecision: Decodable, Equatable, Sendable {
    let eligible: Bool
    let active: Bool
    let exhausted: Bool
    let reason: String
    let ageMs: UInt64
    let coverageMs: UInt64
    let thresholdMs: UInt64
    let action: StreamingDisplayRecoveryAction?
    let effect: String?
}
