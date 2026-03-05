import { TypedEventEmitter } from '../../../player/api/events'
import type {
  StreamRuntime,
  StreamRuntimeCapabilities,
  StreamRuntimeControllerInputController,
  StreamRuntimeDisplayState,
  StreamRuntimeEventMap,
  StreamRuntimeReconnectReason,
  StreamRuntimeStatsController,
  StreamRuntimeStartContext,
  StreamRuntimeViewportController,
  StreamRuntimeViewportHost
} from '../contracts'
import type { RustOwnedXbxEngineClient } from './xbxengine-client'

const RUST_OWNED_CAPABILITIES: StreamRuntimeCapabilities = {
  transportOwner: 'sidecar',
  decodeOwner: 'sidecar',
  renderOwner: 'sidecar',
  controllerInputOwner: 'sidecar'
}

function createUnavailableError(): Error {
  return new Error('rustOwnedRuntimeUnavailable')
}

/**
 * `RustOwnedRuntime` 现在只做 xbxEngine client 壳，实时细节后续继续下沉到 Rust。
 */
export class RustOwnedRuntime implements StreamRuntime {
  readonly mode = 'rust-owned' as const
  readonly capabilities = RUST_OWNED_CAPABILITIES

  private readonly emitter = new TypedEventEmitter<StreamRuntimeEventMap>()
  private readonly client: RustOwnedXbxEngineClient
  private viewportHost: StreamRuntimeViewportHost
  private readonly eventCleanups: Array<() => void> = []
  private microphoneState = {
    capturing: false,
    paused: false
  }

  constructor(viewportElementId: string, client: RustOwnedXbxEngineClient) {
    this.viewportHost = {
      elementId: viewportElementId
    }
    this.client = client
    this.microphoneState = client.getMicrophoneState()
    this.bindClientEvents()
  }

  async start(context: StreamRuntimeStartContext): Promise<void> {
    this.viewportHost = context.viewportHost
    await this.client.attachViewport(context.viewportHost)
    await this.client.startRuntime(context)
  }

  async requestReconnect(reason: StreamRuntimeReconnectReason): Promise<void> {
    await this.client.requestReconnect(reason)
  }

  async stop(): Promise<void> {
    await this.client.stopRuntime()
    for (const cleanup of this.eventCleanups.splice(0)) {
      cleanup()
    }
    this.emitter.clear()
  }

  viewport(): StreamRuntimeViewportController {
    return {
      attach: (host) => {
        this.viewportHost = host
        void this.client.attachViewport(host).catch((error) => {
          this.reportError(error)
        })
      },
      detach: () => {
        this.viewportHost = {
          elementId: this.viewportHost.elementId
        }
        void this.client.detachViewport().catch((error) => {
          this.reportError(error)
        })
      },
      applyDisplayState: (state: StreamRuntimeDisplayState) => {
        void this.client.applyDisplayState(state).catch((error) => {
          this.reportError(error)
        })
      },
      bindFrameTracking: (onFrame) =>
        this.emitter.on('stats.videoFrameProcessed', () => {
          onFrame()
        })
    }
  }

  controllerInput(): StreamRuntimeControllerInputController {
    return {
      pressButton: (button, durationMs) => {
        void this.client.pressControllerButton(button, durationMs).catch((error) => {
          this.reportError(error)
        })
      }
    }
  }

  audio() {
    return {
      setVolumeDirect: (value: number) => {
        void this.client.setAudioVolume(value).catch((error) => {
          this.reportError(error)
        })
      },
      startMic: async () => {
        await this.client.startMicrophone()
      },
      stopMic: async () => {
        await this.client.stopMicrophone()
      },
      getMicState: () => this.microphoneState
    }
  }

  stats(): StreamRuntimeStatsController {
    return {
      snapshot: async () => await this.client.snapshotStats()
    }
  }

  events() {
    return this.emitter
  }

  private bindClientEvents(): void {
    this.eventCleanups.push(
      this.client.events().on('runtime.phaseChanged', (payload) => {
        this.emitter.emit('runtime.phaseChanged', payload)
      })
    )
    this.eventCleanups.push(
      this.client.events().on('transport.connectionState', (payload) => {
        this.emitter.emit('transport.connectionState', payload)
      })
    )
    this.eventCleanups.push(
      this.client.events().on('chat.stateChanged', (payload) => {
        this.microphoneState = payload
        this.emitter.emit('chat.stateChanged', payload)
      })
    )
    this.eventCleanups.push(
      this.client.events().on('media.videoReady', (payload) => {
        this.emitter.emit('media.videoReady', payload)
      })
    )
    this.eventCleanups.push(
      this.client.events().on('media.surfaceReady', (payload) => {
        this.emitter.emit('media.surfaceReady', payload)
      })
    )
    this.eventCleanups.push(
      this.client.events().on('stats.videoFrameProcessed', (payload) => {
        this.emitter.emit('stats.videoFrameProcessed', payload)
      })
    )
    this.eventCleanups.push(
      this.client.events().on('error', (payload) => {
        this.reportError(payload.error)
      })
    )
  }

  private reportError(error: unknown): void {
    this.emitter.emit('error', {
      error: error ?? createUnavailableError()
    })
  }
}
