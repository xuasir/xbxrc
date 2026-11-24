import CoreGraphics
import Foundation

enum StreamingVideoContentMode: String, CaseIterable, Equatable, Sendable {
    case fit
    case fill
    case stretch, ratio16x9, ratio16x10, ratio18x9, ratio21x9, ratio4x3

    var title: String {
        switch self {
        case .fit: "适应"
        case .fill: "填充"
        case .stretch: "拉伸"
        case .ratio16x9: "16:9"
        case .ratio16x10: "16:10"
        case .ratio18x9: "18:9"
        case .ratio21x9: "21:9"
        case .ratio4x3: "4:3"
        }
    }
}

enum StreamingConnectionSummary: String, Equatable, Sendable {
    case good
    case unstable
    case recovering

    var title: String {
        switch self {
        case .good: "连接良好"
        case .unstable: "连接波动"
        case .recovering: "正在恢复"
        }
    }
}

struct StreamingControlCapabilities: Equatable, Sendable {
    let canMuteAudio: Bool
    let canSendText: Bool
    let canPowerOff: Bool

    static let unavailable = StreamingControlCapabilities(
        canMuteAudio: false,
        canSendText: false,
        canPowerOff: false
    )
}

struct StreamingSessionDiagnostics: Equatable, Sendable {
    let peerStats: StreamingPeerStats?
    let displayHealth: StreamingDisplayHealthSnapshot?
    let peer: StreamingPeerDebugSnapshot?
    let audioLifecycleState: String?
    let audioRoute: String?
    let audioTrackReady: Bool
    let updatedAtMilliseconds: Double
    var supervision: StreamingSupervisionDecision? = nil
    var metrics: StreamingSampleMetrics = .init()
    var connectedElapsedMilliseconds: Double? = nil
    var mediaReadyElapsedMilliseconds: Double? = nil
    var requestedVideoBitrateKbps: Int? = nil
    var negotiatedVideoBitrateKbps: Int? = nil

    static let empty = StreamingSessionDiagnostics(
        peerStats: nil,
        displayHealth: nil,
        peer: nil,
        audioLifecycleState: nil,
        audioRoute: nil,
        audioTrackReady: false,
        updatedAtMilliseconds: 0
    )

    var compactSummary: String {
        let rate =
            peerStats?.receiveBitrateBps.map { String(format: "%.1f Mbps", $0 / 1_000_000) }
            ?? "— Mbps"
        let rtt =
            peerStats?.roundTripTimeSeconds.map { String(format: "%.0f ms", $0 * 1_000) } ?? "— ms"
        let supply = peerStats?.frameSupplyDelta.map { "\($0) frames/Δ" } ?? "— frames/Δ"
        return "\(rate) · \(rtt) · \(supply)"
    }

    func connectionSummary(featureState: StreamingFeatureState) -> StreamingConnectionSummary {
        if featureState == .recovering { return .recovering }
        guard let peerStats else { return .good }
        let rttMilliseconds = peerStats.roundTripTimeSeconds.map { $0 * 1_000 } ?? 0
        let jitterMilliseconds = peerStats.jitterSeconds.map { $0 * 1_000 } ?? 0
        if rttMilliseconds >= 120 || jitterMilliseconds >= 30
            || peerStats.frameSupplyDelta.map({ $0 <= 0 }) == true
        {
            return .unstable
        }
        return .good
    }
}

enum StreamingControlsSurface: Equatable, Sendable {
    case hidden
    case dock
    case pictureSheet
    case debugSheet
    case touchLayoutPicker
    case sharing
    case confirmingExit
    case confirmingPowerOff
}

struct StreamingControlsState: Equatable, Sendable {
    private(set) var surface: StreamingControlsSurface = .hidden
    private var showsQuickControls = true

    var dockVisible: Bool { surface == .dock }
    var quickControlsVisible: Bool { surface == .hidden && showsQuickControls }
    var suspendsAutoHide: Bool { surface != .hidden && surface != .dock }

    mutating func reveal() { surface = .dock }

    mutating func toggleDock() {
        if surface == .hidden { reveal() } else { hide() }
    }

    mutating func present(_ next: StreamingControlsSurface) {
        surface = next
    }

    mutating func dismissPresentedSurface() {
        surface = .dock
    }

    mutating func revealQuickControls() {
        guard surface == .hidden else { return }
        showsQuickControls = true
    }

    mutating func hide() {
        surface = .hidden
        showsQuickControls = true
    }

    mutating func hideAutomatically() {
        surface = .hidden
        showsQuickControls = false
    }
}

struct StreamingInwardGestureSample: Equatable, Sendable {
    let leftStart: CGPoint
    let rightStart: CGPoint
    let leftEnd: CGPoint
    let rightEnd: CGPoint
    let startDeltaMilliseconds: Double
    let durationMilliseconds: Double
    let viewWidth: CGFloat
}

enum StreamingInwardGesturePolicy {
    static func recognizes(_ sample: StreamingInwardGestureSample) -> Bool {
        guard sample.viewWidth > 0,
            sample.startDeltaMilliseconds <= 120,
            sample.durationMilliseconds <= 450,
            sample.leftStart.x <= sample.viewWidth * 0.24,
            sample.rightStart.x >= sample.viewWidth * 0.76
        else { return false }

        let requiredTravel = min(max(sample.viewWidth * 0.09, 72), 120)
        let leftTravel = sample.leftEnd.x - sample.leftStart.x
        let rightTravel = sample.rightStart.x - sample.rightEnd.x
        let leftVerticalDrift = abs(sample.leftEnd.y - sample.leftStart.y)
        let rightVerticalDrift = abs(sample.rightEnd.y - sample.rightStart.y)
        return leftTravel >= requiredTravel
            && rightTravel >= requiredTravel
            && leftVerticalDrift <= 40
            && rightVerticalDrift <= 40
            && sample.leftEnd.x < sample.rightEnd.x
    }
}
