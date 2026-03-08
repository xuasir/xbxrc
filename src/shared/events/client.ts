export type EventUnsubscribe = () => void

export type EventSubscribe<TSchema extends object> = <TEvent extends keyof TSchema & string>(
  event: TEvent,
  listener: (payload: TSchema[TEvent]) => void,
) => EventUnsubscribe

export interface EventClient<TSchema extends object> {
  on: EventSubscribe<TSchema>
}

/**
 * 创建事件客户端门面
 * - 与 rpc 门面一致，renderer 只依赖统一 client 接口
 */
export function createEventClient<TSchema extends object>(
  subscribe: EventSubscribe<TSchema>,
): EventClient<TSchema> {
  return {
    on: subscribe,
  }
}
