import { execFileSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import { existsSync, readFileSync, readdirSync, realpathSync } from 'node:fs'
import { basename, dirname, isAbsolute, join, relative, resolve } from 'node:path'

function command(name, args) {
  return execFileSync(name, args, { encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] })
}

function within(root, path) {
  const rel = relative(root, path)
  return rel === '' || (!isAbsolute(rel) && rel !== '..' && !rel.startsWith('../'))
}

function newerThan(value, maximum) {
  const actual = value.split('.').map(Number)
  const limit = maximum.split('.').map(Number)
  for (let i = 0; i < Math.max(actual.length, limit.length); i++) {
    const difference = (actual[i] ?? 0) - (limit[i] ?? 0)
    if (difference !== 0)
      return difference > 0
  }
  return false
}

export function inspectMachO(path) {
  const loads = command('otool', ['-L', path])
  const commands = command('otool', ['-l', path])
  return {
    dependencies: [...loads.matchAll(/^\s+(.+?) \(compatibility version /gm)].map(match => match[1]),
    rpaths: [...commands.matchAll(/cmd LC_RPATH\s+cmdsize \d+\s+path (.+?) \(offset/g)].map(match => match[1]),
    minimumSystemVersion: commands.match(/cmd LC_BUILD_VERSION[\s\S]*?\bminos ([\d.]+)/)?.[1]
      ?? commands.match(/cmd LC_VERSION_MIN_MACOSX\s+cmdsize \d+\s+version ([\d.]+)/)?.[1],
    architectures: command('lipo', ['-archs', path]).trim().split(/\s+/),
  }
}

// 根据实际Mach-O搜索路径逐层检查；系统库由dyld shared cache提供。
export function checkMacosRuntime(input, options = {}) {
  const root = realpathSync(input)
  const app = root.endsWith('.app')
  const minimumSystemVersion = options.minimumSystemVersion
    ?? (app ? command('plutil', ['-extract', 'LSMinimumSystemVersion', 'raw', '-o', '-', join(root, 'Contents/Info.plist')]).trim() : '26.0')
  const entries = app
    ? [join(root, 'Contents/MacOS', command('plutil', ['-extract', 'CFBundleExecutable', 'raw', '-o', '-', join(root, 'Contents/Info.plist')]).trim())]
    : readdirSync(join(root, 'lib')).filter(name => name.endsWith('.dylib')).map(name => join(root, 'lib', name))
  if (entries.length === 0)
    throw new Error('没有可检查的Mach-O运行库')
  const requiredArchitectures = options.architectures ?? inspectMachO(entries[0]).architectures
  const executableDir = app ? dirname(entries[0]) : join(root, 'lib')
  const pending = entries.map(path => ({ path, inheritedRpaths: app ? [] : [join(root, 'lib')] }))
  const checked = new Set()
  const errors = []
  const expand = (value, loaderDir) => value
    .replace(/^@loader_path(?=\/|$)/, loaderDir)
    .replace(/^@executable_path(?=\/|$)/, executableDir)

  while (pending.length > 0) {
    const { path, inheritedRpaths } = pending.pop()
    const binary = realpathSync(path)
    if (!within(root, binary)) {
      errors.push(`${relative(root, path)}: 符号链接指向包外 ${binary}`)
      continue
    }
    if (checked.has(binary))
      continue
    checked.add(binary)
    const facts = inspectMachO(binary)
    const label = relative(root, binary)
    if (!facts.minimumSystemVersion || newerThan(facts.minimumSystemVersion, minimumSystemVersion))
      errors.push(`${label}: minimumSystemVersion=${facts.minimumSystemVersion ?? 'missing'}，应用要求=${minimumSystemVersion}`)
    for (const arch of requiredArchitectures) {
      if (!facts.architectures.includes(arch))
        errors.push(`${label}: 缺少架构 ${arch}`)
    }
    const loaderDir = dirname(binary)
    const rpaths = [...facts.rpaths.map(value => expand(value, loaderDir)), ...inheritedRpaths]
    for (const dependency of facts.dependencies) {
      if (dependency.startsWith('/usr/lib/') || dependency.startsWith('/System/Library/'))
        continue
      if (isAbsolute(dependency)) {
        errors.push(`${label}: 包外或缺失依赖 ${dependency}（绝对install name）`)
        continue
      }
      const candidates = dependency.startsWith('@rpath/')
        ? rpaths.map(rpath => resolve(rpath, dependency.slice('@rpath/'.length)))
        : [expand(dependency, loaderDir)]
      const found = candidates.find(candidate => isAbsolute(candidate) && existsSync(candidate))
      if (!found || !within(root, realpathSync(found))) {
        errors.push(`${label}: 包外或缺失依赖 ${dependency}`)
        continue
      }
      pending.push({ path: found, inheritedRpaths: rpaths })
    }
  }

  if (errors.length > 0)
    throw new Error(`macOS运行依赖检查失败:\n${[...new Set(errors)].map(error => `  - ${error}`).join('\n')}`)
  if (options.verifySignatures !== false) {
    for (const binary of checked)
      command('codesign', ['--verify', '--strict', binary])
    if (app)
      command('codesign', ['--verify', '--deep', '--strict', root])
  }
  // SDK清单把预编译资产纳入构建身份；正式app只消费已验证的同一份库。
  if (!app) {
    const manifestPath = join(root, 'lib/runtime-manifest.json')
    if (!existsSync(manifestPath))
      throw new Error('缺少SDK运行库清单: lib/runtime-manifest.json')
    {
      const manifest = JSON.parse(readFileSync(manifestPath, 'utf8'))
      const names = new Set(manifest.runtimeLibraries.map(file => file.name))
      if (names.size !== entries.length || entries.some(path => !names.has(basename(path))))
        throw new Error('SDK运行库清单与实际文件集合不一致')
      for (const file of manifest.runtimeLibraries) {
        const path = join(root, 'lib', file.name)
        if (basename(file.name) !== file.name || !within(root, realpathSync(path)))
          throw new Error(`运行库清单路径非法: ${file.name}`)
        const sha256 = createHash('sha256').update(readFileSync(path)).digest('hex')
        if (sha256 !== file.sha256)
          throw new Error(`运行库清单摘要不匹配: ${file.name}`)
      }
    }
  }
  return { root, architectures: requiredArchitectures, minimumSystemVersion, binaries: checked.size }
}
