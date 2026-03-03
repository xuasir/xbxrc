import type { RpcInvokePayload } from './protocol'
import type { RpcClient } from './types'

export type RpcInvoke = (payload: RpcInvokePayload) => Promise<unknown>

export function createRpcClient<TSchema>(invoke: RpcInvoke): RpcClient<TSchema> {
  const namespaceCache = new Map<string, object>()

  // 通过双层 Proxy 把 rpc.app.getVersion() 映射为统一 invoke 调用
  return new Proxy(
    {},
    {
      get(_target, namespace: string | symbol) {
        if (typeof namespace !== 'string') {
          return undefined
        }

        const cachedNamespace = namespaceCache.get(namespace)
        if (cachedNamespace !== undefined) {
          return cachedNamespace
        }

        const namespaceProxy = new Proxy(
          {},
          {
            get(_namespaceTarget, method: string | symbol) {
              if (typeof method !== 'string') {
                return undefined
              }

              return (params?: unknown) =>
                invoke({
                  namespace,
                  method,
                  params
                })
            }
          }
        )

        namespaceCache.set(namespace, namespaceProxy)
        return namespaceProxy
      }
    }
  ) as RpcClient<TSchema>
}
