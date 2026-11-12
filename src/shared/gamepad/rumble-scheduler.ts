import type { RumbleResult } from './backend'
import type { GamepadRumbleEffectDto } from './contract'

/** 官方游戏震动：首次等待 delay，随后按 duration + delay 追加 repeat 次。 */
export class RumbleScheduler {
  private generation = 0
  private delayTimer?: ReturnType<typeof setTimeout>
  private repeatTimer?: ReturnType<typeof setInterval>
  private settleFirst?: (result: RumbleResult) => void

  play(effect: GamepadRumbleEffectDto, apply: (effect: GamepadRumbleEffectDto) => Promise<RumbleResult>): Promise<RumbleResult> {
    this.cancel()
    const generation = this.generation
    const delay = Number.isFinite(effect.startDelayMs) ? Math.max(0, effect.startDelayMs) : 0
    const duration = Number.isFinite(effect.durationMs) ? Math.max(0, effect.durationMs) : 0
    let remaining = Number.isFinite(effect.repeat) ? Math.max(0, Math.min(255, Math.floor(effect.repeat))) : 0
    const playback = { ...effect, startDelayMs: 0, durationMs: duration, repeat: 0 }
    return new Promise((resolve) => {
      this.settleFirst = resolve
      let first = true
      const play = () => {
        const isFirst = first
        first = false
        // 播放完成 Promise 与开始时钟独立，硬件回调延迟保持官方重复节奏。
        try {
          void apply({ ...playback }).then((result) => {
            if (isFirst && generation === this.generation)
              this.finish(result)
          }, () => {
            if (isFirst && generation === this.generation)
              this.finish('failed')
          })
        }
        catch {
          if (isFirst && generation === this.generation)
            this.finish('failed')
        }
      }
      const start = () => {
        if (generation !== this.generation)
          return
        this.delayTimer = undefined
        play()
        if (remaining && generation === this.generation) {
          this.repeatTimer = setInterval(() => {
            if (generation !== this.generation)
              return
            play()
            if (--remaining === 0 && generation === this.generation) {
              clearInterval(this.repeatTimer)
              this.repeatTimer = undefined
            }
          }, duration + delay)
        }
      }
      if (delay)
        this.delayTimer = setTimeout(start, delay)
      else start()
    })
  }

  cancel(): void {
    ++this.generation
    clearTimeout(this.delayTimer)
    clearInterval(this.repeatTimer)
    this.delayTimer = this.repeatTimer = undefined
    this.finish('failed')
  }

  private finish(result: RumbleResult): void {
    this.settleFirst?.(result)
    this.settleFirst = undefined
  }
}
