import { describe, expect, it } from 'vitest'
import { buildEasuFragmentSource } from './fsr1-reconstruction-pass'

describe('eASU sample budget', () => {
  it.each([true, false])('retains twelve spatial taps and four RGB groups in compatibility mode %s', (compatibility) => {
    const shader = buildEasuFragmentSource(compatibility)
    const main = shader.split('void main(){')[1]!
    expect(main.match(/FsrEasuTapF\(/g)).toHaveLength(12)
    expect(main.match(/FsrEasuGatherRGB\(/g)).toHaveLength(4)
    expect(shader.match(/texelFetch\(/g) ?? []).toHaveLength(compatibility ? 4 : 0)
    expect(shader.match(/textureGather\(/g) ?? []).toHaveLength(compatibility ? 0 : 3)
  })
})
