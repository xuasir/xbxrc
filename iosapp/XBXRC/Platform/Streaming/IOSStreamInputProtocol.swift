import Foundation

struct IOSStreamGamepadState: Equatable, Sendable {
    var buttonMask: UInt16 = 0
    var leftThumbX: Float = 0
    var leftThumbY: Float = 0
    var rightThumbX: Float = 0
    var rightThumbY: Float = 0
    var leftTrigger: Float = 0
    var rightTrigger: Float = 0

    static let neutral = IOSStreamGamepadState()
}

enum IOSStreamInputPacketEncoder {
    static func encodeGamepad(
        sequence: UInt32,
        timestampMilliseconds: Double,
        gamepadIndex: UInt8 = 0,
        state: IOSStreamGamepadState
    ) -> Data {
        var data = Data()
        data.appendUInt16LE(2)
        data.appendUInt32LE(sequence)
        data.appendFloat64LE(max(0, timestampMilliseconds))
        data.append(1)
        data.append(gamepadIndex)
        data.appendUInt16LE(state.buttonMask)
        data.appendInt16LE(normalizeAxis(state.leftThumbX))
        data.appendInt16LE(normalizeAxis(state.leftThumbY))
        data.appendInt16LE(normalizeAxis(state.rightThumbX))
        data.appendInt16LE(normalizeAxis(state.rightThumbY))
        data.appendUInt16LE(normalizeTrigger(state.leftTrigger))
        data.appendUInt16LE(normalizeTrigger(state.rightTrigger))
        data.appendUInt32LE(0)
        data.appendUInt32BE(0)
        return data
    }

    private static func normalizeAxis(_ value: Float) -> Int16 {
        Int16((Double(value.clamped(to: -1 ... 1)) * 32_767).rounded())
    }

    private static func normalizeTrigger(_ value: Float) -> UInt16 {
        UInt16((Double(value.clamped(to: 0 ... 1)) * 65_535).rounded())
    }
}

struct IOSStreamInputSendGate: Equatable, Sendable {
    static let highWatermarkBytes: UInt64 = 1_024
    static let lowWatermarkBytes: UInt64 = 512
    static let idleKeepaliveMilliseconds: Double = 250

    private(set) var blocked = false
    private(set) var lastSentState: IOSStreamGamepadState?
    private(set) var lastSentAtMilliseconds: Double = 0

    mutating func shouldSend(
        state: IOSStreamGamepadState,
        bufferedAmount: UInt64,
        nowMilliseconds: Double,
        force: Bool = false
    ) -> Bool {
        if force {
            blocked = false
            return true
        }
        if blocked {
            if bufferedAmount > Self.lowWatermarkBytes {
                return false
            }
            blocked = false
        } else if bufferedAmount >= Self.highWatermarkBytes {
            blocked = true
            return false
        }

        let changed = lastSentState != state
        let keepaliveDue = lastSentState != nil
            && nowMilliseconds - lastSentAtMilliseconds >= Self.idleKeepaliveMilliseconds
        return changed || keepaliveDue
    }

    mutating func markSent(state: IOSStreamGamepadState, at nowMilliseconds: Double) {
        lastSentState = state
        lastSentAtMilliseconds = nowMilliseconds
    }

    mutating func reset() {
        self = IOSStreamInputSendGate()
    }
}

struct IOSStreamInputDrainPolicy: Equatable, Sendable {
    static let standard = IOSStreamInputDrainPolicy(
        lowWatermarkBytes: IOSStreamInputSendGate.lowWatermarkBytes,
        maximumWaitMilliseconds: 64,
        pollIntervalMilliseconds: 8,
        postNeutralGraceMilliseconds: 16
    )

    let lowWatermarkBytes: UInt64
    let maximumWaitMilliseconds: Int
    let pollIntervalMilliseconds: Int
    let postNeutralGraceMilliseconds: Int

    var maximumTotalMilliseconds: Int {
        maximumWaitMilliseconds + postNeutralGraceMilliseconds
    }

    func shouldEnqueueNeutral(
        bufferedAmount: UInt64,
        elapsedMilliseconds: Int
    ) -> Bool {
        bufferedAmount <= lowWatermarkBytes
            || elapsedMilliseconds >= maximumWaitMilliseconds
    }
}

enum IOSStreamNeutralFrameOutcome: String, Equatable, Sendable {
    case sent
    case sendFailed
    case inputUnavailable
}

