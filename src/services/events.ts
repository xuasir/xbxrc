import type { XBoxEventSchema } from '@shared/events/contract'
import { createEventClient } from '@shared/events/client'
import { EVENT_CHANNEL_MAP } from '@shared/events/contract'
import { listen } from '@tauri-apps/api/event'

function subscribeByPreload<TEvent extends keyof XBoxEventSchema & string>(event: TEvent, listener: (payload: XBoxEventSchema[TEvent]) => void): (() => void) {
  const channel = EVENT_CHANNEL_MAP[event]

  const unlistenPromise = listen<XBoxEventSchema[TEvent]>(channel, (tauriEvent) => {
    console.info(`[rust->ui][event] ${event} (${channel})`, tauriEvent.payload)
    listener(tauriEvent.payload)
  })

  return () => {
    unlistenPromise.then(unlisten => unlisten())
  }
}

// 与 rpc 门面一致：renderer 通过统一 client 订阅事件
export const events = createEventClient<XBoxEventSchema>(subscribeByPreload)
