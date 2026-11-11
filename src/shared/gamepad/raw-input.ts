import type { RawPadState } from './backend'

export const STANDARD_BUTTON_COUNT = 17
export const MAX_RAW_BUTTONS = 128
export const MAX_RAW_AXES = 64
export const MAX_EXTRA_BINDINGS = 128
export const MAX_LAYOUT_KEY_LENGTH = 2048

/** 扩展控件在输入交接时各自等待释放，和标准控件采用相同的持键语义。 */
export class RawButtonReleaseBarrier {
  private held = new Set<number>()
  private heldAxes = new Map<number, number>()
  private axisThresholds = new Map<number, number>()

  capture(raw?: RawPadState, axisRests = new Map<number, number>(), axisThresholds = new Map<number, number>()): void {
    this.held = new Set(raw?.buttons.flatMap((value, index) => value > 0.03 ? [index] : []) ?? [])
    this.axisThresholds = new Map(axisThresholds)
    this.heldAxes = new Map([...axisRests].filter(([index, rest]) => Math.abs((raw?.axes[index] ?? rest) - rest) > (this.axisThresholds.get(index) ?? 0.15)))
  }

  apply(raw?: RawPadState): RawPadState | undefined {
    if (!raw)
      return undefined
    const buttons = [...raw.buttons]
    for (const index of this.held) {
      if (index >= buttons.length || (buttons[index] ?? 0) <= 0.03)
        this.held.delete(index)
      if (index < buttons.length)
        buttons[index] = 0
    }
    const axes = [...raw.axes]
    for (const [index, rest] of this.heldAxes) {
      if (index >= axes.length || Math.abs(axes[index] - rest) <= (this.axisThresholds.get(index) ?? 0.15))
        this.heldAxes.delete(index)
      if (index < axes.length)
        axes[index] = rest
    }
    return { buttons, axes }
  }
}
