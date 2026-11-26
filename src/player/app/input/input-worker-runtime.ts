import type { GamepadFrame } from '../../domain/input'
import type { InputPacketStats } from '../../domain/stats'
import type { InputWorkerCommand, InputWorkerReply } from './input-worker-protocol'
import { clonePadState } from '@shared/gamepad/state'
import { InputCore } from './InputCore'

interface WorkerEndpoint { postMessage: (message: InputWorkerReply, transfer?: Transferable[]) => void }

/** 真正的 RTCDataChannel.send 和 accepted 出队在输入执行域内同步完成。 */
export class InputWorkerRuntime {
  private channel?: RTCDataChannel
  private core?: InputCore
  private channelGeneration = 0
  private inputEpoch = 0
  private active = false
  private closed = false
  private owner = false
  private latestFrame?: GamepadFrame
  private mainTimeOriginMs = 0
  private offsetMs = 0
  private timer?: ReturnType<typeof setTimeout>
  private nextAggregateAtMs = 0
  private snapshotInFlight = false
  private packetBatch = this.emptyPackets()
  private traces: Array<{ event: string, payload: Record<string, unknown> }> = []
  private traceOverflows = 0
  private lastStop?: ReturnType<InputCore['stop']>
  private readonly handoff = { batches: 0, states: 0, lastAgeMs: undefined as number | undefined, maxAgeMs: 0 }
  private readonly listeners: Array<() => void> = []

  constructor(private readonly endpoint: WorkerEndpoint) {}

  handle(command: InputWorkerCommand): void {
    if (command.type === 'clock') {
      const receivedAtEpochMs = performance.timeOrigin + performance.now()
      this.endpoint.postMessage({ type: 'clock', id: command.id, receivedAtEpochMs, sentAtEpochMs: performance.timeOrigin + performance.now() })
      return
    }
    if (command.type === 'probe') {
      this.endpoint.postMessage({ type: 'probeResult', id: command.id, supported: typeof RTCDataChannel === 'function' })
      return
    }
    if (command.type === 'attach') {
      if (this.channel || this.closed) {
        command.channel.close()
        return
      }
      this.attach(command)
      return
    }
    if (this.closed || command.channelGeneration !== this.channelGeneration)
      return
    if (command.type === 'clockSync') {
      this.offsetMs = command.clockOffsetMs
    }
    else if (command.type === 'snapshotAck') {
      this.snapshotInFlight = false
    }
    else if (command.type === 'barrier') {
      if (this.active && command.inputEpoch >= this.inputEpoch) {
        this.inputEpoch = command.inputEpoch
        this.owner = command.streamOwner
        this.latestFrame = this.convertFrame(command.frame)
        this.core?.resetSource(this.latestFrame, command.reason)
      }
      this.endpoint.postMessage({ type: 'barrierAck', channelGeneration: this.channelGeneration, inputEpoch: command.inputEpoch })
    }
    else if (command.type === 'configure') {
      this.core?.updateConfig(command.config)
      this.endpoint.postMessage({ type: 'configured', channelGeneration: this.channelGeneration, revision: command.revision })
    }
    else if (command.type === 'start' && command.inputEpoch > this.inputEpoch) {
      this.inputEpoch = command.inputEpoch
      this.active = true
      this.core?.start({
        send: (data) => {
          try {
            if (this.channel?.readyState !== 'open')
              return false
            this.channel.send(data)
            return true
          }
          catch { return false }
        },
        getReadyState: () => this.channel?.readyState ?? 'closed',
        bufferedAmount: () => this.channel?.bufferedAmount ?? 0,
        onWritable: (threshold, callback) => {
          const channel = this.channel!
          channel.bufferedAmountLowThreshold = threshold
          channel.addEventListener('bufferedamountlow', callback)
          return () => channel.removeEventListener('bufferedamountlow', callback)
        },
      })
    }
    else if (command.type === 'stop' && command.inputEpoch >= this.inputEpoch) {
      this.inputEpoch = command.inputEpoch
      this.stop()
      this.publish('stopped')
    }
    else if (command.type === 'frames') {
      if (this.active && command.inputEpoch >= this.inputEpoch) {
        this.inputEpoch = command.inputEpoch
        this.owner = command.streamOwner
        const age = performance.timeOrigin + performance.now() + this.offsetMs - command.sentAtEpochMs
        this.handoff.batches++
        this.handoff.states += command.frames.length
        this.handoff.lastAgeMs = age >= -2 ? Math.max(0, age) : undefined
        this.handoff.maxAgeMs = Math.max(this.handoff.maxAgeMs, this.handoff.lastAgeMs ?? 0)
        if (command.resetReason && command.frames[0]) {
          this.latestFrame = this.convertFrame(command.frames[0])
          this.core?.resetSource(this.latestFrame, command.resetReason)
        }
        for (const raw of command.resetReason ? command.frames.slice(1) : command.frames) {
          const frame = this.convertFrame(raw)
          this.latestFrame = frame
          this.core?.queueGamepadState(frame)
        }
        for (const feedback of command.feedback)
          this.core?.addProcessedFrame(feedback.frame, feedback.enqueuedAtEpochMs - this.offsetMs - performance.timeOrigin)
        if (command.feedbackAvailable !== undefined)
          this.core?.observeFrameFeedbackAvailability(command.feedbackAvailable)
      }
      this.endpoint.postMessage({ type: 'framesAck', channelGeneration: this.channelGeneration, inputEpoch: command.inputEpoch, batchId: command.batchId, pendingInputs: this.core?.pendingCount ?? 0 })
    }
    else if (command.type === 'close') {
      this.close()
    }
  }

