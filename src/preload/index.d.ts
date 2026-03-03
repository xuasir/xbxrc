import { ElectronAPI } from '@electron-toolkit/preload'
import type { RpcInvokePayload } from '../shared/rpc/protocol'
import type { XBoxEventName, XBoxEventSchema } from '../shared/events/contract'

interface PreloadApi {
  rpcInvoke(payload: RpcInvokePayload): Promise<unknown>
  eventOn<TEvent extends XBoxEventName>(
    event: TEvent,
    listener: (payload: XBoxEventSchema[TEvent]) => void
  ): () => void
}

declare global {
  interface Window {
    electron: ElectronAPI
    api: PreloadApi
  }
}
