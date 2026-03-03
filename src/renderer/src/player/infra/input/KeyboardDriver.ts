import { DEFAULT_INPUT_FRAME, InputFrame } from '../../domain/input'

const KEYCODE_ARROW_LEFT = 'ArrowLeft'
const KEYCODE_ARROW_UP = 'ArrowUp'
const KEYCODE_ARROW_RIGHT = 'ArrowRight'
const KEYCODE_ARROW_DOWN = 'ArrowDown'
const KEYCODE_KEY_A = 'a'
const KEYCODE_ENTER = 'Enter'
const KEYCODE_KEY_B = 'b'
const KEYCODE_BACKSPACE = 'Backspace'
const KEYCODE_KEY_X = 'x'
const KEYCODE_KEY_Y = 'y'
const KEYCODE_KEY_N = 'n'
const KEYCODE_KEY_LEFT_BRACKET = '['
const KEYCODE_KEY_RIGHT_BRACKET = ']'
const KEYCODE_KEY_V = 'v'
const KEYCODE_KEY_M = 'm'
const KEYCODE_MINUS = '-'
const KEYCODE_EQUALS = '='

export type MouseKeyboardMapping = {
  [keyCode: string]: VirtualButton | undefined;
};

type AxisDirection = 'positive' | 'negative'

type AxisBinding =
  | 'LeftThumbXAxisPlus'
  | 'LeftThumbXAxisMinus'
  | 'LeftThumbYAxisPlus'
  | 'LeftThumbYAxisMinus'
  | 'RightThumbXAxisPlus'
  | 'RightThumbXAxisMinus'
  | 'RightThumbYAxisPlus'
  | 'RightThumbYAxisMinus'

export type VirtualButton = keyof InputFrame | AxisBinding

export class MouseKeyboardConfig {
    readonly keymapping: MouseKeyboardMapping

    constructor(args: { keymapping?: MouseKeyboardMapping }) {
        this.keymapping = args.keymapping ?? MouseKeyboardConfig.defaultMapping()
    }

    private static defaultMapping(): MouseKeyboardMapping {
        return {
            [KEYCODE_ARROW_LEFT]: 'DPadLeft',
            [KEYCODE_ARROW_UP]: 'DPadUp',
            [KEYCODE_ARROW_RIGHT]: 'DPadRight',
            [KEYCODE_ARROW_DOWN]: 'DPadDown',
            [KEYCODE_ENTER]: 'A',
            [KEYCODE_KEY_A]: 'A',
            [KEYCODE_BACKSPACE]: 'B',
            [KEYCODE_KEY_B]: 'B',
            [KEYCODE_KEY_X]: 'X',
            [KEYCODE_KEY_Y]: 'Y',
            [KEYCODE_KEY_LEFT_BRACKET]: 'LeftShoulder',
            [KEYCODE_KEY_RIGHT_BRACKET]: 'RightShoulder',
            [KEYCODE_MINUS]: 'LeftTrigger',
            [KEYCODE_EQUALS]: 'RightTrigger',
            [KEYCODE_KEY_V]: 'View',
            [KEYCODE_KEY_M]: 'Menu',
            [KEYCODE_KEY_N]: 'Nexus',
        }
    }

    static default(): MouseKeyboardConfig {
        return new MouseKeyboardConfig({})
    }
}

export class KeyboardDriver {
    private keyboardState: InputFrame = DEFAULT_INPUT_FRAME()
    private enabled = true
    private readonly downFunc = (e: KeyboardEvent) => this.onKeyChange(e, true)
    private readonly upFunc = (e: KeyboardEvent) => this.onKeyChange(e, false)

    constructor(private readonly config: MouseKeyboardConfig) {}

    start(): void {
        window.addEventListener('keydown', this.downFunc)
        window.addEventListener('keyup', this.upFunc)
    }

    stop(): void {
        window.removeEventListener('keydown', this.downFunc)
        window.removeEventListener('keyup', this.upFunc)
    }

    requestState(): InputFrame {
        return { ...this.keyboardState }
    }

    setEnabled(enabled: boolean): void {
        this.enabled = enabled
        if (!enabled) {
            this.keyboardState = DEFAULT_INPUT_FRAME()
        }
    }

    pressButton(button: keyof InputFrame): void {
        this.keyboardState[button] = 1 as never
        window.setTimeout(() => {
            this.keyboardState[button] = 0 as never
        }, 60)
    }

    private onKeyChange(event: KeyboardEvent, down: boolean): void {
        if (!this.enabled) {
            return
        }
        const mappedButton = this.config.keymapping[event.key]
        if (!mappedButton) {
            return
        }
        const axisBinding = this.resolveAxisBinding(mappedButton)
        if (axisBinding) {
            const value = down ? (axisBinding.direction === 'positive' ? 1 : -1) : 0
            ;(this.keyboardState[axisBinding.axis] as any) = value
            return
        }

        const value = down ? 1 : 0;
        (this.keyboardState[mappedButton] as any) = value
    }

    private resolveAxisBinding(binding: VirtualButton): { axis: keyof InputFrame; direction: AxisDirection } | null {
        switch (binding) {
            case 'LeftThumbXAxisPlus':
                return { axis: 'LeftThumbXAxis', direction: 'positive' }
            case 'LeftThumbXAxisMinus':
                return { axis: 'LeftThumbXAxis', direction: 'negative' }
            case 'LeftThumbYAxisPlus':
                return { axis: 'LeftThumbYAxis', direction: 'positive' }
            case 'LeftThumbYAxisMinus':
                return { axis: 'LeftThumbYAxis', direction: 'negative' }
            case 'RightThumbXAxisPlus':
                return { axis: 'RightThumbXAxis', direction: 'positive' }
            case 'RightThumbXAxisMinus':
                return { axis: 'RightThumbXAxis', direction: 'negative' }
            case 'RightThumbYAxisPlus':
                return { axis: 'RightThumbYAxis', direction: 'positive' }
            case 'RightThumbYAxisMinus':
                return { axis: 'RightThumbYAxis', direction: 'negative' }
            default:
                return null
        }
    }
}
