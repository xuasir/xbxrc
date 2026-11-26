import type { GamepadFrame, InputRuntimeConfig, ProcessedVideoFrameMetadata } from '../../domain/input'
import type { InputPacketStats } from '../../domain/stats'
import type { BaseChannel } from '../../protocol/channels/BaseChannel'
import type { InputWorkerCommand, InputWorkerReply, InputWorkerSnapshot, TransferredInputChannelBinding } from './input-worker-protocol'
import type { InputCore } from './InputCore'
import { clonePadState, neutralState, stateEquals } from '@shared/gamepad/state'
import { DecodedFrameClock } from '../../infra/render/decoded-frame-clock'
import { MAX_PENDING_INPUTS, releasesControl, sameControls } from './InputCore'

interface InputWorkerHooks {
  trace: (event: string, payload: Record<string, unknown>) => void
  packet: (packet: InputPacketStats) => void
  rumble: (event: MessageEvent<ArrayBuffer>) => void
  failed: (reason: string) => void
  detached: (host: InputWorkerHost) => void
}

let channelGeneration = 0

/** 有界状态交接只确认入队；真实 send 的接受结果继续由 Worker InputCore 决定。 */
export class InputWorkerHost {
  static canAttempt(config: Partial<InputRuntimeConfig>): boolean {
    return config.inputExecution !== 'main' && typeof Worker === 'function' && typeof RTCDataChannel === 'function'
  }

  static async prepare(config: Partial<InputRuntimeConfig>, hooks: InputWorkerHooks): Promise<InputWorkerHost | undefined> {
    if (!this.canAttempt(config))
      return
    const host = new InputWorkerHost(config, hooks)
    try {
      await host.prepare()
      return host
    }
    catch {
      host.close()
    }
  }

  private readonly generation = ++channelGeneration
  private worker?: Worker
  private readonly clock = new DecodedFrameClock()
  private clockTimer?: ReturnType<typeof setTimeout>
  private preparationTimer?: ReturnType<typeof setTimeout>
  private preparationResolve?: () => void
  private preparationReject?: (reason: Error) => void
  private supported = false
  private attached = false
  private closed = false
  private active = false
  private epoch = 0
  private handler?: BaseChannel
  private channelState: RTCDataChannelState = 'connecting'
  private bufferedAmount = 0
  private snapshot?: InputWorkerSnapshot
  private readyResolve?: () => void
  private readyReject?: (reason: Error) => void
  private attachmentTimer?: ReturnType<typeof setTimeout>
  private readyPromise?: Promise<void>
  private stopTimer?: ReturnType<typeof setTimeout>
  private stopWaitTimer?: ReturnType<typeof setTimeout>
  private stopPromise?: Promise<void>
  private stopResolve?: () => void
  private stopEpoch?: number
  private pendingFrames: GamepadFrame[] = []
  private pendingFeedback: Array<{ frame: ProcessedVideoFrameMetadata, enqueuedAtEpochMs: number }> = []
  private feedbackAvailable?: boolean
  private streamOwner = false
  private lastDispatchedState = neutralState()
  private barrierFlight?: number
  private barrierPending?: Extract<InputWorkerCommand, { type: 'barrier' }>
  private batchId = 0
  private flight?: { id: number, epoch: number, states: number }
  private resetReason?: string
  private remotePending = 0
  private flushQueued = false
  private configRevision = 0
  private configFlight?: number
  private configPending = false
  private messagesSent = 0
  private messagesReceived = 0
  private stagedOverflows = 0

  private constructor(private config: Partial<InputRuntimeConfig>, private readonly hooks: InputWorkerHooks) {}

  private async prepare(): Promise<void> {
    const worker = new Worker(new URL('./input-worker.ts', import.meta.url), { type: 'module', name: 'xbx-input-send' })
    this.worker = worker
    worker.onmessage = (event: MessageEvent<InputWorkerReply>) => this.onMessage(event.data)
    worker.onerror = () => this.fail('workerError')
    worker.onmessageerror = () => this.fail('workerMessageError')
    await new Promise<void>((resolve, reject) => {
      this.preparationResolve = resolve
      this.preparationReject = reject
      this.preparationTimer = setTimeout(() => reject(new Error('inputWorkerProbeTimeout')), 2000)
      this.send({ type: 'probe', id: this.generation })
      this.probeClock()
    })
    clearTimeout(this.preparationTimer)
    this.preparationTimer = undefined
    this.preparationResolve = this.preparationReject = undefined
  }

