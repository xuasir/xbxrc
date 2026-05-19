/**
 * 串流业务输入消费侧：Rust 渲染器走 `setStreamPadForwarding`；浏览器直连走 RTC + `stream-ui-input-mode`（见 `InputService`）。
 */

export interface StreamInputConsumerAdapter {
  activateStreamInput: () => Promise<void>
  deactivateStreamInput: () => Promise<void>
}

/** Rust 渲染：与历史 `Stream.vue` route controller 共用同一 RPC 合同。 */
export function createRustEngineStreamInputAdapter(gamepad: {
  setStreamPadForwarding: (input: { enabled: boolean }) => Promise<unknown>
  stopRumble: (input: { target: { kind: 'auto' } }) => Promise<unknown>
}): StreamInputConsumerAdapter {
  return {
    async activateStreamInput() {
      await gamepad.setStreamPadForwarding({ enabled: true })
      await gamepad.stopRumble({ target: { kind: 'auto' } }).catch(() => {})
    },
    async deactivateStreamInput() {
      await gamepad.setStreamPadForwarding({ enabled: false })
    },
  }
}

/** 浏览器直连：物理样本仍经 `GamepadDriver`；RTC 侧 overlay 由页面 `stream-ui-input-mode` 事件控制，此处不占位 RPC。 */
export function createBrowserPlayerStreamInputAdapter(): StreamInputConsumerAdapter {
  return {
    async activateStreamInput() {},
    async deactivateStreamInput() {},
  }
}
