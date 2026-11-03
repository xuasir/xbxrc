import { describe, expect, it } from 'vitest'
import { getFocusScrollDelta } from './focus-scroll'

describe('focus scroll safe area', () => {
  const container = { top: 0, bottom: 500, left: 0, right: 600 }
  const padding = { top: 80, bottom: 32, left: 16, right: 16 }
  it('moves a focused row below the sticky header', () => {
    expect(getFocusScrollDelta({ top: 40, bottom: 100, left: 20, right: 400 }, container, padding)).toEqual({ top: -40, left: 0 })
  })
  it('preserves bottom and horizontal focus effects', () => {
    expect(getFocusScrollDelta({ top: 430, bottom: 490, left: 400, right: 598 }, container, padding)).toEqual({ top: 22, left: 14 })
  })
  it('keeps visible rows stable and supports horizontal category navigation', () => {
    expect(getFocusScrollDelta({ top: 90, bottom: 150, left: 20, right: 400 }, container, padding)).toEqual({ top: 0, left: 0 })
    expect(getFocusScrollDelta({ top: 90, bottom: 150, left: -30, right: 100 }, container, padding)).toEqual({ top: 0, left: -46 })
  })
})
