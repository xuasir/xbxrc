import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
import fs from 'node:fs'
import path from 'node:path'
import vm from 'node:vm'
import ts from 'typescript'

// 使用下载的微软公开 bundle，回放实际方法并保存源码摘录。
const assets = process.argv[2]
assert.ok(assets, '传入官方脚本目录')
const output = import.meta.dirname
const repository = path.resolve(output, '../../..')
const printer = ts.createPrinter({ newLine: ts.NewLineKind.LineFeed })
const sources = new Map()
for (const filename of ['8128.1aca8038.chunk.js', 'web-rtc-stream.d6f6b903.chunk.js']) {
  const text = fs.readFileSync(path.join(assets, filename), 'utf8')
  sources.set(filename, {
    text,
    file: ts.createSourceFile(filename, text, ts.ScriptTarget.Latest, true, ts.ScriptKind.JS),
    url: `https://assets.play.xbox.com/playxbox/static/js/${filename}`,
    sha256: createHash('sha256').update(text).digest('hex'),
  })
}
const model = sources.get('8128.1aca8038.chunk.js')
const rtc = sources.get('web-rtc-stream.d6f6b903.chunk.js')
function find(source, predicate) {
  let found
  function visit(node) {
    if (!found && predicate(node)) found = node
    if (!found) ts.forEachChild(node, visit)
  }
  visit(source.file)
  assert.ok(found, '官方源码包含目标节点')
  return found
}
const method = name => find(rtc, node => ts.isMethodDeclaration(node) && node.name.getText(rtc.file) === name)
function enumCall(source, marker) {
  return find(source, node => ts.isCallExpression(node) && ts.isFunctionExpression(node.expression) && node.getText(source.file).includes(marker))
}
const shareEnumCall = enumCall(model, 'e.Share=11')
const maskEnumCall = enumCall(rtc, 'e.Nexus=2')
const buttonIds = vm.runInNewContext(`var y; (${shareEnumCall.getText(model.file)}); y`)
const reliableMask = vm.runInNewContext(`var z; (${maskEnumCall.getText(rtc.file)}); z`)
const vibrationClass = find(rtc, node => ts.isClassDeclaration(node) && node.members.some(member => ts.isMethodDeclaration(member) && member.name.getText(rtc.file) === 'onVibration'))
const vibrationDefaults = find(rtc, node => ts.isVariableDeclaration(node) && node.name.getText(rtc.file) === 'C' && node.initializer?.getText(rtc.file).includes('supportsDualRumble')).initializer

function fakeClock() {
  let now = 0
  let nextId = 0
  const timers = new Map()
  const schedule = (callback, delay, interval) => {
    const id = ++nextId
    timers.set(id, { callback, due: now + delay, interval })
    return id
  }
  return {
    now: () => now,
    setTimeout: (callback, delay) => schedule(callback, delay, 0),
    setInterval: (callback, delay) => schedule(callback, delay, delay),
    clearTimeout: id => timers.delete(id),
    clearInterval: id => timers.delete(id),
    advance(until) {
      let budget = 1000
      while (budget-- > 0) {
        const next = [...timers].sort((a, b) => a[1].due - b[1].due)[0]
        if (!next || next[1].due > until) break
        const [id, timer] = next
        now = timer.due
        if (timer.interval) timer.due += timer.interval
        else timers.delete(id)
        timer.callback()
      }
      assert.ok(budget > 0, '计时器回放保持有界')
      now = until
    },
  }
}
function pad(clock, events) {
  return {
    index: 0, id: 'Xbox', connected: true, mapping: 'standard',
    buttons: Array.from({ length: 18 }, () => ({ value: 0, pressed: false })),
    axes: [0, 0, 0, 0],
    vibrationActuator: {
      effects: ['dual-rumble'],
      playEffect: async (type, parameters) => {
        events.push({ atMs: clock.now(), type, ...parameters })
        return 'complete'
      },
      reset: async () => 'complete',
    },
  }
}
const runtime = { inputController: {} }
const moduleCache = new Map()
function loadSource(filename) {
  filename = path.resolve(filename)
  if (moduleCache.has(filename)) return moduleCache.get(filename)
  const exports = {}
  moduleCache.set(filename, exports)
  const compiled = ts.transpileModule(fs.readFileSync(filename, 'utf8'), {
    compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS },
  }).outputText
  vm.runInNewContext(compiled, {
    exports, performance,
    require(id) {
      if (id === '@shared/gamepad/runtime') return runtime
      assert.ok(id.startsWith('.'), `定点源码依赖：${id}`)
      return loadSource(path.resolve(path.dirname(filename), `${id}.ts`))
    },
  }, { filename })
  return exports
}
const { BrowserGamepadBackend } = loadSource(path.join(repository, 'src/shared/gamepad/browser-backend.ts'))
const { RumbleService } = loadSource(path.join(repository, 'src/player/app/input/RumbleService.ts'))
const scenarios = [
  { name: 'delay-and-repeat', magnitude: 50, durationMs: 200, delayMs: 100, repeat: 2 },
  { name: 'long-duration', magnitude: 50, durationMs: 5000, delayMs: 0, repeat: 0 },
  { name: 'short-weak-pulse', magnitude: 1, durationMs: 4, delayMs: 0, repeat: 0 },
]
const timing = []
for (const scenario of scenarios) {
  const officialClock = fakeClock()
  const officialEvents = []
  const officialPad = pad(officialClock, officialEvents)
  const OfficialVibration = vm.runInNewContext(`const C=${vibrationDefaults.getText(rtc.file)}; ${vibrationClass.getText(rtc.file)}; ${vibrationClass.name.text}`, {
    ...officialClock,
    window: { addEventListener() {}, removeEventListener() {} },
    navigator: { getGamepads: () => [officialPad] },
    s: () => (object, key, value) => { object[key] = value },
    a: () => Object.assign,
  })
  const official = new OfficialVibration()
  official.setEnableVibration(true)
  official.onGamepadConnected(0)
  official.onVibration(0, {
    ...scenario, leftMotorPercent: scenario.magnitude / 100,
    rightMotorPercent: scenario.magnitude / 100,
    leftTriggerMotorPercent: 0, rightTriggerMotorPercent: 0,
  })
  officialClock.advance(10_000)

  const oursClock = fakeClock()
  const oursEvents = []
  const oursPad = pad(oursClock, oursEvents)
  let deviceId
  const backend = new BrowserGamepadBackend({
    getGamepads: () => [oursPad], requestFrame: () => 1, cancelFrame() {}, now: oursClock.now,
  })
  await backend.start(snapshot => { deviceId = snapshot.devices[0].id })
  runtime.inputController.getSnapshot = () => ({ owner: 'stream' })
  runtime.inputController.rumble = effect => backend.rumble(deviceId, effect)
  runtime.inputController.stopRumble = () => backend.stopRumble(deviceId)
  const packet = new Uint8Array(13)
  const view = new DataView(packet.buffer)
  view.setUint16(0, 128, true)
  packet[4] = packet[5] = scenario.magnitude
  view.setUint16(8, scenario.durationMs, true)
  view.setUint16(10, scenario.delayMs, true)
  packet[12] = scenario.repeat
  new RumbleService().onMessage(view)
  oursClock.advance(10_000)
  timing.push({ scenario, official: officialEvents, xbxrc: oursEvents })
  await backend.stop()
}
assert.deepEqual(timing[0].official.map(event => event.atMs), [100, 400, 700])
assert.deepEqual(timing[0].xbxrc.map(event => event.atMs), [0])
assert.equal(timing[1].official[0].duration, 5000)
assert.equal(timing[1].xbxrc[0].duration, 1000)
assert.equal(timing[2].official.length, 1)
assert.equal(timing[2].xbxrc.length, 0)

