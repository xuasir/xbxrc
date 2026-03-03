export interface InputFrame {
  GamepadIndex: number;
  Nexus: number;
  Menu: number;
  View: number;
  A: number;
  B: number;
  X: number;
  Y: number;
  DPadUp: number;
  DPadDown: number;
  DPadLeft: number;
  DPadRight: number;
  LeftShoulder: number;
  RightShoulder: number;
  LeftThumb: number;
  RightThumb: number;
  LeftThumbXAxis: number;
  LeftThumbYAxis: number;
  RightThumbXAxis: number;
  RightThumbYAxis: number;
  LeftTrigger: number;
  RightTrigger: number;
}

export interface PointerFrame {
  events: Array<any>;
}

export interface MouseFrame {
  X: number;
  Y: number;
  WheelX: number;
  WheelY: number;
  Buttons: number;
  Relative: number;
}

export interface KeyboardFrame {
  pressed: boolean;
  keyCode: number;
  key: string;
}

export interface ProcessedVideoFrameMetadata {
  serverDataKey: number;
  firstFramePacketArrivalTimeMs: number;
  frameSubmittedTimeMs: number;
  frameDecodedTimeMs: number;
  frameRenderedTimeMs: number;
}

export interface InputRuntimeConfig {
  pollingRate: number;
  mouseSensitivity: number;
  legacyKeyboard: boolean;
  mouseKeyboard: boolean;
  touch: boolean;
  vibrationEnabled: boolean;
  vibrationMode: 'Native' | 'Device' | 'Webview';
  gamepadKernel: string;
  gamepadIndex: number;
  gamepadMix: boolean;
  gamepadDeadZone: number;
  edgeCompensation: number;
  customGamepadMapping: Record<string, string> | null;
  forceTriggerRumble: string;
}

export const DEFAULT_INPUT_FRAME = (): InputFrame => ({
    GamepadIndex: 0,
    Nexus: 0,
    Menu: 0,
    View: 0,
    A: 0,
    B: 0,
    X: 0,
    Y: 0,
    DPadUp: 0,
    DPadDown: 0,
    DPadLeft: 0,
    DPadRight: 0,
    LeftShoulder: 0,
    RightShoulder: 0,
    LeftThumb: 0,
    RightThumb: 0,
    LeftThumbXAxis: 0,
    LeftThumbYAxis: 0,
    RightThumbXAxis: 0,
    RightThumbYAxis: 0,
    LeftTrigger: 0,
    RightTrigger: 0,
})
