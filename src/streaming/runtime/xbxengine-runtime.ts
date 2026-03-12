import type { StreamStats } from '../../player'
import type { RuntimeDisplayState, RuntimeEvent, RuntimePort, StreamRuntimeReconnectReason } from './runtime-contract'
import { events } from '../../services/events'
import { rpc } from '../../services/rpc'

function toRuntimeReconnectReason(reason: StreamRuntimeReconnectReason) {
  if (reason === 'ice-failed') {
    return 'iceFailed' as const
  }
  if (reason === 'media-stalled') {
    return 'mediaStalled' as const
  }
  return 'networkLost' as const
}

function toStreamingMode(targetType: 'home' | 'cloud') {
  return targetType === 'cloud' ? 'cloudGaming' as const : 'localHost' as const
}

/**
 * Rust runtime 通过 RPC 适配到同一套 runtime/UI 协议。
 */
export function createXbxEngineRuntime(options: {
  playerElementId: string
  initialAudioVolume: number
}): RuntimePort {
  const listeners = new Set<(event: RuntimeEvent) => void>()
  const viewportElementId = options.playerElementId
  let audioVolume = options.initialAudioVolume
  let microphoneState = {
    capturing: false,
    paused: false,
  }

  function emit(event: RuntimeEvent): void {
    for (const listener of listeners) {
      listener(event)
    }
  }

  const unsubscribe = events.on('streaming.xbxEngineRuntimeEvent', (event) => {
    if (event.type === 'runtime.phaseChanged') {
      emit({ type: 'phaseChanged', phase: event.phase })
      return
    }
    if (event.type === 'transport.connectionState') {
      emit({
        type: 'connectionStateChanged',
        state: event.state as RTCPeerConnectionState,
      })
      return
    }
    if (event.type === 'chat.stateChanged') {
      microphoneState = {
        capturing: event.capturing,
        paused: event.paused,
      }
      emit({ type: 'microphoneStateChanged', ...microphoneState })
      return
    }
    if (event.type === 'stats.videoFrameProcessed') {
      emit({ type: 'framePresented' })
      return
    }
    if (event.type === 'error') {
      emit({
        type: 'error',
        error: new Error(`${event.code}:${event.message}`),
      })
    }
  })

  return {
    async launch(spec) {
      await rpc.xbxEngine.attachViewport({
        viewportId: viewportElementId,
      })
      await rpc.xbxEngine.startRuntime({
        session: {
          sessionId: spec.sessionId,
          targetType: spec.targetType,
          turnServer: spec.runtime.turnServer ?? null,
        },
        viewport: {
          viewportId: viewportElementId,
        },
        mode: toStreamingMode(spec.targetType),
        runtime: spec.runtime,
        render: spec.render,
        audioVolume,
      })
    },
    async stop() {
      unsubscribe()
      microphoneState = {
        capturing: false,
        paused: false,
      }
      await rpc.xbxEngine.stopRuntime()
    },
    async requestReconnect(reason) {
      await rpc.xbxEngine.requestReconnect({
        reason: toRuntimeReconnectReason(reason),
      })
    },
    applyDisplayState(state: RuntimeDisplayState) {
      void rpc.xbxEngine.applyDisplayState({
        state: {
          display_options: {
            sharpness: state.displayOptions.sharpness,
            saturation: state.displayOptions.saturation,
            contrast: state.displayOptions.contrast,
            brightness: state.displayOptions.brightness,
          },
        },
      })
    },
    setAudioVolume(value) {
      audioVolume = value
      void rpc.xbxEngine.setAudioVolume({ value })
    },
    async setMicrophoneEnabled(enabled) {
      if (enabled) {
        await rpc.xbxEngine.startMicrophone()
        return true
      }
      await rpc.xbxEngine.stopMicrophone()
      return false
    },
    pressHome(durationMs) {
      void rpc.xbxEngine.pressControllerButton({
        button: 'home',
        durationMs,
      })
    },
    snapshotStats: async (): Promise<StreamStats> => {
      const snapshot = await rpc.xbxEngine.snapshotStats()
      return {
        resolution: snapshot.resolution,
        rtt: snapshot.rtt,
        fps: snapshot.fps,
        pl: snapshot.pl,
        fl: snapshot.fl,
        jit: snapshot.jit,
        br: snapshot.br,
        decode: snapshot.decode,
      }
    },
    subscribe(listener) {
      listeners.add(listener)
      return () => {
        listeners.delete(listener)
      }
    },
  }
}