const payload = new ArrayBuffer(37)
const reporter = {
  reportArrayView: new DataView(payload), payloadSizePerUnreliableGamepadFrame: 37,
  appendUint8(view, offset, value) { view.setUint8(offset, value); return offset + 1 },
  appendUint16(view, offset, value) { view.setUint16(offset, value, true); return offset + 2 },
  appendInt16(view, offset, value) { view.setInt16(offset, value, true); return offset + 2 },
  appendUint32(view, offset, value) { view.setUint32(offset, value, true); return offset + 4 },
  normalizeAxisValue: value => value, normalizeTriggerValue: value => value,
}
const counters = new Uint8Array(buttonIds.EnumLength)
counters[buttonIds.Share] = 1
const serialize = vm.runInNewContext(`({${method('generateUnreliableGamepadFrame').getText(rtc.file)}}).generateUnreliableGamepadFrame`, {
  c: { t: buttonIds }, u: { r: { Instance: { throw(message) { throw new Error(message) } } } }, b: { ws: { BadLength: 'length' } },
})
serialize.call(reporter, {
  gamepadId: 0, transitionCount: counters,
  triggerLeft: 0, triggerRight: 0, thumbLX: 0, thumbLY: 0, thumbRX: 0, thumbRY: 0,
  physicalPhysicality: 0, virtualPhysicality: 0,
}, 0)
assert.equal(new Uint8Array(payload)[12], 1)
const input = {
  shareCounterIndex: buttonIds.Share, counterCount: buttonIds.EnumLength,
  unreliableGamepadPayloadBytes: 37, sharePayloadOffset: 12,
  payloadHex: Buffer.from(payload).toString('hex'),
  reliableButtons: Object.keys(reliableMask).filter(key => !/^\d+$/.test(key)),
  reliableDeclaredBits: Object.keys(reliableMask).filter(key => !/^\d+$/.test(key)).reduce((mask, key) => mask | reliableMask[key], 0),
}
const banner = source => `// 来源：${source.url}\n// SHA-256：${source.sha256}\n// 2026-11-12 源码摘录，别名沿用官方 bundle。\n`
fs.writeFileSync(path.join(output, 'official-vibration.excerpt.js'), banner(rtc) + printer.printNode(ts.EmitHint.Unspecified, vibrationClass, rtc.file))
const inputExcerpts = [
  banner(model), `(${printer.printNode(ts.EmitHint.Unspecified, shareEnumCall, model.file)});\n`,
  banner(rtc), `(${printer.printNode(ts.EmitHint.Unspecified, maskEnumCall, rtc.file)});\n`,
  ...['generateGamepadsFrame', 'generateUnreliableGamepadFrame', 'updateCurrentGamepadInputFrame', 'setupUnreliableInputModelDataChannels'].map(name =>
    `\n// ${name}\nclass ${name}Excerpt {\n${printer.printNode(ts.EmitHint.Unspecified, method(name), rtc.file)}\n}\n`),
]
fs.writeFileSync(path.join(output, 'official-input.excerpt.js'), inputExcerpts.join('\n'))
const results = {
  source: [...sources].map(([filename, source]) => ({ filename, url: source.url, sha256: source.sha256, bytes: Buffer.byteLength(source.text) })),
  timing, input,
  scope: '实际源码方法 + 模拟计时器/actuator；强度按直通比较；Share 编码器直接注入计数器。',
}
fs.writeFileSync(path.join(output, 'results.json'), JSON.stringify(results, null, 2) + '\n')
process.stdout.write(JSON.stringify(results, null, 2) + '\n')
