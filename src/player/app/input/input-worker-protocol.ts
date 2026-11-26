import type { GamepadFrame, InputRuntimeConfig, ProcessedVideoFrameMetadata } from '../../domain/input'
import type { InputPacketStats } from '../../domain/stats'
import type { WorkerClockReply } from '../../infra/render/decoded-frame-clock'
import type { InputCore } from './InputCore'

export interface InputWorkerSnapshot {
  channelGeneration: number
  inputEpoch: number
  observedAtEpochMs: number
  readyState: RTCDataChannelState
  bufferedAmount: number
  pendingInputs: number
  feedbackState: ReturnType<InputCore['getResolutionFeedbackState']>
  feedback?: ReturnType<InputCore['snapshotFeedback']>
  packets?: InputPacketStats
  traces?: Array<{ event: string, payload: Record<string, unknown> }>
  traceOverflows: number
  handoff: { batches: number, states: number, lastAgeMs?: number, maxAgeMs: number }
  stop?: ReturnType<InputCore['stop']>
}

export type InputWorkerCommand
  = | { type: 'probe', id: number }
    | { type: 'clock', id: number }
    | { type: 'attach', channelGeneration: number, channel: RTCDataChannel, mainTimeOriginMs: number, clockOffsetMs: number, config: Partial<InputRuntimeConfig> }
    | { type: 'clockSync', channelGeneration: number, clockOffsetMs: number }
    | { type: 'snapshotAck', channelGeneration: number }
    | { type: 'barrier', channelGeneration: number, inputEpoch: number, frame: GamepadFrame, streamOwner: boolean, reason: string }
    | { type: 'start' | 'stop', channelGeneration: number, inputEpoch: number }
    | { type: 'configure', channelGeneration: number, config: Partial<InputRuntimeConfig>, revision: number }
    | { type: 'frames', channelGeneration: number, inputEpoch: number, batchId: number, sentAtEpochMs: number, streamOwner: boolean, frames: GamepadFrame[], feedback: Array<{ frame: ProcessedVideoFrameMetadata, enqueuedAtEpochMs: number }>, feedbackAvailable?: boolean, resetReason?: string }
    | { type: 'close', channelGeneration: number }

export type InputWorkerReply
  = | WorkerClockReply
    | { type: 'probeResult', id: number, supported: boolean }
    | { type: 'channelEvent', channelGeneration: number, event: 'open' | 'closing' | 'close' | 'error' }
    | { type: 'rumble', channelGeneration: number, data: ArrayBuffer }
    | { type: 'framesAck', channelGeneration: number, inputEpoch: number, batchId: number, pendingInputs: number }
    | { type: 'barrierAck', channelGeneration: number, inputEpoch: number }
    | { type: 'configured', channelGeneration: number, revision: number }
    | { type: 'snapshot' | 'attached' | 'stopped' | 'closed', snapshot: InputWorkerSnapshot }

export interface TransferredInputChannelBinding {
  context: import('../../protocol/channels/BaseChannel').ChannelContext
  setHandler: (handler: import('../../protocol/channels/BaseChannel').BaseChannel) => void
  dispose: () => void
}
