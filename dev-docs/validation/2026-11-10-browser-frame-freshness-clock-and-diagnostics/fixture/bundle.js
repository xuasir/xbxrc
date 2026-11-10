// ../../../../../../private/tmp/xbxrc-2026-11-10-clock-diagnostics-probe/external-source.ts
async function externalWebRtc(video) {
  const offer = await (await fetch("/rtc/offer", { method: "POST" })).json();
  const receiver = new RTCPeerConnection({ iceServers: [] });
  const playErrors = [];
  receiver.ontrack = (event) => {
    if (event.track.kind === "video") {
      video.srcObject = new MediaStream([event.track]);
      void video.play().catch((error) => playErrors.push(String(error)));
    }
  };
  await receiver.setRemoteDescription(offer.description);
  await receiver.setLocalDescription(await receiver.createAnswer());
  await new Promise((resolve, reject) => {
    if (receiver.iceGatheringState === "complete") {
      resolve();
      return;
    }
    const timer = setTimeout(() => reject(new Error("receiver ICE gather timeout")), 1e4);
    const changed = () => {
      if (receiver.iceGatheringState === "complete") {
        clearTimeout(timer);
        receiver.removeEventListener("icegatheringstatechange", changed);
        resolve();
      }
    };
    receiver.addEventListener("icegatheringstatechange", changed);
  });
  await fetch("/rtc/answer", { method: "POST", body: JSON.stringify({ id: offer.id, description: receiver.localDescription }) });
  const limit = performance.now() + 12e3;
  while (video.readyState < 2 || video.videoWidth !== 2560 || video.videoHeight !== 1440) {
    if (performance.now() > limit) {
      const entries = [...(await receiver.getStats()).values()];
      const inbound = entries.find((entry) => entry.type === "inbound-rtp" && (entry.kind ?? entry.mediaType) === "video");
      const sender = await (await fetch("/rtc/debug", { method: "POST", body: JSON.stringify({ id: offer.id }) })).json();
      throw new Error(JSON.stringify({
        phase: "external1440p.readiness",
        connection: receiver.connectionState,
        ice: receiver.iceConnectionState,
        playErrors,
        video: { readyState: video.readyState, paused: video.paused, width: video.videoWidth, height: video.videoHeight, currentTime: video.currentTime },
        sourceSetup: offer.sourceSetup,
        inbound: inbound ? { framesReceived: inbound.framesReceived, framesDecoded: inbound.framesDecoded, framesDropped: inbound.framesDropped, frameWidth: inbound.frameWidth, frameHeight: inbound.frameHeight, packetsReceived: inbound.packetsReceived } : null,
        sender
      }));
    }
    await new Promise((resolve) => setTimeout(resolve, 50));
  }
  return {
    sourceSetup: offer.sourceSetup,
    senderFrames: () => 0,
    stats: async () => {
      const entries = [...(await receiver.getStats()).values()];
      const inbound = entries.find((entry) => entry.type === "inbound-rtp" && (entry.kind ?? entry.mediaType) === "video");
      const codec = entries.find((entry) => entry.type === "codec" && entry.id === inbound?.codecId);
      if (!inbound) throw new Error("No external video statistics");
      const sender = await (await fetch("/rtc/debug", { method: "POST", body: JSON.stringify({ id: offer.id }) })).json();
      return {
        timestamp: inbound.timestamp,
        framesReceived: inbound.framesReceived,
        framesDecoded: inbound.framesDecoded,
        framesDropped: inbound.framesDropped,
        packetsLost: inbound.packetsLost,
        nackCount: inbound.nackCount,
        freezeCount: inbound.freezeCount,
        totalFreezesDuration: inbound.totalFreezesDuration,
        totalDecodeTime: inbound.totalDecodeTime,
        frameWidth: inbound.frameWidth,
        frameHeight: inbound.frameHeight,
        codec: codec?.mimeType,
        fmtp: codec?.sdpFmtpLine,
        jitter: inbound.jitter,
        jitterBufferDelay: inbound.jitterBufferDelay,
        jitterBufferEmittedCount: inbound.jitterBufferEmittedCount,
        sender
      };
    },
    close: () => {
      receiver.close();
      void fetch("/rtc/close", { method: "POST", body: JSON.stringify({ id: offer.id }) });
    }
  };
}

// ../../../../../../private/tmp/xbxrc-2026-11-10-clock-diagnostics-probe/drawable-identity.ts
function createDrawableIdentitySampler(video) {
  const canvas = document.createElement("canvas");
  canvas.width = 32;
  canvas.height = 1;
  const gl = canvas.getContext("webgl2", { alpha: false, antialias: false, depth: false, stencil: false });
  if (!gl) throw new Error("Drawable marker WebGL2 unavailable");
  const compile = (kind, source) => {
    const shader = gl.createShader(kind);
    gl.shaderSource(shader, source);
    gl.compileShader(shader);
    if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) throw new Error(gl.getShaderInfoLog(shader) ?? "marker shader error");
    return shader;
  };
  const vertex = compile(gl.VERTEX_SHADER, `#version 300 es
  void main() { vec2 p=vec2(float((gl_VertexID<<1)&2),float(gl_VertexID&2)); gl_Position=vec4(p*2.0-1.0,0.0,1.0); }`);
  const fragment = compile(gl.FRAGMENT_SHADER, `#version 300 es
  precision highp float;
  uniform sampler2D source;
  out vec4 color;
  void main() { color=texture(source,vec2((16.0+floor(gl_FragCoord.x)*32.0)/2560.0,32.0/1440.0)); }`);
  const program = gl.createProgram();
  gl.attachShader(program, vertex);
  gl.attachShader(program, fragment);
  gl.linkProgram(program);
  if (!gl.getProgramParameter(program, gl.LINK_STATUS)) throw new Error(gl.getProgramInfoLog(program) ?? "marker link error");
  gl.useProgram(program);
  const texture = gl.createTexture();
  gl.bindTexture(gl.TEXTURE_2D, texture);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
  const slots = Array.from({ length: 3 }, () => {
    const buffer = gl.createBuffer();
    gl.bindBuffer(gl.PIXEL_PACK_BUFFER, buffer);
    gl.bufferData(gl.PIXEL_PACK_BUFFER, 128, gl.STREAM_READ);
    return { buffer, sync: null, sample: void 0 };
  });
  gl.bindBuffer(gl.PIXEL_PACK_BUFFER, null);
  const samples = [];
  const cpuCosts = [];
  let stopped = false;
  let busySkips = 0;
  const poll = () => {
    for (const slot of slots) {
      if (!slot.sync || !slot.sample) continue;
      const status = gl.clientWaitSync(slot.sync, 0, 0);
      if (status === gl.TIMEOUT_EXPIRED && performance.now() - slot.sample.atMs < 500) continue;
      const sample = slot.sample;
      sample.completedAtMs = performance.now();
      if (status === gl.ALREADY_SIGNALED || status === gl.CONDITION_SATISFIED) {
        const bytes = new Uint8Array(128);
        gl.bindBuffer(gl.PIXEL_PACK_BUFFER, slot.buffer);
        gl.getBufferSubData(gl.PIXEL_PACK_BUFFER, 0, bytes);
        gl.bindBuffer(gl.PIXEL_PACK_BUFFER, null);
        let id = 0, magic = 0;
        for (let bit = 0; bit < 32; bit++) {
          const value = Number((bytes[bit * 4] + bytes[bit * 4 + 1] + bytes[bit * 4 + 2]) / 3 > 128);
          if (bit < 24) id |= value << bit;
          else magic |= value << bit - 24;
        }
        Object.assign(sample, { id, magic, valid: magic === 211, state: "completed" });
      } else sample.state = status === gl.WAIT_FAILED ? "waitFailed" : "expired";
      samples.push({ ...sample });
      gl.deleteSync(slot.sync);
      slot.sync = null;
      slot.sample = void 0;
    }
  };
  return {
    sample: (context) => {
      if (stopped) return;
      const atMs = performance.now();
      poll();
      const slot = slots.find((value) => !value.sync);
      if (!slot) {
        busySkips++;
        return;
      }
      gl.bindTexture(gl.TEXTURE_2D, texture);
      gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, gl.RGBA, gl.UNSIGNED_BYTE, video);
      gl.drawArrays(gl.TRIANGLES, 0, 3);
      gl.bindBuffer(gl.PIXEL_PACK_BUFFER, slot.buffer);
      gl.readPixels(0, 0, 32, 1, gl.RGBA, gl.UNSIGNED_BYTE, 0);
      gl.bindBuffer(gl.PIXEL_PACK_BUFFER, null);
      slot.sync = gl.fenceSync(gl.SYNC_GPU_COMMANDS_COMPLETE, 0);
      slot.sample = { ...context, atMs, state: "pending" };
      gl.flush();
      cpuCosts.push({ atMs, durationMs: performance.now() - atMs });
    },
    finish: async () => {
      stopped = true;
      const deadline = performance.now() + 550;
      while (slots.some((slot) => slot.sync) && performance.now() < deadline) {
        poll();
        await new Promise((resolve) => setTimeout(resolve, 4));
      }
    },
    snapshot: (startAtMs, endAtMs) => {
      const window2 = samples.filter((sample) => sample.atMs >= startAtMs && sample.atMs <= endAtMs).sort((a, b) => a.atMs - b.atMs);
      const valid = window2.filter((sample) => sample.valid);
      const changed = [], extra = [], unqualified = [];
      let duplicateCount = 0, skippedSourceIds = 0;
      for (let i = 1; i < valid.length; i++) {
        const before = valid[i - 1], after = valid[i];
        if (before.id === after.id) {
          duplicateCount++;
          continue;
        }
        changed.push(after);
        if (after.id > before.id) skippedSourceIds += after.id - before.id - 1;
        if (before.nativeSerial === after.nativeSerial) {
          extra.push(after);
          if (before.localSequence === after.localSequence) unqualified.push(after);
        }
      }
      const cost = cpuCosts.filter((value) => value.atMs >= startAtMs && value.atMs <= endAtMs).map((value) => value.durationMs).sort((a, b) => a - b);
      return {
        source: "32-pixel synthetic frame barcode; asynchronous PBO readback",
        busySkips,
        sampleCount: window2.length,
        validCount: valid.length,
        uniqueFrameIds: new Set(valid.map((value) => value.id)).size,
        changedCount: changed.length,
        drawableChangeFps: changed.length * 1e3 / (endAtMs - startAtMs),
        duplicateCount,
        skippedSourceIds,
        newImageWithoutNativeCallbackCount: extra.length,
        newImageWithoutSourceQualificationCount: unqualified.length,
        cpuP95Ms: cost[Math.max(0, Math.ceil(cost.length * 0.95) - 1)] ?? null,
        cpuMaxMs: cost.at(-1) ?? null,
        extraExamples: extra.slice(0, 12),
        samples: window2
      };
    },
    close: () => {
      stopped = true;
      for (const slot of slots) {
        if (slot.sync) gl.deleteSync(slot.sync);
        gl.deleteBuffer(slot.buffer);
      }
      gl.deleteTexture(texture);
      gl.deleteProgram(program);
      gl.deleteShader(vertex);
      gl.deleteShader(fragment);
      gl.getExtension("WEBGL_lose_context")?.loseContext();
      canvas.width = canvas.height = 1;
    }
  };
}

// src/shared/latency-window.ts
var LatencyWindow = class {
  values;
  count = 0;
  cached;
  maximumObservedMs;
  constructor(capacity = 256) {
    this.values = new Float64Array(capacity);
  }
  record(ms) {
    if (!Number.isFinite(ms) || ms < 0)
      return;
    this.values[this.count++ % this.values.length] = ms;
    this.maximumObservedMs = Math.max(this.maximumObservedMs ?? 0, ms);
    this.cached = void 0;
  }
  reset() {
    this.count = 0;
    this.cached = void 0;
    this.maximumObservedMs = void 0;
  }
  snapshot() {
    if (!this.cached) {
      const retained = Math.min(this.count, this.values.length);
      const sorted = Array.from(this.values.subarray(0, retained)).sort((a, b) => a - b);
      const percentile = (fraction) => sorted[Math.max(0, Math.ceil(retained * fraction) - 1)];
      this.cached = { count: this.count, retained, capacity: this.values.length, maxObservedMs: this.maximumObservedMs, meanMs: retained ? sorted.reduce((total, value) => total + value, 0) / retained : void 0, p50Ms: percentile(0.5), p95Ms: percentile(0.95), p99Ms: percentile(0.99), maxMs: sorted.at(-1) };
    }
    return { ...this.cached };
  }
};

// src/player/infra/render/decoded-frame-clock.ts
var FRAME_CLOCK_QUANTIZATION_MS = 1;
var DecodedFrameClock = class {
  sequence = 0;
  pending;
  calibration;
  samples = 0;
  rejected = 0;
  probe(atMs) {
    if (this.pending && atMs - this.pending.sentAtMs < 1e3)
      return;
    this.pending = { id: ++this.sequence, sentAtMs: atMs };
    return { type: "clock", id: this.sequence };
  }
  accept(reply, atMs) {
    if (!this.pending || this.pending.id !== reply.id)
      return false;
    const start = this.pending.sentAtMs;
    this.pending = void 0;
    const { receivedAtEpochMs: received, sentAtEpochMs: sent } = reply;
    const rttMs = atMs - start - (sent - received);
    if (![start, atMs, received, sent].every(Number.isFinite) || atMs < start || sent < received || rttMs < 0 || rttMs > 8) {
      this.rejected++;
      return false;
    }
    const offsetMs = (start - received + (atMs - sent)) / 2;
    const uncertaintyMs = rttMs / 2 + 2 * FRAME_CLOCK_QUANTIZATION_MS;
    if (!this.calibration || atMs - this.calibration.sampledAtMs >= 5e3 || rttMs <= this.calibration.rttMs)
      this.calibration = { offsetMs, uncertaintyMs, sampledAtMs: atMs, rttMs };
    this.samples++;
    return true;
  }
  convert(workerAtMs, mainAtMs) {
    const c = this.calibration;
    if (!c || mainAtMs - c.sampledAtMs > 15e3 || mainAtMs < c.sampledAtMs || !Number.isFinite(workerAtMs))
      return;
    return { atMs: workerAtMs + c.offsetMs, uncertaintyMs: c.uncertaintyMs };
  }
  snapshot(atMs) {
    const valid = this.convert(0, atMs);
    return { state: valid ? "calibrated" : this.calibration ? "expired" : "uncalibrated", probes: this.sequence, samples: this.samples, rejected: this.rejected, offsetMs: valid ? this.calibration.offsetMs : void 0, uncertaintyMs: valid?.uncertaintyMs, ageMs: this.calibration ? Math.max(0, atMs - this.calibration.sampledAtMs) : void 0 };
  }
};

// src/player/infra/render/frame-supply-diagnostics.ts
var FrameSupplyDiagnostics = class {
  anomalies = [];
  tasks = [];
  observer;
  nextSampleAtMs = -Infinity;
  count = 0;
  suppressed = 0;
  taskState = "unavailable";
  constructor() {
    try {
      if (typeof PerformanceObserver === "function" && PerformanceObserver.supportedEntryTypes?.includes("longtask")) {
        this.observer = new PerformanceObserver((list) => {
          if (this.taskState === "stopped")
            return;
          for (const entry of list.getEntries()) {
            this.tasks.push({ atMs: entry.startTime, durationMs: entry.duration });
            if (this.tasks.length > 8)
              this.tasks.shift();
          }
        });
        this.observer.observe({ type: "longtask" });
        this.taskState = "observing";
      }
    } catch {
      this.taskState = "failed";
    }
  }
  record(value) {
    if (this.taskState === "stopped" || !value.reasons.length)
      return;
    this.count++;
    if (value.atMs < this.nextSampleAtMs) {
      this.suppressed++;
      return;
    }
    this.nextSampleAtMs = value.atMs + 1e3;
    this.anomalies.push(value);
    if (this.anomalies.length > 4)
      this.anomalies.shift();
  }
  ackSent(sequence, atEpochMs) {
    const sample = this.anomalies.find((value) => value.sequence === sequence);
    if (sample)
      sample.times[5] = atEpochMs;
  }
  ackReceived(sequence, atEpochMs) {
    const sample = this.anomalies.find((value) => value.sequence === sequence);
    if (sample)
      sample.times[6] = atEpochMs;
  }
  snapshot() {
    return { count: this.count, suppressed: this.suppressed, taskState: this.taskState, samples: this.anomalies.map((value) => ({ ...value, reasons: [...value.reasons], times: [...value.times], longTasks: this.tasks.filter((task) => task.atMs <= value.atMs && task.atMs + task.durationMs >= value.atMs - 150).map((task) => ({ ...task })) })) };
  }
  dispose() {
    this.observer?.disconnect();
    this.observer = void 0;
    this.taskState = "stopped";
    this.tasks.length = 0;
  }
};

