export interface RuntimeFeatureCapabilities {
  nativeMedia: boolean
  nativeGamepad: boolean
}

export const BROWSER_ONLY_FEATURES: RuntimeFeatureCapabilities = { nativeMedia: false, nativeGamepad: false }
