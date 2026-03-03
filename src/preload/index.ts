import { contextBridge, ipcRenderer, type IpcRendererEvent } from 'electron'
import { electronAPI } from '@electron-toolkit/preload'
import { RPC_INVOKE_CHANNEL, type RpcInvokePayload } from '../shared/rpc/protocol'
import {
  EVENT_CHANNEL_MAP,
  type XBoxEventName,
  type XBoxEventSchema
} from '../shared/events/contract'

// Custom APIs for renderer
const api = {
  // 仅暴露统一调用入口，具体函数门面在 renderer 侧组装
  rpcInvoke(payload: RpcInvokePayload) {
    return ipcRenderer.invoke(RPC_INVOKE_CHANNEL, payload)
  },
  // 通用事件订阅：由事件名映射到具体 IPC channel
  eventOn<TEvent extends XBoxEventName>(
    event: TEvent,
    listener: (payload: XBoxEventSchema[TEvent]) => void
  ): () => void {
    const channel = EVENT_CHANNEL_MAP[event]
    const wrappedListener = (_event: IpcRendererEvent, payload: XBoxEventSchema[TEvent]): void => {
      listener(payload)
    }

    ipcRenderer.on(channel, wrappedListener)
    return () => {
      ipcRenderer.removeListener(channel, wrappedListener)
    }
  }
}

// Use `contextBridge` APIs to expose Electron APIs to
// renderer only if context isolation is enabled, otherwise
// just add to the DOM global.
if (process.contextIsolated) {
  try {
    contextBridge.exposeInMainWorld('electron', electronAPI)
    contextBridge.exposeInMainWorld('api', api)
  } catch (error) {
    console.error(error)
  }
} else {
  // @ts-ignore (define in dts)
  window.electron = electronAPI
  // @ts-ignore (define in dts)
  window.api = api
}
