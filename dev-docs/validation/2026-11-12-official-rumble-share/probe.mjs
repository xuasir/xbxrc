import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
import fs from 'node:fs'
import path from 'node:path'
import vm from 'node:vm'
import ts from 'typescript'

const assets = process.argv[2]
assert.ok(assets, '传入下载的官方公开脚本目录')
const output = import.meta.dirname
const repository = path.resolve(output, '../../..')
const filenames = ['8128.1aca8038.chunk.js', 'web-rtc-stream.d6f6b903.chunk.js']
const sources = filenames.map(filename => ({
  filename,
  text: fs.readFileSync(path.join(assets, filename), 'utf8'),
  url: `https://assets.play.xbox.com/playxbox/static/js/${filename}`,
}))
for (const source of sources) {
  source.sha256 = createHash('sha256').update(source.text).digest('hex')
  source.file = ts.createSourceFile(source.filename, source.text, ts.ScriptTarget.Latest, true, ts.ScriptKind.JS)
}
function find(source, predicate) {
  let found
  function visit(node) {
    if (!found && predicate(node)) found = node
    if (!found) ts.forEachChild(node, visit)
  }
  visit(source.file)
  assert.ok(found, '官方源码含目标节点')
  return found
}
const [model, rtc] = sources
const enumNode = find(model, node => ts.isCallExpression(node) && ts.isFunctionExpression(node.expression) && node.getText(model.file).includes('e.Share=11'))
const buttonIds = vm.runInNewContext(`var y; (${enumNode.getText(model.file)}); y`)
const reporterNode = find(rtc, node => ts.isClassDeclaration(node) && node.name?.getText(rtc.file) === 'O')
let now = 17.5
const Reporter = vm.runInNewContext(`${reporterNode.getText(rtc.file)}; O`, {
  TextEncoder,
  s: () => (target, name, value) => { target[name] = value },
  c: { t: buttonIds, $B: { UnreliableInputReport: 512 }, Qs: { EnumLength: 2 }, sl: { ButtonCount: 5 } },
  u: { r: { Instance: { info() {}, throw(message) { throw new Error(message) } } } },
  b: { ws: { BadLength: 'BadLength' } },
  performance: { now: () => now },
})
const transitions = new Uint8Array(16)
for (const id of [buttonIds.DpadUp, buttonIds.Nexus, buttonIds.Share, buttonIds.A]) transitions[id] = 1
const gamepad = { gamepadId: 0, transitionCount: transitions, triggerLeft: 0.25, triggerRight: 0.75, thumbLX: 0.5, thumbLY: 0.25, thumbRX: -1, thumbRY: -1, physicalPhysicality: 0, virtualPhysicality: 0 }
const changes = { changedGamepads: [gamepad], changedPointers: [], changedKeyboard: undefined, changedMouse: undefined, lockKeysState: { isInitialized: () => false } }
const golden = {}
for (const version of [9, 10]) {
  const reporter = new Reporter()
  const packet = reporter.generateUnreliableInputFrameReport(version, 1, changes, 1080, 1920)
  assert.equal(packet.byteLength, version === 9 ? 60 : 61)
  assert.equal(packet[31], 1)
  golden[version] = Array.from(packet)
}
const fixture = path.join(output, 'unreliable-share-official.json')
fs.mkdirSync(path.dirname(fixture), { recursive: true })
fs.writeFileSync(fixture, `${JSON.stringify(golden, null, 2)}\n`)
const printer = ts.createPrinter({ newLine: ts.NewLineKind.LineFeed })
const excerpts = [enumNode, ...reporterNode.members.filter(member => ts.isConstructorDeclaration(member) || (ts.isMethodDeclaration(member) && ['calculateHeaderSize', 'generateHeader', 'generateUnreliableInputFrameReport', 'generateUnreliableGamepadFrame'].includes(member.name.getText(rtc.file))))]
fs.writeFileSync(path.join(output, 'official-input.excerpt.js'), excerpts.map(node => printer.printNode(ts.EmitHint.Unspecified, node, node === enumNode ? model.file : rtc.file)).join('\n\n'))

