import { listen } from '@tauri-apps/api/event'
import { createEventClient } from '@shared/events/client'
import { EVENT_CHANNEL_MAP, type XBoxEventSchema } from '@shared/events/contract'

const subscribeByPreload = <TEvent extends keyof XBoxEventSchema & string>(
  event: TEvent,
  listener: (payload: XBoxEventSchema[TEvent]) => void
): (() => void) => {
  let unlistenFn: (() => void) | null = null

  const channel = EVENT_CHANNEL_MAP[event]

  // tauri's listen returns a promise that resolves to an unlisten function
  listen<XBoxEventSchema[TEvent]>(channel, (tauriEvent) => {
    console.info(`[rust->ui][event] ${event} (${channel})`, tauriEvent.payload)
    listener(tauriEvent.payload)
  }).then((fn) => {
    unlistenFn = fn
  })

  return () => {
    if (unlistenFn) {
      unlistenFn()
    }
  }
}

// 与 rpc 门面一致：renderer 通过统一 client 订阅事件
export const events = createEventClient<XBoxEventSchema>(subscribeByPreload)
