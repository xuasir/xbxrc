import type { StreamStats } from '../../../player'
import { TypedEventEmitter } from '../../../player/api/events'
import { events } from '../../../services/events'
import { rpc } from '../../../services/rpc'
import type {
  StreamRuntimeDisplayState,
  StreamRuntimeEventMap,
  StreamRuntimeReconnectReason,
  StreamRuntimeStartContext,
  StreamRuntimeViewportHost
} from '../contracts'
import type { RustOwnedXbxEngineClient } from './xbxengine-client'

function toRuntimeReconnectReason(reason: StreamRuntimeReconnectReason) {
  if (reason === 'ice-failed') {
    return 'iceFailed' as const
  }
  if (reason === 'media-stalled') {
    return 'mediaStalled' as const
  }
  return 'networkLost' as const
}

/**
 * `rust-owned` client 只做 renderer -> main 的轻桥接，实时细节全部留给 xbxEngine。
 */
export function createRustOwnedXbxEngineClient(): RustOwnedXbxEngineClient {
  const emitter = new TypedEventEmitter<StreamRuntimeEventMap>()
  let microphoneState = {
    capturing: false,
    paused: false
  }

  const unsubscribe = events.on('streaming.xbxEngineRuntimeEvent', (event) => {
    if (event.type === 'runtime.phaseChanged') {
      emitter.emit('runtime.phaseChanged', {
        phase: event.phase
      })
      return
    }
    if (event.type === 'transport.connectionState') {
      emitter.emit('transport.connectionState', {
        state: event.state as RTCPeerConnectionState
      })
      return
    }
    if (event.type === 'chat.stateChanged') {
      microphoneState = {
        capturing: event.capturing,
        paused: event.paused
      }
      emitter.emit('chat.stateChanged', microphoneState)
      return
    }
    if (event.type === 'media.videoReady') {
      emitter.emit('media.videoReady', {
        width: event.width,
        height: event.height
      })
      return
    }
    if (event.type === 'media.surfaceReady') {
      emitter.emit('media.surfaceReady', {
        surfaceId: event.surfaceId
      })
      return
    }
    if (event.type === 'stats.videoFrameProcessed') {
      emitter.emit('stats.videoFrameProcessed', {
        firstFramePacketArrivalTimeMs: event.firstFramePacketArrivalTimeMs,
        frameDecodedTimeMs: event.frameDecodedTimeMs,
        frameRenderedTimeMs: event.frameRenderedTimeMs
      })
      return
    }
    emitter.emit('error', {
      error: new Error(`${event.code}:${event.message}`)
    })
  })

  return {
    async startRuntime(context: StreamRuntimeStartContext): Promise<void> {
      await rpc.xbxEngine.startRuntime({
        sessionId: context.session.sessionId,
        targetType: context.session.targetType,
        turnServer: context.session.turnServer ?? null,
        viewportId: context.viewportHost.elementId,
        audioVolume: context.audioVolume
      })
    },
    async requestReconnect(reason: StreamRuntimeReconnectReason): Promise<void> {
      await rpc.xbxEngine.requestReconnect({
        reason: toRuntimeReconnectReason(reason)
      })
    },
    async stopRuntime(): Promise<void> {
      unsubscribe()
      microphoneState = {
        capturing: false,
        paused: false
      }
      await rpc.xbxEngine.stopRuntime()
      emitter.clear()
    },
    async attachViewport(host: StreamRuntimeViewportHost): Promise<void> {
      await rpc.xbxEngine.attachViewport({
        viewportId: host.elementId
      })
    },
    async detachViewport(): Promise<void> {
      await rpc.xbxEngine.detachViewport()
    },
    async applyDisplayState(state: StreamRuntimeDisplayState): Promise<void> {
      await rpc.xbxEngine.applyDisplayState({
        state: {
          display_options: {
            sharpness: state.displayOptions.sharpness,
            saturation: state.displayOptions.saturation,
            contrast: state.displayOptions.contrast,
            brightness: state.displayOptions.brightness
          }
        }
      })
    },
    async pressControllerButton(button: string, durationMs: number): Promise<void> {
      await rpc.xbxEngine.pressControllerButton({
        button,
        durationMs
      })
    },
    async setAudioVolume(value: number): Promise<void> {
      await rpc.xbxEngine.setAudioVolume({
        value
      })
    },
    async startMicrophone(): Promise<void> {
      await rpc.xbxEngine.startMicrophone()
    },
    async stopMicrophone(): Promise<void> {
      await rpc.xbxEngine.stopMicrophone()
    },
    getMicrophoneState() {
      return microphoneState
    },
    async snapshotStats(): Promise<StreamStats> {
      const snapshot = await rpc.xbxEngine.snapshotStats()
      return {
        resolution: snapshot.resolution,
        rtt: snapshot.rtt,
        fps: snapshot.fps,
        pl: snapshot.pl,
        fl: snapshot.fl,
        jit: snapshot.jit,
        br: snapshot.br,
        decode: snapshot.decode
      }
    },
    events() {
      return emitter
    }
  }
}
