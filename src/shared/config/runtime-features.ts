export interface RuntimeFeatureCapabilities {
  nativeMedia: boolean
  nativeGamepad: boolean
}

export const BROWSER_ONLY_FEATURES: RuntimeFeatureCapabilities = { nativeMedia: false, nativeGamepad: false }

export function effectiveGamepadBackend(config: Record<string, unknown>, capabilities: RuntimeFeatureCapabilities): 'browser' | 'native' {
  return config.native_gamepad_preview === true && capabilities.nativeGamepad
    ? 'native'
    : 'browser'
}

export function effectiveMediaRuntime(config: Record<string, unknown>, capabilities: RuntimeFeatureCapabilities): 'webrtc-direct' | 'rust-owned' {
  return config.rust_media_preview === true && capabilities.nativeMedia
    ? 'rust-owned'
    : 'webrtc-direct'
}
