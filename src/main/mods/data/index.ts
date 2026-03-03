import { DataService } from './application/data-service'
import { HostService } from './application/services/host-service'
import { ProfileService } from './application/services/profile-service'
import { StreamingQueryService } from './application/services/streaming-query-service'
import { XcloudService } from './application/services/xcloud-service'
import { AuthServiceBridge } from './infrastructure/bridges/auth-service-bridge'
import { XboxWebApiProvider } from './infrastructure/xbox-webapi-provider'

let dataService: DataService | undefined

function createDataService(): DataService {
  return new DataService({
    authPort: new AuthServiceBridge(),
    webApiProvider: new XboxWebApiProvider(),
    hostService: new HostService(),
    xcloudService: new XcloudService(),
    profileService: new ProfileService(),
    streamingQueryService: new StreamingQueryService()
  })
}

/**
 * 数据域统一入口
 * - 会话初始化与子服务生命周期由该单例统一管理
 */
export function getDataService(): DataService {
  if (dataService === undefined) {
    dataService = createDataService()
  }
  return dataService
}
