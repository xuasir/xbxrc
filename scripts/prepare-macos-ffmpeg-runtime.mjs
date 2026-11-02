#!/usr/bin/env node
import { execFileSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import { chmodSync, copyFileSync, cpSync, existsSync, readFileSync, readdirSync, realpathSync, writeFileSync } from 'node:fs'
import { basename, dirname, join, relative, resolve } from 'node:path'
import { checkMacosRuntime, inspectMachO } from './macos-runtime-lib.mjs'

const root = resolve(import.meta.dirname, '..')
const target = process.argv[2]
const output = process.argv[3] && resolve(process.argv[3])
const dependencyRoot = resolve(process.argv[4] ?? '/opt/homebrew')
const sha256 = path => createHash('sha256').update(readFileSync(path)).digest('hex')
const run = (command, args) => execFileSync(command, args, { encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] })

try {
  if (process.platform !== 'darwin' || !['macos-arm64', 'macos-x64'].includes(target) || !output)
    throw new Error('用法: node scripts/prepare-macos-ffmpeg-runtime.mjs <macos-arm64|macos-x64> <新SDK目录> [传递依赖来源目录]')
  if (existsSync(output))
    throw new Error(`输出目录已存在: ${output}`)
  const source = join(root, 'src-tauri/resources/ffmpeg', target)
  const versions = JSON.parse(readFileSync(join(root, 'src-tauri/resources/ffmpeg/versions.json'), 'utf8'))
  const previousManifestPath = join(source, 'lib/runtime-manifest.json')
  const previous = existsSync(previousManifestPath) ? JSON.parse(readFileSync(previousManifestPath, 'utf8')) : null
  if (previous)
    checkMacosRuntime(source)
  cpSync(source, output, { recursive: true, verbatimSymlinks: true })
  const libDir = join(output, 'lib')
  const pending = readdirSync(libDir).filter(name => name.endsWith('.dylib'))
  const originalHashes = new Map(pending.map(name => [name, previous?.runtimeLibraries.find(file => file.name === name)?.sourceSha256 ?? sha256(join(libDir, name))]))
  const components = new Map(previous?.components.map(component => [component.name, component]) ?? [])
  const imported = new Map()
  const normalized = new Set()

  // 将SDK现有的完整依赖闭包固定在lib目录中，再统一改成可移动的install name。
  while (pending.length > 0) {
    const name = pending.pop()
    const binary = realpathSync(join(libDir, name))
    if (normalized.has(binary))
      continue
    normalized.add(binary)
    // Homebrew运行库默认只读；app打包需要清理属性及写入签名。
    chmodSync(binary, 0o755)
    const identity = run('otool', ['-D', binary]).trim().split('\n').at(-1)
    const dependencies = inspectMachO(binary).dependencies
    for (const dependency of dependencies) {
      if (dependency === identity || dependency.startsWith('/usr/lib/') || dependency.startsWith('/System/Library/'))
        continue
      const dependencyName = basename(dependency)
      const destination = join(libDir, dependencyName)
      if (!existsSync(destination)) {
        const original = realpathSync(dependency)
        const rel = relative(realpathSync(dependencyRoot), original)
        if (rel === '..' || rel.startsWith('../') || !dependencyName.endsWith('.dylib'))
          throw new Error(`传递依赖超出指定来源: ${dependency}`)
        copyFileSync(original, destination)
        originalHashes.set(dependencyName, sha256(original))
        imported.set(dependencyName, original)
        pending.push(dependencyName)
        const packageRoot = dirname(dirname(original))
        const version = basename(packageRoot)
        const packageName = basename(dirname(packageRoot))
        if (!components.has(packageName)) {
          const licenseName = `COPYING.${packageName}`
          copyFileSync(join(packageRoot, 'COPYING'), join(libDir, licenseName))
          const formula = readFileSync(join(packageRoot, '.brew', `${packageName}.rb`), 'utf8')
          components.set(packageName, {
            name: packageName, version, licenseFile: licenseName,
            source: formula.match(/^\s*url "([^"]+)"/m)?.[1],
            sourceSha256: formula.match(/^\s*sha256 "([^"]+)"/m)?.[1],
          })
        }
      }
      else if (imported.has(dependencyName) && !dependency.startsWith('@')) {
        if (sha256(realpathSync(dependency)) !== originalHashes.get(dependencyName))
          throw new Error(`同名传递依赖内容冲突: ${dependencyName}`)
      }
      run('install_name_tool', ['-change', dependency, `@rpath/${dependencyName}`, binary])
    }
    run('install_name_tool', ['-id', `@rpath/${basename(identity)}`, binary])
    run('codesign', ['--force', '--sign', '-', binary])
  }
  const manifest = {
    target, version: versions[target].version, source: versions[target].source,
    sourceSha256: versions[target].sha256, minimumSystemVersion: '26.0',
    license: 'LGPL-3.0-or-later',
    components: [...components.values()].sort((a, b) => a.name.localeCompare(b.name)),
    runtimeLibraries: [...originalHashes].sort(([a], [b]) => a.localeCompare(b)).map(([name, sourceSha256]) => ({ name, sourceSha256, sha256: sha256(join(libDir, name)) })),
  }
  writeFileSync(join(libDir, 'runtime-manifest.json'), `${JSON.stringify(manifest, null, 2)}\n`)
  console.log(checkMacosRuntime(output, { architectures: [target === 'macos-arm64' ? 'arm64' : 'x86_64'] }))
  console.log(`SDK ready: ${output}; imported libraries: ${imported.size}`)
}
catch (error) {
  console.error(error instanceof Error ? error.message : error)
  process.exitCode = 1
}
