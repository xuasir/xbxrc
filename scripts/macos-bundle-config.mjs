import { existsSync, readFileSync, readdirSync } from 'node:fs'
import { basename, join, resolve } from 'node:path'
import { inspectMachO } from './macos-runtime-lib.mjs'

export function createMacosBundleConfig(sdkRoot, { signingIdentity = '-', hardenedRuntime } = {}) {
  const manifest = JSON.parse(readFileSync(join(sdkRoot, 'lib/runtime-manifest.json'), 'utf8'))
  const identities = new Set(manifest.runtimeLibraries.map(file => inspectMachO(join(sdkRoot, 'lib', file.name)).dependencies[0]))
  const frameworks = [...identities].sort().map((identity) => {
    if (!identity?.startsWith('@rpath/'))
      throw new Error(`SDK install name未归一化: ${identity}`)
    const name = identity.slice('@rpath/'.length)
    const path = resolve(sdkRoot, 'lib', name)
    if (basename(name) !== name || !existsSync(path))
      throw new Error(`SDK install name缺少对应文件: ${identity}`)
    return path
  })
  const ffmpegRoot = resolve(sdkRoot, '..')
  const resources = Object.fromEntries([
    ...['LICENSE-NOTICE.md', 'COPYING.LGPLv2.1', 'COPYING.LGPLv3'].map(name => [join(ffmpegRoot, name), `licenses/ffmpeg/${name}`]),
    ...readdirSync(join(sdkRoot, 'lib')).filter(name => name.startsWith('COPYING.')).map(name => [join(sdkRoot, 'lib', name), `licenses/ffmpeg/${name}`]),
    [join(sdkRoot, 'lib/runtime-manifest.json'), 'licenses/ffmpeg/runtime-manifest.json'],
  ])
  return {
    bundle: {
      resources,
      macOS: {
        frameworks,
        // ad-hoc没有Team ID；证书签名的Frameworks由Tauri统一签名后启用校验。
        hardenedRuntime: hardenedRuntime ?? signingIdentity !== '-',
      },
    },
  }
}
