import type { AuthConfigPort } from '../../domain/config-port'
import type { AuthProvider } from '../../domain/types'
import { getConfigService } from '../../../config'

/**
 * auth 基础设施桥接
 * - 将 config 域服务适配为 auth 域端口
 */
export class ConfigServiceBridge implements AuthConfigPort {
  getAuthProvider(): AuthProvider {
    const config = getConfigService().getByKeys(['use_msal'])
    return config.use_msal === true ? 'msal' : 'xal'
  }

  getForceRegionIp(): string {
    const config = getConfigService().getByKeys(['force_region_ip'])
    return typeof config.force_region_ip === 'string' ? config.force_region_ip.trim() : ''
  }
}
