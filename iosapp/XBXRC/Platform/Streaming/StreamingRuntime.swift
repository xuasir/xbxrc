import Foundation

@MainActor
protocol StreamingRuntime: AnyObject {
    var state: StreamingRuntimeState { get }

    func connect(to target: StreamingTarget) async throws
    func disconnect() async
}

enum StreamingRuntimeState: Equatable, Sendable {
    case idle
    case connecting
    case connected
    case disconnecting
    case failed(message: String)
}

struct StreamingTarget: Equatable, Sendable {
    let id: String
    let kind: Kind

    enum Kind: Equatable, Sendable {
        case cloud
        case console
    }
}
