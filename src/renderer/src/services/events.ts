import { createEventClient } from '../../../shared/events/client'
import type { XBoxEventSchema } from '../../../shared/events/contract'

const subscribeByPreload = <TEvent extends keyof XBoxEventSchema & string>(
  event: TEvent,
  listener: (payload: XBoxEventSchema[TEvent]) => void
): (() => void) => {
  return window.api.eventOn(event, listener)
}

// 与 rpc 门面一致：renderer 通过统一 client 订阅事件
export const events = createEventClient<XBoxEventSchema>(subscribeByPreload)
