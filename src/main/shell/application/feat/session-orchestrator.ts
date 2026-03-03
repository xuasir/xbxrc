import { getAuthService } from '../../../mods/auth'

/**
 * 会话编排器
 * - 负责触发认证静默恢复，不承担数据域自动预取
 */
export class SessionOrchestrator {
  private readonly authService = getAuthService()
  private started = false

  onAppReady(): void {
    if (this.started) {
      return
    }
    this.started = true

    // 启动时主动触发静默认证：已登录用户恢复会话，未登录保持未认证态
    void this.authService.checkAuthentication().catch((error) => {
      console.error('[SessionOrchestrator] silent auth bootstrap failed', error)
    })
  }
}