function fakeClock() {
  let time = 0
  let nextId = 0
  const timers = new Map()
  const schedule = (callback, delay, interval) => {
    const id = ++nextId
    timers.set(id, { callback, due: time + delay, interval })
    return id
  }
  return {
    now: () => time,
    setTimeout: (callback, delay) => schedule(callback, delay, 0),
    setInterval: (callback, delay) => schedule(callback, Math.max(1, delay), Math.max(1, delay)),
    clearTimeout: id => timers.delete(id),
    clearInterval: id => timers.delete(id),
    advance(until) {
      let budget = 1000
      while (budget-- > 0) {
        const next = [...timers].sort((a, b) => a[1].due - b[1].due)[0]
        if (!next || next[1].due > until) break
        const [id, timer] = next
        time = timer.due
        if (timer.interval) timer.due += timer.interval
        else timers.delete(id)
        timer.callback()
      }
      assert.ok(budget > 0, '计时器回放保持有界')
      time = until
    },
    pending: () => timers.size,
  }
}
function pad(clock, events) {
  return {
    index: 0, id: 'Xbox', connected: true, mapping: 'standard',
    buttons: Array.from({ length: 18 }, () => ({ value: 0, pressed: false })), axes: [0, 0, 0, 0],
    vibrationActuator: {
      effects: ['dual-rumble'],
      // dual-rumble 的缺省扳机参数按 API 的零值记录。
      playEffect: async (type, parameters) => { events.push({ atMs: clock.now(), type, leftTrigger: 0, rightTrigger: 0, ...parameters }); return 'complete' },
      reset: async () => 'complete',
    },
  }
}
function sourceLoader(clock, runtime) {
  const cache = new Map()
  function load(filename) {
    filename = path.resolve(filename)
    if (cache.has(filename)) return cache.get(filename)
    if (filename.endsWith('.json')) return JSON.parse(fs.readFileSync(filename, 'utf8'))
    const exports = {}
    cache.set(filename, exports)
    const code = ts.transpileModule(fs.readFileSync(filename, 'utf8'), { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS } }).outputText
    vm.runInNewContext(code, {
      exports, structuredClone, ...clock, performance: { now: clock.now },
      require(id) {
        if (id === '@shared/gamepad/runtime') return runtime
        assert.ok(id.startsWith('.'), `定点源码依赖：${id}`)
        return load(path.resolve(path.dirname(filename), id.endsWith('.json') ? id : `${id}.ts`))
      },
    }, { filename })
    return exports
  }
  return load
}
const vibrationClass = find(rtc, node => ts.isClassDeclaration(node) && node.members.some(member => ts.isMethodDeclaration(member) && member.name.getText(rtc.file) === 'onVibration'))
const vibrationDefaults = find(rtc, node => ts.isVariableDeclaration(node) && node.name.getText(rtc.file) === 'C' && node.initializer?.getText(rtc.file).includes('supportsDualRumble')).initializer
const basic = { atMs: 0, magnitude: 50, durationMs: 200, delayMs: 100, repeat: 2 }
const scenarios = [
  { name: 'delay-and-repeat', actions: [basic] },
  { name: 'long-duration', actions: [{ ...basic, durationMs: 5000, delayMs: 0, repeat: 0 }] },
  { name: 'short-weak-pulse', actions: [{ ...basic, magnitude: 1, durationMs: 4, delayMs: 0, repeat: 0 }] },
  { name: 'new-effect-cancels-delay', actions: [basic, { ...basic, atMs: 50, magnitude: 25, durationMs: 20, delayMs: 0, repeat: 0 }] },
  { name: 'stop-cancels-repeat', actions: [basic, { ...basic, atMs: 200, magnitude: 0 }] },
]
const timing = []
for (const scenario of scenarios) {
  const officialClock = fakeClock()
  const officialEvents = []
  const officialPad = pad(officialClock, officialEvents)
  const Official = vm.runInNewContext(`const C=${vibrationDefaults.getText(rtc.file)}; ${vibrationClass.getText(rtc.file)}; ${vibrationClass.name.text}`, {
    ...officialClock, window: { addEventListener() {}, removeEventListener() {} }, navigator: { getGamepads: () => [officialPad] },
    s: () => (target, name, value) => { target[name] = value }, a: () => Object.assign,
  })
  const official = new Official()
  official.setEnableVibration(true)
  official.onGamepadConnected(0)
  const clock = fakeClock()
  const events = []
  const controllerModule = {}
  const load = sourceLoader(clock, controllerModule)
  const { InputController } = load(path.join(repository, 'src/shared/gamepad/input-controller.ts'))
  const { parseGamepadPreferences } = load(path.join(repository, 'src/shared/gamepad/preferences.ts'))
  const { BrowserGamepadBackend } = load(path.join(repository, 'src/shared/gamepad/browser-backend.ts'))
  const { RumbleService } = load(path.join(repository, 'src/player/app/input/RumbleService.ts'))
  const controller = new InputController()
  controllerModule.inputController = controller
  const device = pad(clock, events)
  const backend = new BrowserGamepadBackend({ getGamepads: () => [device], requestFrame: () => 1, cancelFrame() {}, now: clock.now, startTimer: () => 0, stopTimer() {} })
  controller.configure(parseGamepadPreferences({ vibration_strength: 'full' }))
  await controller.start(backend)
  controller.setStreamReady(true)
  const rumble = new RumbleService()
  for (const action of scenario.actions) {
    officialClock.advance(action.atMs)
    clock.advance(action.atMs)
    official.onVibration(0, { ...action, leftMotorPercent: action.magnitude / 100, rightMotorPercent: action.magnitude / 100, leftTriggerMotorPercent: 0, rightTriggerMotorPercent: 0 })
    const packet = new Uint8Array(13)
    const view = new DataView(packet.buffer)
    view.setUint16(0, 128, true)
    packet[4] = packet[5] = action.magnitude
    view.setUint16(8, action.durationMs, true)
    view.setUint16(10, action.delayMs, true)
    packet[12] = action.repeat
    rumble.onMessage(view)
  }
  officialClock.advance(10_000)
  clock.advance(10_000)
  assert.deepEqual(events, officialEvents, scenario.name)
  assert.equal(clock.pending(), 0)
  timing.push({ scenario, official: officialEvents, xbxrc: events, match: true })
  await controller.stop()
}
assert.deepEqual(timing[0].xbxrc.map(event => event.atMs), [100, 400, 700])
assert.equal(timing[1].xbxrc[0].duration, 5000)
assert.equal(timing[2].xbxrc[0].duration, 4)
fs.writeFileSync(path.join(output, 'official-vibration.excerpt.js'), printer.printNode(ts.EmitHint.Unspecified, vibrationClass, rtc.file))
const result = {
  capturedOn: '2026-11-12',
  sources: sources.map(({ filename, url, sha256 }) => ({ filename, url, sha256 })),
  shareButtonId: buttonIds.Share,
  gamepadPayloadBytes: 37,
  sharePayloadOffset: 12,
  sharePacketOffset: 31,
  inputGolden: { file: path.relative(repository, fixture), versions: [9, 10], status: 'official-generated' },
  shareExperiment: { status: 'removed', remoteAction: 'no-response', source: 'user local verification on 2026-11-12' },
  rumbleTiming: timing,
  rumblePath: 'RumbleService -> actual InputController/RumbleScheduler -> BrowserGamepadBackend -> simulated actuator',
}
fs.writeFileSync(path.join(output, 'results.json'), `${JSON.stringify(result, null, 2)}\n`)
process.stdout.write(`${JSON.stringify(result, null, 2)}\n`)
