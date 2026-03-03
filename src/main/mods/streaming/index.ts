import { StreamingService } from './application/streaming-service'
import { ConfigServiceBridge } from './infrastructure/bridges/config-service-bridge'
import { AuthServiceBridge } from './infrastructure/bridges/auth-service-bridge'

let streamingService: StreamingService | undefined

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
