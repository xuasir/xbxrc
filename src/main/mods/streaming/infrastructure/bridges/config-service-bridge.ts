import { AppConfig, getConfigService } from '../../../config'
import type { StreamingConfigPort } from '../../domain/config-port'

/**
 * 串流域配置桥
 * - 聚合串流首版真正需要的配置，避免 service 直接读取 config 域细节
 */
export class ConfigServiceBridge implements StreamingConfigPort {
  getStreamingConfig(): Pick<
    AppConfig,
    'resolution' | 'preferred_game_language' | 'ipv6' | 'force_region_ip'
  > {
    const config = getConfigService().getByKeys([
      'resolution',
      'preferred_game_language',
      'ipv6',
      'force_region_ip'
    ])

    return {
      resolution: config.resolution ?? 1080,
      preferred_game_language: config.preferred_game_language ?? 'en-US',
      ipv6: config.ipv6 ?? false,
      force_region_ip: config.force_region_ip ?? ''
    }
  }
}
