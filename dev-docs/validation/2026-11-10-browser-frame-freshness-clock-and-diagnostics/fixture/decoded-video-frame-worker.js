// src/player/infra/render/decoded-frame-clock.ts
var FRAME_CLOCK_QUANTIZATION_MS = 1;

// src/player/infra/render/decoded-video-frame-worker-source.ts
var DecodedVideoFrameWorkerSource = class {
  constructor(endpoint, now = () => performance.timeOrigin + performance.now()) {
    this.endpoint = endpoint;
    this.now = now;
  }
  reader;
  track;
  stopped = false;
  stopping;
  pending;
  inFlight;
  readCount = 0;
  sentCount = 0;
  overwriteCount = 0;
  ackCount = 0;
  lastAckRoundTripMs;
  lastAckDeliveryMs;
  lastAckSequence;
  lastAckReceivedAtEpochMs;
  lastAckDeliveryStatus;
  handle(message) {
    if (message.type === "clock") {
      if (!this.stopped && message.id !== void 0) {
        const receivedAtEpochMs = this.now();
        this.endpoint.postMessage({ type: "clock", id: message.id, receivedAtEpochMs, sentAtEpochMs: this.now() });
      }
      return;
    }
    if (message.type === "stop") {
      void this.stop();
    } else if (message.type === "ack") {
      if (this.stopped || !this.inFlight || message.sequence !== this.inFlight.sequence)
        return;
      const atMs = this.now();
      const roundTrip = atMs - this.inFlight.sentAtEpochMs;
      this.lastAckRoundTripMs = Number.isFinite(roundTrip) && roundTrip >= -FRAME_CLOCK_QUANTIZATION_MS ? Math.max(0, roundTrip) : void 0;
      const delivery = message.acknowledgedAtWorkerEpochMs === void 0 ? void 0 : atMs - message.acknowledgedAtWorkerEpochMs;
      const uncertainty = message.clockUncertaintyMs ?? 0;
      const valid = delivery !== void 0 && Number.isFinite(delivery) && Number.isFinite(uncertainty) && uncertainty >= 0 && delivery >= -uncertainty;
      this.lastAckDeliveryMs = valid ? Math.max(0, delivery) : void 0;
      this.lastAckDeliveryStatus = delivery === void 0 ? "uncalibrated" : valid ? delivery < 0 ? "uncertaintyClamped" : "valid" : "invalid";
      this.lastAckSequence = message.sequence;
      this.lastAckReceivedAtEpochMs = atMs;
      this.ackCount++;
      this.inFlight = void 0;
      this.sendLatest();
    } else if (!this.reader && !this.stopped && message.track) {
      this.track = message.track;
      void this.readFrames();
    }
  }
  async readFrames() {
    try {
      const Processor = this.endpoint.MediaStreamTrackProcessor;
      if (!Processor || !this.track) {
        void this.stop("unsupported");
        return;
      }
      this.reader = new Processor({ track: this.track, maxBufferSize: 1 }).readable.getReader();
      this.endpoint.postMessage({ type: "ready" });
      while (!this.stopped) {
        const readStartedAtEpochMs = this.now();
        const { value: frame, done } = await this.reader.read();
        const readFinishedAtEpochMs = this.now();
        if (done) {
          if (!this.stopped)
            void this.stop("trackEnded");
          return;
        }
        if (this.stopped) {
          frame.close();
          return;
        }
        if (this.pending) {
          this.pending.frame.close();
          this.overwriteCount++;
        }
        this.pending = { frame, sequence: ++this.readCount, readStartedAtEpochMs, readFinishedAtEpochMs };
        this.sendLatest();
      }
    } catch {
      if (!this.stopped)
        void this.stop("readerFailed");
    } finally {
      this.reader?.releaseLock();
    }
  }
  sendLatest() {
    if (this.stopped || this.inFlight || !this.pending)
      return;
    const pending = this.pending;
    this.pending = void 0;
    const sentAtEpochMs = this.now();
    this.inFlight = { sequence: pending.sequence, sentAtEpochMs };
    try {
      this.endpoint.postMessage({
        type: "frame",
        frame: pending.frame,
        timing: { sequence: pending.sequence, readStartedAtEpochMs: pending.readStartedAtEpochMs, readFinishedAtEpochMs: pending.readFinishedAtEpochMs, sentAtEpochMs, readCount: this.readCount, sentCount: ++this.sentCount, overwriteCount: this.overwriteCount, ackCount: this.ackCount, lastAckRoundTripMs: this.lastAckRoundTripMs, lastAckDeliveryMs: this.lastAckDeliveryMs, lastAckSequence: this.lastAckSequence, lastAckReceivedAtEpochMs: this.lastAckReceivedAtEpochMs, lastAckDeliveryStatus: this.lastAckDeliveryStatus }
      }, [pending.frame]);
    } catch {
      pending.frame.close();
      this.inFlight = void 0;
      void this.stop("readerFailed");
    }
  }
  stop(reason) {
    if (this.stopped)
      return this.stopping ?? Promise.resolve();
    this.stopped = true;
    this.pending?.frame.close();
    this.pending = void 0;
    this.inFlight = void 0;
    this.track?.stop();
    if (reason)
      this.notifyStopped({ type: "failed", reason });
    this.stopping = (async () => {
      try {
        await this.reader?.cancel();
      } catch {
      }
      this.notifyStopped({ type: "stopped" });
    })();
    return this.stopping;
  }
  notifyStopped(message) {
    try {
      this.endpoint.postMessage(message);
    } catch {
    }
  }
};

// src/player/infra/render/decoded-video-frame-worker.ts
var worker = globalThis;
var source = new DecodedVideoFrameWorkerSource(worker);
worker.onmessage = (event) => source.handle(event.data);
