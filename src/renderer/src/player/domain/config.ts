import { InputRuntimeConfig } from './input'
import { AudioRuntimeConfig, RendererRuntimeConfig } from './media'
import { TransportRuntimeConfig } from './session'
import { MouseKeyboardConfig } from '../infra/input/KeyboardDriver'

export interface PlayerClientOptions {
  container: string | HTMLElement;
  uiSystem: Array<number>;
  uiVersion: Array<number>;
  inputDriver?: any;
  input: InputRuntimeConfig;
  audio: AudioRuntimeConfig;
  renderer: RendererRuntimeConfig;
  transport: TransportRuntimeConfig;
  mouseKeyboardConfig: MouseKeyboardConfig;
}

export const DEFAULT_PLAYER_OPTIONS = (): PlayerClientOptions => ({
    container: '',
    uiSystem: [10, 19, 31, 27, 32, -41],
    uiVersion: [0, 2, 0],
    input: {
        pollingRate: 250,
        mouseSensitivity: 0.5,
        legacyKeyboard: true,
        mouseKeyboard: false,
        touch: false,
        vibrationEnabled: true,
        vibrationMode: 'Native',
        gamepadKernel: 'Native',
        gamepadIndex: -1,
        gamepadMix: false,
        gamepadDeadZone: 0.2,
        edgeCompensation: 0,
        customGamepadMapping: null,
        forceTriggerRumble: '',
    },
    audio: {
        volume: 1,
        enableAudioControl: false,
        enableAudioRumble: false,
        audioRumbleThreshold: 0.15,
    },
    renderer: {
        enabled: false,
        sharpness: 2,
        mode: 'webgl2',
        format: 'Contain',
    },
    transport: {
        maxVideoBitrateKbps: 0,
        maxAudioBitrateKbps: 0,
        forceMonoAudio: false,
    },
    mouseKeyboardConfig: MouseKeyboardConfig.default(),
})