enum IOSStreamRumbleWireFormat: String, Equatable, Sendable {
    case betterXcloud
    case legacy
}

struct IOSStreamRumbleEffect: Equatable, Sendable {
    let gamepadIndex: UInt8?
    let strongMagnitude: Float
    let weakMagnitude: Float
    let leftTrigger: Float
    let rightTrigger: Float
    let durationMilliseconds: UInt16
    let format: IOSStreamRumbleWireFormat

    var isStop: Bool {
        durationMilliseconds == 0
            || strongMagnitude == 0
            && weakMagnitude == 0
            && leftTrigger == 0
            && rightTrigger == 0
    }
}

enum IOSStreamRumblePacketDecoder {
    private static let reportType: UInt16 = 128

    static func decode(_ data: Data) -> [IOSStreamRumbleEffect] {
        let bytes = [UInt8](data)
        if let effect = decodeBetterXcloud(bytes) {
            return [effect]
        }
        return decodeLegacy(bytes)
    }

    private static func decodeBetterXcloud(_ bytes: [UInt8]) -> IOSStreamRumbleEffect? {
        guard bytes.count >= 13 else { return nil }
        let v8Type = UInt16(bytes[0]) | UInt16(bytes[1]) << 8
        let messageTypeSize = v8Type & reportType != 0 ? 2 : 1
        let messageType = messageTypeSize == 2 ? v8Type : UInt16(bytes[0])
        guard messageType & reportType != 0 else { return nil }

        var offset = messageTypeSize
        guard bytes.indices.contains(offset), bytes[offset] == 0 else { return nil }
        offset += 1
        guard offset + 6 < bytes.count else { return nil }

        return IOSStreamRumbleEffect(
            gamepadIndex: normalizedGamepadIndex(bytes[offset]),
            strongMagnitude: percent(bytes[offset + 1]),
            weakMagnitude: percent(bytes[offset + 2]),
            leftTrigger: percent(bytes[offset + 3]),
            rightTrigger: percent(bytes[offset + 4]),
            durationMilliseconds: UInt16(bytes[offset + 5]) | UInt16(bytes[offset + 6]) << 8,
            format: .betterXcloud
        )
    }

    private static func decodeLegacy(_ bytes: [UInt8]) -> [IOSStreamRumbleEffect] {
        guard bytes.count >= 14, bytes[0] == UInt8(reportType) else { return [] }
        var effects: [IOSStreamRumbleEffect] = []
        var offset = 2
        while offset + 11 < bytes.count {
            effects.append(
                IOSStreamRumbleEffect(
                    gamepadIndex: normalizedGamepadIndex(bytes[offset + 1]),
                    strongMagnitude: legacyMotor(bytes, offset: offset + 8),
                    weakMagnitude: legacyMotor(bytes, offset: offset + 6),
                    leftTrigger: legacyMotor(bytes, offset: offset + 2),
                    rightTrigger: legacyMotor(bytes, offset: offset + 4),
                    durationMilliseconds: UInt16(bytes[offset + 10])
                        | UInt16(bytes[offset + 11]) << 8,
                    format: .legacy
                )
            )
            offset += 12
        }
        return effects
    }

    private static func percent(_ value: UInt8) -> Float {
        Float(min(value, 100)) / 100
    }

    private static func legacyMotor(_ bytes: [UInt8], offset: Int) -> Float {
        let raw = UInt16(bytes[offset]) | UInt16(bytes[offset + 1]) << 8
        return min(1, Float(raw) / 1_023)
    }

    private static func normalizedGamepadIndex(_ value: UInt8) -> UInt8? {
        value < 4 ? value : nil
    }
}

enum IOSStreamHapticsRoute: String, CaseIterable, Equatable, Hashable, Sendable {
    case leftHandle
    case rightHandle
    case leftTrigger
    case rightTrigger
    case handles
    case triggers
    case defaultLocality
}

struct IOSStreamHapticsPulse: Equatable, Sendable {
    let route: IOSStreamHapticsRoute
    let intensity: Float
    let sharpness: Float
    let durationMilliseconds: UInt16
}

struct IOSStreamHapticsTransition: Equatable, Sendable {
    let routesToStop: Set<IOSStreamHapticsRoute>
    let pulsesToPlay: [IOSStreamHapticsPulse]
}

