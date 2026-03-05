import { z, ZodError, type ZodType } from 'zod'
import type { RpcInvokePayload } from '../../shared/rpc/protocol'

const targetTypeSchema = z.enum(['home', 'cloud'])
const voidParamsSchema = z.undefined().optional()
const logicalPadIdSchema = z.enum(['pad-0', 'pad-1', 'pad-2', 'pad-3'])
const gamepadBindingModeSchema = z.enum([
  'single-active',
  'fixed-device',
  'merged',
  'split',
  'last-active-failover'
])
const gamepadStreamPushModeSchema = z.enum(['on-change', 'fixed-rate'])
const gamepadSamplingModeSchema = z.enum(['merge', 'primary-preferred'])
const xbxEngineReconnectReasonSchema = z.enum(['networkLost', 'iceFailed', 'mediaStalled'])
const xbxEngineInputPointerSchema = z
  .object({
    kind: z.literal('pointer'),
    at_ms: z.number(),
    event: z.string(),
    pointer_type: z.string(),
    x: z.number(),
    y: z.number(),
    delta_x: z.number().optional(),
    delta_y: z.number().optional(),
    button: z.number().optional()
  })
  .strict()
const xbxEngineInputKeyboardSchema = z
  .object({
    kind: z.literal('keyboard'),
    at_ms: z.number(),
    event: z.string(),
    code: z.string(),
    key: z.string(),
    repeat: z.boolean(),
    ctrl_key: z.boolean(),
    shift_key: z.boolean(),
    alt_key: z.boolean(),
    meta_key: z.boolean()
  })
  .strict()
const xbxEngineInputEventSchema = z.discriminatedUnion('kind', [
  xbxEngineInputPointerSchema,
  xbxEngineInputKeyboardSchema
])
const xbxEngineDisplayOptionsSchema = z
  .object({
    sharpness: z.number(),
    saturation: z.number(),
    contrast: z.number(),
    brightness: z.number()
  })
  .strict()
const xbxEngineDisplayStateSchema = z
  .object({
    display_options: xbxEngineDisplayOptionsSchema
  })
  .strict()
const streamingTurnServerSchema = z
  .object({
    url: z.string().min(1),
    username: z.string(),
    credential: z.string()
  })
  .strict()

const streamingIceCandidateSchema = z
  .object({
    candidate: z.string(),
    sdpMLineIndex: z.number().nullable().optional(),
    sdpMid: z.string().nullable().optional(),
    usernameFragment: z.string().nullable().optional(),
    messageType: z.string().optional()
  })
  .strict()

const gamepadRouteTargetSchema = z.discriminatedUnion('kind', [
  z.object({ kind: z.literal('shell-ui') }).strict(),
  z
    .object({
      kind: z.literal('stream-session'),
      sessionId: z.string().min(1)
    })
    .strict()
])

const gamepadSamplingConfigSchema = z
  .object({
    backendPollRateHz: z.number().int().positive(),
    logicalPadSampleRateHz: z.number().int().positive(),
    uiPushRateHz: z.number().int().positive(),
    streamPushMode: gamepadStreamPushModeSchema,
    streamPushRateHz: z.number().int().positive().nullable()
  })
  .strict()

const gamepadSamplingStrategySchema = z
  .object({
    mode: gamepadSamplingModeSchema,
    primaryDeviceId: z.string().min(1).nullable(),
    pausedDeviceIds: z.array(z.string().min(1)),
    enableKeyboardFallback: z.boolean()
  })
  .strict()

const logicalPadBindingSchema = z
  .object({
    padId: logicalPadIdSchema,
    mode: gamepadBindingModeSchema,
    deviceIds: z.array(z.string().min(1))
  })
  .strict()

const gamepadRumbleTargetSchema = z.discriminatedUnion('kind', [
  z
    .object({
      kind: z.literal('logical-pad'),
      padId: logicalPadIdSchema
    })
    .strict(),
  z
    .object({
      kind: z.literal('device'),
      deviceId: z.string().min(1)
    })
    .strict()
])

const gamepadRumbleEffectSchema = z
  .object({
    startDelayMs: z.number().int().min(0),
    durationMs: z.number().int().min(0),
    strongMagnitude: z.number().min(0).max(1),
    weakMagnitude: z.number().min(0).max(1),
    leftTrigger: z.number().min(0).max(1),
    rightTrigger: z.number().min(0).max(1),
    repeat: z.number().int().min(0).max(255)
  })
  .strict()

const gamepadRumbleRequestSchema = z
  .object({
    target: gamepadRumbleTargetSchema,
    effect: gamepadRumbleEffectSchema
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
  gamepad: {
    getRuntimeSnapshot: voidParamsSchema,
    setRouteTarget: z.object({ target: gamepadRouteTargetSchema }).strict(),
    updateSampling: z.object({ sampling: gamepadSamplingConfigSchema }).strict(),
    rebindLogicalPad: z.object({ binding: logicalPadBindingSchema }).strict(),
    setSamplingStrategy: z.object({ strategy: gamepadSamplingStrategySchema }).strict(),
    setPrimarySamplingDevice: z.object({ deviceId: z.string().min(1).nullable() }).strict(),
    pauseSamplingDevice: z.object({ deviceId: z.string().min(1) }).strict(),
    resumeSamplingDevice: z.object({ deviceId: z.string().min(1) }).strict(),
    playRumble: z.object({ request: gamepadRumbleRequestSchema }).strict(),
    stopRumble: z.object({ target: gamepadRumbleTargetSchema }).strict()
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
  xbxEngine: {
    startRuntime: z
      .object({
        sessionId: z.string().min(1),
        targetType: targetTypeSchema,
        turnServer: streamingTurnServerSchema.nullish(),
        viewportId: z.string().min(1),
        audioVolume: z.number()
      })
      .strict(),
    requestReconnect: z
      .object({
        reason: xbxEngineReconnectReasonSchema
      })
      .strict(),
    stopRuntime: voidParamsSchema,
    attachViewport: z
      .object({
        viewportId: z.string().min(1)
      })
      .strict(),
    detachViewport: voidParamsSchema,
    applyDisplayState: z
      .object({
        state: xbxEngineDisplayStateSchema
      })
      .strict(),
    pressControllerButton: z
      .object({
        button: z.string().min(1),
        durationMs: z.number().int().min(0)
      })
      .strict(),
    setKeyboardPointerEnabled: z
      .object({
        enabled: z.boolean()
      })
      .strict(),
    pushKeyboardPointerInput: z
      .object({
        event: xbxEngineInputEventSchema
      })
      .strict(),
    setAudioVolume: z
      .object({
        value: z.number()
      })
      .strict(),
    startMicrophone: voidParamsSchema,
    stopMicrophone: voidParamsSchema,
    snapshotStats: voidParamsSchema,
    getLastRuntimeEvent: voidParamsSchema
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
    throw toValidationError(
      `Invalid RPC params for ${envelope.namespace}.${envelope.method}`,
      error
    )
  }
}
