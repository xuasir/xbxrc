import { StreamingService } from './application/streaming-service'
import { StreamHostBridgeService } from './application/stream-host-bridge-service'
import { XbxEngineService } from './application/xbxengine-service'
import { ConfigServiceBridge } from './infrastructure/bridges/config-service-bridge'
import { AuthServiceBridge } from './infrastructure/bridges/auth-service-bridge'

let streamingService: StreamingService | undefined
let streamHostBridgeService: StreamHostBridgeService | undefined
let xbxEngineService: XbxEngineService | undefined

function createStreamingService(): StreamingService {
  return new StreamingService({
    authPort: new AuthServiceBridge(),
    configPort: new ConfigServiceBridge()
  })
}

// streaming 域统一控制单例生命周期，避免 RPC 层持有内部实现细节
export function getStreamingService(): StreamingService {
  if (streamingService === undefined) {
    streamingService = createStreamingService()
  }
  return streamingService
}

export function getStreamHostBridgeService(): StreamHostBridgeService {
  if (streamHostBridgeService === undefined) {
    streamHostBridgeService = new StreamHostBridgeService(getStreamingService())
  }
  return streamHostBridgeService
}

export function getXbxEngineService(): XbxEngineService {
  if (xbxEngineService === undefined) {
    xbxEngineService = new XbxEngineService(getStreamHostBridgeService())
  }
  return xbxEngineService
}

export async function shutdownXbxEngineService(): Promise<void> {
  if (xbxEngineService === undefined) {
    return
  }
  await xbxEngineService.shutdown()
}
