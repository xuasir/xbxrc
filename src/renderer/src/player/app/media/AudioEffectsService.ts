import { AudioRuntimeConfig } from '../../domain/media'
import { NativeBridge } from '../../infra/bridge/NativeBridge'

function getVolume(analyser: AnalyserNode, dataArray: Uint8Array<ArrayBuffer>): number {
    analyser.getByteTimeDomainData(dataArray)
    let sumSquares = 0
    for (let i = 0; i < dataArray.length; i++) {
        const normalized = (dataArray[i] - 128) / 128
        sumSquares += normalized * normalized
    }
    return Math.sqrt(sumSquares / dataArray.length)
}

export class AudioEffectsService {
    private audioGainNode: GainNode | null = null
    private timer?: number

    constructor(
    private readonly nativeBridge: NativeBridge,
    ) {}

    attach(stream: MediaStream, audioElement: HTMLAudioElement, config: AudioRuntimeConfig): void {
        this.destroy()
        if (!config.enableAudioControl && !config.enableAudioRumble) {
            return
        }
        const audioCtx = new AudioContext()
        const source = audioCtx.createMediaStreamSource(stream)
        if (config.enableAudioControl) {
            this.audioGainNode = audioCtx.createGain()
            source.connect(this.audioGainNode).connect(audioCtx.destination)
            this.audioGainNode.gain.value = config.volume || 1
            audioElement.muted = true
        }
        if (config.enableAudioRumble) {
            const analyser = audioCtx.createAnalyser()
            analyser.fftSize = 512
            source.connect(analyser)
            const data = new Uint8Array(new ArrayBuffer(analyser.fftSize))
            this.timer = window.setInterval(() => this.handleAudioRumble(analyser, data, config.audioRumbleThreshold), 16)
        }
    }

    updateVolume(value: number): void {
        if (this.audioGainNode) {
            this.audioGainNode.gain.value = value
        }
    }

    destroy(): void {
        if (this.timer) {
            window.clearInterval(this.timer)
            this.timer = undefined
        }
        this.audioGainNode = null
    }

    private handleAudioRumble(analyser: AnalyserNode, data: Uint8Array<ArrayBuffer>, threshold: number): void {
        const volume = getVolume(analyser, data)
        if (volume <= threshold) {
            return
        }
        if (this.nativeBridge.isAvailable()) {
            this.nativeBridge.post({
                type: 'audioVibration',
                message: {
                    rumbleData: {
                        startDelay: 0,
                        duration: 100,
                        weakMagnitude: 1.0 * (volume / 0.5),
                        strongMagnitude: 0,
                        leftTrigger: 0,
                        rightTrigger: 0,
                    },
                    repeat: false,
                },
            })
            return
        }
        const gamepads = navigator.getGamepads()
        for (const gp of Array.from(gamepads)) {
            if (gp?.vibrationActuator) {
                gp.vibrationActuator.playEffect('dual-rumble', {
                    startDelay: 0,
                    duration: 100,
                    weakMagnitude: 1.0 * (volume / 0.5),
                    strongMagnitude: 0,
                })
            }
        }
    }
}
