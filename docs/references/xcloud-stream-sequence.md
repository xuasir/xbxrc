# xCloud 串流时序图

本文记录当前代码库中 xCloud 串流从点击游戏到播放器建立连接的关键时序，便于后续排障、回归和功能迭代时对照。

## 主链路

```mermaid
sequenceDiagram
    participant U as 用户
    participant R as 渲染层 XCloud/XStream
    participant S as useStreamSession
    participant RPC as streaming RPC
    participant SS as StreamSessionService
    participant API as StreamingSessionApi
    participant AUTH as AuthServiceBridge
    participant A as useStreamController
    participant HB as StreamRuntimeHostBridge
    participant RT as useStreamRuntime / StreamRuntime
    participant SIG as StreamSignalingService

    U->>R: 选择 xCloud 游戏并进入串流页
    R->>S: startStream()
    S->>RPC: streaming.createSession(targetType=cloud, targetId)
    RPC->>SS: createSession()
    SS->>API: POST /v5/sessions/cloud/play
    API-->>SS: sessionPath
    SS-->>S: sessionId + playerState=pending

    loop 每秒轮询
        S->>RPC: streaming.getSession(sessionId)
        RPC->>SS: getSession()
        SS->>API: GET /v5/sessions/cloud/{sessionId}/state
        API-->>SS: streamState

        alt Provisioning
            SS-->>S: playerState=pending
            S-->>R: 显示“正在准备串流...”
        else ReadyToConnect
            SS->>AUTH: getTransferToken()
            AUTH-->>SS: connect token
            SS->>API: POST /v5/sessions/cloud/{sessionId}/connect
            API-->>SS: accepted
            SS-->>S: 继续 pending，等待下一次状态推进
        else WaitingForResources
            SS->>API: GET /v1/waittime/{titleId}
            API-->>SS: queue details
            SS-->>S: playerState=queued
            S-->>R: 显示排队信息
        else Provisioned
            SS-->>S: playerState=started
            S-->>R: 请求启动播放器
        else Failed
            SS-->>S: playerState=failed + errorDetails
            S-->>R: 显示启动失败
        end
    end

    R->>A: 触发 runtime 启动
    A->>RT: startRuntime(sessionContext, hostBridge)
    RT->>HB: exchangeOffer(channel=media)
    HB->>RPC: streaming.exchangeOffer()
    RPC->>SIG: exchangeOffer()
    SIG-->>HB: answer
    HB-->>RT: answer
    RT->>HB: exchangeIce()
    HB->>RPC: streaming.exchangeIce()
    RPC->>SIG: exchangeIce()
    SIG-->>HB: remote candidates
    HB-->>RT: remote candidates
    RT-->>A: transport connected
    A-->>R: 更新连接状态
    R-->>U: 显示串流画面
```

## 关键状态说明

- `Provisioning`
  说明上游会话已创建，但资源和连接前置条件还没准备好。前端显示“正在准备串流...”。

- `ReadyToConnect`
  说明上游要求客户端发送 `/connect` token。这个阶段是否能顺利推进，取决于 `AuthServiceBridge.getTransferToken()` 是否能拿到正确的 cloud transfer token。

- `WaitingForResources`
  说明会话进入排队或等待资源阶段。前端会切到排队文案，而不是启动播放器。

- `Provisioned`
  说明会话已经允许进入 WebRTC 协商。只有到这个状态，渲染层才会真正启动本地 runtime；后续 offer/ICE、ICE restart 与本地重连都由 runtime 通过 host bridge 自行完成。