  /** 在 createDataChannel 的同一 turn、绑定事件和发送之前执行实际转移。 */
  adopt(channel: RTCDataChannel): TransferredInputChannelBinding | undefined {
    if (this.closed || !this.worker)
      return
    const c = this.clock.convert(0, performance.timeOrigin + performance.now())
    if (!c)
      return
    this.channelState = channel.readyState
    this.bufferedAmount = channel.bufferedAmount
    try {
      this.send({ type: 'attach', channelGeneration: this.generation, channel, mainTimeOriginMs: performance.timeOrigin, clockOffsetMs: c.atMs, config: this.config }, [channel as unknown as Transferable])
      this.attached = true
      this.readyPromise = new Promise<void>((resolve, reject) => {
        this.readyResolve = resolve
        this.readyReject = reject
        this.attachmentTimer = setTimeout(() => this.fail('inputChannelAttachTimeout'), 2000)
      })
      return {
        context: { readyState: () => this.channelState, bufferedAmount: () => this.bufferedAmount, send: () => false },
        setHandler: (handler) => { this.handler = handler },
        dispose: () => {
          const handler = this.handler
          this.handler = undefined
          handler?.onClose()
          this.close()
        },
      }
    }
    catch {
      this.hooks.trace('inputExecutionCapabilityObserved', { mode: 'main', reason: 'dataChannelTransferRejected', channelGeneration: this.generation })
      this.close()
    }
  }

  whenAttached(): Promise<void> { return this.readyPromise ?? Promise.resolve() }

  start(): void {
    if (this.active)
      this.stop()
    this.active = true
    this.stopPromise = undefined
    this.send({ type: 'start', channelGeneration: this.generation, inputEpoch: ++this.epoch })
  }

  stop(): void {
    if (!this.active)
      return
    this.active = false
    this.pendingFrames = []
    this.pendingFeedback = []
    this.flight = undefined
    this.remotePending = 0
    this.resetReason = undefined
    this.barrierPending = undefined
    this.lastDispatchedState = neutralState()
    if (!this.closed && this.attached) {
      const epoch = ++this.epoch
      this.stopEpoch = epoch
      this.stopPromise = new Promise((resolve) => {
        this.stopResolve = resolve
      })
      this.stopWaitTimer = setTimeout(() => this.finishStop(), 250)
      this.send({ type: 'stop', channelGeneration: this.generation, inputEpoch: epoch })
    }
  }

  whenStopped(): Promise<void> | undefined { return this.stopPromise }

  private finishStop(): void {
    clearTimeout(this.stopWaitTimer)
    this.stopWaitTimer = undefined
    this.stopResolve?.()
    this.stopResolve = undefined
    this.stopEpoch = undefined
  }

  updateConfig(config: Partial<InputRuntimeConfig>): void {
    this.config = { ...this.config, ...config }
    this.configRevision++
    this.configPending = true
    this.flushConfig()
  }

  private flushConfig(): void {
    if (!this.attached || this.closed || !this.configPending || this.configFlight !== undefined)
      return
    this.configPending = false
    this.configFlight = this.configRevision
    this.send({ type: 'configure', channelGeneration: this.generation, config: this.config, revision: this.configRevision })
  }

  queueFrame(frame: GamepadFrame, streamOwner: boolean): void {
    if (!this.active || this.closed)
      return
    this.streamOwner = streamOwner
    const next = { ...frame, state: clonePadState(frame.state), observedAtMs: frame.observedAtMs ?? performance.now() }
    const last = this.pendingFrames.at(-1)
    const previous = this.pendingFrames.at(-2)?.state ?? this.lastDispatchedState
    if (next.resetReason || (!streamOwner && stateEquals(next.state, neutralState()))) {
      this.pendingFrames = [next]
      this.resetReason = next.resetReason ?? 'owner-changed'
      this.remotePending = 0
      this.epoch++
      this.barrierPending = { type: 'barrier', channelGeneration: this.generation, inputEpoch: this.epoch, frame: next, streamOwner, reason: this.resetReason }
      this.flushBarrier()
    }
    else if (last && !last.resetReason && sameControls(previous, last.state) && sameControls(last.state, next.state)
      && !releasesControl(previous, last.state) && !releasesControl(last.state, next.state)) {
      // 只合并同控件态的连续轴值；数字边沿与归零保持顺序。
      this.pendingFrames[this.pendingFrames.length - 1] = next
    }
    else {
      this.pendingFrames.push(next)
    }
    if (this.pendingFrames.length + this.remotePending + (this.flight?.states ?? 0) > MAX_PENDING_INPUTS) {
      this.stagedOverflows++
      this.pendingFrames = [{ gamepadIndex: next.gamepadIndex, state: neutralState(), observedAtMs: performance.now() }, next]
      this.resetReason = 'handoff-budget-exhausted'
      this.remotePending = 0
      this.epoch++
      this.barrierPending = { type: 'barrier', channelGeneration: this.generation, inputEpoch: this.epoch, frame: next, streamOwner, reason: this.resetReason }
      this.flushBarrier()
    }
    this.scheduleFlush()
  }

