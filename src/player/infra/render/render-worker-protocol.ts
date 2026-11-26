import type { RendererRuntimeConfig } from '../../domain/media'
import type { WorkerClockReply } from './decoded-frame-clock'
import type { DecodedFrameWorkerMessage } from './decoded-video-frame-reader'
import type { DecodedFrameWorkerRequest } from './decoded-video-frame-worker-source'
import type { RendererResources } from './render-observation'
import type { RenderSurfaceStyle } from './webgl-frame-processor'

export interface FrameLinkScope {
  surfaceGeneration: number
  videoGeneration: number
  readerGeneration: number
  linkEpoch: number
}

export function sameFrameScope(a: FrameLinkScope | undefined, b: FrameLinkScope | undefined): boolean {
  return Boolean(a && b && a.surfaceGeneration === b.surfaceGeneration && a.videoGeneration === b.videoGeneration
    && a.readerGeneration === b.readerGeneration && a.linkEpoch === b.linkEpoch)
}

export type SerializableRenderConfig = Omit<RendererRuntimeConfig, 'rendererFrameReadyNotifier' | 'rendererRuntimeFailureNotifier' | 'superResolutionRuntimeDegradeNotifier' | 'rendererExecutionNotifier'>

export function serializeRenderConfig(config: RendererRuntimeConfig): SerializableRenderConfig {
  const { rendererFrameReadyNotifier: _ready, rendererRuntimeFailureNotifier: _failure, superResolutionRuntimeDegradeNotifier: _degrade, rendererExecutionNotifier: _execution, ...value } = config
  return value
}

export type FramePortMessage = DecodedFrameWorkerMessage & { scope: FrameLinkScope }
export type FramePortRequest = DecodedFrameWorkerRequest & { scope: FrameLinkScope, outcome?: 'submitted' | 'rateLimited' | 'stale' | 'paused' | 'failed' | 'retired' }

export interface RenderWorkerSnapshot {
  scope: FrameLinkScope
  renderEpoch: number
  observedAtEpochMs: number
  timeOriginMs: number
  sourceWidth: number
  sourceHeight: number
  latestSequence?: number
  resources: RendererResources
  context: { contextLost: boolean | null, contextAttributes: WebGLContextAttributes | null }
  link: { frames: number, acknowledgements: number, rejectedFrames: number, heldFrames: number, frameQueueLimit: 1, applicationFrameLimit: 3, messagesReceived: number, messagesSent: number, bytesReceived: number, bytesSent: number, readFrames?: number, deliveredFrames?: number, overwrittenFrames?: number, staleDrops?: number }
  observation: { snapshots: number, aggregateCount: number, cpuTotalMs: number, cpuMaxMs: number }
}

export interface RenderCapturePixels { width: number, height: number, pixels: Uint8ClampedArray<ArrayBuffer>, frameSequence?: number }

export type RenderWorkerCommand
  = | { type: 'probe', id: number, config?: SerializableRenderConfig }
    | { type: 'clock', id: number }
    | { type: 'init', scope: FrameLinkScope, canvas: OffscreenCanvas, port: MessagePort, config: SerializableRenderConfig, revision: number }
    | { type: 'bindFrames', scope: FrameLinkScope, port: MessagePort }
    | { type: 'configure', scope: FrameLinkScope, config: SerializableRenderConfig, revision: number }
    | { type: 'pause', scope: FrameLinkScope }
    | { type: 'snapshotAck', scope: FrameLinkScope }
    | { type: 'capture', scope: FrameLinkScope, requestId: number }
    | { type: 'cancelCapture', scope: FrameLinkScope, requestId: number }
    | { type: 'stop', scope: FrameLinkScope }

export type RenderWorkerReply
  = | WorkerClockReply
    | { type: 'probeResult', id: number, supported: boolean, readerWorker: boolean, reason?: string }
    | { type: 'ready' | 'frameReady' | 'snapshot' | 'stopped', snapshot: RenderWorkerSnapshot }
    | { type: 'configured', scope: FrameLinkScope, revision: number }
    | { type: 'style', scope: FrameLinkScope, style: RenderSurfaceStyle }
    | { type: 'failed', scope: FrameLinkScope, renderEpoch: number, reason: string }
    | { type: 'captureResult', scope: FrameLinkScope, requestId: number, requestIds?: number[], renderEpoch: number, frameSequence?: number, image?: ImageBitmap, rgba?: RenderCapturePixels }

/** 成本只计算应用元数据；VideoFrame 的引擎/驱动引用独立由宿主实现管理。 */
export function frameMessageBytes(message: FramePortMessage | FramePortRequest): number {
  const { frame: _frame, track: _track, port: _port, ...metadata } = message as FramePortMessage & FramePortRequest & { frame?: VideoFrame, port?: MessagePort }
  return JSON.stringify(metadata).length * 2
}
