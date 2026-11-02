import { existsSync, readFileSync } from 'node:fs'
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
  return {
    bundle: {
      macOS: {
        frameworks,
        // ad-hoc没有Team ID；证书签名的Frameworks由Tauri统一签名后启用校验。
        hardenedRuntime: hardenedRuntime ?? signingIdentity !== '-',
      },
    },
  }
}
