export enum NavigationIntent {
  Up = 'up',
  Down = 'down',
  Left = 'left',
  Right = 'right',
  Action = 'action',
  Back = 'back',
  TabPrev = 'tab-prev',
  TabNext = 'tab-next',
  View = 'view',
  Menu = 'menu',
}

export type IntentHandler = (intent: NavigationIntent, event?: Event) => void

/**
 * 意图调度中心
 * 现已作为纯粹的消息中转站，不再直接监听 DOM 键盘事件。
 * 所有的输入（包括手柄和后端映射的键盘）均通过 gamepad-listener 统一输入。
 */
export class InputDispatcher {
  private handlers: Set<IntentHandler> = new Set()
  private isEnabled: boolean = true

  enable(): void {
    this.isEnabled = true
  }

  disable(): void {
    this.isEnabled = false
  }

  subscribe(handler: IntentHandler): () => void {
    this.handlers.add(handler)
    return () => {
      this.handlers.delete(handler)
    }
  }

  dispatch(intent: NavigationIntent, event?: Event): void {
    if (!this.isEnabled) return

    for (const handler of this.handlers) {
      handler(intent, event)
    }
  }
}

export const inputDispatcher = new InputDispatcher()
