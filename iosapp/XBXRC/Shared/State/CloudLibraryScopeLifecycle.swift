import Foundation

struct CloudLibraryScopeTransition {
    let previousAccess: PreparedCloudAccess?
    let previousAccountID: String?
    let persistenceDrain: CloudCatalogPersistenceCoordinator.Drain
    let refreshTask: Task<Void, Never>?
    let cancelledRefresh: Bool
}