// src/player/infra/render/decoded-video-frame-reader.ts
var DECODED_FRAME_DELIVERY_BUDGET_MS = 50;
var DecodedVideoFrameReader = class {
  constructor(video, consume, fallback) {
    this.consume = consume;
    this.fallback = fallback;
    const stream = video.srcObject;
    const sourceTrack = stream?.getVideoTracks?.().find((track) => track.readyState === "live");
    if (!sourceTrack || typeof sourceTrack.clone !== "function") {
      this.fail("trackUnavailable");
      return;
    }
    try {
      this.sourceTrack = sourceTrack;
      sourceTrack.addEventListener?.("unmute", this.onUnmute);
      sourceTrack.addEventListener?.("ended", this.onTrackEnded);
      this.mediaStarted = sourceTrack.muted === false;
      this.track = sourceTrack.clone();
      const Processor = globalThis.MediaStreamTrackProcessor;
      if (typeof Processor === "function") {
        this.reader = new Processor({ track: this.track, maxBufferSize: 1 }).readable.getReader();
        this.markProcessorReady();
        void this.readFrames();
      } else if (typeof Worker === "function") {
        this.timeout = setTimeout(() => this.fail("initializationTimeout"), 2e3);
        this.worker = new Worker(new URL("./decoded-video-frame-worker.ts", import.meta.url), { type: "module" });
        this.worker.onmessage = (event) => this.onWorkerMessage(event.data);
        this.worker.onerror = () => this.fail("readerFailed");
        this.worker.postMessage({ type: "start", track: this.track }, [this.track]);
        this.probeClock();
      } else {
        this.fail("unsupported");
      }
    } catch {
      this.fail("initializationFailed");
    }
  }
  mode = "initializing";
  fallbackReason;
  receivedFrames = 0;
  sourceTrack;
  track;
  reader;
  worker;
  clockTimer;
  clock = new DecodedFrameClock();
  diagnostics = new FrameSupplyDiagnostics();
  unknownClockCount = 0;
  adjustedClockCount = 0;
  framePixelFormat;
  invalidClockReasons = {};
  timeout;
  processorReady = false;
  mediaStarted = false;
  stopped = false;
  receivedMessages = 0;
  staleFrameDropCount = 0;
  invalidClockCount = 0;
  lastReceivedAtMs;
  workerTiming;
  timing = {
    readWait: new LatencyWindow(),
    pendingWait: new LatencyWindow(),
    transfer: new LatencyWindow(),
    consume: new LatencyWindow(),
    receiveInterval: new LatencyWindow(),
    ackRoundTrip: new LatencyWindow(),
    ackDelivery: new LatencyWindow()
  };
  onUnmute = () => this.observeMediaProgress();
  onTrackEnded = () => this.fail("trackEnded");
  /** 轨在连接阶段可以先于媒体创建，首帧期限从实际供给进展开始。 */
  observeMediaProgress() {
    if (this.stopped || this.receivedFrames > 0)
      return;
    this.mediaStarted = true;
    this.armFirstFrameDeadline();
  }
  markProcessorReady() {
    if (this.stopped || this.processorReady)
      return;
    this.processorReady = true;
    this.clearTimeout();
    this.armFirstFrameDeadline();
  }
  armFirstFrameDeadline() {
    if (this.processorReady && this.mediaStarted && this.receivedFrames === 0 && this.timeout === void 0)
      this.timeout = setTimeout(() => this.fail("firstFrameTimeout"), 2e3);
  }
  clearTimeout() {
    if (this.timeout !== void 0)
      clearTimeout(this.timeout);
    this.timeout = void 0;
  }
  snapshot() {
    const atMs = performance.now();
    const epoch = performance.timeOrigin + atMs;
    const sent = this.workerTiming ? this.clock.convert(this.workerTiming.sentAtEpochMs, epoch) : void 0;
    const workerAgeMs = sent ? epoch - sent.atMs : Number.NaN;
    return {
      schemaVersion: 1,
      clockDomain: "performanceEpoch",
      observedAtMs: performance.timeOrigin + atMs,
      receivedMessages: this.receivedMessages,
      deliveredFrames: this.receivedFrames,
      staleFrameDropCount: this.staleFrameDropCount,
      invalidClockCount: this.invalidClockCount,
      unknownClockCount: this.unknownClockCount,
      adjustedClockCount: this.adjustedClockCount,
      invalidClockReasons: { ...this.invalidClockReasons },
      clock: this.clock.snapshot(epoch),
      diagnostics: this.diagnostics.snapshot(),
      ackDeliveryStatus: this.workerTiming?.lastAckDeliveryStatus,
      framePixelFormat: this.framePixelFormat,
      deliveryBudgetMs: DECODED_FRAME_DELIVERY_BUDGET_MS,
      workerReadFrames: this.workerTiming?.readCount ?? 0,
      workerSentFrames: this.workerTiming?.sentCount ?? 0,
      workerOverwriteCount: this.workerTiming?.overwriteCount ?? 0,
      workerAckCount: this.workerTiming?.ackCount ?? 0,
      lastReadSequence: this.workerTiming?.sequence,
      receiveAgeMs: this.lastReceivedAtMs === void 0 ? void 0 : Math.max(0, atMs - this.lastReceivedAtMs),
      workerObservationAgeMs: Number.isFinite(workerAgeMs) && workerAgeMs >= 0 ? workerAgeMs : void 0,
      timing: {
        readWait: this.timing.readWait.snapshot(),
        pendingWait: this.timing.pendingWait.snapshot(),
        transfer: this.timing.transfer.snapshot(),
        consume: this.timing.consume.snapshot(),
        receiveInterval: this.timing.receiveInterval.snapshot(),
        ackRoundTrip: this.timing.ackRoundTrip.snapshot(),
        ackDelivery: this.timing.ackDelivery.snapshot()
      }
    };
  }
  accept(frame, mode, workerTiming) {
    if (this.stopped || this.mode === "presentation") {
      frame.close();
      return;
    }
    const receivedAtMs = performance.now();
    this.framePixelFormat = frame.format;
    const receivedAtEpochMs = performance.timeOrigin + receivedAtMs;
    const receiveIntervalMs = this.lastReceivedAtMs === void 0 ? 0 : receivedAtMs - this.lastReceivedAtMs;
    const startup = this.receivedFrames === 0;
    this.receivedMessages++;
    if (this.lastReceivedAtMs !== void 0)
      this.timing.receiveInterval.record(receivedAtMs - this.lastReceivedAtMs);
    this.lastReceivedAtMs = receivedAtMs;
    let readFinishedAtEpochMs;
    let clockUncertaintyMs;
    let readWaitMs = 0;
    let transferMs = 0;
    let stale = false;
    let clockReason;
    if (workerTiming) {
      this.workerTiming = workerTiming;
      if (workerTiming.lastAckSequence !== void 0 && workerTiming.lastAckReceivedAtEpochMs !== void 0)
        this.diagnostics.ackReceived(workerTiming.lastAckSequence, workerTiming.lastAckReceivedAtEpochMs);
      this.timing.ackRoundTrip.record(workerTiming.lastAckRoundTripMs ?? Number.NaN);
      this.timing.ackDelivery.record(workerTiming.lastAckDeliveryMs ?? Number.NaN);
      const { readStartedAtEpochMs } = workerTiming;
      const rawFinished = workerTiming.readFinishedAtEpochMs;
      const rawSent = workerTiming.sentAtEpochMs;
      const valid = [readStartedAtEpochMs, rawFinished, rawSent, receivedAtEpochMs].every(Number.isFinite) && readStartedAtEpochMs - rawFinished <= FRAME_CLOCK_QUANTIZATION_MS && rawFinished - rawSent <= FRAME_CLOCK_QUANTIZATION_MS;
      if (valid) {
        const finished = Math.max(readStartedAtEpochMs, rawFinished);
        const sentAtEpochMs = Math.max(finished, rawSent);
        if (finished !== rawFinished || sentAtEpochMs !== rawSent)
          this.adjustedClockCount++;
        readWaitMs = finished - readStartedAtEpochMs;
        this.timing.readWait.record(readWaitMs);
        this.timing.pendingWait.record(sentAtEpochMs - finished);
        const sent = this.clock.convert(sentAtEpochMs, receivedAtEpochMs);
        if (sent) {
          transferMs = receivedAtEpochMs - sent.atMs;
          clockUncertaintyMs = sent.uncertaintyMs;
          if (transferMs >= -sent.uncertaintyMs) {
            if (transferMs < 0)
              this.adjustedClockCount++;
            this.timing.transfer.record(Math.max(0, transferMs));
            readFinishedAtEpochMs = finished + (sent.atMs - sentAtEpochMs);
            stale = receivedAtEpochMs - readFinishedAtEpochMs - sent.uncertaintyMs > DECODED_FRAME_DELIVERY_BUDGET_MS;
          } else {
            clockReason = "outsideCalibration";
          }
        } else {
          clockReason = this.clock.snapshot(receivedAtEpochMs).state;
        }
      } else {
        clockReason = "workerOrderOrNonFinite";
      }
      if (clockReason === "outsideCalibration" || clockReason === "workerOrderOrNonFinite") {
        this.invalidClockCount++;
        this.invalidClockReasons[clockReason] = (this.invalidClockReasons[clockReason] ?? 0) + 1;
      } else if (clockReason) {
        this.unknownClockCount++;
      }
      if (stale)
        this.staleFrameDropCount++;
    }
    const consumeStartedAtMs = performance.now();
    try {
      if (stale) {
        frame.close();
        return;
      }
      this.clearTimeout();
      this.processorReady = true;
      this.mode = mode;
      this.receivedFrames++;
      this.consume(frame, { receivedAtMs, readFinishedAtEpochMs, clockUncertaintyMs });
    } catch (error) {
      frame.close();
      throw error;
    } finally {
      const finishedAtMs = performance.now();
      this.timing.consume.record(finishedAtMs - consumeStartedAtMs);
      if (workerTiming && (readWaitMs > 1e3 / 30 || receiveIntervalMs > 1e3 / 30 || transferMs > 1e3 / 30 || finishedAtMs - consumeStartedAtMs > 1e3 / 30 || stale || clockReason)) {
        const c = this.clock.snapshot(receivedAtEpochMs);
        this.diagnostics.record({
          sequence: workerTiming.sequence,
          atMs: receivedAtMs,
          startup,
          reasons: [readWaitMs > 1e3 / 30 ? "readWait" : "", receiveIntervalMs > 1e3 / 30 ? "receiveGap" : "", transferMs > 1e3 / 30 ? "transfer" : "", finishedAtMs - consumeStartedAtMs > 1e3 / 30 ? "consume" : "", stale ? "staleDrop" : "", clockReason ?? ""].filter(Boolean),
          // [Worker read开始/完成/send，main receive/consume结束/ACK发送，Worker同序号ACK接收]
          times: [workerTiming.readStartedAtEpochMs, workerTiming.readFinishedAtEpochMs, workerTiming.sentAtEpochMs, receivedAtEpochMs, performance.timeOrigin + finishedAtMs, 0, 0],
          offsetMs: c.offsetMs,
          uncertaintyMs: c.uncertaintyMs
        });
      }
    }
  }
  probeClock() {
    if (this.stopped || !this.worker)
      return;
    const probe = this.clock.probe(performance.timeOrigin + performance.now());
    try {
      if (probe)
        this.worker.postMessage(probe);
    } catch {
    }
    if (this.clockTimer !== void 0)
      clearTimeout(this.clockTimer);
    this.clockTimer = setTimeout(() => this.probeClock(), 5e3);
  }
  async readFrames() {
    const reader = this.reader;
    try {
      while (!this.stopped) {
        const { value, done } = await reader.read();
        if (done) {
          if (!this.stopped)
            this.fail("trackEnded");
          return;
        }
        this.accept(value, "main-thread");
      }
    } catch {
      if (!this.stopped)
        this.fail("readerFailed");
    }
  }
  onWorkerMessage(message) {
    if (message.type === "ready") {
      this.markProcessorReady();
    } else if (message.type === "clock") {
      this.clock.accept(message, performance.timeOrigin + performance.now());
      const state = this.clock.snapshot(performance.timeOrigin + performance.now());
      if (state.samples < 3 && state.probes < 8)
        this.probeClock();
    } else if (message.type === "frame") {
      try {
        this.accept(message.frame, "worker", message.timing);
      } catch {
        this.fail("readerFailed");
      } finally {
        const epoch = performance.timeOrigin + performance.now();
        const c = this.clock.convert(0, epoch);
        const ack = message.timing ? { type: "ack", sequence: message.timing.sequence, acknowledgedAtEpochMs: epoch, acknowledgedAtWorkerEpochMs: c === void 0 ? void 0 : epoch - c.atMs, clockUncertaintyMs: c?.uncertaintyMs } : { type: "ack" };
        try {
          this.worker?.postMessage(ack);
          if (message.timing)
            this.diagnostics.ackSent(message.timing.sequence, epoch);
        } catch {
          this.fail("readerFailed");
        }
      }
    } else if (message.type === "failed") {
      this.fail(message.reason);
    }
  }
  fail(reason) {
    if (this.stopped)
      return;
    this.mode = "presentation";
    this.fallbackReason = reason;
    this.dispose();
    this.fallback?.(reason);
  }
  dispose() {
    if (this.stopped)
      return;
    this.stopped = true;
    this.clearTimeout();
    if (this.clockTimer !== void 0)
      clearTimeout(this.clockTimer);
    this.clockTimer = void 0;
    this.diagnostics.dispose();
    this.sourceTrack?.removeEventListener?.("unmute", this.onUnmute);
    this.sourceTrack?.removeEventListener?.("ended", this.onTrackEnded);
    this.sourceTrack = void 0;
    const reader = this.reader;
    this.reader = void 0;
    void reader?.cancel().catch(() => {
    }).finally(() => reader.releaseLock());
    this.track?.stop();
    this.track = void 0;
    const worker = this.worker;
    this.worker = void 0;
    if (worker) {
      const timer = setTimeout(() => worker.terminate(), 100);
      worker.onmessage = (event) => {
        if (event.data.type === "frame")
          event.data.frame.close();
        if (event.data.type === "stopped") {
          clearTimeout(timer);
          worker.terminate();
        }
      };
      worker.onerror = () => {
        clearTimeout(timer);
        worker.terminate();
      };
      try {
        worker.postMessage({ type: "stop" });
      } catch {
        clearTimeout(timer);
        worker.terminate();
      }
    }
  }
};

// src/player/infra/render/video-frame-identity-observation.ts
function finiteIdentityValue(value) {
  return value !== void 0 && Number.isFinite(value) ? value : null;
}
function identityValueState(value, previous) {
  if (value === void 0)
    return "missing";
  if (!Number.isFinite(value))
    return "invalid";
  if (previous === void 0 || !Number.isFinite(previous))
    return "initial";
  return value > previous ? "advanced" : value === previous ? "unchanged" : "regressed";
}
function callbackTiming() {
  return { count: 0, intervalCount: 0, intervalTotalMs: 0, intervalMaxMs: 0, clockLagCount: 0, clockLagTotalMs: 0, clockLagMaxMs: 0 };
}
var VideoFrameIdentityObservation = class {
  enabled = false;
  startedAtMs;
  observed = { videoFrameCallback: 0, animationFrame: 0, timeupdate: 0, decodedFrame: 0 };
  accepted = { videoFrameCallback: 0, animationFrame: 0, timeupdate: 0, decodedFrame: 0 };
  reasons = {};
  samples = [];
  nextAcceptedAtMs = 0;
  nextRejectedAtMs = 0;
  sampledCount = 0;
  nativeUnchangedWithMediaAdvanceCount = 0;
  lastCallback = /* @__PURE__ */ new Map();
  timing = { videoFrameCallback: callbackTiming(), animationFrame: callbackTiming(), timeupdate: callbackTiming(), decodedFrame: callbackTiming() };
  counterCadence = { updateCount: 0, advanceTotal: 0, deltaMax: 0, intervalCount: 0, intervalTotalMs: 0, intervalMaxMs: 0, resetCount: 0, updatesBySource: { videoFrameCallback: 0, animationFrame: 0, timeupdate: 0, decodedFrame: 0 } };
  previousCounterPair;
  resetTimingBaseline() {
    this.lastCallback.clear();
    this.previousCounterPair = void 0;
  }
  recordTiming(source, atMs, enteredAtMs, counterDelta, progressAgeMs, counterReset) {
    const previous = this.lastCallback.get(source);
    const interval = previous ? enteredAtMs - previous.enteredAtMs : void 0;
    const timestampInterval = previous ? atMs - previous.atMs : void 0;
    const clockLag = enteredAtMs - atMs;
    this.lastCallback.set(source, { atMs, enteredAtMs });
    const timing = this.timing[source];
    timing.count++;
    if (interval !== void 0 && Number.isFinite(interval) && interval >= 0) {
      timing.intervalCount++;
      timing.intervalTotalMs += interval;
      timing.intervalMaxMs = Math.max(timing.intervalMaxMs, interval);
    }
    if (Number.isFinite(clockLag)) {
      timing.clockLagCount++;
      timing.clockLagTotalMs += clockLag;
      timing.clockLagMaxMs = Math.max(timing.clockLagMaxMs, clockLag);
    }
    this.counterCadence.resetCount += Number(counterReset);
    if (counterDelta !== void 0 && Number.isFinite(counterDelta) && counterDelta > 0) {
      this.counterCadence.updateCount++;
      this.counterCadence.advanceTotal += counterDelta;
      this.counterCadence.deltaMax = Math.max(this.counterCadence.deltaMax, counterDelta);
      this.counterCadence.updatesBySource[source]++;
      if (Number.isFinite(progressAgeMs) && progressAgeMs > 0) {
        this.counterCadence.intervalCount++;
        this.counterCadence.intervalTotalMs += progressAgeMs;
        this.counterCadence.intervalMaxMs = Math.max(this.counterCadence.intervalMaxMs, progressAgeMs);
      }
    }
    return { callbackEnteredAtMs: enteredAtMs, callbackIntervalMs: finiteIdentityValue(interval), callbackTimestampIntervalMs: finiteIdentityValue(timestampInterval), callbackClockLagMs: finiteIdentityValue(clockLag) };
  }
  pairCounters(observedAtMs, values) {
    const previous = this.previousCounterPair;
    const delta = (value, before) => value === null || before == null ? null : value - before;
    const pair = { ...values, observedAtMs, previousObservedAtMs: previous?.observedAtMs ?? null, playbackQualityDeltaSincePreviousSample: delta(values.playbackQualityFrames, previous?.playbackQualityFrames), webkitDeltaSincePreviousSample: delta(values.webkitFrames, previous?.webkitFrames) };
    this.previousCounterPair = pair;
    return pair;
  }
  count(source, accepted, reason, identityConflict) {
    this.observed[source]++;
    if (accepted)
      this.accepted[source]++;
    this.reasons[reason] = (this.reasons[reason] ?? 0) + 1;
    if (identityConflict)
      this.nativeUnchangedWithMediaAdvanceCount++;
  }
  shouldSample(atMs, accepted) {
    return Number.isFinite(atMs) && atMs >= (accepted ? this.nextAcceptedAtMs : this.nextRejectedAtMs);
  }
  capture(sample) {
    if (sample.newFrame)
      this.nextAcceptedAtMs = sample.atMs + 500;
    else
      this.nextRejectedAtMs = sample.atMs + 500;
    this.sampledCount++;
    this.samples.push(sample);
    if (this.samples.length > 8)
      this.samples.shift();
  }
  snapshot() {
    if (!this.enabled) {
      this.enabled = true;
      this.startedAtMs = performance.now();
    }
    return {
      schemaVersion: 2,
      collectionStartedAtMs: this.startedAtMs,
      observed: { ...this.observed },
      accepted: { ...this.accepted },
      rejected: {
        videoFrameCallback: this.observed.videoFrameCallback - this.accepted.videoFrameCallback,
        animationFrame: this.observed.animationFrame - this.accepted.animationFrame,
        timeupdate: this.observed.timeupdate - this.accepted.timeupdate
      },
      reasons: { ...this.reasons },
      nativeUnchangedWithMediaAdvanceCount: this.nativeUnchangedWithMediaAdvanceCount,
      callbackTiming: { videoFrameCallback: { ...this.timing.videoFrameCallback }, animationFrame: { ...this.timing.animationFrame }, timeupdate: { ...this.timing.timeupdate } },
      counterCadence: { ...this.counterCadence, updatesBySource: { ...this.counterCadence.updatesBySource } },
      sampledCount: this.sampledCount,
      evidenceLimit: 8,
      evidence: this.samples.map((sample) => ({ ...sample, pairedCounters: sample.pairedCounters ? { ...sample.pairedCounters } : void 0 }))
    };
  }
};

// src/player/infra/render/gpu-completion-probe.ts
var GpuCompletionProbe = class {
  constructor(gl, now = () => performance.now(), warmupMs = 1e3) {
    this.gl = gl;
    this.now = now;
    this.warmupMs = warmupMs;
    this.state = ["fenceSync", "clientWaitSync", "deleteSync", "flush"].every((key) => typeof gl[key] === "function") ? "ready" : "unavailable";
  }
  sync;
  timer;
  nextAtMs = -Infinity;
  firstFrameAtMs;
  insertedAtMs = 0;
  insertionCpuMs = 0;
  insertionCpuMaxMs = 0;
  failureReason;
  submittedAtMs = 0;
  lowerBoundMs = 0;
  polls = 0;
  completed = 0;
  timeouts = 0;
  stopped = false;
  state;
  latest;
  upperBound = new LatencyWindow();
  submit(atMs) {
    this.firstFrameAtMs ??= atMs;
    if (this.stopped || this.sync || this.timer !== void 0 || this.state === "unavailable" || this.state === "failed" || atMs < this.firstFrameAtMs + this.warmupMs || atMs < this.nextAtMs) {
      return;
    }
    this.nextAtMs = atMs + 1e3;
    this.submittedAtMs = atMs;
    this.state = "pending";
    this.timer = setTimeout(() => this.insert(), 0);
  }
  insert() {
    this.timer = void 0;
    if (this.stopped)
      return;
    const startedAtMs = this.now();
    try {
      this.sync = this.gl.fenceSync(this.gl.SYNC_GPU_COMMANDS_COMPLETE, 0) ?? void 0;
      if (!this.sync) {
        this.fail();
        return;
      }
      this.insertedAtMs = startedAtMs;
      this.lowerBoundMs = this.polls = 0;
      this.gl.flush();
      this.insertionCpuMs = this.now() - startedAtMs;
      this.insertionCpuMaxMs = Math.max(this.insertionCpuMaxMs, this.insertionCpuMs);
      if (this.insertionCpuMs > 4) {
        this.fail("overBudget");
        return;
      }
      this.timer = setTimeout(() => this.poll(), 4);
    } catch {
      this.fail();
    }
  }
  poll() {
    this.timer = void 0;
    if (this.stopped || !this.sync)
      return;
    const atMs = this.now();
    try {
      const result = this.gl.clientWaitSync(this.sync, 0, 0);
      const observedAtMs = this.now();
      const elapsed = Math.max(0, observedAtMs - this.submittedAtMs);
      this.polls++;
      if (result === this.gl.ALREADY_SIGNALED || result === this.gl.CONDITION_SATISFIED) {
        this.latest = { submittedAtMs: this.submittedAtMs, insertedAtMs: this.insertedAtMs, observedAtMs, lowerBoundMs: this.lowerBoundMs, upperBoundMs: elapsed, polls: this.polls };
        this.upperBound.record(elapsed);
        this.completed++;
        this.release();
        this.state = "ready";
      } else if (result === this.gl.WAIT_FAILED) {
        this.fail();
      } else if (elapsed >= 1e3 || this.polls >= 8) {
        this.timeouts++;
        this.release();
        this.state = "ready";
      } else {
        this.lowerBoundMs = Math.max(0, atMs - this.submittedAtMs);
        this.timer = setTimeout(() => this.poll(), Math.min(250, 4 * 2 ** this.polls));
      }
    } catch {
      this.fail();
    }
  }
  release() {
    if (this.timer !== void 0)
      clearTimeout(this.timer);
    this.timer = void 0;
    const sync = this.sync;
    this.sync = void 0;
    if (sync)
      this.gl.deleteSync(sync);
  }
  fail(reason = "driver") {
    this.state = "failed";
    this.failureReason = reason;
    try {
      this.release();
    } catch {
    }
  }
  dispose() {
    this.stopped = true;
    try {
      this.release();
    } catch {
    }
    this.state = "stopped";
  }
  snapshot() {
    return { source: "webglFenceCompletionBounds", state: this.state, fenceCount: Number(this.sync !== void 0), completed: this.completed, timeouts: this.timeouts, insertionCpuMaxMs: this.insertionCpuMaxMs, failureReason: this.failureReason, latest: this.latest ? { ...this.latest } : void 0, upperBound: this.upperBound.snapshot() };
  }
};

// src/player/infra/render/gpu-frame-timer.ts
var GpuFrameTimer = class {
  constructor(gl) {
    this.gl = gl;
    this.completion = new GpuCompletionProbe(gl);
    try {
      this.extension = typeof gl.getExtension === "function" ? gl.getExtension("EXT_disjoint_timer_query_webgl2") ?? void 0 : void 0;
      if (this.extension)
        this.state = "ready";
    } catch {
      this.state = "unavailable";
    }
  }
  completion;
  extension;
  duration = new LatencyWindow();
  query;
  active = false;
  nextSampleAtMs = Number.NEGATIVE_INFINITY;
  sampledAtMs;
  queryStartedAtMs;
  state = "unavailable";
  disjointCount = 0;
  begin(atMs) {
    if (!this.extension || this.state === "failed" || atMs < this.nextSampleAtMs)
      return;
    this.nextSampleAtMs = atMs + 1e3;
    try {
      if (this.gl.getParameter(this.extension.GPU_DISJOINT_EXT)) {
        this.disjointCount++;
        this.duration.reset();
        this.sampledAtMs = void 0;
        if (this.disposeQuery())
          this.state = "disjoint";
        return;
      }
      if (this.query) {
        if (!this.gl.getQueryParameter(this.query, this.gl.QUERY_RESULT_AVAILABLE)) {
          if (atMs - (this.queryStartedAtMs ?? atMs) >= 5e3) {
            this.disposeQuery();
            this.invalidate();
          }
          return;
        }
        const ns = this.gl.getQueryParameter(this.query, this.gl.QUERY_RESULT);
        if (typeof ns !== "number" || !Number.isFinite(ns) || ns < 0)
          throw new Error("gpuTimingInvalid");
        this.duration.record(ns / 1e6);
        this.sampledAtMs = atMs;
        if (!this.disposeQuery())
          return;
      }
      this.query = this.gl.createQuery() ?? void 0;
      if (!this.query) {
        this.invalidate();
        return;
      }
      this.gl.beginQuery(this.extension.TIME_ELAPSED_EXT, this.query);
      this.queryStartedAtMs = atMs;
      this.active = true;
      this.state = "pending";
    } catch {
      this.disposeQuery();
      this.invalidate();
    }
  }
  end() {
    if (!this.active)
      return;
    try {
      this.gl.endQuery(this.extension.TIME_ELAPSED_EXT);
    } catch {
      this.active = false;
      this.disposeQuery();
      this.invalidate();
    }
    this.active = false;
  }
  disposeQuery() {
    this.end();
    try {
      if (this.query)
        this.gl.deleteQuery(this.query);
    } catch {
      this.invalidate();
    }
    this.query = void 0;
    this.queryStartedAtMs = void 0;
    return this.state !== "failed";
  }
  invalidate() {
    this.state = "failed";
    this.duration.reset();
    this.sampledAtMs = void 0;
  }
  submitted(atMs) {
    this.completion.submit(atMs);
  }
  dispose() {
    this.disposeQuery();
    this.completion.dispose();
  }
  snapshot() {
    return { source: "EXT_disjoint_timer_query_webgl2", state: this.state, sampledAtMs: this.sampledAtMs, disjointCount: this.disjointCount, queryCount: Number(this.query !== void 0), duration: this.duration.snapshot(), completion: this.completion.snapshot() };
  }
};

// src/player/infra/render/webgl-runtime-guard.ts
function releaseWebglContext(gl) {
  try {
    gl?.getExtension?.("WEBGL_lose_context")?.loseContext();
  } catch {
  }
}
var WebglRuntimeGuard = class {
  constructor(gl) {
    this.gl = gl;
    this.timer = new GpuFrameTimer(gl);
    this.textureLimit = gl.getParameter(gl.MAX_TEXTURE_SIZE);
    this.viewportLimit = gl.getParameter(gl.MAX_VIEWPORT_DIMS);
  }
  timer;
  textureLimit;
  viewportLimit;
  checkedAtMs = Number.NEGATIVE_INFINITY;
  errorChecks = 0;
  assertSize(sourceWidth, sourceHeight, outputWidth, outputHeight) {
    if (Math.max(sourceWidth, sourceHeight, outputWidth, outputHeight) > this.textureLimit || outputWidth > this.viewportLimit[0] || outputHeight > this.viewportLimit[1]) {
      throw new Error("sourceOrOutputExceedsGpuLimit");
    }
  }
  assertBacking(outputWidth, outputHeight) {
    if (this.gl.drawingBufferWidth < outputWidth || this.gl.drawingBufferHeight < outputHeight)
      throw new Error("gpuBackingBelowSourcePlan");
  }
  check(atMs, force = false) {
    if (!force && atMs - this.checkedAtMs < 1e3)
      return;
    this.checkedAtMs = atMs;
    this.errorChecks++;
    const error = this.gl.getError();
    if (error !== this.gl.NO_ERROR)
      throw new Error(`gpuRuntimeError:${error}`);
  }
};