  private flushBarrier(): void {
    if (!this.barrierPending || this.barrierFlight !== undefined || this.closed || !this.active)
      return
    const command = this.barrierPending
    this.barrierPending = undefined
    this.barrierFlight = command.inputEpoch
    this.send(command)
  }

  addProcessedFrame(frame: ProcessedVideoFrameMetadata): void {
    if (!this.active || this.closed)
      return
    this.pendingFeedback.push({ frame: { ...frame, observation: frame.observation ? { ...frame.observation } : undefined }, enqueuedAtEpochMs: performance.timeOrigin + performance.now() })
    if (this.pendingFeedback.length > 58)
      this.pendingFeedback.splice(0, this.pendingFeedback.length - 58)
    this.scheduleFlush()
  }

  observeFeedbackAvailability(available: boolean): void {
    if (!this.active || this.closed)
      return
    this.feedbackAvailable = available
    this.scheduleFlush()
  }

  private scheduleFlush(): void {
    if (this.flushQueued)
      return
    this.flushQueued = true
    queueMicrotask(() => {
      this.flushQueued = false
      this.flushFrames()
    })
  }

  private flushFrames(): void {
    if (!this.active || this.closed || this.flight || (!this.pendingFrames.length && !this.pendingFeedback.length && this.feedbackAvailable === undefined))
      return
    const frames = this.pendingFrames.splice(0, 16)
    if (frames.at(-1))
      this.lastDispatchedState = clonePadState(frames.at(-1)!.state)
    const feedback = this.pendingFeedback.splice(0, 58)
    const id = ++this.batchId
    this.flight = { id, epoch: this.epoch, states: frames.length }
    this.send({ type: 'frames', channelGeneration: this.generation, inputEpoch: this.epoch, batchId: id, sentAtEpochMs: performance.timeOrigin + performance.now(), streamOwner: this.streamOwner, frames, feedback, feedbackAvailable: this.feedbackAvailable, resetReason: this.resetReason })
    this.feedbackAvailable = undefined
    this.resetReason = undefined
  }

  getFeedbackState(): ReturnType<InputCore['getResolutionFeedbackState']> {
    return this.config.preventResolutionDrops === false ? 'disabled' : this.snapshot?.feedbackState ?? 'awaiting-metadata'
  }

  snapshotFeedback(): ReturnType<InputCore['snapshotFeedback']> | undefined { return this.snapshot?.feedback }

