import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { chmod, mkdtemp, readdir, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { dirname, join, resolve } from 'node:path'
import { test } from 'node:test'
import { captureProfileDirectory, parseCaptureArgs, validateBrowserPath } from '../../scripts/official-xbox-capture-options.mjs'

const captureScript = resolve(import.meta.dirname, '../../scripts/official-xbox-capture.mjs')

test('帮助和无效启动参数在创建输出与连接浏览器之前返回', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'xbxrc-capture-cli-'))
  try {
    const run = args => spawnSync(process.execPath, [captureScript, ...args], { cwd: directory, encoding: 'utf8', timeout: 10_000 })
    const help = run(['--help'])
    assert.equal(help.status, 0, help.stderr)
    assert.match(help.stdout, /--browser-path/)
    const missing = run([])
    assert.equal(missing.status, 1)
    assert.match(missing.stderr, /需要 --browser-path/)
    const invalid = run(['--browser-path', join(directory, 'missing-browser')])
    assert.equal(invalid.status, 1)
    assert.match(invalid.stderr, /文件必须存在且可执行/)
    assert.deepEqual(await readdir(directory), [])
  }
  finally { await rm(directory, { recursive: true, force: true }) }
})

test('显式浏览器文件通过验证，目录与相对路径被拒绝', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'xbxrc-capture-browser-'))
  try {
    const browserPath = join(directory, 'browser')
    await writeFile(browserPath, 'fixture executable', { mode: 0o755 })
    const options = parseCaptureArgs(['--browser-path', browserPath, '--port', '9230'])
    await validateBrowserPath(options)
    assert.equal(options.browserPath, browserPath)
    assert.equal(options.port, 9230)
    await assert.rejects(validateBrowserPath({ browserPath: directory }), /普通文件/)
    await assert.rejects(validateBrowserPath({ browserPath: 'relative-browser' }), /绝对路径/)
    if (process.platform !== 'win32') {
      await chmod(browserPath, 0o644)
      await assert.rejects(validateBrowserPath(options), /可执行/)
    }
  }
  finally { await rm(directory, { recursive: true, force: true }) }
})

test('no-launch 允许省略浏览器路径，参数值保持显式校验', async () => {
  const options = parseCaptureArgs(['--no-launch'])
  await validateBrowserPath(options)
  assert.equal(options.noLaunch, true)
  assert.equal(await captureProfileDirectory(options), null)
  for (const values of [
    ['--browser-path'],
    ['--no-launch', '--port', '0'],
    ['--no-launch', '--port', '65536'],
    ['--no-launch', '--sample-interval-ms', '0'],
    ['--no-launch', '--url', 'file:///tmp/page'],
    ['--no-launch', '--unknown'],
  ]) assert.throws(() => parseCaptureArgs(values))
})

test('默认 profile 使用独立系统临时目录，显式 profile 可以复用', async () => {
  const first = await captureProfileDirectory({})
  const second = await captureProfileDirectory({})
  try {
    assert.notEqual(first, second)
    assert.equal(dirname(first), resolve(tmpdir()))
    assert.equal(dirname(second), resolve(tmpdir()))
    assert.equal(await captureProfileDirectory({ profile: first }), first)
    assert.equal(await captureProfileDirectory({ noLaunch: true, profile: first }), first)
  }
  finally {
    await rm(first, { recursive: true, force: true })
    await rm(second, { recursive: true, force: true })
  }
})