// src/player/infra/render/video-frame-pixel-observation.ts
var WIDTH = 16;
var HEIGHT = 9;
var BYTES = WIDTH * HEIGHT * 4;
var INTERVAL_MS = 500;
var MIN_GAP_MS = 8;
var MAX_GAP_MS = 50;
var READ_BUDGET_MS = 8;
var COLD_BUDGET_MS = 50;
var PREPARATION_BUDGET_MS = 2e3;
var VERTEX = `#version 300 es
precision mediump float;
out vec2 uv;
void main() {
  vec2 p = vec2(float((gl_VertexID << 1) & 2), float(gl_VertexID & 2));
  uv = p;
  gl_Position = vec4(p * 2.0 - 1.0, 0.0, 1.0);
}`;
var FRAGMENT = `#version 300 es
precision mediump float;
uniform sampler2D videoTexture;
in vec2 uv;
out vec4 color;
void main() { color = texture(videoTexture, uv); }`;
var VideoFramePixelObservation = class {
  constructor(video, isCurrent) {
    this.video = video;
    this.isCurrent = isCurrent;
  }
  errorCheckedAtMs = Number.NEGATIVE_INFINITY;
  errorChecks = 0;
  cpuTiming = new LatencyWindow();
  gpuWaitTiming = new LatencyWindow();
  canvas;
  gl;
  texture;
  target;
  framebuffer;
  program;
  shaders = [];
  buffer;
  sync;
  timer;
  cancelEpoch = 0;
  baseline;
  nextBaselineAtMs = 0;
  cold = true;
  importedWidth = 0;
  importedHeight = 0;
  prepared = false;
  preparation;
  preparationEpoch = 0;
  preparationTiming = {
    count: 0,
    totalMs: 0,
    maxMs: 0,
    context: { count: 0, totalMs: 0, maxMs: 0 },
    shader: { count: 0, totalMs: 0, maxMs: 0 },
    resources: { count: 0, totalMs: 0, maxMs: 0 }
  };
  lastPreparation;
  eligibility;
  baselineCount = 0;
  pairCount = 0;
  changedPairCount = 0;
  readCount = 0;
  cancelledReadCount = 0;
  counterRebaseCount = 0;
  readDurationTotalMs = 0;
  readDurationMaxMs = 0;
  disabledReason;
  costs = {
    initialization: { count: 0, totalMs: 0, maxMs: 0 },
    upload: { count: 0, totalMs: 0, maxMs: 0 },
    readback: { count: 0, totalMs: 0, maxMs: 0 },
    gpuWait: { count: 0, totalMs: 0, maxMs: 0 },
    coldReads: { count: 0, totalMs: 0, maxMs: 0 },
    warmReads: { count: 0, totalMs: 0, maxMs: 0 }
  };
  firstRead;
  lastRead;
  discarded = {};
  evidence = [];
  onContextLost = () => this.fail("contextUnavailable");
  /** dev首次取证惰性准备；失败后停止该video的诊断读取。 */
  prepare() {
    if (this.disabledReason)
      return Promise.resolve();
    if (this.prepared)
      return Promise.resolve();
    if (this.preparation)
      return this.preparation;
    const epoch = this.preparationEpoch;
    const preparation = Promise.resolve().then(() => {
      if (epoch !== this.preparationEpoch)
        return;
      const startedAtMs = performance.now();
      let outcome = "success";
      try {
        this.initialize();
        if (performance.now() - startedAtMs > PREPARATION_BUDGET_MS)
          outcome = "initializationBudgetExceeded";
        else
          this.prepared = true;
      } catch {
        outcome = "contextUnavailable";
      }
      const durationMs = performance.now() - startedAtMs;
      this.preparationTiming.count++;
      this.preparationTiming.totalMs += durationMs;
      this.preparationTiming.maxMs = Math.max(this.preparationTiming.maxMs, durationMs);
      this.cost("initialization", durationMs);
      this.lastPreparation = { startedAtMs, finishedAtMs: performance.now(), durationMs, outcome };
      if (outcome !== "success")
        this.fail(outcome);
    }).finally(() => {
      if (this.preparation === preparation)
        this.preparation = void 0;
    });
    this.preparation = preparation;
    return preparation;
  }
  observe(input) {
    const state = this.disabledReason ? "disabled" : this.video.ownerDocument?.visibilityState === "hidden" ? "hidden" : this.video.paused ? "paused" : this.video.readyState < 2 ? "videoNotReady" : !this.video.videoWidth || !this.video.videoHeight ? "dimensionsMissing" : !this.prepared ? "preparing" : "active";
    this.eligibility = { observedAtMs: performance.now(), state, visibilityState: this.video.ownerDocument?.visibilityState ?? "unknown", paused: this.video.paused, readyState: this.video.readyState, sourceWidth: this.video.videoWidth, sourceHeight: this.video.videoHeight, drawableTargetFps: 0 };
    if (this.disabledReason || !Number.isFinite(input.enteredAtMs))
      return;
    if (!this.drawable()) {
      this.discard("videoNotDrawable");
      return;
    }
    if (this.importedWidth && (this.importedWidth !== this.video.videoWidth || this.importedHeight !== this.video.videoHeight)) {
      this.discard("sourceSizeChanged");
      this.cold = true;
      this.importedWidth = this.importedHeight = 0;
    }
    if (!this.prepared) {
      void this.prepare();
      return;
    }
    if (input.counterReset || input.trackingSource === "videoFrameCallback" || input.newFrame) {
      this.discard(input.counterReset ? "counterReset" : input.trackingSource === "videoFrameCallback" ? "nativeIntervened" : "identityChanged");
      if (input.newFrame && !input.counterReset && input.enteredAtMs >= this.nextBaselineAtMs) {
        this.nextBaselineAtMs = input.enteredAtMs + INTERVAL_MS + this.baselineCount % 7 * 17;
        this.read(input);
      }
      return;
    }
    if (input.trackingSource !== "animationFrame" || this.sync || !this.baseline)
      return;
    const baseline = this.baseline;
    if (input.counterPaired && baseline.epoch === input.epoch && baseline.localSequence === input.localSequence && baseline.nativeCallbackCount === input.nativeCallbackCount && baseline.counterSource === input.counterSource) {
      baseline.counterFrames = input.counterFrames;
      this.counterRebaseCount++;
    }
    if (!this.sameIdentity(baseline, input)) {
      this.discard("identityChanged");
      return;
    }
    const gapMs = input.enteredAtMs - baseline.readStartedAtMs;
    if (gapMs > MAX_GAP_MS) {
      this.discard("expired");
      return;
    }
    if (gapMs >= MIN_GAP_MS)
      this.read(input, baseline);
  }
  checkError(force = false) {
    const now = performance.now();
    if (!force && now - this.errorCheckedAtMs < 1e3)
      return;
    this.errorCheckedAtMs = now;
    this.errorChecks++;
    if (this.gl.getError() !== this.gl.NO_ERROR)
      throw new Error("probeGpuError");
  }
  drawable() {
    return this.video.ownerDocument?.visibilityState !== "hidden" && !this.video.paused && this.video.readyState >= 2 && this.video.videoWidth > 0 && this.video.videoHeight > 0;
  }
  sameIdentity(a, b) {
    return a.epoch === b.epoch && a.localSequence === b.localSequence && a.nativeCallbackCount === b.nativeCallbackCount && a.counterSource === b.counterSource && a.counterFrames === b.counterFrames;
  }
  initialize() {
    this.measurePreparation("context", () => this.initializeContext());
    this.measurePreparation("shader", () => this.initializeProgram());
    this.measurePreparation("resources", () => this.initializeResources());
  }
  measurePreparation(phase, action) {
    const startedAtMs = performance.now();
    try {
      action();
    } finally {
      const value = this.preparationTiming[phase];
      const durationMs = performance.now() - startedAtMs;
      value.count++;
      value.totalMs += durationMs;
      value.maxMs = Math.max(value.maxMs, durationMs);
    }
  }
  initializeContext() {
    this.canvas = this.video.ownerDocument.createElement("canvas");
    this.canvas.width = WIDTH;
    this.canvas.height = HEIGHT;
    this.canvas.addEventListener("webglcontextlost", this.onContextLost);
    const gl = this.canvas.getContext("webgl2", { alpha: false, antialias: false, depth: false, stencil: false, preserveDrawingBuffer: false });
    this.gl = gl ?? void 0;
    if (!gl || typeof gl.fenceSync !== "function" || typeof gl.getBufferSubData !== "function")
      throw new Error("contextUnavailable");
  }
  initializeProgram() {
    const gl = this.gl;
    const program = gl.createProgram();
    if (!program)
      throw new Error("programUnavailable");
    this.program = program;
    for (const [type, source] of [[gl.VERTEX_SHADER, VERTEX], [gl.FRAGMENT_SHADER, FRAGMENT]]) {
      const shader = gl.createShader(type);
      if (!shader)
        throw new Error("shaderUnavailable");
      this.shaders.push(shader);
      gl.shaderSource(shader, source);
      gl.compileShader(shader);
      if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS))
        throw new Error("shaderFailed");
      gl.attachShader(program, shader);
    }
    gl.linkProgram(program);
    if (!gl.getProgramParameter(program, gl.LINK_STATUS))
      throw new Error("programFailed");
  }
  initializeResources() {
    const gl = this.gl;
    this.texture = gl.createTexture() ?? void 0;
    this.target = gl.createTexture() ?? void 0;
    this.framebuffer = gl.createFramebuffer() ?? void 0;
    this.buffer = gl.createBuffer() ?? void 0;
    if (!this.texture || !this.target || !this.framebuffer || !this.buffer)
      throw new Error("resourceUnavailable");
    for (const texture of [this.texture, this.target]) {
      gl.bindTexture(gl.TEXTURE_2D, texture);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
    }
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, WIDTH, HEIGHT, 0, gl.RGBA, gl.UNSIGNED_BYTE, null);
    gl.bindFramebuffer(gl.FRAMEBUFFER, this.framebuffer);
    gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, this.target, 0);
    if (gl.checkFramebufferStatus(gl.FRAMEBUFFER) !== gl.FRAMEBUFFER_COMPLETE)
      throw new Error("framebufferFailed");
    gl.bindBuffer(gl.PIXEL_PACK_BUFFER, this.buffer);
    gl.bufferData(gl.PIXEL_PACK_BUFFER, BYTES, gl.STREAM_READ);
    gl.useProgram(this.program);
    gl.uniform1i(gl.getUniformLocation(this.program, "videoTexture"), 0);
    gl.viewport(0, 0, WIDTH, HEIGHT);
  }
  measure(phase, timing, action) {
    const start = performance.now();
    try {
      action();
    } finally {
      const ms = performance.now() - start;
      timing[`${phase}Ms`] = ms;
      timing.durationMs += ms;
      this.cost(phase, ms);
    }
  }
  cost(phase, ms) {
    const value = this.costs[phase];
    value.count++;
    value.totalMs += ms;
    value.maxMs = Math.max(value.maxMs, ms);
  }
  read(input, baseline) {
    if (this.sync)
      return;
    const epoch = this.cancelEpoch;
    const sourceWidth = this.video.videoWidth;
    const sourceHeight = this.video.videoHeight;
    const timing = { readStartedAtMs: performance.now(), readFinishedAtMs: 0, durationMs: 0, cold: this.cold, initializationMs: 0, uploadMs: 0, readbackMs: 0, gpuWaitMs: 0, trackingSource: input.trackingSource, localSequence: input.localSequence, outcome: "success" };
    this.readCount++;
    try {
      const gl = this.gl;
      this.measure("upload", timing, () => {
        gl.activeTexture(gl.TEXTURE0);
        gl.bindTexture(gl.TEXTURE_2D, this.texture);
        gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, gl.RGBA, gl.UNSIGNED_BYTE, this.video);
        this.importedWidth = sourceWidth;
        this.importedHeight = sourceHeight;
        gl.drawArrays(gl.TRIANGLES, 0, 3);
        gl.readPixels(0, 0, WIDTH, HEIGHT, gl.RGBA, gl.UNSIGNED_BYTE, 0);
        this.sync = gl.fenceSync(gl.SYNC_GPU_COMMANDS_COMPLETE, 0) ?? void 0;
        gl.flush();
        if (!this.sync)
          throw new Error("readFailed");
        this.checkError(timing.cold);
      });
      if (this.overBudget(timing)) {
        this.finish(timing, "readbackBudgetExceeded");
        return;
      }
      this.cold = false;
      const submittedAtMs = performance.now();
      const poll = () => {
        this.timer = void 0;
        if (epoch !== this.cancelEpoch)
          return;
        timing.gpuWaitMs = performance.now() - submittedAtMs;
        if (!this.drawable() || !this.isCurrent(input) || sourceWidth !== this.video.videoWidth || sourceHeight !== this.video.videoHeight) {
          this.finish(timing, "discarded");
          this.discard("asyncIdentityChanged", true);
          return;
        }
        if (timing.gpuWaitMs > MAX_GAP_MS) {
          this.finish(timing, "gpuWaitExpired");
          return;
        }
        try {
          const status = gl.clientWaitSync(this.sync, 0, 0);
          if (status === gl.TIMEOUT_EXPIRED) {
            this.timer = setTimeout(poll, 1);
            return;
          }
          if (status !== gl.ALREADY_SIGNALED && status !== gl.CONDITION_SATISFIED)
            throw new Error("fenceFailed");
          const pixels = new Uint8Array(BYTES);
          this.measure("readback", timing, () => gl.getBufferSubData(gl.PIXEL_PACK_BUFFER, 0, pixels));
          this.checkError();
          if (this.overBudget(timing)) {
            this.finish(timing, "readbackBudgetExceeded");
            return;
          }
          if (!this.isCurrent(input) || sourceWidth !== this.video.videoWidth || sourceHeight !== this.video.videoHeight) {
            this.finish(timing, "discarded");
            this.discard("asyncSampleExpired", true);
            return;
          }
          this.finish(timing, "success");
          this.cold = false;
          const sample = { ...input, capturedCounterFrames: input.counterFrames, sourceWidth, sourceHeight, pixels, readStartedAtMs: timing.readStartedAtMs, readFinishedAtMs: timing.readFinishedAtMs };
          if (baseline)
            this.compare(baseline, sample);
          else
            this.baselineCount++;
          this.baseline = baseline ? void 0 : sample;
        } catch {
          this.finish(timing, "readFailed");
        }
      };
      this.timer = setTimeout(poll, 1);
    } catch {
      this.finish(timing, this.gl && typeof this.gl.fenceSync === "function" ? "readFailed" : "contextUnavailable");
    }
  }
  overBudget(timing) {
    return timing.durationMs > (timing.cold ? COLD_BUDGET_MS : READ_BUDGET_MS);
  }
  finish(timing, outcome) {
    this.cpuTiming.record(timing.durationMs);
    this.gpuWaitTiming.record(timing.gpuWaitMs);
    timing.readFinishedAtMs = performance.now();
    timing.outcome = outcome;
    this.readDurationTotalMs += timing.durationMs;
    this.readDurationMaxMs = Math.max(this.readDurationMaxMs, timing.durationMs);
    this.cost(timing.cold ? "coldReads" : "warmReads", timing.durationMs);
    this.cost("gpuWait", timing.gpuWaitMs);
    this.lastRead = { ...timing };
    this.firstRead ??= { ...timing };
    this.cancelRead(true);
    if (outcome !== "success" && outcome !== "discarded")
      this.fail(outcome);
  }
  compare(baseline, sample) {
    let changedPixelCount = 0;
    let maxChannelDelta = 0;
    let absoluteRgbDeltaTotal = 0;
    for (let i = 0; i < BYTES; i += 4) {
      let changed = false;
      for (let channel = 0; channel < 3; channel++) {
        const delta = Math.abs(sample.pixels[i + channel] - baseline.pixels[i + channel]);
        changed ||= delta > 0;
        maxChannelDelta = Math.max(maxChannelDelta, delta);
        absoluteRgbDeltaTotal += delta;
      }
      changedPixelCount += Number(changed);
    }
    this.pairCount++;
    this.changedPairCount += Number(changedPixelCount > 0);
    this.evidence.push({ baselineAtMs: baseline.enteredAtMs, observedAtMs: sample.enteredAtMs, baselineReadStartedAtMs: baseline.readStartedAtMs, baselineReadFinishedAtMs: baseline.readFinishedAtMs, readStartedAtMs: sample.readStartedAtMs, readFinishedAtMs: sample.readFinishedAtMs, gapMs: sample.readStartedAtMs - baseline.readStartedAtMs, localSequence: sample.localSequence, nativeCallbackCount: sample.nativeCallbackCount, counterSource: sample.counterSource, counterFrames: sample.counterFrames, baselineCounterFrames: baseline.capturedCounterFrames, sourceWidth: sample.sourceWidth, sourceHeight: sample.sourceHeight, changedPixelCount, maxChannelDelta, absoluteRgbDeltaTotal });
    if (this.evidence.length > 8)
      this.evidence.shift();
    return changedPixelCount > 0;
  }
  cancelRead(completed = false) {
    this.cancelledReadCount += Number(!completed && (this.sync !== void 0 || this.timer !== void 0));
    if (this.timer !== void 0)
      clearTimeout(this.timer);
    if (this.sync)
      this.gl?.deleteSync(this.sync);
    this.timer = this.sync = void 0;
  }
  discard(reason, completedRead = false) {
    if (completedRead || this.baseline || this.sync)
      this.discarded[reason] = (this.discarded[reason] ?? 0) + 1;
    this.cancelEpoch++;
    this.cancelRead();
    this.baseline = void 0;
  }
  fail(reason) {
    this.disabledReason = reason;
    this.reset();
  }
  reset() {
    this.preparationEpoch++;
    this.preparation = void 0;
    this.prepared = false;
    this.discard("observationInterrupted");
    const gl = this.gl;
    if (gl) {
      if (this.buffer)
        gl.deleteBuffer(this.buffer);
      if (this.texture)
        gl.deleteTexture(this.texture);
      if (this.target)
        gl.deleteTexture(this.target);
      if (this.framebuffer)
        gl.deleteFramebuffer(this.framebuffer);
      if (this.program)
        gl.deleteProgram(this.program);
      for (const shader of this.shaders) gl.deleteShader(shader);
    }
    this.canvas?.removeEventListener("webglcontextlost", this.onContextLost);
    if (this.canvas)
      this.canvas.width = this.canvas.height = 1;
    releaseWebglContext(gl);
    this.canvas = this.gl = this.texture = this.target = this.framebuffer = this.program = this.buffer = void 0;
    this.shaders.length = 0;
    this.cold = true;
    this.importedWidth = this.importedHeight = 0;
  }
  snapshotResources() {
    return { pixelProbeCanvases: Number(this.canvas !== void 0), pixelProbePendingPairs: Number(this.baseline !== void 0), pixelProbeGpuReads: Number(this.sync !== void 0), pixelProbeTimers: Number(this.timer !== void 0), pixelProbePreparations: Number(this.preparation !== void 0) };
  }
  snapshot() {
    return {
      evidence: "webgl2AsyncVideoPixels",
      supplyMode: "diagnostic",
      gpuErrorChecks: this.errorChecks,
      timing: { cpu: this.cpuTiming.snapshot(), gpuWait: this.gpuWaitTiming.snapshot() },
      sampleWidth: WIDTH,
      sampleHeight: HEIGHT,
      readbackBytes: BYTES,
      minimumPairIntervalMs: INTERVAL_MS,
      minimumGapMs: MIN_GAP_MS,
      maximumGapMs: MAX_GAP_MS,
      readBudgetMs: READ_BUDGET_MS,
      coldReadBudgetMs: COLD_BUDGET_MS,
      preparationBudgetMs: PREPARATION_BUDGET_MS,
      drawableTargetFps: 0,
      preparationState: this.disabledReason ? "disabled" : this.prepared ? "ready" : this.preparation ? "preparing" : "idle",
      preparationTiming: {
        ...this.preparationTiming,
        context: { ...this.preparationTiming.context },
        shader: { ...this.preparationTiming.shader },
        resources: { ...this.preparationTiming.resources },
        last: this.lastPreparation ? { ...this.lastPreparation } : void 0
      },
      eligibility: this.eligibility ? { ...this.eligibility } : void 0,
      baselineCount: this.baselineCount,
      pairCount: this.pairCount,
      changedPairCount: this.changedPairCount,
      deliveredFrameCount: 0,
      readCount: this.readCount,
      completedReadCount: this.costs.coldReads.count + this.costs.warmReads.count,
      cancelledReadCount: this.cancelledReadCount,
      counterRebaseCount: this.counterRebaseCount,
      readDurationTotalMs: this.readDurationTotalMs,
      readDurationMaxMs: this.readDurationMaxMs,
      readTiming: {
        initialization: { ...this.costs.initialization },
        upload: { ...this.costs.upload },
        readback: { ...this.costs.readback },
        gpuWait: { ...this.costs.gpuWait },
        coldReads: { ...this.costs.coldReads },
        warmReads: { ...this.costs.warmReads },
        firstRead: this.firstRead ? { ...this.firstRead } : void 0,
        lastRead: this.lastRead ? { ...this.lastRead } : void 0
      },
      disabledReason: this.disabledReason,
      discarded: { ...this.discarded },
      evidenceLimit: 8,
      pairs: this.evidence.map((pair) => ({ ...pair })),
      ...this.snapshotResources()
    };
  }
};

