import { session } from 'electron'
import { AppService } from './application/app-service'
import { AuthServiceBridge } from './infrastructure/bridges/auth-service-bridge'
import { getMainStore } from '../../store'

let appService: AppService | undefined

function createAppService(): AppService {
  const store = getMainStore()
  return new AppService({
    authPort: new AuthServiceBridge(),
    clearStorageData: async () => {
      await session.defaultSession.clearStorageData()
    },
    store
  })
}

export function getAppService(): AppService {
  if (appService === undefined) {
    appService = createAppService()
  }
  return appService
}
