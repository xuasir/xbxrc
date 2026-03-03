import { z, ZodError, type ZodType } from 'zod'
import type { RpcInvokePayload } from '../../shared/rpc/protocol'

const targetTypeSchema = z.enum(['home', 'cloud'])
const voidParamsSchema = z.undefined().optional()

const streamingIceCandidateSchema = z
  .object({
    candidate: z.string(),
    sdpMLineIndex: z.number().nullable().optional(),
    sdpMid: z.string().nullable().optional(),
    usernameFragment: z.string().nullable().optional(),
    messageType: z.string().optional()
  })
  .strict()

const rpcInvokeEnvelopeSchema = z
  .object({
    namespace: z.string().min(1),
    method: z.string().min(1),
    params: z.unknown().optional()
  })
  .strict()

const rpcParamSchemas: Record<string, Record<string, ZodType>> = {
  app: {
    getVersion: voidParamsSchema,
    ping: z.object({ message: z.string() }).strict(),
    isFullscreen: voidParamsSchema,
    toggleFullscreen: voidParamsSchema,
    enterFullscreen: voidParamsSchema,
    exitFullscreen: voidParamsSchema,
    getStartupFlags: voidParamsSchema,
    resetAutoConnect: voidParamsSchema,
    clearUserData: voidParamsSchema,
    clearData: voidParamsSchema,
    quit: voidParamsSchema,
    restart: voidParamsSchema
  },
  auth: {
    getState: voidParamsSchema,
    checkAuthentication: voidParamsSchema,
    login: voidParamsSchema,
    clearAuthCache: z.object({ scope: z.enum(['ephemeral', 'all']) }).strict(),
    logout: voidParamsSchema
  },
  config: {
    get: z.object({ keys: z.array(z.string()) }).strict(),
    set: z.object({ patch: z.record(z.string(), z.unknown()) }).strict(),
    getGroups: voidParamsSchema
  },
  data: {
    getUserProfile: voidParamsSchema,
    getHosts: voidParamsSchema,
    getRemoteConsoles: voidParamsSchema,
    getStreamingTitleInputConfig: z.object({ xboxTitleId: z.string().min(1) }).strict(),
    powerOnConsole: z.object({ consoleId: z.string().min(1) }).strict(),
    powerOffConsole: z.object({ consoleId: z.string().min(1) }).strict(),
    sendTextToConsole: z.object({ consoleId: z.string().min(1), text: z.string() }).strict(),
    getXcloudTitles: voidParamsSchema
  },
  streaming: {
    getFallbackTurnServer: z.object({ targetType: targetTypeSchema }).strict(),
    createSession: z.object({ targetType: targetTypeSchema, targetId: z.string().min(1) }).strict(),
    getSession: z.object({ sessionId: z.string().min(1) }).strict(),
    closeSession: z.object({ sessionId: z.string().min(1) }).strict(),
    exchangeOffer: z
      .object({
        sessionId: z.string().min(1),
        sdp: z.string().min(1),
        channel: z.enum(['media', 'chat']).optional()
      })
      .strict(),
    exchangeIce: z
      .object({
        sessionId: z.string().min(1),
        candidate: z.array(streamingIceCandidateSchema)
      })
      .strict(),
    sendKeepAlive: z.object({ sessionId: z.string().min(1) }).strict(),
    listActiveSessions: z
      .object({
        targetType: targetTypeSchema.optional()
      })
      .strict()
      .optional()
  },
  system: {
    openExternal: z.object({ url: z.string().min(1) }).strict()
  }
}

function toValidationError(prefix: string, error: unknown): Error {
  if (!(error instanceof ZodError)) {
    return new Error(prefix)
  }

  const firstIssue = error.issues[0]
  const issuePath = firstIssue?.path.join('.')
  return new Error(
    `${prefix}${issuePath !== undefined && issuePath !== '' ? ` at ${issuePath}` : ''}: ${firstIssue?.message ?? 'invalid payload'}`
  )
}

/**
 * 统一校验 renderer -> main 的 RPC 载荷
 * - 在进入 handler 前完成 envelope 与 params 校验，避免主进程直接解构 unknown
 */
export function parseRpcInvokePayload(payload: unknown): RpcInvokePayload {
  let envelope: RpcInvokePayload
  try {
    envelope = rpcInvokeEnvelopeSchema.parse(payload)
  } catch (error) {
    throw toValidationError('Invalid RPC payload', error)
  }

  const namespaceSchemas = rpcParamSchemas[envelope.namespace]
  if (namespaceSchemas === undefined) {
    throw new Error(`RPC namespace not found: ${envelope.namespace}`)
  }

  const paramsSchema = namespaceSchemas[envelope.method]
  if (paramsSchema === undefined) {
    throw new Error(`RPC method not found: ${envelope.namespace}.${envelope.method}`)
  }

  try {
    const parsedParams = paramsSchema.parse(envelope.params)
    if (parsedParams === undefined) {
      return {
        namespace: envelope.namespace,
        method: envelope.method
      }
    }

    return {
      namespace: envelope.namespace,
      method: envelope.method,
      params: parsedParams
    }
  } catch (error) {
    throw toValidationError(`Invalid RPC params for ${envelope.namespace}.${envelope.method}`, error)
  }
}
