import Foundation

struct StreamingPlaybackReadiness: Equatable, Sendable {
    enum Fact: Hashable, Sendable {
        case peerConnected
        case firstVideoFrame
        case controlReady
        case displayPipelineReady
        case frameAcceptedToMailbox
        case frameSubmitted
        case framePresented
        case steadyMedia
    }

    private static let requiredFacts: Set<Fact> = [
        .peerConnected,
        .controlReady,
        .displayPipelineReady,
        .frameAcceptedToMailbox,
        .frameSubmitted,
        .framePresented,
    ]

    private(set) var facts: Set<Fact> = []

    var isPlaybackReady: Bool {
        facts.isSuperset(of: Self.requiredFacts)
    }

    func contains(_ fact: Fact) -> Bool {
        facts.contains(fact)
    }

    @discardableResult
    mutating func mark(_ fact: Fact) -> Bool {
        facts.insert(fact).inserted
    }

    mutating func remove(_ fact: Fact) {
        facts.remove(fact)
    }

    mutating func resetForPeer() {
        facts.removeAll(keepingCapacity: true)
    }
}

struct StreamingLocalIceState: Equatable, Sendable {
    var remoteDescriptionApplied = false
    var flushInFlight = false
    var gatheringComplete = false
    var completionSent = false
    var traceCompleted = false

    mutating func reset() {
        self = StreamingLocalIceState()
    }
}

enum StreamingCleanupStage: String, CaseIterable, Sendable {
    case presentationDetached
    case inputStopped
    case hapticsStopped
    case iceTasksCancelled
    case peerClosed
    case remoteSessionClosed
    case accessReleased
}
