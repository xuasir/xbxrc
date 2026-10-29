import { describe, expect, it } from 'vitest'

describe('browser display geometry', () => {
  it('resolves fullscreen 16:9 viewport and backing size', async () => {
    const mod = await import('./display-viewport').catch(() => null) as null | {
      resolveDisplayViewport?: (input: {
        containerWidthCss: number
        containerHeightCss: number
        devicePixelRatio: number
        maxOutputDevicePixelRatio?: number
        maxOutputPixels?: number
        format: string
        fullscreen: boolean
        sourceWidth: number
        sourceHeight: number
      }) => {
        viewportWidthCss: number
        viewportHeightCss: number
        outputWidth: number
        outputHeight: number
      }
    }

    expect(mod?.resolveDisplayViewport).toBeTypeOf('function')
    const viewport = mod?.resolveDisplayViewport?.({
      containerWidthCss: 1920,
      containerHeightCss: 1200,
      devicePixelRatio: 1.5,
      format: 'Contain',
      fullscreen: true,
      sourceWidth: 1920,
      sourceHeight: 1080,
    })

    expect(viewport).toMatchObject({
      viewportWidthCss: 1920,
      viewportHeightCss: 1080,
      outputWidth: 2880,
      outputHeight: 1620,
    })
  })

  it('caps backing size when max output constraints are provided', async () => {
    const mod = await import('./display-viewport').catch(() => null) as null | {
      resolveDisplayViewport?: (input: {
        containerWidthCss: number
        containerHeightCss: number
        devicePixelRatio: number
        maxOutputDevicePixelRatio?: number
        maxOutputPixels?: number
        format: string
        fullscreen: boolean
        sourceWidth: number
        sourceHeight: number
      }) => {
        viewportWidthCss: number
        viewportHeightCss: number
        outputWidth: number
        outputHeight: number
      }
    }

    const viewport = mod?.resolveDisplayViewport?.({
      containerWidthCss: 1920,
      containerHeightCss: 1200,
      devicePixelRatio: 2,
      maxOutputDevicePixelRatio: 1.5,
      maxOutputPixels: 2560 * 1440,
      format: 'Contain',
      fullscreen: true,
      sourceWidth: 1920,
      sourceHeight: 1080,
    })

    expect(viewport).toMatchObject({
      viewportWidthCss: 1920,
      viewportHeightCss: 1080,
      outputWidth: 2560,
      outputHeight: 1440,
    })
  })
})