  private onMessage(message: InputWorkerReply): void {
    this.messagesReceived++
    if (message.type === 'stopped' && message.snapshot.channelGeneration === this.generation && message.snapshot.inputEpoch === this.stopEpoch)
      this.finishStop()
    if (message.type === 'closed') {
      if (message.snapshot.channelGeneration === this.generation) {
        this.snapshot = message.snapshot
        this.retire()
      }
      return
    }
    if (this.closed)
      return
    if (message.type === 'clock') {
      const epoch = performance.timeOrigin + performance.now()
      this.clock.accept(message, epoch)
      const c = this.clock.snapshot(epoch)
      if (this.attached && c.offsetMs !== undefined)
        this.send({ type: 'clockSync', channelGeneration: this.generation, clockOffsetMs: c.offsetMs })
      if (this.supported && c.state === 'calibrated')
        this.preparationResolve?.()
      else if (c.samples < 3 && (c.probes ?? 0) < 8)
        this.probeClock()
      return
    }
    if (message.type === 'probeResult') {
      if (message.id !== this.generation)
        return
      this.supported = message.supported
      if (!message.supported)
        this.preparationReject?.(new Error('workerDataChannelUnavailable'))
      else if (this.clock.snapshot(performance.timeOrigin + performance.now()).state === 'calibrated')
        this.preparationResolve?.()
      return
    }
    const generation = 'snapshot' in message ? message.snapshot.channelGeneration : message.channelGeneration
    if (generation !== this.generation)
      return
    if ('snapshot' in message) {
      const raw = message.snapshot
      this.channelState = raw.readyState
      this.bufferedAmount = raw.bufferedAmount
      this.remotePending = raw.inputEpoch === this.epoch ? raw.pendingInputs : this.remotePending
      this.snapshot = { ...raw, feedback: raw.feedback ?? this.snapshot?.feedback }
      if (message.type === 'snapshot')
        this.send({ type: 'snapshotAck', channelGeneration: this.generation })
      if (raw.packets && raw.packets.packetBytes > 0)
        this.hooks.packet(raw.packets)
      for (const trace of raw.traces ?? [])
        this.hooks.trace(trace.event, trace.payload)
      if (message.type === 'attached') {
        clearTimeout(this.attachmentTimer)
        this.attachmentTimer = undefined
        this.readyResolve?.()
        this.readyResolve = this.readyReject = undefined
        this.hooks.trace('inputExecutionCapabilityObserved', { mode: 'input-worker', channelGeneration: this.generation, clock: this.clock.snapshot(performance.timeOrigin + performance.now()) })
        this.flushConfig()
      }
      if (raw.feedback)
        this.hooks.trace('inputExecutionObserved', { ...raw, packets: undefined, traces: undefined, clock: this.clock.snapshot(performance.timeOrigin + performance.now()), pendingHandoff: this.pendingFrames.length, handoffInFlight: this.flight?.states ?? 0, stagedOverflows: this.stagedOverflows, messagesSent: this.messagesSent, messagesReceived: this.messagesReceived })
    }
    else if (message.type === 'framesAck' && message.inputEpoch === this.flight?.epoch && message.batchId === this.flight.id) {
      this.remotePending = message.inputEpoch === this.epoch ? message.pendingInputs : 0
      this.flight = undefined
      this.flushFrames()
    }
    else if (message.type === 'barrierAck' && message.inputEpoch === this.barrierFlight) {
      this.barrierFlight = undefined
      this.flushBarrier()
    }
    else if (message.type === 'configured' && message.revision === this.configFlight) {
      this.configFlight = undefined
      this.flushConfig()
    }
    else if (message.type === 'rumble') {
      this.hooks.rumble(new MessageEvent('message', { data: message.data }))
    }
    else if (message.type === 'channelEvent') {
      if (message.event === 'open') {
        this.channelState = 'open'
        this.handler?.onOpen()
      }
      else if (message.event === 'closing') {
        this.channelState = 'closing'
        this.handler?.onClosing()
      }
      else if (message.event === 'close') {
        this.channelState = 'closed'
        this.handler?.onClose()
      }
      else {
        this.handler?.onError()
      }
    }
  }

  private probeClock(): void {
    if (this.closed)
      return
    const probe = this.clock.probe(performance.timeOrigin + performance.now())
    if (probe)
      this.send(probe)
    clearTimeout(this.clockTimer)
    this.clockTimer = setTimeout(() => this.probeClock(), 5000)
  }

  private send(command: InputWorkerCommand, transfer: Transferable[] = []): void {
    this.messagesSent++
    this.worker?.postMessage(command, transfer)
  }

  private fail(reason: string): void {
    this.preparationReject?.(new Error(reason))
    this.readyReject?.(new Error(reason))
    if (this.attached) {
      this.hooks.failed(reason)
      this.close()
    }
  }

  close(): void {
    if (this.closed)
      return
    this.closed = true
    this.active = false
    this.pendingFrames = []
    this.pendingFeedback = []
    this.flight = undefined
    this.handler = undefined
    clearTimeout(this.preparationTimer)
    clearTimeout(this.attachmentTimer)
    clearTimeout(this.clockTimer)
    this.preparationTimer = this.attachmentTimer = this.clockTimer = undefined
    this.preparationReject?.(new Error('inputWorkerRetired'))
    this.readyReject?.(new Error('inputWorkerRetired'))
    this.hooks.detached(this)
    if (!this.attached) {
      this.retire()
      return
    }
    try {
      this.send({ type: 'close', channelGeneration: this.generation })
      if (this.worker)
        this.stopTimer = setTimeout(() => this.retire(), 250)
    }
    catch { this.retire() }
  }

  private retire(): void {
    this.finishStop()
    clearTimeout(this.stopTimer)
    this.stopTimer = undefined
    if (!this.worker)
      return
    this.worker?.terminate()
    this.worker = undefined
    this.hooks.trace('inputExecutionStopped', { channelGeneration: this.generation, inputEpoch: this.epoch, stop: this.snapshot?.stop, pendingHandoff: this.pendingFrames.length, messagesSent: this.messagesSent, messagesReceived: this.messagesReceived })
  }
}