enum IOSStreamHapticsTransitionPlanner {
    static func plan(
        activeRoutes: Set<IOSStreamHapticsRoute>,
        nextPulses: [IOSStreamHapticsPulse],
        stopAll: Bool = false
    ) -> IOSStreamHapticsTransition {
        let pulses = stopAll ? [] : nextPulses
        let nextRoutes = Set(pulses.map(\.route))
        return IOSStreamHapticsTransition(
            routesToStop: stopAll ? activeRoutes : activeRoutes.subtracting(nextRoutes),
            pulsesToPlay: pulses
        )
    }
}

enum IOSStreamHapticsRouter {
    static func pulses(
        for effect: IOSStreamRumbleEffect,
        supportedRoutes: Set<IOSStreamHapticsRoute>
    ) -> [IOSStreamHapticsPulse] {
        guard !effect.isStop else { return [] }
        let duration = max(10, effect.durationMilliseconds)
        var pulses: [IOSStreamHapticsPulse] = []

        appendMotor(
            magnitude: effect.strongMagnitude,
            preferred: .leftHandle,
            grouped: .handles,
            sharpness: 0.2,
            duration: duration,
            supportedRoutes: supportedRoutes,
            pulses: &pulses
        )
        appendMotor(
            magnitude: effect.weakMagnitude,
            preferred: .rightHandle,
            grouped: .handles,
            sharpness: 0.8,
            duration: duration,
            supportedRoutes: supportedRoutes,
            pulses: &pulses
        )
        appendMotor(
            magnitude: effect.leftTrigger,
            preferred: .leftTrigger,
            grouped: .triggers,
            sharpness: 0.55,
            duration: duration,
            supportedRoutes: supportedRoutes,
            pulses: &pulses
        )
        appendMotor(
            magnitude: effect.rightTrigger,
            preferred: .rightTrigger,
            grouped: .triggers,
            sharpness: 0.65,
            duration: duration,
            supportedRoutes: supportedRoutes,
            pulses: &pulses
        )

        return coalesced(pulses)
    }

    private static func appendMotor(
        magnitude: Float,
        preferred: IOSStreamHapticsRoute,
        grouped: IOSStreamHapticsRoute,
        sharpness: Float,
        duration: UInt16,
        supportedRoutes: Set<IOSStreamHapticsRoute>,
        pulses: inout [IOSStreamHapticsPulse]
    ) {
        guard magnitude > 0 else { return }
        let route: IOSStreamHapticsRoute
        if supportedRoutes.contains(preferred) {
            route = preferred
        } else if supportedRoutes.contains(grouped) {
            route = grouped
        } else if supportedRoutes.contains(.defaultLocality) {
            route = .defaultLocality
        } else {
            return
        }
        pulses.append(
            IOSStreamHapticsPulse(
                route: route,
                intensity: magnitude.clamped(to: 0 ... 1),
                sharpness: sharpness,
                durationMilliseconds: duration
            )
        )
    }

    private static func coalesced(_ pulses: [IOSStreamHapticsPulse]) -> [IOSStreamHapticsPulse] {
        var result: [IOSStreamHapticsPulse] = []
        for pulse in pulses {
            if let index = result.firstIndex(where: { $0.route == pulse.route }) {
                let current = result[index]
                result[index] = IOSStreamHapticsPulse(
                    route: pulse.route,
                    intensity: max(current.intensity, pulse.intensity),
                    sharpness: max(current.sharpness, pulse.sharpness),
                    durationMilliseconds: max(
                        current.durationMilliseconds,
                        pulse.durationMilliseconds
                    )
                )
            } else {
                result.append(pulse)
            }
        }
        return result
    }
}

private extension Float {
    func clamped(to range: ClosedRange<Float>) -> Float {
        min(range.upperBound, max(range.lowerBound, self))
    }
}

private extension Data {
    mutating func appendUInt16LE(_ value: UInt16) {
        append(contentsOf: Swift.withUnsafeBytes(of: value.littleEndian, Array.init))
    }

    mutating func appendUInt32LE(_ value: UInt32) {
        append(contentsOf: Swift.withUnsafeBytes(of: value.littleEndian, Array.init))
    }

    mutating func appendUInt32BE(_ value: UInt32) {
        append(contentsOf: Swift.withUnsafeBytes(of: value.bigEndian, Array.init))
    }

    mutating func appendInt16LE(_ value: Int16) {
        append(contentsOf: Swift.withUnsafeBytes(of: value.littleEndian, Array.init))
    }

    mutating func appendFloat64LE(_ value: Double) {
        append(contentsOf: Swift.withUnsafeBytes(of: value.bitPattern.littleEndian, Array.init))
    }
}
