const IMAGE_WIDTHS = [128, 192, 256, 384, 512, 768, 1024, 1440, 1920]

export function imageWidth(cssSize: number, dpr: number): number {
  const requested = Math.max(1, cssSize) * Math.max(1, Math.min(3, dpr || 1))
  return IMAGE_WIDTHS.find(size => size >= requested) ?? IMAGE_WIDTHS.at(-1)!
}

/** 仅对已实测支持 w/h 参数的官方 CDN 选择尺寸；原地址紧随缩略图用于回退。 */
export function remoteImageCandidates(src: string, sources: string[], cssSize = 0, dpr = 1): string[] {
  const candidates: string[] = []
  for (const source of [src, ...sources].map(value => value.trim()).filter(Boolean)) {
    if (cssSize > 0) {
      try {
        const url = new URL(source.startsWith('//') ? `https:${source}` : source)
        if (url.hostname === 'store-images.s-microsoft.com' && ['http:', 'https:'].includes(url.protocol)) {
          url.searchParams.set('w', String(imageWidth(cssSize, dpr)))
          url.searchParams.set('h', String(imageWidth(cssSize, dpr)))
          candidates.push(url.toString())
        }
      }
      catch {}
    }
    candidates.push(source)
  }
  return [...new Set(candidates)]
}
