import '../2026-11-11-gamepad-locked-input-and-extra-buttons/project-fixture.js'

await import('../2026-11-11-gamepad-locked-input-and-extra-buttons/gamepad-fixture.js')
const pads = navigator.getGamepads()
// 在独立验证入口补充四路能力，原手柄夹具保持原样。
for (const pad of pads) {
  if (!pad)
    continue
  pad.vibrationActuator = {
    effects: ['dual-rumble', 'trigger-rumble'],
    playEffect: async () => 'complete',
    reset: async () => 'complete',
  }
}
location.hash = '/controller/vibration'
await import('/src/main.ts')