// src/player/infra/render/video-frame-source.ts
var sources = /* @__PURE__ */ new WeakMap();
var videoGeneration = 0;
var NATIVE_COUNTER_CREDIT_WINDOW_MS = 1e3;
function readWebkitFrameCounter(video) {
  try {
    const webkit = video;
    const total = webkit.webkitDecodedFrameCount;
    const dropped = webkit.webkitDroppedFrameCount ?? 0;
    if (total !== void 0 && Number.isFinite(total) && Number.isFinite(dropped))
      return { source: "webkit", frames: Math.max(0, total - dropped), webkitTotalFrames: total, webkitDroppedFrames: dropped };
  } catch {
  }
}
function readVideoFrameCounter(video) {
  let qualityFrames;
  let playbackQualityTotalFrames;
  let playbackQualityDroppedFrames;
  try {
    const quality = video.getVideoPlaybackQuality?.();
    if (quality && Number.isFinite(quality.totalVideoFrames) && Number.isFinite(quality.droppedVideoFrames)) {
      qualityFrames = Math.max(0, quality.totalVideoFrames - quality.droppedVideoFrames);
      playbackQualityTotalFrames = quality.totalVideoFrames;
      playbackQualityDroppedFrames = quality.droppedVideoFrames;
    }
  } catch {
  }
  if (qualityFrames !== void 0 && qualityFrames > 0)
    return { source: "playbackQuality", frames: qualityFrames, playbackQualityTotalFrames, playbackQualityDroppedFrames };
  const webkit = readWebkitFrameCounter(video);
  if (webkit)
    return { ...webkit, playbackQualityTotalFrames, playbackQualityDroppedFrames };
  return qualityFrames === void 0 ? void 0 : { source: "playbackQuality", frames: qualityFrames, playbackQualityTotalFrames, playbackQualityDroppedFrames };
}
var VideoFrameSource = class {
  constructor(video) {
    this.video = video;
    this.pixelObservation = new VideoFramePixelObservation(video, (input) => this.pixelIdentityCurrent(input));
    this.counter = readVideoFrameCounter(video);
    this.counterSource = this.counter?.source;
    if (typeof video.requestVideoFrameCallback !== "function")
      this.lastMediaTime = video.currentTime;
  }
  pending = /* @__PURE__ */ new Map();
  decodedReader;
  decodedFrame;
  renderFrame;
  renderSequence = 0;
  nextRequest = 0;
  frameSequence = 0;
  lastPresentedFrames;
  lastMediaTime;
  counter;
  counterSource;
  uncountedNativeFrames = 0;
  nativeCreditStartedAtMs;
  counterProgressAtMs;
  expiredNativeFramesTotal = 0;
  nativeCreditWindowExpirationCount = 0;
  polledFramesSinceNative = 0;
  epoch = 0;
  nativeHandle;
  animationHandle;
  presentationWatchdog;
  lastNativeCallbackAtMs;
  nativeStalled = false;
  timeUpdateBound = false;
  nativeCallbackCount = 0;
  polledFrameCount = 0;
  identityObservation = new VideoFrameIdentityObservation();
  pixelObservation;
  pixelDiagnosticsEnabled = false;
  diagnosticRtpTimestamp;
  videoGeneration = ++videoGeneration;
  lastFrame;
  get lastRenderFrame() {
    return this.renderFrame;
  }
  get renderSource() {
    return this.decodedFrame;
  }
  get renderSize() {
    return { width: this.decodedFrame?.displayWidth ?? 0, height: this.decodedFrame?.displayHeight ?? 0 };
  }
  get renderSupply() {
    return { renderFrameSource: this.decodedReader?.mode ?? "presentation", renderFrameFallbackReason: this.decodedReader?.fallbackReason, decodedFramesReceived: this.decodedReader?.receivedFrames ?? 0, decodedFrameSupply: this.decodedReader?.snapshot() };
  }
  onTimeUpdate = () => {
    this.observe(performance.now(), "timeupdate");
    this.schedule();
  };
  snapshotResources() {
    return { videoGeneration: this.videoGeneration, renderFrameSource: this.decodedReader?.mode ?? "presentation", renderFrameFallbackReason: this.decodedReader?.fallbackReason, decodedFramesReceived: this.decodedReader?.receivedFrames ?? 0, decodedFrameReaders: Number(this.decodedReader !== void 0 && this.decodedReader.mode !== "presentation"), retainedDecodedFrames: Number(this.decodedFrame !== void 0), renderFrameTimestampUs: this.decodedFrame?.timestamp, frameRequests: this.pending.size, framePreparationRequests: 0, nativeFrameCallbacks: Number(this.nativeHandle !== void 0), animationCallbacks: Number(this.animationHandle !== void 0), localSequence: this.frameSequence, frameCounterSource: this.counterSource, nativeCallbackCount: this.nativeCallbackCount, polledFrameCount: this.polledFrameCount, uncountedNativeFrames: this.uncountedNativeFrames, expiredNativeFramesTotal: this.expiredNativeFramesTotal, nativeCreditWindowExpirationCount: this.nativeCreditWindowExpirationCount, nativeCounterCreditWindowMs: NATIVE_COUNTER_CREDIT_WINDOW_MS, ...this.pixelObservation.snapshotResources() };
  }
  snapshotDiagnostics(options) {
    if (options?.videoPixels !== void 0 && options.videoPixels !== this.pixelDiagnosticsEnabled) {
      this.pixelDiagnosticsEnabled = options.videoPixels;
      if (!options.videoPixels)
        this.pixelObservation.reset();
    }
    return { videoGeneration: this.videoGeneration, localSequence: this.frameSequence, ...this.identityObservation.snapshot(), videoPixels: { ...this.pixelObservation.snapshot(), enabled: this.pixelDiagnosticsEnabled } };
  }
  request(callback) {
    const id = ++this.nextRequest;
    this.pending.set(id, { callback });
    this.schedule();
    return id;
  }
  requestRender(callback, onFailure) {
    const id = ++this.nextRequest;
    this.pending.set(id, { callback, render: true, onFailure });
    if (!this.decodedReader) {
      this.renderSequence = Math.max(this.renderSequence, this.lastFrame?.localSequence ?? 0);
      const reader = new DecodedVideoFrameReader(this.video, (frame, delivery) => this.observeDecoded(frame, delivery), (reason) => {
        queueMicrotask(() => {
          if (this.decodedReader === reader)
            this.failRenderRequests(reason);
        });
      });
      this.decodedReader = reader;
    } else if (this.decodedReader.fallbackReason) {
      const reader = this.decodedReader;
      queueMicrotask(() => {
        if (this.decodedReader === reader && reader.fallbackReason)
          this.failRenderRequests(reader.fallbackReason);
      });
    }
    this.schedule();
    return id;
  }
  failRenderRequests(reason) {
    this.releaseDecodedFrame();
    const requests = [...this.pending.entries()].filter(([, request]) => request.render);
    for (const [id] of requests)
      this.pending.delete(id);
    for (const [, request] of requests) {
      try {
        request.onFailure?.(reason);
      } catch (error) {
        if (typeof reportError === "function")
          reportError(error);
      }
    }
    this.schedule();
  }
  observeDecoded(frame, delivery) {
    if (![...this.pending.values()].some((request) => request.render)) {
      frame.close();
      return;
    }
    const previous = this.decodedFrame;
    this.decodedFrame = frame;
    const metadata = { newFrame: true, trackingSource: "decodedFrame", localSequence: ++this.renderSequence, videoGeneration: this.videoGeneration, mediaTime: frame.timestamp / 1e6, ...delivery };
    this.renderFrame = metadata;
    this.notify(performance.now(), metadata);
    previous?.close();
  }
  releaseDecodedFrame() {
    this.decodedFrame?.close();
    this.decodedFrame = void 0;
    this.renderFrame = void 0;
  }
  cancel(id) {
    this.pending.delete(id);
    if (![...this.pending.values()].some((request) => request.render)) {
      this.decodedReader?.dispose();
      this.decodedReader = void 0;
      this.releaseDecodedFrame();
    }
    if (this.pending.size === 0)
      this.cancelScheduled();
  }
  pixelIdentityCurrent(input) {
    const counter = readVideoFrameCounter(this.video);
    return this.pending.size > 0 && input.epoch === this.epoch && input.localSequence === this.frameSequence && input.nativeCallbackCount === this.nativeCallbackCount && counter?.source === input.counterSource && counter?.frames === input.counterFrames;
  }
  cancelScheduled() {
    this.cancelPresentationCallbacks();
    this.uncountedNativeFrames = this.polledFramesSinceNative = 0;
    this.nativeCreditStartedAtMs = this.counterProgressAtMs = void 0;
    this.identityObservation.resetTimingBaseline();
    this.pixelObservation.reset();
    this.pixelDiagnosticsEnabled = false;
    this.epoch++;
  }
  cancelPresentationCallbacks() {
    if (this.nativeHandle !== void 0)
      this.video.cancelVideoFrameCallback?.(this.nativeHandle);
    if (this.animationHandle !== void 0)
      cancelAnimationFrame(this.animationHandle);
    if (this.timeUpdateBound) {
      this.video.removeEventListener?.("timeupdate", this.onTimeUpdate);
      this.timeUpdateBound = false;
    }
    this.nativeHandle = this.animationHandle = void 0;
    if (this.presentationWatchdog !== void 0)
      clearTimeout(this.presentationWatchdog);
    this.presentationWatchdog = void 0;
    this.lastNativeCallbackAtMs = void 0;
    this.nativeStalled = false;
  }
  schedule() {
    if (this.pending.size === 0) {
      this.cancelScheduled();
      return;
    }
    if (![...this.pending.values()].some((request) => !request.render) && !this.pixelDiagnosticsEnabled && this.decodedReader?.mode !== "initializing") {
      this.cancelPresentationCallbacks();
      return;
    }
    const epoch = this.epoch;
    const direct = this.decodedReader !== void 0 && this.decodedReader.mode !== "presentation";
    if (typeof this.video.requestVideoFrameCallback === "function") {
      if (direct && this.presentationWatchdog === void 0) {
        this.lastNativeCallbackAtMs ??= performance.now();
        this.presentationWatchdog = setTimeout(() => {
          this.presentationWatchdog = void 0;
          if (epoch !== this.epoch)
            return;
          this.nativeStalled = performance.now() - (this.lastNativeCallbackAtMs ?? performance.now()) >= 250;
          this.schedule();
        }, 250);
      }
      if (this.nativeHandle === void 0) {
        this.nativeHandle = this.video.requestVideoFrameCallback((atMs, metadata) => {
          if (epoch !== this.epoch)
            return;
          this.nativeHandle = void 0;
          if (this.decodedReader && this.decodedReader.mode !== "presentation")
            this.lastNativeCallbackAtMs = performance.now();
          this.nativeStalled = false;
          if (direct && this.animationHandle !== void 0) {
            cancelAnimationFrame(this.animationHandle);
            this.animationHandle = void 0;
          }
          this.nativeCallbackCount++;
          this.observe(atMs, "videoFrameCallback", metadata);
          this.schedule();
        });
      }
    } else if (!this.timeUpdateBound) {
      this.video.addEventListener?.("timeupdate", this.onTimeUpdate);
      this.timeUpdateBound = true;
    }
    const needsPolling = !direct || typeof this.video.requestVideoFrameCallback !== "function" || this.nativeStalled;
    if (!needsPolling && this.animationHandle !== void 0) {
      cancelAnimationFrame(this.animationHandle);
      this.animationHandle = void 0;
    }
    if (needsPolling && this.animationHandle === void 0 && typeof requestAnimationFrame === "function") {
      this.animationHandle = requestAnimationFrame((atMs) => {
        if (epoch !== this.epoch)
          return;
        this.animationHandle = void 0;
        this.observe(atMs, "animationFrame");
        this.schedule();
      });
    }
  }
  observe(atMs, trackingSource, metadata) {
    if (this.pending.size === 0)
      return;
    const callbackEnteredAtMs = this.identityObservation.enabled ? performance.now() : void 0;
    const counter = readVideoFrameCounter(this.video);
    this.counterSource = counter?.source;
    const sameCounter = counter?.source === this.counter?.source;
    const uncountedNativeBefore = this.uncountedNativeFrames;
    const polledAdvanceBefore = this.polledFramesSinceNative;
    const previousMediaTime = this.lastMediaTime;
    const counterDelta = counter && sameCounter && this.counter && counter.frames >= this.counter.frames ? counter.frames - this.counter.frames : void 0;
    const counterReset = Boolean(counter && (!sameCounter || this.counter && counter.frames < this.counter.frames));
    const nativeReset = metadata?.presentedFrames !== void 0 && this.lastPresentedFrames !== void 0 && metadata.presentedFrames < this.lastPresentedFrames;
    if (counterReset || nativeReset) {
      this.uncountedNativeFrames = 0;
      this.polledFramesSinceNative = 0;
      this.nativeCreditStartedAtMs = void 0;
      this.counterProgressAtMs = atMs;
    }
    const nativeDelta = metadata?.presentedFrames !== void 0 && this.lastPresentedFrames !== void 0 ? metadata.presentedFrames >= this.lastPresentedFrames ? metadata.presentedFrames - this.lastPresentedFrames : 1 : void 0;
    const mediaTime = metadata?.mediaTime ?? (typeof this.video.requestVideoFrameCallback === "function" ? void 0 : this.video.currentTime);
    const advance = counter && counter.frames > 0 ? counterDelta ?? 1 : 0;
    this.counterProgressAtMs ??= atMs;
    const counterProgressAgeMs = atMs - this.counterProgressAtMs;
    const counterWindowExpired = counterProgressAgeMs > NATIVE_COUNTER_CREDIT_WINDOW_MS;
    const nativeCreditWindowExpired = counterWindowExpired || this.nativeCreditStartedAtMs !== void 0 && atMs - this.nativeCreditStartedAtMs > NATIVE_COUNTER_CREDIT_WINDOW_MS;
    const expiredNativeFrames = nativeCreditWindowExpired ? this.uncountedNativeFrames : 0;
    if (nativeCreditWindowExpired) {
      this.uncountedNativeFrames = 0;
      this.nativeCreditStartedAtMs = void 0;
      this.expiredNativeFramesTotal += expiredNativeFrames;
      this.nativeCreditWindowExpirationCount += Number(expiredNativeFrames > 0);
    }
    if (advance > 0)
      this.counterProgressAtMs = atMs;
    const credited = Math.min(advance, this.uncountedNativeFrames);
    this.uncountedNativeFrames -= credited;
    if (this.uncountedNativeFrames === 0)
      this.nativeCreditStartedAtMs = void 0;
    const counterAdvance = advance - credited;
    let presentedFramesDelta = counter ? counterAdvance : void 0;
    let newFrame = counterAdvance > 0;
    let nativeNewFrame = false;
    let nativeFrames = 0;
    let nativeAdvance = 0;
    let creditedPollFrames = 0;
    let expiredPollFrames = 0;
    if (trackingSource === "videoFrameCallback") {
      nativeNewFrame = metadata?.presentedFrames !== void 0 ? this.lastPresentedFrames === void 0 || (nativeDelta ?? 0) > 0 : mediaTime === void 0 || this.lastMediaTime === void 0 || mediaTime !== this.lastMediaTime;
      nativeFrames = nativeNewFrame ? nativeDelta ?? (metadata?.presentedFrames === 0 ? 0 : 1) : 0;
      creditedPollFrames = Math.min(nativeFrames, this.polledFramesSinceNative);
      nativeAdvance = nativeFrames - creditedPollFrames;
      if (nativeNewFrame) {
        expiredPollFrames = this.polledFramesSinceNative - creditedPollFrames;
        this.polledFramesSinceNative = 0;
      }
      if (this.counter && (!counterWindowExpired || advance > 0) && (this.lastFrame !== void 0 || this.counter.frames === 0)) {
        this.uncountedNativeFrames += Math.max(0, nativeAdvance - counterAdvance);
        if (this.uncountedNativeFrames > 0)
          this.nativeCreditStartedAtMs ??= atMs;
      }
      presentedFramesDelta = Math.max(counterAdvance, nativeAdvance);
      newFrame ||= nativeAdvance > 0 || nativeNewFrame && this.lastFrame === void 0;
    } else if (counter) {
      this.polledFramesSinceNative += counterAdvance;
    } else {
      newFrame = counter === void 0 && mediaTime !== void 0 && Number.isFinite(mediaTime) && mediaTime !== this.lastMediaTime;
    }
    if (this.identityObservation.enabled) {
      const nativeState = identityValueState(metadata?.presentedFrames, this.lastPresentedFrames);
      const mediaTimeState = identityValueState(mediaTime, previousMediaTime);
      const reason = newFrame ? counterAdvance > 0 ? "counterAdvanced" : nativeAdvance > 0 ? "nativeAdvanced" : nativeNewFrame ? "initialNativeFrame" : "mediaTimeAdvanced" : trackingSource === "videoFrameCallback" ? nativeNewFrame && nativeFrames > 0 && nativeAdvance === 0 ? "pollAlreadyDelivered" : metadata?.presentedFrames !== void 0 ? "nativeCounterUnchanged" : mediaTime === void 0 ? "identityUnavailable" : "mediaTimeUnchanged" : advance > 0 && credited >= advance ? "nativeAlreadyDelivered" : counter ? "counterUnchanged" : mediaTime === void 0 ? "identityUnavailable" : "mediaTimeUnchanged";
      this.identityObservation.count(trackingSource, newFrame, reason, nativeState === "unchanged" && mediaTimeState === "advanced");
      const timing = this.identityObservation.recordTiming(trackingSource, atMs, callbackEnteredAtMs, counterDelta, counterProgressAgeMs, counterReset);
      if (this.identityObservation.shouldSample(atMs, newFrame)) {
        const webkit = counter?.source === "webkit" ? counter : counter?.source === "playbackQuality" && counter.frames > 0 ? readWebkitFrameCounter(this.video) : void 0;
        const pairedCounters = this.identityObservation.pairCounters(performance.now(), {
          playbackQualityTotalFrames: finiteIdentityValue(counter?.playbackQualityTotalFrames),
          playbackQualityDroppedFrames: finiteIdentityValue(counter?.playbackQualityDroppedFrames),
          playbackQualityFrames: finiteIdentityValue(counter?.playbackQualityTotalFrames !== void 0 && counter?.playbackQualityDroppedFrames !== void 0 ? Math.max(0, counter.playbackQualityTotalFrames - counter.playbackQualityDroppedFrames) : void 0),
          webkitTotalFrames: finiteIdentityValue(webkit?.webkitTotalFrames),
          webkitDroppedFrames: finiteIdentityValue(webkit?.webkitDroppedFrames),
          webkitFrames: finiteIdentityValue(webkit?.frames)
        });
        this.identityObservation.capture({
          atMs,
          ...timing,
          pairedCounters,
          trackingSource,
          newFrame,
          reason,
          localSequence: this.frameSequence + Number(newFrame),
          nativePresentedFrames: finiteIdentityValue(metadata?.presentedFrames),
          previousNativePresentedFrames: finiteIdentityValue(this.lastPresentedFrames),
          nativeDelta: finiteIdentityValue(nativeDelta),
          rawNativeDelta: finiteIdentityValue(metadata?.presentedFrames !== void 0 && this.lastPresentedFrames !== void 0 ? metadata.presentedFrames - this.lastPresentedFrames : void 0),
          nativeState,
          mediaTime: finiteIdentityValue(mediaTime),
          previousMediaTime: finiteIdentityValue(previousMediaTime),
          mediaTimeDelta: finiteIdentityValue(mediaTime !== void 0 && previousMediaTime !== void 0 ? mediaTime - previousMediaTime : void 0),
          mediaTimeState,
          presentationTime: finiteIdentityValue(metadata?.presentationTime),
          expectedDisplayTime: finiteIdentityValue(metadata?.expectedDisplayTime),
          rtpTimestamp: finiteIdentityValue(metadata?.rtpTimestamp),
          previousRtpTimestamp: finiteIdentityValue(this.diagnosticRtpTimestamp),
          rtpTimestampState: identityValueState(metadata?.rtpTimestamp, this.diagnosticRtpTimestamp),
          counterSource: counter?.source,
          counterFrames: finiteIdentityValue(counter?.frames),
          previousCounterFrames: finiteIdentityValue(this.counter?.frames),
          previousCounterSource: this.counter?.source,
          counterDelta: finiteIdentityValue(counterDelta),
          counterReset,
          nativeReset,
          counterAdvance: finiteIdentityValue(counterAdvance),
          nativeAdvance: finiteIdentityValue(nativeAdvance),
          creditedNativeFrames: finiteIdentityValue(credited),
          expiredNativeFrames,
          nativeCreditWindowExpired,
          counterProgressAgeMs: finiteIdentityValue(counterProgressAgeMs),
          presentedFramesDelta: finiteIdentityValue(presentedFramesDelta),
          uncountedNativeBefore: finiteIdentityValue(uncountedNativeBefore),
          uncountedNativeAfter: finiteIdentityValue(this.uncountedNativeFrames),
          polledAdvanceBefore: finiteIdentityValue(polledAdvanceBefore),
          polledAdvanceAfter: finiteIdentityValue(this.polledFramesSinceNative),
          creditedPollFrames,
          expiredPollFrames
        });
      }
      if (trackingSource === "videoFrameCallback")
        this.diagnosticRtpTimestamp = metadata?.rtpTimestamp;
    }
    this.counter = counter ?? this.counter;
    if (metadata?.presentedFrames !== void 0)
      this.lastPresentedFrames = metadata.presentedFrames;
    if (mediaTime !== void 0)
      this.lastMediaTime = mediaTime;
    const pixelInput = this.pixelDiagnosticsEnabled ? { enteredAtMs: callbackEnteredAtMs, trackingSource, newFrame, localSequence: this.frameSequence + Number(newFrame), nativeCallbackCount: this.nativeCallbackCount, counterSource: counter?.source, counterFrames: counter?.frames, counterReset: counterReset || nativeReset, counterPaired: advance > 0 && credited === advance, epoch: this.epoch } : void 0;
    if (!newFrame && trackingSource !== "videoFrameCallback") {
      if (pixelInput)
        this.pixelObservation.observe(pixelInput);
      return;
    }
    if (newFrame && trackingSource === "animationFrame")
      this.polledFrameCount++;
    const observed = { ...metadata, mediaTime, presentedFramesDelta, trackingSource, newFrame, localSequence: this.frameSequence += Number(newFrame), videoGeneration: this.videoGeneration };
    this.notify(atMs, observed);
    if (pixelInput && this.pending.size > 0 && pixelInput.epoch === this.epoch)
      this.pixelObservation.observe(pixelInput);
  }
  notify(atMs, observed) {
    const decoded = observed.trackingSource === "decodedFrame";
    if (observed.newFrame && !decoded) {
      this.decodedReader?.observeMediaProgress();
      this.lastFrame = observed;
    }
    for (const id of [...this.pending.keys()]) {
      const request = this.pending.get(id);
      if (!request || decoded !== Boolean(request.render))
        continue;
      this.pending.delete(id);
      try {
        request.callback(atMs, observed);
      } catch (error) {
        if (typeof reportError === "function")
          reportError(error);
      }
    }
  }
};
function getVideoFrameSource(video) {
  let source = sources.get(video);
  if (!source) {
    source = new VideoFrameSource(video);
    sources.set(video, source);
  }
  return source;
}

// ../../../../../../private/tmp/xbxrc-2026-11-10-clock-diagnostics-probe/direct-frames.ts
async function installDirectFrameProbe(video, renderer) {
  const source = getVideoFrameSource(video);
  const processor = renderer.processor ?? renderer.player;
  const gl = processor.gl;
  const originalNotify = source.notify.bind(source);
  const originalUpload = gl.texImage2D.bind(gl);
  let activeFrame;
  let frames = 0, errors = 0, stopped = false, requestId = 0;
  let firstTimestamp, lastTimestamp;
  let worker, reader;
  const clone = video.srcObject.getVideoTracks()[0].clone();
  const consume = (frame) => {
    if (stopped) {
      frame.close();
      return;
    }
    activeFrame = frame;
    frames++;
    firstTimestamp ??= frame.timestamp;
    lastTimestamp = frame.timestamp;
    try {
      originalNotify(performance.now(), { videoGeneration: source.videoGeneration, localSequence: ++source.frameSequence, newFrame: true, trackingSource: "decodedFrame", mediaTime: frame.timestamp / 1e6 });
    } catch {
      errors++;
    } finally {
      activeFrame = void 0;
      frame.close();
    }
  };
  source.notify = (at, metadata) => originalNotify(at, { ...metadata, newFrame: false });
  gl.texImage2D = (...args) => originalUpload(...args.map((value) => value === video && activeFrame ? activeFrame : value));
  const Constructor = window.MediaStreamTrackProcessor;
  let mode;
  if (typeof Constructor === "function") {
    mode = "main-thread";
    reader = new Constructor({ track: clone, maxBufferSize: 1 }).readable.getReader();
    void (async () => {
      while (!stopped) {
        const item = await reader.read();
        if (item.done) return;
        consume(item.value);
      }
    })().catch(() => errors++);
  } else {
    mode = "worker";
    const script = new Blob([`
      let reader, track, stopped=false, acknowledge;
      onmessage=async ({data})=>{
        if(data.type==='ack'){acknowledge?.();return}
        if(data.type==='stop'){stopped=true;acknowledge?.();await reader?.cancel();track?.stop();return}
        if(data.type!=='start')return;
        try{
          track=data.track;
          reader=new MediaStreamTrackProcessor({track,maxBufferSize:1}).readable.getReader();
          postMessage({type:'ready'});
          while(!stopped){
            const {value:frame,done}=await reader.read();if(done)return;
            const acknowledged=new Promise(resolve=>acknowledge=resolve);
            postMessage({type:'frame',frame},[frame]);await acknowledged;
          }
        }catch(error){postMessage({type:'error',name:error.name,message:error.message})}
      }`], { type: "text/javascript" });
    const url = URL.createObjectURL(script);
    worker = new Worker(url);
    URL.revokeObjectURL(url);
    await new Promise((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error("decoded processor initialization timeout")), 5e3);
      worker.onerror = () => {
        clearTimeout(timer);
        reject(new Error("decoded processor worker error"));
      };
      worker.onmessage = ({ data }) => {
        if (data.type === "ready") {
          clearTimeout(timer);
          resolve();
        }
        if (data.type === "error") {
          clearTimeout(timer);
          reject(new Error(JSON.stringify(data)));
        }
        if (data.type === "frame") {
          consume(data.frame);
          worker.postMessage({ type: "ack" });
        }
      };
      worker.postMessage({ type: "start", track: clone }, [clone]);
    });
  }
  const keepAlive = () => source.request(() => {
    requestId = keepAlive();
  });
  requestId = keepAlive();
  return { snapshot: () => ({ mode, frames, errors, firstTimestamp, lastTimestamp }), close: async () => {
    stopped = true;
    source.cancel(requestId);
    source.notify = originalNotify;
    gl.texImage2D = originalUpload;
    if (worker) {
      worker.postMessage({ type: "stop" });
      await new Promise((resolve) => setTimeout(resolve, 50));
      worker.terminate();
    }
    await reader?.cancel();
    clone.stop();
  } };
}