  private attach(command: Extract<InputWorkerCommand, { type: 'attach' }>): void {
    this.channel = command.channel
    this.channelGeneration = command.channelGeneration
    this.mainTimeOriginMs = command.mainTimeOriginMs
    this.offsetMs = command.clockOffsetMs
    const channel = this.channel
    channel.binaryType = 'arraybuffer'
    for (const event of ['open', 'closing', 'close', 'error'] as const) {
      const listener = () => {
        this.endpoint.postMessage({ type: 'channelEvent', channelGeneration: this.channelGeneration, event })
        if (event === 'close')
          this.close()
      }
      channel.addEventListener(event, listener)
      this.listeners.push(() => channel.removeEventListener(event, listener))
    }
    const onMessage = (event: MessageEvent) => {
      if (!this.closed && event.data instanceof ArrayBuffer)
        this.endpoint.postMessage({ type: 'rumble', channelGeneration: this.channelGeneration, data: event.data }, [event.data])
    }
    channel.addEventListener('message', onMessage)
    this.listeners.push(() => channel.removeEventListener('message', onMessage))
    this.core = new InputCore({
      isStreamOwner: () => this.owner,
      currentState: () => this.latestFrame ? { ...this.latestFrame, observedAtMs: performance.now() } : undefined,
      wireNow: () => performance.timeOrigin + performance.now() + this.offsetMs - this.mainTimeOriginMs,
      packet: (packet) => {
        for (const key of ['packetBytes', 'metadataFrames', 'gamepadFrames', 'pointerFrames', 'mouseFrames', 'keyboardFrames'] as const)
          this.packetBatch[key] += packet[key]
        this.packetBatch.inputPackets! += Number(packet.gamepadFrames + packet.pointerFrames + packet.mouseFrames + packet.keyboardFrames > 0)
        this.packetBatch.metadataPackets! += Number(packet.metadataFrames > 0)
      },
      processedFrame: () => {},
      trace: (event, raw) => {
        const payload = { ...raw, executionDomain: 'input-worker', channelGeneration: this.channelGeneration, inputEpoch: this.inputEpoch }
        for (const key of ['observedAtMs', 'sampledAtMs'] as const) {
          if (typeof raw[key] === 'number')
            Object.assign(payload, { [key]: raw[key] + performance.timeOrigin + this.offsetMs - this.mainTimeOriginMs })
        }
        if (this.traces.length < 64)
          this.traces.push({ event, payload })
        else
          this.traceOverflows++
      },
    }, command.config, 4)
    this.publish('attached')
    this.scheduleSnapshot()
  }

  private convertFrame(frame: GamepadFrame): GamepadFrame {
    const convert = (value: number | undefined) => value === undefined ? undefined : value + this.mainTimeOriginMs - this.offsetMs - performance.timeOrigin
    return { ...frame, state: clonePadState(frame.state), observedAtMs: convert(frame.observedAtMs), sampledAtMs: convert(frame.sampledAtMs) }
  }

  private emptyPackets(): InputPacketStats { return { packetBytes: 0, metadataFrames: 0, gamepadFrames: 0, pointerFrames: 0, mouseFrames: 0, keyboardFrames: 0, inputPackets: 0, metadataPackets: 0 } }

  private publish(type: 'attached' | 'snapshot' | 'stopped' | 'closed'): void {
    if (type === 'snapshot') {
      if (this.snapshotInFlight)
        return
      this.snapshotInFlight = true
    }
    const now = performance.now()
    const aggregate = now >= this.nextAggregateAtMs || type === 'stopped' || type === 'closed'
    if (aggregate)
      this.nextAggregateAtMs = now + 1000
    this.endpoint.postMessage({ type, snapshot: { channelGeneration: this.channelGeneration, inputEpoch: this.inputEpoch, observedAtEpochMs: performance.timeOrigin + now, readyState: this.channel?.readyState ?? 'closed', bufferedAmount: this.channel?.bufferedAmount ?? 0, pendingInputs: this.core?.pendingCount ?? 0, feedbackState: this.core?.getResolutionFeedbackState() ?? 'disabled', feedback: aggregate ? this.core?.snapshotFeedback() : undefined, packets: this.packetBatch, traces: this.traces, traceOverflows: this.traceOverflows, handoff: { ...this.handoff }, stop: this.lastStop } })
    this.packetBatch = this.emptyPackets()
    this.traces = []
  }

  private scheduleSnapshot(): void {
    this.timer = setTimeout(() => {
      if (this.closed)
        return
      this.publish('snapshot')
      this.scheduleSnapshot()
    }, 100)
  }

  private stop(): void {
    this.active = false
    this.owner = false
    const result = this.core?.stop()
    if (result?.active)
      this.lastStop = result
    this.latestFrame = undefined
  }

  private close(): void {
    if (this.closed)
      return
    this.closed = true
    this.stop()
    clearTimeout(this.timer)
    this.timer = undefined
    for (const dispose of this.listeners)
      dispose()
    this.listeners.length = 0
    this.channel?.close()
    this.channel = undefined
    this.publish('closed')
  }
}
