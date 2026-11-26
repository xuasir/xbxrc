import Foundation

struct InputConfigurationSnapshot: Equatable, Sendable {
    let revision: UInt64
    let preferences: MobilePreferences
    let controllerID: String?
    let layoutKey: String
}

struct InputSampleTransition: Equatable, Sendable {
    let state: IOSStreamGamepadState
    let collectedAtMs: Double
    let sourceActive: Bool
}

struct InputSampleBatch: Sendable {
    let sourceID: String
    let sequence: UInt64
    let inputEpoch: UInt64
    let transitions: [InputSampleTransition]
    let finalState: IOSStreamGamepadState
    let sourceActive: Bool
}

struct StreamingControllerSample: Sendable {
    var raw: IOSStreamGamepadState = .neutral
    var buttons: [String: Float] = [:]
    var axes: [String: Float] = [:]
    var connected = false
    var collectedAtMs: Double?
}

struct StreamingInputCoreSnapshot: Sendable {
    let source: StreamInputSource
    let state: IOSStreamGamepadState
    let pendingEdges: Int
    let acceptedEdges: UInt64
    let discardedEdges: UInt64
    let barrierCount: UInt64
    let barrierReason: String?
}

/// 唯一网络输入算法。生产实例归 InputQueue，设置预览复用同一算法的独立实例。
struct StreamInputCore {
    struct Pending: Sendable {
        let state: IOSStreamGamepadState
        let collectedAtMs: Double
        let digital: Bool
        let barrier: Bool
    }

    static let maximumEdges = 128
    static let maximumEdgeAgeMs: Double = 64
    private var edges = StreamingBoundedRing<Pending>(capacity: maximumEdges)
    private var latest = IOSStreamGamepadState.neutral
    private var latestAtMs: Double = 0
    private var mappedPhysical = IOSStreamGamepadState.neutral
    private var transformedPhysical = IOSStreamGamepadState.neutral
    private var physicalNeutral = true
    private var physical = StreamingControllerSample()
    private var touch = IOSStreamGamepadState.neutral
    private var touchActive = false
    private var sourceSequences: [String: UInt64] = [:]
    private var configuration: InputConfigurationSnapshot?
    private var shortcuts = MobileShortcutMatcher()
    private var requiresNeutral = true
    private var requiresTouchNeutral = false
    private var suppressed = false
    private var barrierPending = true
    private(set) var epoch: UInt64 = 0
    private(set) var source: StreamInputSource = .idle
    private(set) var shortcutAction: MobileShortcutTarget?
    private var acceptedEdges: UInt64 = 0
    private var discardedEdges: UInt64 = 0
    private var barrierCount: UInt64 = 0
    private var barrierReason: String?
    var isSuppressed: Bool { suppressed }
    var needsSamplingTicks: Bool { !suppressed && transformedPhysical != .neutral }

    mutating func configure(_ value: InputConfigurationSnapshot, at now: Double) {
        guard configuration != value else { return }
        let old = configuration
        configuration = value
        normalizePhysical()
        if let old,
            old.controllerID != value.controllerID
                || old.preferences.gamepadBindings != value.preferences.gamepadBindings
                || old.preferences.shortcutButtons(for: .menu)
                    != value.preferences.shortcutButtons(for: .menu)
                || old.preferences.diagnosticsButtons != value.preferences.diagnosticsButtons
                || old.preferences.menuHoldDuration != value.preferences.menuHoldDuration
        {
            barrier(epoch: epoch, suppressed: suppressed, reason: "configurationChanged")
        }
        compose(at: now)
    }

    mutating func barrier(epoch: UInt64, suppressed: Bool, reason: String) {
        self.epoch = epoch
        self.suppressed = suppressed
        discardedEdges += UInt64(edges.count)
        edges.removeAll()
        sourceSequences.removeAll(keepingCapacity: true)
        latest = .neutral
        latestAtMs = 0
        touch = .neutral
        touchActive = false
        requiresNeutral = true
        requiresTouchNeutral = reason == "digitalEdgeCapacity" || reason == "digitalEdgeExpired"
        barrierPending = true
        shortcuts.reset()
        shortcutAction = nil
        barrierCount &+= 1
        barrierReason = reason
    }

    mutating func sampleController(_ value: StreamingControllerSample, at now: Double) {
        physical = value
        normalizePhysical()
        compose(at: value.collectedAtMs.map { $0.isFinite && $0 >= 0 ? min(now, $0) : now } ?? now)
    }

