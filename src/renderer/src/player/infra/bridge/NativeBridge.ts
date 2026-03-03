export interface NativeBridgeMessage {
  type: string;
  message: any;
}

export class NativeBridge {
    isAvailable(): boolean {
        return typeof window !== 'undefined' && !!(window as any).ReactNativeWebView
    }

    post(message: NativeBridgeMessage): void {
        if (!this.isAvailable()) {
            return
        }
        (window as any).ReactNativeWebView.postMessage(JSON.stringify(message))
    }
}
