import assert from 'node:assert/strict'
import { execFileSync } from 'node:child_process'
import { copyFileSync, mkdtempSync, mkdirSync, rmSync, symlinkSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import { afterEach, test } from 'node:test'
import { checkMacosRuntime } from '../../scripts/macos-runtime-lib.mjs'
import { createMacosBundleConfig } from '../../scripts/macos-bundle-config.mjs'

const temporary = []
afterEach(() => {
  for (const path of temporary.splice(0))
    rmSync(path, { recursive: true, force: true })
})

function fixture({ external = false } = {}) {
  const root = mkdtempSync(join(tmpdir(), 'xbxrc-runtime-gate-'))
  temporary.push(root)
  const app = join(root, 'Fixture.app')
  const libraries = join(app, 'Contents/Frameworks')
  const executables = join(app, 'Contents/MacOS')
  mkdirSync(libraries, { recursive: true })
  mkdirSync(executables, { recursive: true })
  const librarySource = join(root, 'library.c')
  const executableSource = join(root, 'main.c')
  const library = join(libraries, 'libfixture.dylib')
  const executable = join(executables, 'fixture')
  const identity = external ? join(root, 'external.dylib') : '@rpath/libfixture.dylib'
  writeFileSync(librarySource, 'int value(void) { return 42; }\n')
  writeFileSync(executableSource, 'int value(void); int main(void) { return value(); }\n')
  writeFileSync(join(app, 'Contents/Info.plist'), `<?xml version="1.0" encoding="UTF-8"?><plist version="1.0"><dict><key>CFBundleExecutable</key><string>fixture</string><key>CFBundleIdentifier</key><string>test.xbxrc.runtime</string><key>LSMinimumSystemVersion</key><string>26.0</string></dict></plist>`)
  const run = (command, args) => execFileSync(command, args, { stdio: ['ignore', 'pipe', 'pipe'] })
  const compileLibrary = (version = '26.0') => run('clang', ['-dynamiclib', `-mmacosx-version-min=${version}`, '-Wl,-headerpad_max_install_names', '-install_name', identity, librarySource, '-o', library])
  compileLibrary()
  run('clang', ['-mmacosx-version-min=26.0', executableSource, library, '-Wl,-rpath,@executable_path/../Frameworks', '-o', executable])
  run('codesign', ['--force', '--deep', '--sign', '-', app])
  return { root, app, library, compileLibrary }
}

const macos = { skip: process.platform !== 'darwin' }
test('accepts a signed app with a complete relative dependency closure', macos, () => {
  const f = fixture()
  const facts = checkMacosRuntime(f.app)
  assert.equal(facts.binaries, 2)
  assert.equal(facts.minimumSystemVersion, '26.0')
})

test('rejects developer-machine absolute install names even when the file exists', macos, () => {
  const f = fixture({ external: true })
  symlinkSync(f.library, join(f.root, 'external.dylib'))
  assert.throws(() => checkMacosRuntime(f.app), /包外或缺失依赖/)
})

test('rejects an internal library requiring a newer OS than the app declares', macos, () => {
  const f = fixture()
  f.compileLibrary('27.0')
  assert.throws(() => checkMacosRuntime(f.app), /minimumSystemVersion=27.0/)
})

test('rejects an rpath target escaping through a symlink', macos, () => {
  const f = fixture()
  const external = join(f.root, 'external.dylib')
  copyFileSync(f.library, external)
  rmSync(f.library)
  symlinkSync(external, f.library)
  assert.throws(() => checkMacosRuntime(f.app), /包外或缺失依赖/)
})

test('rejects a dependency without the app architecture', macos, () => {
  const f = fixture()
  const required = process.arch === 'arm64' ? 'x86_64' : 'arm64'
  assert.throws(() => checkMacosRuntime(f.app, { architectures: [required] }), /缺少架构/)
})

test('rejects an app with a missing executable signature', macos, () => {
  const f = fixture()
  execFileSync('codesign', ['--remove-signature', join(f.app, 'Contents/MacOS/fixture')])
  assert.throws(() => checkMacosRuntime(f.app), /signed|signature/)
})

test('rejects SDK assets that differ from the recorded binary hash', macos, () => {
  const f = fixture()
  const sdk = join(f.root, 'sdk')
  mkdirSync(join(sdk, 'lib'), { recursive: true })
  copyFileSync(f.library, join(sdk, 'lib/libfixture.dylib'))
  writeFileSync(join(sdk, 'lib/runtime-manifest.json'), JSON.stringify({ runtimeLibraries: [{ name: 'libfixture.dylib', sha256: '0'.repeat(64) }] }))
  assert.throws(() => checkMacosRuntime(sdk), /清单摘要不匹配/)
})

test('loads every FFmpeg library with the configured ad-hoc signing policy', macos, () => {
  const f = fixture()
  const target = process.arch === 'arm64' ? 'macos-arm64' : 'macos-x64'
  const sdk = resolve(import.meta.dirname, '../../src-tauri/resources/ffmpeg', target)
  const config = createMacosBundleConfig(sdk)
  const certificateConfig = createMacosBundleConfig(sdk, { signingIdentity: 'Developer ID Application: Test' })
  assert.equal(config.bundle.macOS.hardenedRuntime, false)
  assert.equal(certificateConfig.bundle.macOS.hardenedRuntime, true)
  assert.equal(config.bundle.macOS.frameworks.length, target === 'macos-arm64' ? 14 : 7)
  const libraries = join(f.app, 'Contents/Frameworks')
  for (const path of config.bundle.macOS.frameworks)
    copyFileSync(path, join(libraries, path.split('/').at(-1)))
  const source = join(f.root, 'ffmpeg-probe.c')
  writeFileSync(source, `
unsigned avcodec_version(void); unsigned avdevice_version(void); unsigned avfilter_version(void);
unsigned avformat_version(void); unsigned avutil_version(void); unsigned swresample_version(void); unsigned swscale_version(void);
int main(void) {
  return !((avcodec_version() >> 16) == 62 && (avdevice_version() >> 16) == 62 &&
    (avfilter_version() >> 16) == 11 && (avformat_version() >> 16) == 62 &&
    (avutil_version() >> 16) == 60 && (swresample_version() >> 16) == 6 && (swscale_version() >> 16) == 9);
}
`)
  const executable = join(f.app, 'Contents/MacOS/fixture')
  const run = (command, args, options = {}) => execFileSync(command, args, { stdio: ['ignore', 'pipe', 'pipe'], ...options })
  run('clang', ['-mmacosx-version-min=26.0', source, `-L${join(sdk, 'lib')}`, '-lavcodec', '-lavdevice', '-lavfilter', '-lavformat', '-lavutil', '-lswresample', '-lswscale', '-Wl,-rpath,@executable_path/../Frameworks', '-o', executable])
  const signatureArgs = config.bundle.macOS.hardenedRuntime ? ['--options', 'runtime'] : []
  run('codesign', ['--force', ...signatureArgs, '--sign', '-', executable])
  run('codesign', ['--force', ...signatureArgs, '--sign', '-', f.app])
  // 只查询版本；与实际Tauri/串流生命周期测试分别取证。
  run(executable, [], { env: { ...process.env, DYLD_LIBRARY_PATH: '', DYLD_FALLBACK_LIBRARY_PATH: '' } })
  assert.equal(checkMacosRuntime(f.app).binaries, 15)
})