// src/player/infra/render/frame-capture-queue.ts
var FrameCaptureQueue = class {
  pending = /* @__PURE__ */ new Set();
  get size() {
    return this.pending.size;
  }
  request(signal) {
    if (signal?.aborted)
      return Promise.resolve(null);
    return new Promise((resolve) => {
      const finish = (frame) => {
        this.pending.delete(finish);
        signal?.removeEventListener("abort", abort);
        resolve(frame);
      };
      function abort() {
        finish(null);
      }
      this.pending.add(finish);
      signal?.addEventListener("abort", abort, { once: true });
    });
  }
  complete(read) {
    if (this.size === 0)
      return;
    let frame = null;
    try {
      frame = read();
    } catch {
    }
    for (const finish of [...this.pending])
      finish(frame);
  }
};

// src/player/infra/render/frame-pacer.ts
var FramePacer = class {
  nextAllowedAtMs;
  targetFps = 60;
  setTargetFps(fps) {
    const next = Number.isFinite(fps) ? Math.max(0, fps) : 60;
    if (next !== this.targetFps) {
      this.targetFps = next;
      this.nextAllowedAtMs = void 0;
    }
  }
  admit(atMs) {
    if (this.targetFps <= 0 || !Number.isFinite(atMs))
      return false;
    if (this.targetFps >= 60)
      return true;
    const intervalMs = 1e3 / this.targetFps;
    const nextAtMs = this.nextAllowedAtMs ?? atMs;
    if (atMs + 1 < nextAtMs)
      return false;
    this.nextAllowedAtMs = (atMs - nextAtMs >= intervalMs ? atMs : nextAtMs) + intervalMs;
    return true;
  }
};

// src/player/infra/render/fsr1-cpu.ts
function au1FromAf1(a) {
  const u = new Uint32Array(1);
  const f = new Float32Array(u.buffer);
  f[0] = a;
  return u[0];
}
function af1FromAu1(u) {
  const view = new Uint32Array(1);
  view[0] = u >>> 0;
  return new Float32Array(view.buffer)[0];
}
function packUvec4AsVec4FloatBits(a, b, c, d) {
  return new Float32Array([
    af1FromAu1(a >>> 0),
    af1FromAu1(b >>> 0),
    af1FromAu1(c >>> 0),
    af1FromAu1(d >>> 0)
  ]);
}
function rcp(x) {
  return 1 / x;
}
function computeFsrEasuCon(inputViewportInPixelsX, inputViewportInPixelsY, inputSizeInPixelsX, inputSizeInPixelsY, outputSizeInPixelsX, outputSizeInPixelsY) {
  const con0 = packUvec4AsVec4FloatBits(
    au1FromAf1(inputViewportInPixelsX * rcp(outputSizeInPixelsX)),
    au1FromAf1(inputViewportInPixelsY * rcp(outputSizeInPixelsY)),
    au1FromAf1(0.5 * inputViewportInPixelsX * rcp(outputSizeInPixelsX) - 0.5),
    au1FromAf1(0.5 * inputViewportInPixelsY * rcp(outputSizeInPixelsY) - 0.5)
  );
  const con1 = packUvec4AsVec4FloatBits(
    au1FromAf1(rcp(inputSizeInPixelsX)),
    au1FromAf1(rcp(inputSizeInPixelsY)),
    au1FromAf1(1 * rcp(inputSizeInPixelsX)),
    au1FromAf1(-1 * rcp(inputSizeInPixelsY))
  );
  const con2 = packUvec4AsVec4FloatBits(
    au1FromAf1(-1 * rcp(inputSizeInPixelsX)),
    au1FromAf1(2 * rcp(inputSizeInPixelsY)),
    au1FromAf1(1 * rcp(inputSizeInPixelsX)),
    au1FromAf1(2 * rcp(inputSizeInPixelsY))
  );
  const con3 = packUvec4AsVec4FloatBits(
    au1FromAf1(0 * rcp(inputSizeInPixelsX)),
    au1FromAf1(4 * rcp(inputSizeInPixelsY)),
    0,
    0
  );
  return { con0, con1, con2, con3 };
}

// src/player/infra/render/fsr1-reconstruction-pass.ts
function buildEasuFragmentSource(useCompatGather) {
  const gatherHelpers = useCompatGather ? `
void FsrEasuGatherRGB(vec2 p, out vec4 r, out vec4 g, out vec4 b){
  ivec2 size = textureSize(easuTex, 0);
  vec2 coord = p * vec2(size) - vec2(0.5);
  ivec2 base = ivec2(floor(coord));
  ivec2 i0j1 = clamp(base + ivec2(0, 1), ivec2(0), size - ivec2(1));
  ivec2 i1j1 = clamp(base + ivec2(1, 1), ivec2(0), size - ivec2(1));
  ivec2 i1j0 = clamp(base + ivec2(1, 0), ivec2(0), size - ivec2(1));
  ivec2 i0j0 = clamp(base, ivec2(0), size - ivec2(1));
  vec3 s0 = texelFetch(easuTex, i0j1, 0).rgb;
  vec3 s1 = texelFetch(easuTex, i1j1, 0).rgb;
  vec3 s2 = texelFetch(easuTex, i1j0, 0).rgb;
  vec3 s3 = texelFetch(easuTex, i0j0, 0).rgb;
  r = vec4(s0.r, s1.r, s2.r, s3.r);
  g = vec4(s0.g, s1.g, s2.g, s3.g);
  b = vec4(s0.b, s1.b, s2.b, s3.b);
}
` : `
void FsrEasuGatherRGB(vec2 p, out vec4 r, out vec4 g, out vec4 b){
  r = textureGather(easuTex, p, 0);
  g = textureGather(easuTex, p, 1);
  b = textureGather(easuTex, p, 2);
}
`;
  const gatherSamples = ["bczz", "ijfe", "klhg", "zzon"].map((name, index) => `
  vec4 ${name}R, ${name}G, ${name}B;
  FsrEasuGatherRGB(p${index}, ${name}R, ${name}G, ${name}B);`).join("");
  return `#version 300 es
precision highp float;
precision highp int;
uniform sampler2D easuTex;
uniform vec4 con0;
uniform vec4 con1;
uniform vec4 con2;
uniform vec4 con3;
out vec4 fragColor;

vec2 AF2_AU2(uvec2 v){ return uintBitsToFloat(v); }
float APrxLoRsqF1(float a){ return uintBitsToFloat(0x5f347d74u - (floatBitsToUint(a) >> 1u)); }
float APrxLoRcpF1(float a){ return uintBitsToFloat(0x7ef07ebbu - floatBitsToUint(a)); }
float APrxMedRcpF1(float a){
  float b = uintBitsToFloat(0x7ef19fffu - floatBitsToUint(a));
  return b * (-b * a + 2.0);
}
float ARcpF1(float a){ return 1.0 / a; }
float ASatF1(float x){ return clamp(x, 0.0, 1.0); }
float AMax3F1(float a,float b,float c){ return max(a, max(b, c)); }
float AMin3F1(float a,float b,float c){ return min(a, min(b, c)); }
vec3 AMax3F3(vec3 a,vec3 b,vec3 c){ return max(a, max(b, c)); }
vec3 AMin3F3(vec3 a,vec3 b,vec3 c){ return min(a, min(b, c)); }

${gatherHelpers}

void FsrEasuTapF(inout vec3 aC, inout float aW, vec2 off, vec2 dir, vec2 len, float lob, float clp, vec3 c){
  vec2 v;
  v.x = (off.x * dir.x) + (off.y * dir.y);
  v.y = (off.x * (-dir.y)) + (off.y * dir.x);
  v *= len;
  float d2 = v.x * v.x + v.y * v.y;
  d2 = min(d2, clp);
  float wB = (2.0 / 5.0) * d2 + (-1.0);
  float wA = lob * d2 + (-1.0);
  wB *= wB;
  wA *= wA;
  wB = (25.0 / 16.0) * wB + (-(25.0 / 16.0 - 1.0));
  float w = wB * wA;
  aC += c * w;
  aW += w;
}

void FsrEasuSetF(inout vec2 dir, inout float len, vec2 pp, bool biS, bool biT, bool biU, bool biV,
  float lA,float lB,float lC,float lD,float lE){
  float w = 0.0;
  if(biS) w = (1.0 - pp.x) * (1.0 - pp.y);
  if(biT) w = pp.x * (1.0 - pp.y);
  if(biU) w = (1.0 - pp.x) * pp.y;
  if(biV) w = pp.x * pp.y;
  float dc = lD - lC;
  float cb = lC - lB;
  float lenX = max(abs(dc), abs(cb));
  lenX = APrxLoRcpF1(lenX);
  float dirX = lD - lB;
  dir.x += dirX * w;
  lenX = ASatF1(abs(dirX) * lenX);
  lenX *= lenX;
  len += lenX * w;
  float ec = lE - lC;
  float ca = lC - lA;
  float lenY = max(abs(ec), abs(ca));
  lenY = APrxLoRcpF1(lenY);
  float dirY = lE - lA;
  dir.y += dirY * w;
  lenY = ASatF1(abs(dirY) * lenY);
  lenY *= lenY;
  len += lenY * w;
}

void main(){
  uvec2 ip = uvec2(ivec2(gl_FragCoord.xy));
  vec2 pp = vec2(ip) * AF2_AU2(uvec2(floatBitsToUint(con0.x), floatBitsToUint(con0.y)))
    + AF2_AU2(uvec2(floatBitsToUint(con0.z), floatBitsToUint(con0.w)));
  vec2 fp = floor(pp);
  pp -= fp;
  vec2 p0 = fp * AF2_AU2(uvec2(floatBitsToUint(con1.x), floatBitsToUint(con1.y)))
    + AF2_AU2(uvec2(floatBitsToUint(con1.z), floatBitsToUint(con1.w)));
  vec2 p1 = p0 + AF2_AU2(uvec2(floatBitsToUint(con2.x), floatBitsToUint(con2.y)));
  vec2 p2 = p0 + AF2_AU2(uvec2(floatBitsToUint(con2.z), floatBitsToUint(con2.w)));
  vec2 p3 = p0 + AF2_AU2(uvec2(floatBitsToUint(con3.x), floatBitsToUint(con3.y)));
${gatherSamples}
  vec4 bczzL = bczzB * 0.5 + (bczzR * 0.5 + bczzG);
  vec4 ijfeL = ijfeB * 0.5 + (ijfeR * 0.5 + ijfeG);
  vec4 klhgL = klhgB * 0.5 + (klhgR * 0.5 + klhgG);
  vec4 zzonL = zzonB * 0.5 + (zzonR * 0.5 + zzonG);
  float bL = bczzL.x;
  float cL = bczzL.y;
  float iL = ijfeL.x;
  float jL = ijfeL.y;
  float fL = ijfeL.z;
  float eL = ijfeL.w;
  float kL = klhgL.x;
  float lL = klhgL.y;
  float hL = klhgL.z;
  float gL = klhgL.w;
  float oL = zzonL.z;
  float nL = zzonL.w;
  vec2 dir = vec2(0.0);
  float len = 0.0;
  FsrEasuSetF(dir, len, pp, true, false, false, false, bL, eL, fL, gL, jL);
  FsrEasuSetF(dir, len, pp, false, true, false, false, cL, fL, gL, hL, kL);
  FsrEasuSetF(dir, len, pp, false, false, true, false, fL, iL, jL, kL, nL);
  FsrEasuSetF(dir, len, pp, false, false, false, true, gL, jL, kL, lL, oL);
  vec2 dir2 = dir * dir;
  float dirR = dir2.x + dir2.y;
  bool zro = dirR < (1.0 / 32768.0);
  dirR = APrxLoRsqF1(dirR);
  dirR = zro ? 1.0 : dirR;
  dir.x = zro ? 1.0 : dir.x;
  dir *= dirR;
  len = len * 0.5;
  len *= len;
  float stretch = (dir.x * dir.x + dir.y * dir.y) * APrxLoRcpF1(max(abs(dir.x), abs(dir.y)));
  vec2 len2 = vec2(1.0 + (stretch - 1.0) * len, 1.0 + (-0.5) * len);
  float lob = 0.5 + ((1.0 / 4.0 - 0.04) - 0.5) * len;
  float clp = APrxLoRcpF1(lob);
  vec3 min4 = min(AMin3F3(vec3(ijfeR.z, ijfeG.z, ijfeB.z), vec3(klhgR.w, klhgG.w, klhgB.w), vec3(ijfeR.y, ijfeG.y, ijfeB.y)),
    vec3(klhgR.x, klhgG.x, klhgB.x));
  vec3 max4 = max(AMax3F3(vec3(ijfeR.z, ijfeG.z, ijfeB.z), vec3(klhgR.w, klhgG.w, klhgB.w), vec3(ijfeR.y, ijfeG.y, ijfeB.y)),
    vec3(klhgR.x, klhgG.x, klhgB.x));
  vec3 aC = vec3(0.0);
  float aW = 0.0;
  FsrEasuTapF(aC, aW, vec2(0.0, -1.0) - pp, dir, len2, lob, clp, vec3(bczzR.x, bczzG.x, bczzB.x));
  FsrEasuTapF(aC, aW, vec2(1.0, -1.0) - pp, dir, len2, lob, clp, vec3(bczzR.y, bczzG.y, bczzB.y));
  FsrEasuTapF(aC, aW, vec2(-1.0, 1.0) - pp, dir, len2, lob, clp, vec3(ijfeR.x, ijfeG.x, ijfeB.x));
  FsrEasuTapF(aC, aW, vec2(0.0, 1.0) - pp, dir, len2, lob, clp, vec3(ijfeR.y, ijfeG.y, ijfeB.y));
  FsrEasuTapF(aC, aW, vec2(0.0, 0.0) - pp, dir, len2, lob, clp, vec3(ijfeR.z, ijfeG.z, ijfeB.z));
  FsrEasuTapF(aC, aW, vec2(-1.0, 0.0) - pp, dir, len2, lob, clp, vec3(ijfeR.w, ijfeG.w, ijfeB.w));
  FsrEasuTapF(aC, aW, vec2(1.0, 1.0) - pp, dir, len2, lob, clp, vec3(klhgR.x, klhgG.x, klhgB.x));
  FsrEasuTapF(aC, aW, vec2(2.0, 1.0) - pp, dir, len2, lob, clp, vec3(klhgR.y, klhgG.y, klhgB.y));
  FsrEasuTapF(aC, aW, vec2(2.0, 0.0) - pp, dir, len2, lob, clp, vec3(klhgR.z, klhgG.z, klhgB.z));
  FsrEasuTapF(aC, aW, vec2(1.0, 0.0) - pp, dir, len2, lob, clp, vec3(klhgR.w, klhgG.w, klhgB.w));
  FsrEasuTapF(aC, aW, vec2(1.0, 2.0) - pp, dir, len2, lob, clp, vec3(zzonR.z, zzonG.z, zzonB.z));
  FsrEasuTapF(aC, aW, vec2(0.0, 2.0) - pp, dir, len2, lob, clp, vec3(zzonR.w, zzonG.w, zzonB.w));
  vec3 pix = min(max4, max(min4, aC * vec3(ARcpF1(aW))));
  fragColor = vec4(pix, 1.0);
}
`;
}
var EASU_FRAGMENT = buildEasuFragmentSource(false);
var EASU_FRAGMENT_COMPAT = buildEasuFragmentSource(true);
var Fsr1ReconstructionPass = class {
  constructor(gl, vertex) {
    this.gl = gl;
    this.vertex = vertex;
  }
  program = null;
  shaders = [];
  texture = null;
  framebuffer = null;
  width = 0;
  height = 0;
  uniforms = [];
  sourceWidth = 0;
  sourceHeight = 0;
  get gpuObjects() {
    return [this.program, this.texture, this.framebuffer, ...this.shaders].filter(Boolean).length;
  }
  prepare() {
    if (this.program)
      return;
    const gl = this.gl;
    let shader;
    try {
      shader = this.compile(EASU_FRAGMENT);
    } catch (error) {
      if (!(error instanceof Error) || !error.message.includes("textureGather"))
        throw error;
      shader = this.compile(EASU_FRAGMENT_COMPAT);
    }
    const program = gl.createProgram();
    if (!program)
      throw new Error("srProgramAllocationFailed");
    this.program = program;
    gl.attachShader(program, this.vertex);
    gl.attachShader(program, shader);
    gl.linkProgram(program);
    if (!gl.getProgramParameter(program, gl.LINK_STATUS))
      throw new Error(`srProgramLinkFailed:${gl.getProgramInfoLog(program) ?? "unknown"}`);
    this.uniforms = ["con0", "con1", "con2", "con3"].map((name) => gl.getUniformLocation(program, name));
    gl.useProgram(program);
    gl.uniform1i(gl.getUniformLocation(program, "easuTex"), 0);
  }
  compile(source) {
    const gl = this.gl;
    const shader = gl.createShader(gl.FRAGMENT_SHADER);
    if (!shader)
      throw new Error("srShaderAllocationFailed");
    gl.shaderSource(shader, source);
    gl.compileShader(shader);
    if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) {
      const message = gl.getShaderInfoLog(shader) ?? "unknown";
      gl.deleteShader(shader);
      throw new Error(`srShaderCompileFailed:${message}`);
    }
    this.shaders.push(shader);
    return shader;
  }
  render(input, sourceWidth, sourceHeight, width, height) {
    this.prepare();
    const gl = this.gl;
    const resized = width !== this.width || height !== this.height;
    if (resized) {
      this.releaseTarget();
      this.texture = gl.createTexture();
      this.framebuffer = gl.createFramebuffer();
      if (!this.texture || !this.framebuffer)
        throw new Error("srTargetAllocationFailed");
      gl.bindTexture(gl.TEXTURE_2D, this.texture);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
      gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, width, height, 0, gl.RGBA, gl.UNSIGNED_BYTE, null);
      gl.bindFramebuffer(gl.FRAMEBUFFER, this.framebuffer);
      gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, this.texture, 0);
      const status = gl.checkFramebufferStatus(gl.FRAMEBUFFER);
      gl.bindFramebuffer(gl.FRAMEBUFFER, null);
      if (status !== gl.FRAMEBUFFER_COMPLETE)
        throw new Error(`srFramebufferIncomplete:${status}`);
      this.width = width;
      this.height = height;
    }
    gl.useProgram(this.program);
    if (resized || sourceWidth !== this.sourceWidth || sourceHeight !== this.sourceHeight) {
      const constants = computeFsrEasuCon(sourceWidth, sourceHeight, sourceWidth, sourceHeight, width, height);
      [constants.con0, constants.con1, constants.con2, constants.con3].forEach((value, index) => gl.uniform4fv(this.uniforms[index], value));
      this.sourceWidth = sourceWidth;
      this.sourceHeight = sourceHeight;
    }
    gl.bindTexture(gl.TEXTURE_2D, input);
    gl.bindFramebuffer(gl.FRAMEBUFFER, this.framebuffer);
    gl.drawArrays(gl.TRIANGLES, 0, 3);
    gl.bindFramebuffer(gl.FRAMEBUFFER, null);
    return this.texture;
  }
  releaseTarget() {
    const gl = this.gl;
    if (this.texture)
      gl.deleteTexture(this.texture);
    if (this.framebuffer)
      gl.deleteFramebuffer(this.framebuffer);
    this.texture = this.framebuffer = null;
    this.width = this.height = 0;
  }
  dispose() {
    this.releaseTarget();
    if (this.program)
      this.gl.deleteProgram(this.program);
    for (const shader of this.shaders) this.gl.deleteShader(shader);
    this.shaders.length = 0;
    this.program = null;
  }
};

// src/player/infra/render/render-observation.ts
var RenderObservation = class {
  duration = new LatencyWindow();
  upload = new LatencyWindow();
  submit = new LatencyWindow();
  interval = new LatencyWindow();
  receiveToSubmit = new LatencyWindow();
  readToSubmit = new LatencyWindow();
  previousDrawAtMs;
  lastDrawSequence;
  drawCount = 0;
  captureDrawCount = 0;
  duplicateDrawCount = 0;
  drawDurationMaxMs = 0;
  realtimeDrawCount = 0;
  drawDurationTotalMs = 0;
  drawCostSampleCount = 0;
  uploadDurationTotalMs = 0;
  submitDurationTotalMs = 0;
  uploadDurationMaxMs = 0;
  submitDurationMaxMs = 0;
  lastDrawAtMs;
  rateLimitDropCount = 0;
  submitFailureCount = 0;
  preparation = {};
  beginPreparation() {
    this.preparation.startedAtMs ??= performance.now();
  }
  endPreparation() {
    this.preparation.finishedAtMs ??= performance.now();
  }
  candidate(frame) {
    if (frame && this.preparation.firstCandidateAtMs === void 0) {
      this.preparation.firstCandidateAtMs = performance.now();
      this.preparation.firstCandidateSequence = frame.localSequence;
    }
  }
  dropRateLimitedFrame() {
    this.rateLimitDropCount++;
  }
  fail() {
    this.submitFailureCount++;
  }
  resetCadence() {
    this.previousDrawAtMs = void 0;
    this.interval.reset();
  }
  record(frame, capture, durationMs, uploadMs, submitMs, startedAtMs) {
    this.drawCount++;
    if (capture) {
      this.captureDrawCount++;
    } else {
      const atMs = performance.now();
      if (frame?.receivedAtMs !== void 0)
        this.receiveToSubmit.record(atMs - frame.receivedAtMs);
      if (frame?.readFinishedAtEpochMs !== void 0) {
        const age = performance.timeOrigin + atMs - frame.readFinishedAtEpochMs;
        this.readToSubmit.record(age >= -(frame.clockUncertaintyMs ?? 0) ? Math.max(0, age) : Number.NaN);
      }
      this.duration.record(durationMs);
      if (uploadMs !== void 0)
        this.upload.record(uploadMs);
      if (submitMs !== void 0)
        this.submit.record(submitMs);
      if (this.previousDrawAtMs !== void 0)
        this.interval.record(atMs - this.previousDrawAtMs);
      this.previousDrawAtMs = atMs;
      this.realtimeDrawCount++;
      this.drawDurationMaxMs = Math.max(this.drawDurationMaxMs, durationMs);
      this.drawDurationTotalMs += durationMs;
      if (uploadMs !== void 0 && submitMs !== void 0 && Number.isFinite(uploadMs) && Number.isFinite(submitMs)) {
        this.drawCostSampleCount++;
        this.uploadDurationTotalMs += uploadMs;
        this.submitDurationTotalMs += submitMs;
        this.uploadDurationMaxMs = Math.max(this.uploadDurationMaxMs, uploadMs);
        this.submitDurationMaxMs = Math.max(this.submitDurationMaxMs, submitMs);
      }
      this.lastDrawAtMs = performance.now();
      if (this.preparation.firstDrawSubmittedAtMs === void 0) {
        this.preparation.firstDrawStartedAtMs = startedAtMs;
        this.preparation.firstDrawSubmittedAtMs = this.lastDrawAtMs;
      }
      if (frame?.localSequence !== void 0 && frame.localSequence === this.lastDrawSequence)
        this.duplicateDrawCount++;
      this.lastDrawSequence = frame?.localSequence;
    }
  }
  snapshot() {
    const timing = { duration: this.duration.snapshot(), upload: this.upload.snapshot(), submit: this.submit.snapshot(), interval: this.interval.snapshot(), receiveToSubmit: this.receiveToSubmit.snapshot(), readToSubmit: this.readToSubmit.snapshot() };
    return { timing, drawCount: this.drawCount, realtimeDrawCount: this.realtimeDrawCount, captureDrawCount: this.captureDrawCount, duplicateDrawCount: this.duplicateDrawCount, lastDrawSequence: this.lastDrawSequence, lastDrawAtMs: this.lastDrawAtMs, drawDurationTotalMs: this.drawDurationTotalMs, drawDurationMaxMs: this.drawDurationMaxMs, drawCostSampleCount: this.drawCostSampleCount, uploadDurationTotalMs: this.uploadDurationTotalMs, submitDurationTotalMs: this.submitDurationTotalMs, uploadDurationMaxMs: this.uploadDurationMaxMs, submitDurationMaxMs: this.submitDurationMaxMs, latestOverwriteCount: 0, rateLimitDropCount: this.rateLimitDropCount, submitFailureCount: this.submitFailureCount, preparation: { ...this.preparation } };
  }
};

