import Foundation

struct StreamExecutionScope: Equatable, Sendable {
    var presentation: StreamingPresentationTraceContext?
    var inputEpoch: UInt64 = 0
    var renderEpoch: UInt64 = 0
    var surfaceRevision: UInt64 = 0
    var configurationRevision: UInt64 = 0
}

/// 短锁只维护代次；SDK 调用和队列交接在锁外执行。
final class StreamingExecutionFence: Sendable {
    private let lock = NSLock()
    private nonisolated(unsafe) var revision: UInt64 = 0

    var current: UInt64 {
        lock.withLock { revision }
    }

    @discardableResult
    func advance() -> UInt64 {
        lock.withLock {
            revision &+= 1
            return revision
        }
    }

    func accepts(_ value: UInt64) -> Bool { lock.withLock { revision == value } }
}

/// 固定槽位复用存储，出队原位释放强引用。
struct StreamingBoundedRing<Element: Sendable>: Sendable {
    private var storage: [Element?]
    private var head = 0
    private(set) var count = 0
    let capacity: Int

    init(capacity: Int) {
        self.capacity = max(1, capacity)
        storage = Array(repeating: nil, count: max(1, capacity))
    }

    var first: Element? { count > 0 ? storage[head] : nil }
    var last: Element? { count > 0 ? storage[(head + count - 1) % capacity] : nil }

    @discardableResult
    mutating func append(_ value: Element) -> Element? {
        let discarded = count == capacity ? removeFirst() : nil
        storage[(head + count) % capacity] = value
        count += 1
        return discarded
    }

    @discardableResult
    mutating func removeFirst() -> Element? {
        guard count > 0 else { return nil }
        let value = storage[head]
        storage[head] = nil
        head = (head + 1) % capacity
        count -= 1
        return value
    }

    mutating func removeAll() {
        while count > 0 { removeFirst() }
        head = 0
    }

    func values(maximumCount: Int = .max) -> [Element] {
        (0..<min(count, maximumCount)).compactMap { storage[(head + $0) % capacity] }
    }

    func forEach(maximumCount: Int = .max, _ consume: (Element) -> Void) {
        for index in 0..<min(count, maximumCount) {
            if let value = storage[(head + index) % capacity] { consume(value) }
        }
    }
}
