import { readFileSync } from 'node:fs'
import { describe, expect, it } from 'vitest'

describe('realtime decoded-frame CPU pixel-copy contract', () => {
  it('keeps supply, GPU import and reconstruction free of CPU pixel materialization', () => {
    const modules = ['decoded-video-frame-reader.ts', 'decoded-video-frame-worker-source.ts', 'video-frame-source.ts', 'Renderers.ts', 'fsr1-reconstruction-pass.ts', 'gpu-completion-probe.ts']
    for (const module of modules) {
      const source = readFileSync(new URL(module, import.meta.url), 'utf8')
      expect(source, module).not.toMatch(/\.\s*(?:copyTo|getImageData|putImageData|readPixels|getBufferSubData|drawImage)\s*\(/)
      expect(source, module).not.toMatch(/new\s+(?:ImageData|VideoFrame)\s*\(/)
      expect(source, module).not.toMatch(/getContext\s*\(\s*['"]2d['"]/)
    }
  })
})