// src/player/infra/render/render-surface-observation.ts
function finite(value) {
  return value !== void 0 && Number.isFinite(value) ? Math.round(value * 10) / 10 : null;
}
function readElement(element) {
  if (!element)
    return void 0;
  let style;
  let rect;
  try {
    if (typeof getComputedStyle === "function")
      style = getComputedStyle(element);
    rect = element.getBoundingClientRect?.();
  } catch {
  }
  return {
    connected: typeof element.isConnected === "boolean" ? element.isConnected : null,
    styleState: style ? "observed" : "unavailable",
    displayNone: style ? style.display === "none" : null,
    visibility: style ? ["visible", "hidden", "collapse"].includes(style.visibility) ? style.visibility : "other" : null,
    opacity: style?.opacity ? finite(Number(style.opacity)) : null,
    position: style ? ["static", "relative", "absolute", "fixed", "sticky"].includes(style.position) ? style.position : "other" : null,
    zIndex: style?.zIndex ? style.zIndex === "auto" ? "auto" : finite(Number(style.zIndex)) : null,
    pointerEventsNone: style ? style.pointerEvents === "none" : null,
    objectFit: style ? ["contain", "cover", "fill", "none", "scale-down"].includes(style.objectFit) ? style.objectFit : "other" : null,
    clippedX: style ? ["hidden", "clip"].includes(style.overflowX) : null,
    clippedY: style ? ["hidden", "clip"].includes(style.overflowY) : null,
    transformed: style ? Boolean(style.transform && style.transform !== "none") : null,
    filtered: style ? Boolean(style.filter && style.filter !== "none") : null,
    rect: rect ? { x: finite(rect.x), y: finite(rect.y), width: finite(rect.width), height: finite(rect.height) } : null
  };
}
function observeRenderSurface(video, canvas, gl) {
  let attributes = null;
  let contextLost = null;
  try {
    attributes = gl?.getContextAttributes?.() ?? null;
    contextLost = gl?.isContextLost?.() ?? null;
  } catch {
  }
  const ancestors = [];
  let parent = video.parentElement;
  while (parent && ancestors.length < 4) {
    ancestors.push(readElement(parent));
    parent = parent.parentElement;
  }
  let canvasFollowsVideo = null;
  if (canvas && typeof video.compareDocumentPosition === "function") {
    try {
      const order = video.compareDocumentPosition(canvas);
      if ((order & 1) === 0)
        canvasFollowsVideo = (order & 4) !== 0;
    } catch {
    }
  }
  return {
    schemaVersion: 1,
    evidence: "domLayout",
    screenPresentState: "unavailable",
    video: { ...readElement(video), paused: video.paused, readyState: video.readyState, currentTime: finite(video.currentTime), width: video.videoWidth, height: video.videoHeight },
    canvas: canvas ? { ...readElement(canvas), backingWidth: canvas.width, backingHeight: canvas.height } : void 0,
    sameParent: canvas ? Boolean(video.parentElement && video.parentElement === canvas.parentElement) : void 0,
    canvasFollowsVideo,
    ancestors,
    ancestorLimit: 4,
    ancestorsTruncated: Boolean(parent),
    contextLost,
    contextAttributes: attributes ? { alpha: attributes.alpha, desynchronized: attributes.desynchronized, preserveDrawingBuffer: attributes.preserveDrawingBuffer, antialias: attributes.antialias } : null
  };
}

// src/player/infra/render/sharpening-tuning.ts
var CAS_EDGE_WEIGHT = 5.2;
var CAS_GAIN_MAX = 2;
var CAS_NOISE_FLOOR = 1.5 / 255;
var CAS_GATE_RAMP = 2;
var CAS_RING_PAD = 0.38;
var CAS_RING_BLEND = 0.92;
var CAS_CHROMA_BLEND = 0.85;
var USM_GAIN_SCALE = 4 / 3;
var USM_NOISE_FLOOR = 1 / 255;

// src/player/infra/render/webgl-frame-capture.ts
function readWebglDefaultFramebufferToCanvas(gl, width, height) {
  const rowBytes = width * 4;
  const raw = new Uint8Array(rowBytes * height);
  gl.readPixels(0, 0, width, height, gl.RGBA, gl.UNSIGNED_BYTE, raw);
  const flipped = new Uint8ClampedArray(raw.length);
  for (let row = 0; row < height; row += 1) {
    flipped.set(
      raw.subarray(row * rowBytes, (row + 1) * rowBytes),
      (height - 1 - row) * rowBytes
    );
  }
  const canvas = document.createElement("canvas");
  canvas.width = width;
  canvas.height = height;
  const ctx = canvas.getContext("2d");
  if (ctx === null) {
    throw new Error("readback2dContextUnavailable");
  }
  ctx.putImageData(new ImageData(flipped, width, height), 0, 0);
  return canvas;
}
function captureVideoElementToCanvas(video) {
  const w = video.videoWidth;
  const h = video.videoHeight;
  if (w < 2 || h < 2) {
    return null;
  }
  const canvas = document.createElement("canvas");
  canvas.width = w;
  canvas.height = h;
  const ctx = canvas.getContext("2d");
  if (ctx === null) {
    return null;
  }
  ctx.drawImage(video, 0, 0, w, h);
  return canvas;
}