    mutating func consumeTouch(_ batch: InputSampleBatch, at now: Double) {
        guard batch.inputEpoch == epoch,
            sourceSequences[batch.sourceID].map({ batch.sequence > $0 }) ?? true
        else { return }
        sourceSequences[batch.sourceID] = batch.sequence
        guard !suppressed else { return }
        for transition in batch.transitions {
            touch = transition.state
            touchActive = transition.sourceActive || transition.state != .neutral
            if !touchActive { requiresTouchNeutral = false }
            let barriersBefore = barrierCount
            compose(at: min(now, transition.collectedAtMs))
            if barrierCount != barriersBefore { return }
        }
        touch = batch.finalState
        touchActive = batch.sourceActive || batch.finalState != .neutral
        if !touchActive { requiresTouchNeutral = false }
        compose(at: now)
    }

    mutating func advance(at now: Double) { compose(at: now) }

    mutating func candidate(at now: Double) -> Pending {
        if let first = edges.first, now - first.collectedAtMs > Self.maximumEdgeAgeMs {
            barrier(epoch: epoch, suppressed: suppressed, reason: "digitalEdgeExpired")
        }
        if barrierPending {
            return Pending(state: .neutral, collectedAtMs: now, digital: false, barrier: true)
        }
        return edges.first
            ?? Pending(state: latest, collectedAtMs: latestAtMs, digital: false, barrier: false)
    }

    mutating func accepted(_ value: Pending) {
        if value.barrier {
            barrierPending = false
        } else if value.digital {
            edges.removeFirst()
            acceptedEdges &+= 1
        }
    }

    var snapshot: StreamingInputCoreSnapshot {
        .init(
            source: source, state: latest, pendingEdges: edges.count, acceptedEdges: acceptedEdges,
            discardedEdges: discardedEdges, barrierCount: barrierCount, barrierReason: barrierReason
        )
    }

    private mutating func normalizePhysical() {
        let preferences = configuration?.preferences ?? MobilePreferences()
        mappedPhysical = MobileControlBinding.apply(
            physical.raw,
            bindings: preferences.gamepadBindings[configuration?.layoutKey ?? ""] ?? [:],
            buttons: physical.buttons, axes: physical.axes)
        transformedPhysical = IOSStreamStickTransform.apply(
            mappedPhysical, preferences: preferences)
        physicalNeutral =
            physical.buttons.values.allSatisfy { $0 <= 0.5 }
            && MobileShortcutButton.allCases.allSatisfy { !$0.isPressed(in: mappedPhysical) }
            && MobileControlAxis.allCases.allSatisfy { abs($0.value(in: physical.raw)) <= 0.15 }
            && MobileControlAxis.allCases.allSatisfy { abs($0.value(in: mappedPhysical)) <= 0.15 }
    }

    private mutating func compose(at now: Double) {
        let preferences = configuration?.preferences ?? MobilePreferences()
        if requiresNeutral {
            if physicalNeutral { requiresNeutral = false }
        }
        let shortcut =
            suppressed || requiresNeutral
            ? (captured: false, action: nil as MobileShortcutTarget?)
            : shortcuts.sample(mappedPhysical, preferences: preferences, now: now)
        if let action = shortcut.action { shortcutAction = action }
        let next: IOSStreamGamepadState
        if suppressed || requiresNeutral || shortcut.captured {
            next = .neutral
        } else if transformedPhysical != .neutral {
            source = .controller
            next = transformedPhysical
        } else if touchActive && !requiresTouchNeutral {
            source = .touch
            next = IOSStreamStickTransform.apply(touch, preferences: preferences)
        } else {
            source = physical.connected ? .controller : .idle
            next = .neutral
        }
        if Self.digitalKey(next) != Self.digitalKey(latest) {
            guard edges.count < Self.maximumEdges else {
                barrier(epoch: epoch, suppressed: suppressed, reason: "digitalEdgeCapacity")
                return
            }
            edges.append(Pending(state: next, collectedAtMs: now, digital: true, barrier: false))
        }
        if latest != next { latestAtMs = now }
        latest = next
    }

    mutating func takeShortcutAction() -> MobileShortcutTarget? {
        defer { shortcutAction = nil }
        return shortcutAction
    }

    private static func digitalKey(_ state: IOSStreamGamepadState) -> UInt32 {
        UInt32(state.buttonMask) | (state.leftTrigger > 0.5 ? 1 << 16 : 0)
            | (state.rightTrigger > 0.5 ? 1 << 17 : 0)
    }
}
