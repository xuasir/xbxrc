import { afterEach, describe, expect, it, vi } from 'vitest'
import { observeRenderSurface } from './render-surface-observation'

afterEach(() => {
  vi.unstubAllGlobals()
})

function element(style: Partial<CSSStyleDeclaration> = {}, parentElement: HTMLElement | null = null) {
  const computed = { display: 'block', visibility: 'visible', opacity: '1', position: 'static', zIndex: 'auto', pointerEvents: 'auto', objectFit: 'contain', transform: 'none', filter: 'none', ...style }
  return {
    computed,
    parentElement,
    isConnected: true,
    getBoundingClientRect: () => ({ x: 10, y: 20, width: 1280, height: 720 }),
  } as unknown as HTMLElement
}

function fixture() {
  const parent = element({ position: 'absolute', zIndex: '0' })
  const video = Object.assign(element({}, parent), { videoWidth: 1280, videoHeight: 720, currentTime: 3.5, readyState: 4, paused: false, compareDocumentPosition: vi.fn(() => 4) }) as unknown as HTMLVideoElement
  const canvas = Object.assign(element({ position: 'absolute', pointerEvents: 'none' }, parent), { width: 1920, height: 1080 }) as unknown as HTMLCanvasElement
  const readStyle = vi.fn((node: HTMLElement) => (node as unknown as { computed: CSSStyleDeclaration }).computed)
  vi.stubGlobal('getComputedStyle', readStyle)
  const gl = { getContextAttributes: vi.fn(() => ({ alpha: false, desynchronized: true, preserveDrawingBuffer: false })), isContextLost: () => false } as unknown as WebGL2RenderingContext
  return { video, canvas, parent, gl, readStyle }
}

describe('renderer DOM evidence', () => {
  it('records owned video/canvas order, geometry and actual context attributes with explicit screen evidence scope', () => {
    const { video, canvas, gl } = fixture()
    expect(observeRenderSurface(video, canvas, gl)).toMatchObject({
      evidence: 'domLayout',
      screenPresentState: 'unavailable',
      sameParent: true,
      canvasFollowsVideo: true,
      video: { connected: true, styleState: 'observed', opacity: 1, paused: false, currentTime: 3.5 },
      canvas: { position: 'absolute', pointerEventsNone: true, backingWidth: 1920, backingHeight: 1080, rect: { x: 10, y: 20, width: 1280, height: 720 } },
      contextAttributes: { alpha: false, desynchronized: true, preserveDrawingBuffer: false },
    })
  })

  it('captures ancestor hiding and bounds layout reads while keeping URLs and node identities out of payloads', () => {
    const { video, canvas, gl, readStyle } = fixture()
    let parent: HTMLElement | null = null
    for (let i = 0; i < 20; i++)
      parent = element({ opacity: '0.4', filter: 'url(https://private.example/session)', transform: 'translateX(2px)' }, parent)
    Object.assign(video, { parentElement: parent })
    const snapshot = observeRenderSurface(video, canvas, gl)
    expect(snapshot.ancestors).toHaveLength(4)
    expect(snapshot.ancestorsTruncated).toBe(true)
    expect(snapshot.ancestors[0]).toMatchObject({ opacity: 0.4, filtered: true, transformed: true })
    expect(readStyle).toHaveBeenCalledTimes(6)
    expect(JSON.stringify(snapshot)).not.toContain('private.example')
  })

  it('keeps unavailable layout and context reads independent from playback', () => {
    const { video, canvas, gl } = fixture()
    vi.stubGlobal('getComputedStyle', () => {
      throw new Error('layoutUnavailable')
    })
    Object.assign(video, { compareDocumentPosition: () => {
      throw new Error('detached')
    } })
    Object.assign(gl, { getContextAttributes: () => {
      throw new Error('contextLost')
    } })
    expect(observeRenderSurface(video, canvas, gl)).toMatchObject({
      video: { styleState: 'unavailable' },
      canvas: { styleState: 'unavailable' },
      canvasFollowsVideo: null,
      contextAttributes: null,
      screenPresentState: 'unavailable',
    })
  })

  it('records native-video-only and disconnected-canvas surfaces without assigning a compositor owner', () => {
    const { video, canvas } = fixture()
    expect(observeRenderSurface(video).canvas).toBeUndefined()
    Object.assign(video, { compareDocumentPosition: () => 5 })
    Object.assign(canvas, { isConnected: false })
    expect(observeRenderSurface(video, canvas)).toMatchObject({ canvas: { connected: false }, canvasFollowsVideo: null, screenPresentState: 'unavailable' })
  })
})
