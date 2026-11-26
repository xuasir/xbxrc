import Foundation

actor StreamingDiagnosticsActor {
    private var task: Task<Void, Never>?
    private var revision: UInt64 = 0
    private var lastUIAtMs: Double = 0

    func start(
        telemetry: StreamingDisplayTelemetry, session: StreamingPresentationSession,
        scope: StreamExecutionScope,
        eventSink: @escaping @Sendable (StreamingPresentationLifecycleEvent) -> Void
    ) {
        guard session.snapshot?.scope == scope else { return }
        task?.cancel()
        revision &+= 1
        let currentRevision = revision
        lastUIAtMs = 0
        task = Task(priority: .utility) { [weak self] in
            while !Task.isCancelled {
                guard let self else { return }
                await self.tick(
                    telemetry: telemetry, session: session, scope: scope,
                    revision: currentRevision, eventSink: eventSink)
                try? await Task.sleep(for: .milliseconds(100))
            }
        }
    }

    func stop() {
        task?.cancel()
        task = nil
        revision &+= 1
    }

    func recordInput(
        _ snapshot: StreamingInputCoreSnapshot, samples: [Double], sentPackets: UInt64,
        feedback: StreamingFrameFeedbackSnapshot?
    ) {
        let sorted = samples.sorted()
        func percentile(_ value: Double) -> IOSRuntimeTraceValue {
            sorted.isEmpty ? .null : .double(sorted[Int(Double(sorted.count - 1) * value)])
        }
        IOSRuntimeTrace.snapshot(
            domain: "ios-streaming", event: "inputQueueSnapshot",
            payload: [
                "source": .string(snapshot.source.rawValue),
                "sentPackets": .integer(Int64(clamping: sentPackets)),
                "pendingEdges": .integer(Int64(snapshot.pendingEdges)),
                "acceptedEdges": .integer(Int64(clamping: snapshot.acceptedEdges)),
                "discardedEdges": .integer(Int64(clamping: snapshot.discardedEdges)),
                "barrierCount": .integer(Int64(clamping: snapshot.barrierCount)),
                "barrierReason": snapshot.barrierReason.map(IOSRuntimeTraceValue.string) ?? .null,
                "eventToSendSampleCount": .integer(Int64(samples.count)),
                "eventToSendP95Ms": percentile(0.95), "eventToSendP99Ms": percentile(0.99),
            ], dimension: .input, importance: .key)
        if let feedback {
            IOSRuntimeTrace.snapshot(
                domain: "ios-streaming", event: "frameFeedback", payload: feedback.tracePayload(),
                dimension: .presentation, importance: .key)
        }
    }

    private func tick(
        telemetry: StreamingDisplayTelemetry, session: StreamingPresentationSession,
        scope: StreamExecutionScope, revision: UInt64,
        eventSink: @Sendable (StreamingPresentationLifecycleEvent) -> Void
    ) {
        guard revision == self.revision, let context = scope.presentation,
            session.snapshot?.scope == scope
        else { return }
        let now = StreamingFrameFeedbackClock.nowMilliseconds
        let snapshot = telemetry.snapshot(at: now)
        session.publish(scope: scope, health: snapshot)
        if now - lastUIAtMs >= 1_000 {
            lastUIAtMs = now
            eventSink(.displayHealth(context, snapshot))
        }
        let batch = session.takeFacts()
        for fact in batch.facts where fact.context == context {
            var payload: [String: IOSRuntimeTraceValue] = [
                "frameId": .integer(Int64(clamping: fact.frameID)),
                "renderEpoch": .integer(Int64(clamping: scope.renderEpoch)),
                "surfaceRevision": .integer(Int64(clamping: scope.surfaceRevision)),
                "atMonotonicMs": .double(fact.atMs),
                "submitCount": .integer(fact.count),
                "presentCount": .integer(fact.count),
                "presentTimestampSource": .string(fact.timestampSource),
            ]
            if let duration = fact.durationMs { payload["drawableAcquireMs"] = .double(duration) }
            if let reason = fact.reason { payload["reason"] = .string(reason) }
            IOSRuntimeTrace.state(
                domain: "ios-streaming", event: fact.event,
                payload: streamingPresentationTracePayload(payload, context: context),
                dimension: .presentation, importance: .key, operationID: context.attemptID)
        }
        if batch.dropped > 0 {
            IOSRuntimeTrace.snapshot(
                domain: "ios-streaming", event: "presentationFactQueuePressure",
                payload: ["discardedFacts": .integer(Int64(clamping: batch.dropped))],
                dimension: .presentation, importance: .key, operationID: context.attemptID)
        }
    }
}
