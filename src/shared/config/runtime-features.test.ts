import { describe, expect, it } from 'vitest'
import { effectiveGamepadBackend, effectiveMediaRuntime } from './runtime-features'

describe('stable runtime feature selection', () => {
  it('requires explicit independent preview consent and supported capabilities', () => {
    for (const media of [false, true]) {
      for (const gamepad of [false, true]) {
        for (const nativeMedia of [false, true]) {
          for (const nativeGamepad of [false, true]) {
            const config = { rust_media_preview: media, native_gamepad_preview: gamepad }
            const caps = { nativeMedia, nativeGamepad }
            expect(effectiveMediaRuntime(config, caps)).toBe(media && nativeMedia ? 'rust-owned' : 'webrtc-direct')
            expect(effectiveGamepadBackend(config, caps)).toBe(gamepad && nativeGamepad ? 'native' : 'browser')
          }
        }
      }
    }
  })
  it('defaults to browser when preview switches are absent', () => {
    const caps = { nativeMedia: true, nativeGamepad: true }
    expect(effectiveMediaRuntime({}, caps)).toBe('webrtc-direct')
    expect(effectiveGamepadBackend({}, caps)).toBe('browser')
  })
})
