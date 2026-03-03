import type { AppConfig } from '../../config'

export interface StreamingConfigPort {
  getStreamingConfig(): Pick<
    AppConfig,
    'resolution' | 'preferred_game_language' | 'ipv6' | 'force_region_ip'
  >
}
