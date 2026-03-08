export interface RpcMethod<TParams = void, TResult = void> {
  params: TParams
  result: TResult
}

export type RpcSchema = Record<string, Record<string, RpcMethod<unknown, unknown>>>

type RpcClientMethod<TMethod> = TMethod extends RpcMethod<infer TParams, infer TResult>
  ? [TParams] extends [void]
      ? () => Promise<TResult>
      : (params: TParams) => Promise<TResult>
  : never

export type RpcClient<TSchema> = {
  [TNamespace in keyof TSchema]: {
    [TMethod in keyof TSchema[TNamespace]]: RpcClientMethod<TSchema[TNamespace][TMethod]>
  }
}

type RpcHandlerMethod<TMethod> = TMethod extends RpcMethod<infer TParams, infer TResult>
  ? [TParams] extends [void]
      ? () => TResult | Promise<TResult>
      : (params: TParams) => TResult | Promise<TResult>
  : never

export type RpcHandlerMap<TSchema> = {
  [TNamespace in keyof TSchema]: {
    [TMethod in keyof TSchema[TNamespace]]: RpcHandlerMethod<TSchema[TNamespace][TMethod]>
  }
}