// src/player/infra/render/Renderers.ts
var BaseCanvasVideoProcessor = class {
  canvas;
  video;
  options = {
    processing: "usm",
    processingMode: "quality",
    targetFps: 60,
    sharpness: 0,
    brightness: 100,
    contrast: 100,
    saturation: 100
  };
  isStopped = false;
  animFrameId = null;
  frameCallback;
  framePacer = new FramePacer();
  drawObservation = new RenderObservation();
  initialized = false;
  initialization;
  onFailure;
  constructor(video) {
    this.video = video;
    this.canvas = document.createElement("canvas");
    this.canvas.width = video.videoWidth;
    this.canvas.height = video.videoHeight;
    this.canvas.style.position = "absolute";
    this.canvas.style.inset = "0";
    this.canvas.style.width = "100%";
    this.canvas.style.height = "100%";
    this.canvas.style.pointerEvents = "none";
    video.insertAdjacentElement("afterend", this.canvas);
    this.frameCallback = (callback) => getVideoFrameSource(video).requestRender(callback, (reason) => {
      this.stopPresentationLoop();
      this.onFailure?.(`decodedFrame:${reason}`);
    });
  }
  init() {
    this.initialization ??= this.initialize();
    return this.initialization;
  }
  async initialize() {
    this.drawObservation.beginPreparation();
    await this.setup();
    if (this.isStopped)
      return;
    this.initialized = true;
    this.drawObservation.endPreparation();
    this.scheduleFrameRequest();
    const latest = getVideoFrameSource(this.video).lastRenderFrame;
    if (latest && this.options.targetFps > 0)
      this.drawFrame(performance.now(), latest);
  }
  updateOptions(newOptions, refresh = false) {
    const wasPaused = this.options.targetFps <= 0;
    if (newOptions.targetFps !== void 0 && newOptions.targetFps !== this.options.targetFps)
      this.drawObservation.resetCadence();
    this.options = { ...this.options, ...newOptions };
    this.framePacer.setTargetFps(this.options.targetFps);
    if (this.options.targetFps <= 0) {
      if (this.animFrameId !== null)
        getVideoFrameSource(this.video).cancel(this.animFrameId);
      this.animFrameId = null;
    } else if (wasPaused && !this.isStopped) {
      this.scheduleFrameRequest();
    }
    if (refresh && this.initialized) {
      this.refresh();
    }
  }
  destroy() {
    this.stopPresentationLoop();
    if (this.canvas.isConnected) {
      this.canvas.remove();
    }
    this.canvas.width = 1;
    this.canvas.height = 1;
  }
  stopPresentationLoop() {
    this.isStopped = true;
    this.initialized = false;
    if (this.animFrameId !== null) {
      getVideoFrameSource(this.video).cancel(this.animFrameId);
      this.animFrameId = null;
    }
  }
  toFilterId(processing) {
    return processing === "cas" ? 2 : 1;
  }
  scheduleFrameRequest() {
    if (this.isStopped || !this.initialized || this.options.targetFps <= 0 || this.animFrameId !== null)
      return;
    const id = this.frameCallback((atMs, metadata) => {
      if (this.animFrameId !== id)
        return;
      this.animFrameId = null;
      this.drawFrame(atMs, metadata);
    });
    this.animFrameId = id;
  }
  drawFrame(_atMs, metadata) {
    if (this.isStopped || !this.initialized || this.options.targetFps <= 0) {
      return;
    }
    this.scheduleFrameRequest();
    if (metadata?.newFrame === false)
      return;
    this.drawObservation.candidate(metadata);
    if (!this.framePacer.admit(performance.now())) {
      this.drawObservation.dropRateLimitedFrame();
      return;
    }
    try {
      this.renderFrame();
      this.onRenderFramePresented();
    } catch (error) {
      this.drawObservation.fail();
      this.onFailure?.(error instanceof Error ? error.message : "webgl2DrawFailed");
    }
  }
  /** Runs after `renderFrame` while WebGL default framebuffer contents are still defined. */
  onRenderFramePresented() {
  }
  setDisplayFormat(format) {
    this.canvas.style.objectFit = resolveCanvasObjectFit(format);
  }
  setCanvasViewport(viewportWidthCss, viewportHeightCss) {
    if (viewportWidthCss !== void 0 && viewportHeightCss !== void 0 && viewportWidthCss > 0 && viewportHeightCss > 0) {
      this.canvas.style.width = `${Math.round(viewportWidthCss)}px`;
      this.canvas.style.height = `${Math.round(viewportHeightCss)}px`;
      this.canvas.style.margin = "auto";
      return;
    }
    this.canvas.style.width = "100%";
    this.canvas.style.height = "100%";
    this.canvas.style.margin = "";
  }
};
function resolveCanvasObjectFit(format) {
  if (format === "Stretch") {
    return "fill";
  }
  if (format === "Zoom") {
    return "cover";
  }
  return "contain";
}
function resolveSharpenStrength(config) {
  const raw = config.sharpenStrength === void 0 ? config.sharpness / 5 : config.sharpenStrength / 100;
  return Math.max(0, Math.min(1, raw));
}
function resolveShaderPreset(config) {
  const strength = resolveSharpenStrength(config);
  if (config.shaderPreset === "clarityL0") {
    return {
      processing: "usm",
      processingMode: "performance",
      sharpenFactor: 0
    };
  }
  if (config.shaderPreset === "clarityL1") {
    return {
      processing: "usm",
      processingMode: "performance",
      sharpenFactor: strength * 0.6
    };
  }
  if (config.shaderPreset === "clarityL2") {
    return {
      processing: "usm",
      processingMode: "quality",
      sharpenFactor: strength * 0.8
    };
  }
  if (config.shaderPreset === "clarityL3") {
    return {
      processing: "cas",
      processingMode: "quality",
      sharpenFactor: strength
    };
  }
  return {
    processing: config.processing,
    processingMode: config.processingMode,
    sharpenFactor: strength
  };
}
var WebGL2Processor = class extends BaseCanvasVideoProcessor {
  gl = null;
  guard;
  srEnabled = false;
  reconstruction;
  program = null;
  presentProgram = null;
  videoTexture = null;
  sharpenFramebuffer = null;
  sharpenTexture = null;
  sharpenTargetWidth = 0;
  sharpenTargetHeight = 0;
  currentSourceWidth = 0;
  currentSourceHeight = 0;
  currentOutputWidth = 0;
  currentOutputHeight = 0;
  currentViewportWidthCss = null;
  currentViewportHeightCss = null;
  explicitPresentTarget = false;
  textureAllocated = false;
  uniformOptions;
  uniformSourceWidth = 0;
  uniformSourceHeight = 0;
  uniformOutputWidth = 0;
  uniformOutputHeight = 0;
  uniformUsesSharpenTarget = false;
  hasDrawnFrame = false;
  contextListenersBound = false;
  outputResolutionUniform = null;
  sourceResolutionUniform = null;
  filterIdUniform = null;
  applyColorUniform = null;
  sharpenFactorUniform = null;
  brightnessUniform = null;
  contrastUniform = null;
  saturationUniform = null;
  presentOutputResolutionUniform = null;
  presentBrightnessUniform = null;
  presentContrastUniform = null;
  presentSaturationUniform = null;
  presentApplyColorUniform = null;
  frameCaptures = new FrameCaptureQueue();
  captureRafId;
  onFrameReady;
  drewToScreenThisFrame = false;
  shaders = [];
  vertexBuffer = null;
  snapshotResources() {
    return { ...this.drawObservation.snapshot(), ...getVideoFrameSource(this.video).renderSupply, superResolutionActive: this.usesReconstruction(), videoUploadMethod: "texImage2D", outputWidth: this.canvas.width, outputHeight: this.canvas.height, gpuErrorChecks: this.guard?.errorChecks, gpuTiming: this.guard?.timer.snapshot(), gpuObjects: (this.reconstruction?.gpuObjects ?? 0) + [this.program, this.presentProgram, this.videoTexture, this.sharpenFramebuffer, this.sharpenTexture, this.vertexBuffer, ...this.shaders].filter(Boolean).length, frameRequests: Number(this.animFrameId !== null), animationCallbacks: Number(this.captureRafId !== void 0), captureWaiters: this.frameCaptures.size, domNodes: Number(this.canvas.isConnected) };
  }
  snapshotDiagnostics() {
    return observeRenderSurface(this.video, this.canvas, this.gl);
  }
  onContextLost = (event) => {
    event.preventDefault();
    this.releaseGl();
    this.hasDrawnFrame = false;
    this.canvas.style.opacity = "0";
    this.onFailure?.("webgl2ContextLost");
  };
  onContextRestored = () => {
    if (this.isStopped) {
      return;
    }
    try {
      this.setup();
    } catch (error) {
      this.onFailure?.(error instanceof Error ? error.message : "webgl2ContextRestoreFailed");
    }
  };
  applyRendererPresentationConfig(config) {
    this.onFailure = config.rendererRuntimeFailureNotifier ?? config.superResolutionRuntimeDegradeNotifier;
    this.onFrameReady = config.rendererFrameReadyNotifier;
    this.srEnabled = config.superResolutionEnabled === true;
    if (!this.srEnabled) {
      this.reconstruction?.dispose();
      this.reconstruction = void 0;
    }
    const configuredWidth = this.srEnabled ? config.superResolutionOutputWidth ?? config.renderOutputWidth : config.renderOutputWidth;
    const configuredHeight = this.srEnabled ? config.superResolutionOutputHeight ?? config.renderOutputHeight : config.renderOutputHeight;
    this.explicitPresentTarget = configuredWidth !== void 0 && configuredHeight !== void 0;
    const nextOutputWidth = this.explicitPresentTarget ? Math.max(1, this.video.videoWidth || 0, Math.round(configuredWidth)) : Math.max(1, this.video.videoWidth || this.currentSourceWidth || 1);
    const nextOutputHeight = this.explicitPresentTarget ? Math.max(1, this.video.videoHeight || 0, Math.round(configuredHeight)) : Math.max(1, this.video.videoHeight || this.currentSourceHeight || 1);
    const nextViewportWidthCss = config.renderViewportWidth !== void 0 && config.renderViewportWidth > 0 ? Math.round(config.renderViewportWidth) : null;
    const nextViewportHeightCss = config.renderViewportHeight !== void 0 && config.renderViewportHeight > 0 ? Math.round(config.renderViewportHeight) : null;
    const outputChanged = nextOutputWidth !== this.currentOutputWidth || nextOutputHeight !== this.currentOutputHeight;
    const viewportChanged = nextViewportWidthCss !== this.currentViewportWidthCss || nextViewportHeightCss !== this.currentViewportHeightCss;
    if (!outputChanged && !viewportChanged) {
      return;
    }
    if (outputChanged) {
      this.guard?.assertSize(this.video.videoWidth, this.video.videoHeight, nextOutputWidth, nextOutputHeight);
      this.currentOutputWidth = nextOutputWidth;
      this.currentOutputHeight = nextOutputHeight;
      this.canvas.width = nextOutputWidth;
      this.canvas.height = nextOutputHeight;
      this.guard?.assertBacking(nextOutputWidth, nextOutputHeight);
    }
    if (viewportChanged) {
      this.currentViewportWidthCss = nextViewportWidthCss;
      this.currentViewportHeightCss = nextViewportHeightCss;
      this.setCanvasViewport(nextViewportWidthCss ?? void 0, nextViewportHeightCss ?? void 0);
    }
    if (this.gl !== null && outputChanged) {
      this.gl.viewport(0, 0, nextOutputWidth, nextOutputHeight);
    }
  }
  setup() {
    const gl = this.canvas.getContext("webgl2", {
      antialias: false,
      alpha: false,
      depth: false,
      desynchronized: true,
      preserveDrawingBuffer: false,
      stencil: false,
      powerPreference: "high-performance"
    });
    if (gl === null) {
      throw new Error("webgl2ContextUnavailable");
    }
    this.gl = gl;
    this.uniformOptions = void 0;
    this.guard = new WebglRuntimeGuard(gl);
    this.guard.assertSize(this.video.videoWidth, this.video.videoHeight, this.currentOutputWidth, this.currentOutputHeight);
    this.guard.assertBacking(this.currentOutputWidth, this.currentOutputHeight);
    this.canvas.style.opacity = "0";
    this.currentOutputWidth = Math.max(1, this.canvas.width);
    this.currentOutputHeight = Math.max(1, this.canvas.height);
    this.currentSourceWidth = Math.max(1, this.video.videoWidth || this.currentOutputWidth);
    this.currentSourceHeight = Math.max(1, this.video.videoHeight || this.currentOutputHeight);
    if (!this.contextListenersBound) {
      this.canvas.addEventListener("webglcontextlost", this.onContextLost);
      this.canvas.addEventListener("webglcontextrestored", this.onContextRestored);
      this.contextListenersBound = true;
    }
    gl.viewport(0, 0, this.currentOutputWidth, this.currentOutputHeight);
    const vShader = gl.createShader(gl.VERTEX_SHADER);
    this.shaders.push(vShader);
    gl.shaderSource(vShader, `#version 300 es
layout(location=0) in vec4 position;
void main(){ gl_Position = position; }`);
    gl.compileShader(vShader);
    if (!gl.getShaderParameter(vShader, gl.COMPILE_STATUS)) {
      throw new Error(`webgl2VertexShaderCompileFailed:${gl.getShaderInfoLog(vShader) ?? "unknown"}`);
    }
    const fShader = gl.createShader(gl.FRAGMENT_SHADER);
    this.shaders.push(fShader);
    gl.shaderSource(fShader, `#version 300 es
precision highp float;
uniform sampler2D data;
uniform vec2 outputResolution;
uniform vec2 sourceResolution;
uniform int filterId;
uniform bool applyColor;
uniform float sharpenFactor;
uniform float brightness;
uniform float contrast;
uniform float saturation;
const vec3 LUMINOSITY_FACTOR = vec3(0.299, 0.587, 0.114);
const float CAS_EDGE_WEIGHT = ${CAS_EDGE_WEIGHT.toFixed(4)};
const float CAS_GAIN_MAX = ${CAS_GAIN_MAX.toFixed(4)};
const float CAS_NOISE_FLOOR = ${CAS_NOISE_FLOOR.toFixed(6)};
const float CAS_GATE_RAMP = ${CAS_GATE_RAMP.toFixed(4)};
const float CAS_RING_PAD = ${CAS_RING_PAD.toFixed(4)};
const float CAS_RING_BLEND = ${CAS_RING_BLEND.toFixed(4)};
const float CAS_CHROMA_BLEND = ${CAS_CHROMA_BLEND.toFixed(4)};
const float USM_GAIN_SCALE = ${USM_GAIN_SCALE.toFixed(4)};
const float USM_NOISE_FLOOR = ${USM_NOISE_FLOOR.toFixed(6)};
vec3 usmBoost(vec2 uv, vec3 center) {
  vec2 texel = 1.0 / sourceResolution.xy;
  vec3 blur;
  vec3 neighborhoodMin;
  vec3 neighborhoodMax;
  if (all(equal(outputResolution, sourceResolution))) {
    // 1:1 \u50CF\u7D20\u4E2D\u5FC3\u4E0A\uFF0C\u56DB\u4E2A\u534A\u50CF\u7D20\u53CC\u7EBF\u6027\u91C7\u6837\u7B49\u4EF7\u4E8E 3x3 \u9AD8\u65AF\u6838 [1,2,1]\xB2/16\u3002
    vec2 halfTexel = texel * 0.5;
    vec3 a = texture(data, uv + halfTexel * vec2(-1.0, 1.0)).rgb;
    vec3 c = texture(data, uv + halfTexel * vec2(1.0, 1.0)).rgb;
    vec3 g = texture(data, uv + halfTexel * vec2(-1.0, -1.0)).rgb;
    vec3 i = texture(data, uv + halfTexel * vec2(1.0, -1.0)).rgb;
    blur = (a + c + g + i) * 0.25;
    // \u590D\u7528\u56DB\u6B21\u53CC\u7EBF\u6027\u8BFB\u53D6\u6784\u5EFA\u4FDD\u5B88\u8FB9\u754C\uFF0C\u4FDD\u6301\u5B8C\u65743x3\u9AD8\u65AF\u4E0E\u8BFB\u53D6\u9884\u7B97\u3002
    neighborhoodMin = min(center, min(min(a, c), min(g, i)));
    neighborhoodMax = max(center, max(max(a, c), max(g, i)));
  } else {
    // \u975E 1:1 \u6620\u5C04\u4FDD\u7559\u539F\u6838\uFF0C\u7EF4\u6301\u4E9A\u50CF\u7D20\u4F4D\u7F6E\u7684\u9510\u5316\u7ED3\u679C\u3002
    vec3 b = texture(data, uv + texel * vec2(0.0, 1.0)).rgb;
    vec3 d = texture(data, uv + texel * vec2(-1.0, 0.0)).rgb;
    vec3 f = texture(data, uv + texel * vec2(1.0, 0.0)).rgb;
    vec3 h = texture(data, uv + texel * vec2(0.0, -1.0)).rgb;
    vec3 a = texture(data, uv + texel * vec2(-1.0, 1.0)).rgb;
    vec3 c = texture(data, uv + texel * vec2(1.0, 1.0)).rgb;
    vec3 g = texture(data, uv + texel * vec2(-1.0, -1.0)).rgb;
    vec3 i = texture(data, uv + texel * vec2(1.0, -1.0)).rgb;
    blur = ((a + c + g + i) + (b + d + f + h) * 2.0 + center * 4.0) / 16.0;
    neighborhoodMin = min(center, min(min(min(a, c), min(g, i)), min(min(b, d), min(f, h))));
    neighborhoodMax = max(center, max(max(max(a, c), max(g, i)), max(max(b, d), max(f, h))));
  }
  vec3 residual = center - blur;
  float residualLuma = dot(residual, LUMINOSITY_FACTOR);
  float gate = smoothstep(USM_NOISE_FLOOR, USM_NOISE_FLOOR * CAS_GATE_RAMP, abs(residualLuma));
  residual = mix(residual, vec3(residualLuma), CAS_CHROMA_BLEND);
  vec3 outColor = center + residual * (clamp(sharpenFactor, 0.0, 1.0) * USM_GAIN_SCALE * gate);
  vec3 range = neighborhoodMax - neighborhoodMin;
  vec3 lo = neighborhoodMin - range * CAS_RING_PAD - 1e-4;
  vec3 hi = neighborhoodMax + range * CAS_RING_PAD + 1e-4;
  outColor = mix(outColor, clamp(outColor, lo, hi), CAS_RING_BLEND);
  return clamp(outColor, 0.0, 1.0);
}
// CAS \u98CE\u683C\u9510\u5316\uFF1A\u663E\u5F0F\u589E\u76CA + luma \u566A\u58F0\u95E8\u9650 + \u90BB\u57DF\u94B3\u5236\u3002
// \u539F\u5F0F\u7B49\u4EF7\u4E8E out = center + (center - mean4) * G\uFF0C\u5E73\u5766\u533A G=3.33 \u4F1A\u628A\u7F16\u7801\u566A\u58F0\u653E\u5927\u6210\u9897\u7C92\uFF0C
// \u5F3A\u8FB9\u7F18 G->0 \u53C8\u8BA9\u8F6E\u5ED3\u4FDD\u6301\u7CCA\uFF1B\u8FD9\u91CC\u7ED9 G \u52A0\u4E0A\u9650\u3001\u7ED9\u4F4E\u5BF9\u6BD4\u6B8B\u5DEE\u52A0\u8F6F\u95E8\u9650\uFF0C\u5E76\u9650\u5236\u8FC7\u51B2\u3002
vec3 casBoost(vec2 uv, vec3 center) {
  vec2 texel = 1.0 / sourceResolution.xy;
  vec3 b = texture(data, uv + texel * vec2(0.0, 1.0)).rgb;
  vec3 d = texture(data, uv + texel * vec2(-1.0, 0.0)).rgb;
  vec3 f = texture(data, uv + texel * vec2(1.0, 0.0)).rgb;
  vec3 h = texture(data, uv + texel * vec2(0.0, -1.0)).rgb;

  vec3 min4 = min(min(min(d, f), min(b, h)), center);
  vec3 max4 = max(max(max(d, f), max(b, h)), center);
  float lumaMin = dot(min4, LUMINOSITY_FACTOR);
  float lumaMax = dot(max4, LUMINOSITY_FACTOR);
  float adaptive = clamp(min(lumaMin, 2.0 - lumaMax) / max(lumaMax, 1e-4), 0.0, 1.0);
  float rootA = sqrt(adaptive);
  float gain = min(4.0 * rootA / max(CAS_EDGE_WEIGHT - 4.0 * rootA, 1e-3), CAS_GAIN_MAX);

  vec3 mean4 = (b + d + f + h) * 0.25;
  vec3 residual = center - mean4;
  float residualLuma = dot(residual, LUMINOSITY_FACTOR);
  float gate = smoothstep(CAS_NOISE_FLOOR, CAS_NOISE_FLOOR * CAS_GATE_RAMP, abs(residualLuma));
  residual = mix(residual, vec3(residualLuma), CAS_CHROMA_BLEND);
  vec3 outColor = center + residual * gain * gate * clamp(sharpenFactor, 0.0, 1.0);

  vec3 range = max4 - min4;
  vec3 lo = min4 - range * CAS_RING_PAD - 1e-4;
  vec3 hi = max4 + range * CAS_RING_PAD + 1e-4;
  outColor = mix(outColor, clamp(outColor, lo, hi), CAS_RING_BLEND);
  return clamp(outColor, 0.0, 1.0);
}
out vec4 fragColor;
void main() {
  vec2 uv = gl_FragCoord.xy / outputResolution.xy;
  vec3 color = texture(data, uv).rgb;
  if (sharpenFactor > 0.0) {
    color = filterId == 1 ? usmBoost(uv, color) : casBoost(uv, color);
  }
  if (applyColor) {
    color = mix(vec3(dot(color, LUMINOSITY_FACTOR)), color, saturation / 100.0);
    color = (contrast / 100.0) * (color - 0.5) + 0.5;
    color = (brightness / 100.0) * color;
  }
  fragColor = vec4(color, 1.0);
}`);
    gl.compileShader(fShader);
    if (!gl.getShaderParameter(fShader, gl.COMPILE_STATUS)) {
      throw new Error(`webgl2FragmentShaderCompileFailed:${gl.getShaderInfoLog(fShader) ?? "unknown"}`);
    }
    const program = gl.createProgram();
    this.program = program;
    gl.attachShader(program, vShader);
    gl.attachShader(program, fShader);
    gl.linkProgram(program);
    if (!gl.getProgramParameter(program, gl.LINK_STATUS)) {
      throw new Error(`webgl2ProgramLinkFailed:${gl.getProgramInfoLog(program) ?? "unknown"}`);
    }
    gl.useProgram(program);
    this.outputResolutionUniform = gl.getUniformLocation(program, "outputResolution");
    this.sourceResolutionUniform = gl.getUniformLocation(program, "sourceResolution");
    this.filterIdUniform = gl.getUniformLocation(program, "filterId");
    this.applyColorUniform = gl.getUniformLocation(program, "applyColor");
    this.sharpenFactorUniform = gl.getUniformLocation(program, "sharpenFactor");
    this.brightnessUniform = gl.getUniformLocation(program, "brightness");
    this.contrastUniform = gl.getUniformLocation(program, "contrast");
    this.saturationUniform = gl.getUniformLocation(program, "saturation");
    gl.useProgram(program);
    gl.activeTexture?.(gl.TEXTURE0);
    const buffer = gl.createBuffer();
    this.vertexBuffer = buffer;
    gl.bindBuffer(gl.ARRAY_BUFFER, buffer);
    gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1, -1, 3, -1, -1, 3]), gl.STATIC_DRAW);
    gl.enableVertexAttribArray(0);
    gl.vertexAttribPointer(0, 2, gl.FLOAT, false, 0, 0);
    const texture = gl.createTexture();
    this.videoTexture = texture;
    gl.bindTexture(gl.TEXTURE_2D, texture);
    gl.pixelStorei(gl.UNPACK_FLIP_Y_WEBGL, true);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
    gl.uniform1i(gl.getUniformLocation(program, "data"), 0);
    this.textureAllocated = false;
    this.refresh();
    this.guard?.check(performance.now(), true);
  }
  ensurePresentProgram(gl) {
    if (this.presentProgram !== null)
      return;
    const presentShader = gl.createShader(gl.FRAGMENT_SHADER);
    this.shaders.push(presentShader);
    gl.shaderSource(presentShader, `#version 300 es
precision highp float;
uniform sampler2D data;
uniform vec2 outputResolution;
uniform float brightness;
uniform float contrast;
uniform float saturation;
uniform bool applyColor;
const vec3 LUMINOSITY_FACTOR = vec3(0.299, 0.587, 0.114);
out vec4 fragColor;
void main() {
  vec2 uv = gl_FragCoord.xy / outputResolution.xy;
  vec3 color = texture(data, uv).rgb;
  if (applyColor) {
    color = mix(vec3(dot(color, LUMINOSITY_FACTOR)), color, saturation / 100.0);
    color = (contrast / 100.0) * (color - 0.5) + 0.5;
    color = (brightness / 100.0) * color;
  }
  fragColor = vec4(color, 1.0);
}`);
    gl.compileShader(presentShader);
    if (!gl.getShaderParameter(presentShader, gl.COMPILE_STATUS)) {
      throw new Error(`webgl2PresentShaderCompileFailed:${gl.getShaderInfoLog(presentShader) ?? "unknown"}`);
    }
    const presentProgram = gl.createProgram();
    this.presentProgram = presentProgram;
    gl.attachShader(presentProgram, this.shaders[0]);
    gl.attachShader(presentProgram, presentShader);
    gl.linkProgram(presentProgram);
    if (!gl.getProgramParameter(presentProgram, gl.LINK_STATUS)) {
      throw new Error(`webgl2PresentProgramLinkFailed:${gl.getProgramInfoLog(presentProgram) ?? "unknown"}`);
    }
    gl.useProgram(presentProgram);
    this.presentOutputResolutionUniform = gl.getUniformLocation(presentProgram, "outputResolution");
    this.presentBrightnessUniform = gl.getUniformLocation(presentProgram, "brightness");
    this.presentContrastUniform = gl.getUniformLocation(presentProgram, "contrast");
    this.presentSaturationUniform = gl.getUniformLocation(presentProgram, "saturation");
    this.presentApplyColorUniform = gl.getUniformLocation(presentProgram, "applyColor");
    gl.uniform1i(gl.getUniformLocation(presentProgram, "data"), 0);
  }
  refresh() {
    const gl = this.gl;
    if (!gl)
      return;
    const previous = this.uniformOptions;
    const width = this.processingWidth;
    const height = this.processingHeight;
    const sourcePass = this.usesSharpenTarget();
    if (!sourcePass && this.sharpenTexture !== null)
      this.releaseSharpenTarget();
    if (previous && this.uniformSourceWidth === width && this.uniformSourceHeight === height && this.uniformOutputWidth === this.currentOutputWidth && this.uniformOutputHeight === this.currentOutputHeight && previous.processing === this.options.processing && previous.sharpness === this.options.sharpness && previous.brightness === this.options.brightness && previous.contrast === this.options.contrast && previous.saturation === this.options.saturation && sourcePass === this.uniformUsesSharpenTarget) {
      return;
    }
    if (sourcePass)
      this.ensurePresentProgram(gl);
    const applyColor = this.options.brightness !== 100 || this.options.contrast !== 100 || this.options.saturation !== 100;
    if (this.program !== null) {
      gl.useProgram(this.program);
    }
    gl.uniform2f(this.outputResolutionUniform, sourcePass ? width : this.currentOutputWidth, sourcePass ? height : this.currentOutputHeight);
    gl.uniform2f(this.sourceResolutionUniform, width, height);
    gl.uniform1i(this.filterIdUniform, this.toFilterId(this.options.processing));
    gl.uniform1i(this.applyColorUniform, !sourcePass && applyColor ? 1 : 0);
    gl.uniform1f(this.sharpenFactorUniform, this.options.sharpness);
    gl.uniform1f(this.brightnessUniform, this.options.brightness);
    gl.uniform1f(this.contrastUniform, this.options.contrast);
    gl.uniform1f(this.saturationUniform, this.options.saturation);
    if (sourcePass && this.presentProgram) {
      gl.useProgram(this.presentProgram);
      gl.uniform2f(this.presentOutputResolutionUniform, this.currentOutputWidth, this.currentOutputHeight);
      gl.uniform1f(this.presentBrightnessUniform, this.options.brightness);
      gl.uniform1f(this.presentContrastUniform, this.options.contrast);
      gl.uniform1f(this.presentSaturationUniform, this.options.saturation);
      gl.uniform1i(this.presentApplyColorUniform, applyColor ? 1 : 0);
    }
    this.uniformOptions = { ...this.options };
    this.uniformSourceWidth = width;
    this.uniformSourceHeight = height;
    this.uniformOutputWidth = this.currentOutputWidth;
    this.uniformOutputHeight = this.currentOutputHeight;
    this.uniformUsesSharpenTarget = sourcePass;
  }
  captureRenderedFrame(signal) {
    if (this.isStopped)
      return Promise.resolve(null);
    const promise = this.frameCaptures.request(signal);
    if (this.frameCaptures.size > 0 && this.captureRafId === void 0) {
      this.captureRafId = requestAnimationFrame(() => {
        this.captureRafId = void 0;
        if (this.isStopped || this.frameCaptures.size === 0)
          return;
        try {
          this.renderFrame(true);
        } catch (error) {
          this.drawObservation.fail();
          this.onFailure?.(error instanceof Error ? error.message : "webgl2CaptureFailed");
        }
        this.onRenderFramePresented();
      });
    }
    return promise.finally(() => this.cancelCaptureDraw());
  }
  cancelCaptureDraw() {
    if (this.captureRafId !== void 0 && this.frameCaptures.size === 0) {
      cancelAnimationFrame(this.captureRafId);
      this.captureRafId = void 0;
    }
  }
  onRenderFramePresented() {
    if (this.frameCaptures.size === 0)
      return;
    this.frameCaptures.complete(() => {
      const gl = this.gl;
      return this.drewToScreenThisFrame && gl !== null && this.currentOutputWidth >= 2 && this.currentOutputHeight >= 2 ? readWebglDefaultFramebufferToCanvas(gl, this.currentOutputWidth, this.currentOutputHeight) : null;
    });
    this.cancelCaptureDraw();
  }
  renderFrame(capture = false) {
    const startedAtMs = performance.now();
    this.drewToScreenThisFrame = false;
    const frameSource = getVideoFrameSource(this.video);
    const frame = frameSource.renderSource;
    if (!frame && (!capture || !this.hasDrawnFrame))
      return;
    const sourceWidth = frame?.displayWidth ?? this.currentSourceWidth;
    const sourceHeight = frame?.displayHeight ?? this.currentSourceHeight;
    if (sourceWidth <= 0 || sourceHeight <= 0)
      return;
    const gl = this.gl;
    if (gl === null || this.program === null) {
      return;
    }
    if (sourceWidth !== this.currentSourceWidth || sourceHeight !== this.currentSourceHeight) {
      this.currentSourceWidth = sourceWidth;
      this.currentSourceHeight = sourceHeight;
      this.guard.assertSize(this.currentSourceWidth, this.currentSourceHeight, Math.max(this.currentOutputWidth, this.currentSourceWidth), Math.max(this.currentOutputHeight, this.currentSourceHeight));
      if (!this.explicitPresentTarget || this.currentOutputWidth < this.currentSourceWidth || this.currentOutputHeight < this.currentSourceHeight) {
        this.currentOutputWidth = this.srEnabled ? this.currentSourceWidth : Math.max(this.currentOutputWidth, this.currentSourceWidth);
        this.currentOutputHeight = this.srEnabled ? this.currentSourceHeight : Math.max(this.currentOutputHeight, this.currentSourceHeight);
        this.canvas.width = this.currentOutputWidth;
        this.canvas.height = this.currentOutputHeight;
        this.guard.assertBacking(this.currentOutputWidth, this.currentOutputHeight);
        gl.viewport(0, 0, this.currentOutputWidth, this.currentOutputHeight);
      }
      this.textureAllocated = false;
      this.refresh();
    }
    this.guard?.timer.begin(startedAtMs);
    let uploadFinishedAtMs;
    const uploadStartedAtMs = performance.now();
    try {
      if (this.videoTexture !== null)
        gl.bindTexture(gl.TEXTURE_2D, this.videoTexture);
      if (frame)
        gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, gl.RGBA, gl.UNSIGNED_BYTE, frame);
      if (!this.textureAllocated) {
        this.guard?.check(performance.now(), true);
        this.textureAllocated = true;
      }
      uploadFinishedAtMs = performance.now();
      if (this.usesReconstruction()) {
        this.reconstruction ??= new Fsr1ReconstructionPass(gl, this.shaders[0]);
        const rebuilt = this.reconstruction.render(this.videoTexture, sourceWidth, sourceHeight, this.currentOutputWidth, this.currentOutputHeight);
        gl.bindTexture(gl.TEXTURE_2D, rebuilt);
      }
      this.refresh();
      if (!this.drawThroughSharpenTarget(gl)) {
        gl.bindFramebuffer?.(gl.FRAMEBUFFER, null);
        gl.useProgram(this.program);
        gl.drawArrays(gl.TRIANGLES, 0, 3);
      }
    } finally {
      this.guard?.timer.end();
    }
    this.guard?.check(startedAtMs);
    this.drewToScreenThisFrame = true;
    const submittedAtMs = performance.now();
    this.guard?.timer.submitted(submittedAtMs);
    this.drawObservation.record(getVideoFrameSource(this.video).lastRenderFrame, capture, submittedAtMs - startedAtMs, uploadFinishedAtMs - uploadStartedAtMs, submittedAtMs - uploadFinishedAtMs, startedAtMs);
    if (!this.hasDrawnFrame) {
      this.hasDrawnFrame = true;
      this.canvas.style.opacity = "1";
      this.onFrameReady?.();
    }
  }
  /**
   * 输出尺寸与源尺寸不一致时，先把锐化按 1:1 渲进源尺寸 FBO，再升采样呈现。
   *
   * 单 pass 在非 1:1 采样下会让 5-tap 核落在非整数源像素上：`min/max` 窗口被双线性抹平、
   * 核也不对称，观感会随窗口尺寸漂移（同一档锐度在窗口/全屏下颗粒感不同）。
   */
  usesSharpenTarget() {
    return this.options.sharpness > 0 && this.explicitPresentTarget && (this.currentOutputWidth !== this.processingWidth || this.currentOutputHeight !== this.processingHeight);
  }
  usesReconstruction() {
    return this.srEnabled && (this.currentOutputWidth > this.currentSourceWidth || this.currentOutputHeight > this.currentSourceHeight);
  }
  get processingWidth() {
    return this.usesReconstruction() ? this.currentOutputWidth : this.currentSourceWidth;
  }
  get processingHeight() {
    return this.usesReconstruction() ? this.currentOutputHeight : this.currentSourceHeight;
  }
  drawThroughSharpenTarget(gl) {
    if (!this.usesSharpenTarget() || this.presentProgram === null) {
      return false;
    }
    if (!this.ensureSharpenTarget(gl)) {
      return false;
    }
    gl.bindFramebuffer(gl.FRAMEBUFFER, this.sharpenFramebuffer);
    gl.viewport(0, 0, this.currentSourceWidth, this.currentSourceHeight);
    gl.useProgram(this.program);
    gl.drawArrays(gl.TRIANGLES, 0, 3);
    gl.bindFramebuffer(gl.FRAMEBUFFER, null);
    gl.viewport(0, 0, this.currentOutputWidth, this.currentOutputHeight);
    gl.useProgram(this.presentProgram);
    gl.bindTexture(gl.TEXTURE_2D, this.sharpenTexture);
    gl.drawArrays(gl.TRIANGLES, 0, 3);
    return true;
  }
  /** 源尺寸目标按资源变化创建；硬故障交由播放资源所有者回退。 */
  ensureSharpenTarget(gl) {
    if (typeof gl.createFramebuffer !== "function" || typeof gl.framebufferTexture2D !== "function" || typeof gl.checkFramebufferStatus !== "function") {
      throw new TypeError("webgl2FramebufferUnsupported");
    }
    if (this.sharpenFramebuffer !== null && this.sharpenTexture !== null && this.sharpenTargetWidth === this.currentSourceWidth && this.sharpenTargetHeight === this.currentSourceHeight) {
      return true;
    }
    this.releaseSharpenTarget();
    const texture = gl.createTexture();
    if (texture === null) {
      throw new Error("webgl2TextureAllocationFailed");
    }
    gl.bindTexture(gl.TEXTURE_2D, texture);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
    gl.texImage2D(
      gl.TEXTURE_2D,
      0,
      gl.RGBA,
      this.currentSourceWidth,
      this.currentSourceHeight,
      0,
      gl.RGBA,
      gl.UNSIGNED_BYTE,
      null
    );
    const framebuffer = gl.createFramebuffer();
    if (framebuffer === null) {
      this.deleteTexture(gl, texture);
      throw new Error("webgl2FramebufferAllocationFailed");
    }
    gl.bindFramebuffer(gl.FRAMEBUFFER, framebuffer);
    gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, texture, 0);
    const complete = gl.checkFramebufferStatus(gl.FRAMEBUFFER) === gl.FRAMEBUFFER_COMPLETE;
    gl.bindFramebuffer(gl.FRAMEBUFFER, null);
    if (!complete) {
      this.deleteFramebuffer(gl, framebuffer);
      this.deleteTexture(gl, texture);
      throw new Error("webgl2FramebufferIncomplete");
    }
    this.sharpenFramebuffer = framebuffer;
    this.sharpenTexture = texture;
    this.sharpenTargetWidth = this.currentSourceWidth;
    this.sharpenTargetHeight = this.currentSourceHeight;
    this.guard?.check(performance.now(), true);
    if (this.videoTexture !== null) {
      gl.bindTexture(gl.TEXTURE_2D, this.videoTexture);
    }
    return true;
  }
  releaseSharpenTarget() {
    const gl = this.gl;
    if (gl === null) {
      this.sharpenFramebuffer = null;
      this.sharpenTexture = null;
      this.sharpenTargetWidth = 0;
      this.sharpenTargetHeight = 0;
      return;
    }
    this.deleteFramebuffer(gl, this.sharpenFramebuffer);
    this.deleteTexture(gl, this.sharpenTexture);
    this.sharpenFramebuffer = null;
    this.sharpenTexture = null;
    this.sharpenTargetWidth = 0;
    this.sharpenTargetHeight = 0;
  }
  deleteFramebuffer(gl, framebuffer) {
    if (framebuffer !== null && typeof gl.deleteFramebuffer === "function") {
      gl.deleteFramebuffer(framebuffer);
    }
  }
  deleteTexture(gl, texture) {
    if (texture !== null && typeof gl.deleteTexture === "function") {
      gl.deleteTexture(texture);
    }
  }
  releaseGl() {
    this.reconstruction?.dispose();
    this.reconstruction = void 0;
    this.releaseSharpenTarget();
    const gl = this.gl;
    if (gl) {
      this.deleteTexture(gl, this.videoTexture);
      if (this.program)
        gl.deleteProgram?.(this.program);
      if (this.presentProgram)
        gl.deleteProgram?.(this.presentProgram);
      if (this.vertexBuffer)
        gl.deleteBuffer?.(this.vertexBuffer);
      for (const shader of this.shaders.splice(0)) gl.deleteShader?.(shader);
    }
    this.shaders.length = 0;
    this.vertexBuffer = this.videoTexture = this.program = this.presentProgram = this.gl = null;
    this.guard?.timer.dispose();
    this.guard = void 0;
  }
  destroy() {
    this.stopPresentationLoop();
    this.frameCaptures.complete(() => null);
    this.cancelCaptureDraw();
    if (this.contextListenersBound) {
      this.canvas.removeEventListener("webglcontextlost", this.onContextLost);
      this.canvas.removeEventListener("webglcontextrestored", this.onContextRestored);
      this.contextListenersBound = false;
    }
    const gl = this.gl;
    try {
      this.releaseGl();
    } finally {
      releaseWebglContext(gl);
      super.destroy();
    }
  }
};
var filterSequence = 0;
var NativeVideoRenderer = class {
  kind = "video";
  filterId = `xbx-filter-${++filterSequence}`;
  config;
  styleElement = null;
  videoElement = null;
  constructor(config) {
    this.config = config;
  }
  snapshotResources() {
    return { outputWidth: this.videoElement?.videoWidth, outputHeight: this.videoElement?.videoHeight, gpuObjects: 0, frameRequests: 0, animationCallbacks: 0, captureWaiters: 0, domNodes: Number(this.styleElement !== null) };
  }
  snapshotDiagnostics() {
    return this.videoElement ? observeRenderSurface(this.videoElement) : void 0;
  }
  attach(video) {
    this.videoElement = video;
    video.dataset.renderFilter = this.filterId;
    this.ensureFilterNodes();
    video.dataset.renderPipeline = "video";
    this.refreshVideoFilterStyle();
  }
  update(config) {
    this.config = { ...this.config, ...config };
    this.refreshVideoFilterStyle();
  }
  async captureRenderedFrame(signal) {
    const video = this.videoElement;
    if (video === null || signal?.aborted) {
      return null;
    }
    return captureVideoElementToCanvas(video);
  }
  destroy() {
    if (this.videoElement?.dataset.renderFilter === this.filterId)
      delete this.videoElement.dataset.renderFilter;
    this.videoElement = null;
    this.styleElement?.remove();
    this.styleElement = null;
  }
  ensureFilterNodes() {
    if (!this.styleElement) {
      this.styleElement = document.createElement("style");
      this.styleElement.id = `${this.filterId}-css`;
      document.documentElement.appendChild(this.styleElement);
    }
  }
  refreshVideoFilterStyle() {
    if (!this.styleElement) {
      return;
    }
    const filters = [];
    if (this.config.saturation !== 100) {
      filters.push(`saturate(${this.config.saturation}%)`);
    }
    if (this.config.contrast !== 100) {
      filters.push(`contrast(${this.config.contrast}%)`);
    }
    if (this.config.brightness !== 100) {
      filters.push(`brightness(${this.config.brightness}%)`);
    }
    this.styleElement.textContent = filters.length > 0 ? `video[data-render-filter="${this.filterId}"] { filter: ${filters.join(" ")} !important; }` : "";
  }
};
var WebGL2VideoRenderer = class {
  get kind() {
    return this.config.superResolutionEnabled ? "webgl2_sr" : "webgl2";
  }
  player = null;
  config;
  constructor(config) {
    this.config = config;
  }
  snapshotResources() {
    return this.player?.snapshotResources();
  }
  snapshotDiagnostics() {
    return this.player?.snapshotDiagnostics();
  }
  async attach(video) {
    this.destroy();
    this.player = new WebGL2Processor(video);
    this.player.applyRendererPresentationConfig(this.config);
    this.player.setDisplayFormat(this.config.format);
    const preset = resolveShaderPreset(this.config);
    this.player.updateOptions({
      targetFps: this.config.targetFps,
      sharpness: preset.sharpenFactor,
      brightness: this.config.brightness,
      contrast: this.config.contrast,
      saturation: this.config.saturation,
      processingMode: preset.processingMode,
      processing: preset.processing
    });
    const player = this.player;
    await player.init();
    if (this.player !== player)
      return;
    video.dataset.renderPipeline = this.kind === "webgl2_sr" ? "webgl2-sr" : "webgl2";
  }
  update(config) {
    this.config = { ...this.config, ...config };
    this.player?.applyRendererPresentationConfig(this.config);
    this.player?.setDisplayFormat(this.config.format);
    const preset = resolveShaderPreset(this.config);
    this.player?.updateOptions({
      targetFps: this.config.targetFps,
      sharpness: preset.sharpenFactor,
      brightness: this.config.brightness,
      contrast: this.config.contrast,
      saturation: this.config.saturation,
      processing: preset.processing,
      processingMode: preset.processingMode
    }, true);
  }
  destroy() {
    this.player?.destroy();
    this.player = null;
  }
  async captureRenderedFrame(signal) {
    if (this.player === null) {
      return null;
    }
    return await this.player.captureRenderedFrame(signal);
  }
};
var SuperResolutionWebGL2Renderer = class extends WebGL2VideoRenderer {
  constructor(config) {
    super({ ...config, superResolutionEnabled: true });
  }
};

