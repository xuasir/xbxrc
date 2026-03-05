import { InputRuntimeConfig } from './input'
import { AudioRuntimeConfig, RendererRuntimeConfig } from './media'
import { TransportRuntimeConfig } from './session'

export interface PlayerClientOptions {
  container: string | HTMLElement
  uiSystem: Array<number>
  uiVersion: Array<number>
  inputDriver?: unknown
  input: InputRuntimeConfig
  audio: AudioRuntimeConfig
  renderer: RendererRuntimeConfig
  transport: TransportRuntimeConfig
}

export const DEFAULT_PLAYER_OPTIONS = (): PlayerClientOptions => ({
  container: '',
  uiSystem: [10, 19, 31, 27, 32, -41],
  uiVersion: [0, 2, 0],
  input: {
    pollingRate: 250,
    vibrationEnabled: true
  },
  audio: {
    volume: 1,
    enableAudioControl: false
  },
  renderer: {
    enabled: false,
    sharpness: 2,
    mode: 'webgl2',
    format: 'Contain'
  },
  transport: {
    maxVideoBitrateKbps: 0,
    maxAudioBitrateKbps: 0,
    forceMonoAudio: false
  }
})
