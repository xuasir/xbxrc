import Foundation

struct CloudLibraryScopeRevision: Equatable, Sendable {
    let generation: Int
    let scope: CloudCatalogScope?
}

@MainActor
final class CloudCatalogPersistenceCoordinator {
    struct Drain: Sendable {
        fileprivate let task: Task<Void, Never>?
    }

    private let repository: any CloudCatalogSnapshotStoring
    private var activeRevision: CloudLibraryScopeRevision?
    private var imagePersistenceTask: Task<Void, Never>?

    init(repository: any CloudCatalogSnapshotStoring) {
        self.repository = repository
    }

    func transition(to revision: CloudLibraryScopeRevision) -> Drain {
        let previousTask = imagePersistenceTask
        previousTask?.cancel()
        imagePersistenceTask = nil
        activeRevision = revision
        return Drain(task: previousTask)
    }

    func synchronize(to revision: CloudLibraryScopeRevision) {
        activeRevision = revision
    }

    func settle(_ drain: Drain) async {
        await drain.task?.value
    }

    func persistSuccessfulImagePreferences(
        _ snapshot: CloudCatalogSnapshot,
        revision: CloudLibraryScopeRevision
    ) {
        guard activeRevision == revision else { return }
        let previousTask = imagePersistenceTask
        imagePersistenceTask = Task { @MainActor [weak self, repository] in
            await previousTask?.value
            guard let self,
                  !Task.isCancelled,
                  self.activeRevision == revision
            else {
                return
            }
            try? await repository.saveSuccessfulImagePreferences(snapshot)
        }
    }
}