// ../../../../../../private/tmp/xbxrc-2026-11-10-clock-diagnostics-probe/entry.ts
var delay = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
var finite2 = (value) => typeof value === "number" && Number.isFinite(value) ? value : null;
function summary(values) {
  const sorted = [...values].sort((a, b) => a - b);
  return { count: values.length, mean: values.reduce((a, b) => a + b, 0) / (values.length || 1), p50: sorted[Math.max(0, Math.ceil(values.length * 0.5) - 1)] ?? null, p95: sorted[Math.max(0, Math.ceil(values.length * 0.95) - 1)] ?? null, p99: sorted[Math.max(0, Math.ceil(values.length * 0.99) - 1)] ?? null, max: sorted.at(-1) ?? null };
}
async function waitFor(predicate, limitMs = 12e3) {
  const started = performance.now();
  while (!predicate()) {
    if (performance.now() - started > limitMs) throw new Error("local fixture readiness timeout");
    await delay(50);
  }
}
async function localWebRtc(video) {
  if (new URLSearchParams(location.search).get("external") === "1") return externalWebRtc(video);
  const canvas = document.createElement("canvas");
  canvas.width = 2560;
  canvas.height = 1440;
  canvas.style.cssText = "position:fixed;right:0;bottom:0;width:160px;height:90px;pointer-events:none";
  document.body.appendChild(canvas);
  const context = canvas.getContext("2d", { alpha: false });
  let frames = 0;
  let raf = 0;
  const tick = () => {
    context.fillStyle = "#142234";
    context.fillRect(0, 0, canvas.width, canvas.height);
    context.fillStyle = frames % 2 ? "#ec6436" : "#287bcf";
    context.fillRect(frames * 16 % 2300, 500, 220, 220);
    context.fillStyle = "#fff";
    context.font = "72px monospace";
    context.fillText(String(frames++), 60, 120);
    raf = requestAnimationFrame(tick);
  };
  tick();
  const stream = canvas.captureStream(60);
  const sender = new RTCPeerConnection({ iceServers: [] });
  const receiver = new RTCPeerConnection({ iceServers: [] });
  let closing = false;
  const candidateErrors = [];
  const eventCounts = { senderCandidates: 0, receiverCandidates: 0, receivedTracks: 0, playFailures: 0 };
  const pendingSender = [];
  const pendingReceiver = [];
  const add = async (peer, candidate) => {
    try {
      await peer.addIceCandidate(candidate);
    } catch (error) {
      candidateErrors.push({ name: error.name });
    }
  };
  sender.onicecandidate = (event) => {
    if (!event.candidate || closing) return;
    eventCounts.senderCandidates++;
    if (receiver.remoteDescription) void add(receiver, event.candidate);
    else pendingSender.push(event.candidate);
  };
  receiver.onicecandidate = (event) => {
    if (!event.candidate || closing) return;
    eventCounts.receiverCandidates++;
    if (sender.remoteDescription) void add(sender, event.candidate);
    else pendingReceiver.push(event.candidate);
  };
  receiver.ontrack = (event) => {
    eventCounts.receivedTracks++;
    video.srcObject = new MediaStream([event.track]);
    void video.play().catch(() => eventCounts.playFailures++);
  };
  const track = stream.getVideoTracks()[0];
  const settings = () => {
    const value = track.getSettings();
    return { width: value.width, height: value.height, frameRate: value.frameRate, resizeMode: value.resizeMode };
  };
  const sourceSetup = { initialTrackSettings: settings() };
  try {
    await track.applyConstraints({ width: { exact: 2560 }, height: { exact: 1440 }, frameRate: { ideal: 60 } });
  } catch (error) {
    sourceSetup.constraintError = { name: error.name, message: error.message };
  }
  sourceSetup.constrainedTrackSettings = settings();
  const transceiver = sender.addTransceiver(track, { direction: "sendonly", streams: [stream], sendEncodings: [{ maxBitrate: 12e6, maxFramerate: 60 }] });
  const codecs = RTCRtpSender.getCapabilities("video").codecs.filter((codec) => codec.mimeType.toLowerCase() === "video/h264");
  if (!codecs.length) throw new Error("H264 codec unavailable");
  transceiver.setCodecPreferences(codecs);
  await sender.setLocalDescription(await sender.createOffer());
  await receiver.setRemoteDescription(sender.localDescription);
  for (const candidate of pendingSender.splice(0)) await add(receiver, candidate);
  await receiver.setLocalDescription(await receiver.createAnswer());
  await sender.setRemoteDescription(receiver.localDescription);
  for (const candidate of pendingReceiver.splice(0)) await add(sender, candidate);
  try {
    const parameters = transceiver.sender.getParameters();
    parameters.degradationPreference = "maintain-resolution";
    for (const encoding of parameters.encodings) encoding.scaleResolutionDownBy = 1;
    await transceiver.sender.setParameters(parameters);
  } catch (error) {
    sourceSetup.senderParametersError = { name: error.name, message: error.message };
  }
  const applied = transceiver.sender.getParameters();
  sourceSetup.senderParameters = { degradationPreference: applied.degradationPreference, encodings: applied.encodings.map((encoding) => ({ active: encoding.active, maxBitrate: encoding.maxBitrate, maxFramerate: encoding.maxFramerate, scaleResolutionDownBy: encoding.scaleResolutionDownBy })) };
  try {
    await waitFor(() => video.readyState >= 2 && video.videoWidth > 0);
  } catch (error) {
    const entries = [...(await receiver.getStats()).values()];
    const inbound = entries.find((entry) => entry.type === "inbound-rtp" && (entry.kind ?? entry.mediaType) === "video");
    throw new Error(JSON.stringify({ phase: "localWebRTC.firstDecodedVideo", error: String(error), senderConnection: sender.connectionState, receiverConnection: receiver.connectionState, senderIce: sender.iceConnectionState, receiverIce: receiver.iceConnectionState, senderGathering: sender.iceGatheringState, receiverGathering: receiver.iceGatheringState, eventCounts, candidateErrors, canvasFrames: frames, trackReady: track.readyState, trackMuted: track.muted, videoReady: video.readyState, videoPaused: video.paused, visibility: document.visibilityState, inbound: inbound ? { framesReceived: inbound.framesReceived, framesDecoded: inbound.framesDecoded, frameWidth: inbound.frameWidth, frameHeight: inbound.frameHeight, packetsReceived: inbound.packetsReceived } : null }));
  }
  await video.play();
  return {
    sourceSetup,
    senderFrames: () => frames,
    stats: async () => {
      const entries = [...(await receiver.getStats()).values()];
      const inbound = entries.find((entry) => entry.type === "inbound-rtp" && (entry.kind ?? entry.mediaType) === "video");
      const codec = entries.find((entry) => entry.type === "codec" && entry.id === inbound?.codecId);
      if (!inbound) throw new Error("No inbound video statistics");
      return { timestamp: inbound.timestamp, framesReceived: inbound.framesReceived, framesDecoded: inbound.framesDecoded, framesDropped: inbound.framesDropped, packetsLost: inbound.packetsLost, nackCount: inbound.nackCount, freezeCount: inbound.freezeCount, totalFreezesDuration: inbound.totalFreezesDuration, totalDecodeTime: inbound.totalDecodeTime, frameWidth: inbound.frameWidth, frameHeight: inbound.frameHeight, codec: codec?.mimeType, fmtp: codec?.sdpFmtpLine };
    },
    close: () => {
      closing = true;
      cancelAnimationFrame(raf);
      track.stop();
      sender.close();
      receiver.close();
      canvas.width = canvas.height = 1;
      canvas.remove();
    }
  };
}
async function measure(input, mode) {
  const video = document.createElement("video");
  video.muted = true;
  video.playsInline = true;
  video.autoplay = true;
  video.style.width = "800px";
  video.style.height = "450px";
  document.querySelector("#stage").appendChild(video);
  let rtc;
  if (input === "mp4") {
    video.src = "source.mp4";
    video.loop = true;
    await video.play();
  } else {
    rtc = await localWebRtc(video);
    try {
      await waitFor(() => video.videoWidth === 2560 && video.videoHeight === 1440, 6e3);
    } catch {
    }
    if (video.videoWidth !== 2560 || video.videoHeight !== 1440) {
      const diagnostics2 = { phase: "localWebRTC.fixtureDimensions", videoWidth: video.videoWidth, videoHeight: video.videoHeight, sourceSetup: rtc.sourceSetup, stats: await rtc.stats() };
      rtc.close();
      throw new Error(JSON.stringify(diagnostics2));
    }
  }
  const source = mode === "native-only" ? void 0 : getVideoFrameSource(video);
  source?.snapshotDiagnostics();
  const failures = [];
  const config = { enabled: true, pipelineType: mode.startsWith("sr") ? "webgl2_sr" : mode.startsWith("sharp") ? "webgl2" : "video", mode: "native", processing: "usm", processingMode: "quality", sharpness: mode.startsWith("sharp") ? 3 : 0, targetFps: 60, format: "Contain", brightness: 100, contrast: 100, saturation: 100, renderOutputWidth: 2560, renderOutputHeight: 1440, renderViewportWidth: 800, renderViewportHeight: 450, superResolutionOutputWidth: 2560, superResolutionOutputHeight: 1440, superResolutionRcasStops: 0.88, rendererRuntimeFailureNotifier: (reason) => failures.push(reason) };
  const renderer = mode.startsWith("sr") ? new SuperResolutionWebGL2Renderer(config) : mode.startsWith("sharp") ? new WebGL2VideoRenderer(config) : mode === "video" ? new NativeVideoRenderer(config) : void 0;
  await renderer?.attach(video);
  const processor = renderer?.processor ?? renderer?.player;
  const ablation = void 0;
  const direct = processor?.gl && new URLSearchParams(location.search).get("direct") === "1" ? await installDirectFrameProbe(video, renderer) : void 0;
  const drawable = new URLSearchParams(location.search).get("drawable") === "1" ? createDrawableIdentitySampler(video) : void 0;
  let driver;
  if (processor?.gl) {
    const gl = processor.gl;
    const extension = gl.getExtension("WEBGL_debug_renderer_info");
    driver = extension ? gl.getParameter(extension.UNMASKED_RENDERER_WEBGL) : gl.getParameter(gl.RENDERER);
    if (mode === "sr-rgba") {
      const original = gl.texImage2D.bind(gl);
      gl.texImage2D = (...args) => {
        if (args.length === 6 && args[5] === video && args[2] === gl.RGB) args[2] = args[3] = gl.RGBA;
        return original(...args);
      };
    }
  }
  let request = 0;
  let raf = 0;
  let measuring = false;
  let callbacks = 0;
  let nativeSerial = 0;
  let nativePresentedFrames;
  let candidates = 0;
  let advanced = 0;
  let rafCount = 0;
  let previousNative;
  let previousRaf;
  const markerStride = Math.max(1, Number(new URLSearchParams(location.search).get("markerStride") ?? 1));
  let markerTicks = 0;
  const samples = [];
  const intervals = [];
  const leads = [];
  const nativeDeltas = [];
  const mediaDeltas = [];
  const counterDeltas = [];
  const counterIntervals = [];
  const counterResets = [];
  let lastCounterAt = 0;
  const visibilityChanges = [];
  const onVisibility = () => visibilityChanges.push({ at: performance.now(), state: document.visibilityState });
  document.addEventListener("visibilitychange", onVisibility);
  const read = () => {
    const quality = video.getVideoPlaybackQuality();
    return { at: performance.now(), currentTime: video.currentTime, visibility: document.visibilityState, paused: video.paused, readyState: video.readyState, qualityTotal: quality.totalVideoFrames, qualityDropped: quality.droppedVideoFrames, webkitTotal: finite2(video.webkitDecodedFrameCount), webkitDropped: finite2(video.webkitDroppedFrameCount) };
  };
  const subscribe = () => {
    const callback = (at, metadata) => {
      const now = performance.now();
      if (!source || metadata?.trackingSource === "videoFrameCallback") {
        nativeSerial++;
        nativePresentedFrames = metadata?.presentedFrames;
      }
      if (measuring && (!source || metadata?.trackingSource === "videoFrameCallback")) {
        callbacks++;
        const row = { ...read(), callbackAt: at, enteredAt: now, presentedFrames: metadata?.presentedFrames, mediaTime: metadata?.mediaTime, presentationTime: metadata?.presentationTime, expectedDisplayTime: metadata?.expectedDisplayTime, rtpTimestamp: metadata?.rtpTimestamp, localSequence: metadata?.localSequence, newFrame: metadata?.newFrame, derivedAdvance: metadata?.presentedFramesDelta };
        if (previousNative) {
          intervals.push(now - previousNative.enteredAt);
          if (finite2(row.presentedFrames) !== null && finite2(previousNative.presentedFrames) !== null) nativeDeltas.push(row.presentedFrames - previousNative.presentedFrames);
          if (finite2(row.mediaTime) !== null && finite2(previousNative.mediaTime) !== null) mediaDeltas.push(row.mediaTime - previousNative.mediaTime);
        }
        if (finite2(row.expectedDisplayTime) !== null) leads.push(row.expectedDisplayTime - now);
        previousNative = row;
        if (samples.length < 24 || callbacks % 30 === 0) samples.push(row);
      }
      if (measuring) {
        candidates += Number(source ? metadata?.newFrame : true);
        advanced += metadata?.presentedFramesDelta ?? 0;
      }
      request = subscribe();
    };
    return source ? source.request(callback) : video.requestVideoFrameCallback(callback);
  };
  request = subscribe();
  const onRaf = () => {
    const row = read();
    if (markerTicks++ % markerStride === 0)
      drawable?.sample({ nativeSerial, nativePresentedFrames, localSequence: source?.lastFrame?.localSequence ?? nativeSerial });
    if (measuring) {
      rafCount++;
      if (previousRaf) {
        const delta = row.qualityTotal - previousRaf.qualityTotal;
        if (delta > 0) {
          counterDeltas.push(delta);
          if (lastCounterAt) counterIntervals.push(row.at - lastCounterAt);
          lastCounterAt = row.at;
        }
        if (delta < 0) counterResets.push({ before: previousRaf, after: row });
      }
      previousRaf = row;
    }
    raf = requestAnimationFrame(onRaf);
  };
  raf = requestAnimationFrame(onRaf);
  await delay(2e3);
  const start = { ...read(), frameSource: source?.snapshotResources(), direct: direct?.snapshot(), draw: renderer?.snapshotResources()?.realtimeDrawCount ?? 0, rtc: await rtc?.stats(), senderFrames: rtc?.senderFrames() };
  measuring = true;
  await delay(Number(new URLSearchParams(location.search).get("durationMs") ?? 6500));
  measuring = false;
  const end = { ...read(), frameSource: source?.snapshotResources(), direct: direct?.snapshot(), draw: renderer?.snapshotResources()?.realtimeDrawCount ?? 0, rtc: await rtc?.stats(), senderFrames: rtc?.senderFrames() };
  const elapsedMs = end.at - start.at;
  const resources = renderer?.snapshotResources();
  const diagnostics = source?.snapshotDiagnostics();
  await drawable?.finish();
  if (failures.length || resources?.submitFailureCount) throw new Error(JSON.stringify({ failures, resources }));
  const foregroundValid = start.visibility === "visible" && end.visibility === "visible" && visibilityChanges.every((change) => change.state === "visible") && rafCount * 1e3 / elapsedMs >= 50;
  const result = { input, mode, markerStride, sourceSetup: rtc?.sourceSetup, foregroundValid, visibilityChanges, elapsedMs, callbackFps: callbacks * 1e3 / elapsedMs, candidateFps: candidates * 1e3 / elapsedMs, rafFps: rafCount * 1e3 / elapsedMs, derivedAdvanceFps: advanced * 1e3 / elapsedMs, rawNativeAdvanceFps: nativeDeltas.reduce((a, b) => a + b, 0) * 1e3 / elapsedMs, rawMediaAdvanceFps: mediaDeltas.reduce((a, b) => a + b, 0) * 60 * 1e3 / elapsedMs, drawFps: mode.startsWith("sr") || mode.startsWith("sharp") ? (end.draw - start.draw) * 1e3 / elapsedMs : null, start, end, driver, callbackIntervalMs: summary(intervals), nativeDelta: summary(nativeDeltas), mediaDeltaSec: summary(mediaDeltas), expectedDisplayLeadMs: summary(leads), qualityUpdateDelta: summary(counterDeltas), qualityUpdateIntervalMs: summary(counterIntervals), counterResets, samples, resources, diagnostics, ablation: ablation?.snapshot(), drawableIdentity: drawable?.snapshot(start.at, end.at) };
  source ? source.cancel(request) : video.cancelVideoFrameCallback(request);
  cancelAnimationFrame(raf);
  document.removeEventListener("visibilitychange", onVisibility);
  ablation?.close();
  await direct?.close();
  drawable?.close();
  renderer?.destroy();
  result.releasedFrameSource = source?.snapshotResources();
  rtc?.close();
  video.pause();
  video.srcObject = null;
  video.removeAttribute("src");
  video.load();
  video.remove();
  await delay(200);
  return result;
}
async function run() {
  const runs = [];
  const parameters = new URLSearchParams(location.search);
  const capabilities = typeof Worker !== "function" ? { state: "unavailable" } : await new Promise((resolve) => {
    const script = new Blob([`postMessage({trackProcessor: typeof MediaStreamTrackProcessor, videoFrame: typeof VideoFrame, videoDecoder: typeof VideoDecoder, trackGenerator: typeof MediaStreamTrackGenerator})`], { type: "text/javascript" });
    const workerUrl = URL.createObjectURL(script);
    const worker = new Worker(workerUrl);
    const done = (result) => {
      worker.terminate();
      URL.revokeObjectURL(workerUrl);
      resolve(result);
    };
    const timer = setTimeout(() => done({ state: "timeout" }), 1e3);
    worker.onmessage = (event) => {
      clearTimeout(timer);
      done(event.data);
    };
    worker.onerror = () => {
      clearTimeout(timer);
      done({ state: "error" });
    };
  });
  const windowCapabilities = { trackProcessor: typeof window.MediaStreamTrackProcessor, videoFrame: typeof VideoFrame, videoDecoder: typeof VideoDecoder, trackGenerator: typeof window.MediaStreamTrackGenerator, worker: capabilities };
  if (parameters.get("modes") === "capabilities")
    return { environment: { userAgent: navigator.userAgent, visible: document.visibilityState, capabilities: windowCapabilities }, runs };
  const modes = parameters.get("modes")?.split(",") ?? (parameters.get("external") === "1" ? ["native-only", "video", "sr-full", "sharp-full", "video"] : ["native-only", "video", "sr-full", "sr-no-upload", "sr-scissor", "sr-transparent", "sharp-full", "sharp-no-upload", "sharp-scissor", "video"]);
  for (const mode of modes) {
    document.querySelector("#status").textContent = `\u672C\u5730 1440p \u5BF9\u7167\uFF1A${mode}`;
    const result = await measure("webrtc", mode);
    runs.push(result);
    const ms = result.end.at - result.start.at;
    const progress = {
      mode,
      callbackFps: result.callbackFps,
      candidateFps: result.candidateFps,
      rafFps: result.rafFps,
      drawFps: result.drawFps,
      receivedFps: (result.end.rtc.framesReceived - result.start.rtc.framesReceived) * 1e3 / ms,
      decodedFps: (result.end.rtc.framesDecoded - result.start.rtc.framesDecoded) * 1e3 / ms,
      decodedFrameMode: result.end.frameSource?.renderFrameSource,
      decodedFramesFps: result.end.frameSource && result.start.frameSource ? (result.end.frameSource.decodedFramesReceived - result.start.frameSource.decodedFramesReceived) * 1e3 / ms : null,
      width: result.end.rtc.frameWidth,
      height: result.end.rtc.frameHeight,
      foregroundValid: result.foregroundValid,
      uploadP95Ms: result.resources?.timing?.upload?.p95Ms,
      intervalP95Ms: result.resources?.timing?.interval?.p95Ms,
      uploads: result.ablation?.uploadCalls,
      skippedUploads: result.ablation?.skippedUploads,
      completedFenceSamples: result.ablation?.fence.samples.filter((s) => s.state === "completed").length,
      drawableChangeFps: result.drawableIdentity?.drawableChangeFps,
      drawableNewWithoutNative: result.drawableIdentity?.newImageWithoutNativeCallbackCount,
      drawableNewUnqualified: result.drawableIdentity?.newImageWithoutSourceQualificationCount,
      drawableValidCount: result.drawableIdentity?.validCount,
      drawableCpuP95Ms: result.drawableIdentity?.cpuP95Ms,
      fenceUpperBoundMs: result.ablation?.fence.samples.filter((s) => s.state === "completed").map((s) => s.upperBoundMs)
    };
    await fetch("/progress", { method: "POST", body: JSON.stringify(progress) });
  }
  return { environment: {
    userAgent: navigator.userAgent,
    visible: document.visibilityState,
    capabilities: windowCapabilities,
    input: "local H264 WebRTC",
    width: 2560,
    height: 1440,
    targetFps: 60,
    qualification: "source supply measured per run; fence latency includes queue and polling; no-upload/scissor are isolated diagnostic controls"
  }, runs };
}
window.XBXProbe = { run: async () => {
  try {
    return await run();
  } catch (error) {
    return { error: String(error), stack: error.stack };
  }
} };
export {
  run
};
