import { describe, expect, it } from 'vitest'
import { resolveDesiredGamepadRouteTarget } from './useGamepadRouteForStreamOverlay'

describe('resolveDesiredGamepadRouteTarget', () => {
  it('returns stream-session when session is present and no overlay is open', () => {
    expect(resolveDesiredGamepadRouteTarget({
      sessionId: 'session-1',
      overlayOpen: false,
      streamSessionPresent: true,
    })).toEqual({ kind: 'stream-session', sessionId: 'session-1' })
  })

  it('returns shell-ui when overlay is open during an active session', () => {
    expect(resolveDesiredGamepadRouteTarget({
      sessionId: 'session-1',
      overlayOpen: true,
      streamSessionPresent: true,
    })).toEqual({ kind: 'shell-ui' })
  })

  it('keeps shell-ui when session id is briefly empty but stream session is still present', () => {
    expect(resolveDesiredGamepadRouteTarget({
      sessionId: '',
      overlayOpen: true,
      streamSessionPresent: true,
    })).toEqual({ kind: 'shell-ui' })
  })

  it('keeps stream-session when session id is briefly empty but stream session is still present and overlay is closed', () => {
    expect(resolveDesiredGamepadRouteTarget({
      sessionId: '',
      overlayOpen: false,
      streamSessionPresent: true,
    })).toEqual({ kind: 'stream-session', sessionId: '' })
  })

  it('returns null once session id is empty and stream session is no longer present', () => {
    expect(resolveDesiredGamepadRouteTarget({
      sessionId: '',
      overlayOpen: false,
      streamSessionPresent: false,
    })).toBeNull()
  })
})
