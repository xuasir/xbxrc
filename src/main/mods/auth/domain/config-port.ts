import type { AuthProvider } from './types'

/**
 * auth 域配置端口
 * - 业务层仅依赖该端口，不直接感知 config 域服务实现
 */
export interface AuthConfigPort {
  getAuthProvider(): AuthProvider
  getForceRegionIp(): string
}
