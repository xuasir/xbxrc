import { describe, expect, it } from 'vitest'
import { imageWidth, remoteImageCandidates } from './remote-image-source'

describe('official artwork sizing and fallback', () => {
  it('chooses a bounded width from actual CSS size and DPR', () => {
    expect(imageWidth(166, 1)).toBe(192)
    expect(imageWidth(166, 2)).toBe(384)
    expect(imageWidth(240, 3)).toBe(768)
    expect(imageWidth(1600, 3)).toBe(1920)
  })
  it('retains original and alternate artwork after each resized candidate', () => {
    const tile = 'https://store-images.s-microsoft.com/image/tile?lang=zh'
    const poster = 'https://store-images.s-microsoft.com/image/poster'
    expect(remoteImageCandidates(tile, [poster], 166, 2)).toEqual([`${tile}&w=384&h=384`, tile, `${poster}?w=384&h=384`, poster])
  })
  it('preserves unverified hosts, data sources and existing callers', () => {
    const source = 'https://example.com/cover?w=1000'
    expect(remoteImageCandidates(source, ['data:image/png;base64,a'], 166, 2)).toEqual([source, 'data:image/png;base64,a'])
    const official = 'https://store-images.s-microsoft.com/image/tile'
    expect(remoteImageCandidates(official, [], 0, 2)).toEqual([official])
  })
})
