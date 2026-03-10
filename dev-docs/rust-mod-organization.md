# Rust Mod 组织规范

本文定义 `src-tauri/src/mods/*` 的推荐组织方式。目标是统一模块结构，降低职责混杂，避免把协议流程、应用状态、持久化和 RPC 入口揉在一个文件里。

## 1. 总原则

1. `crates/*` 承载协议能力：
   - `xbox-webapi` 只负责 HTTP API、DTO、transport、error。
   - `*-flow` crate 只负责协议流程编排。
2. `src-tauri/src/mods/*` 承载应用能力：
   - 应用状态
   - 持久化
   - 宿主协作（Tauri window、event、AppHandle）
   - RPC 入口
3. 模块内部按职责拆文件，不按 DDD 术语堆目录。

一句话约束：

- 协议和流程放 crate。
- 应用状态和宿主协作放 mod。

## 2. 推荐目录模板

按需裁剪，但优先遵循下面的文件布局：

```text
src-tauri/src/mods/<mod>/
  mod.rs
  service.rs
  runtime_state.rs
  persistence_service.rs
  storage_repository.rs
  <mod>_policy.rs
  types.rs
  rpc.rs
  events.rs
```

不是每个模块都必须有所有文件：

1. 没有运行态，不建 `runtime_state.rs`
2. 没有持久化，不建 `persistence_service.rs` / `storage_repository.rs`
3. 没有纯判定逻辑，不建 `<mod>_policy.rs`
4. 没有事件，不建 `events.rs`

## 3. 各文件职责

### 3.1 `mod.rs`

只负责：

1. 模块导出
2. 对外 trait
3. 少量 glue code

不负责：

1. 业务流程实现
2. 大量类型定义
3. HTTP 调用

### 3.2 `service.rs`

模块主编排器，负责：

1. 调用 `*-flow`
2. 调用 `persistence_service`
3. 调用 `runtime_state`
4. 处理宿主动作（如窗口关闭、事件协作）
5. 应用级策略判断

不负责：

1. 直接写 HTTP 请求
2. 直接访问 store key
3. 堆大量 token/session 判定细节

### 3.3 `runtime_state.rs`

只负责运行态和并发控制：

1. `Mutex/RwLock`
2. pending 状态
3. in-flight 标记
4. cooldown
5. 内存态迁移

不负责：

1. 持久化
2. HTTP
3. 业务 DTO 映射

### 3.4 `persistence_service.rs`

持久化装配层，负责：

1. repository 读写组合
2. `flow DTO <-> store DTO` 映射
3. 为 `service` 提供应用友好的持久化接口

适合放这里的方法例子：

1. `load_auth_flow_seed`
2. `persist_auth_bundle`
3. `get_valid_session_snapshot`
4. `get_refresh_token`

### 3.5 `storage_repository.rs`

真正的 repository，职责只有存取：

1. 读写 token / session / config 原始值
2. 删除缓存
3. store key 访问

禁止放入：

1. validity 判定
2. snapshot 计算
3. app level 推导

### 3.6 `<mod>_policy.rs`

纯判定和派生逻辑：

1. token 是否有效
2. session snapshot 如何计算
3. app level / session level 如何推导

禁止放入：

1. IO
2. store 访问
3. Tauri 依赖

### 3.7 `types.rs`

放模块公共类型：

1. RPC 出入参
2. event payload
3. 跨文件共享的稳定结构

不要把所有内部临时结构都塞进这里。

### 3.8 `rpc.rs`

只负责：

1. 入参解析
2. 命令分发
3. 调用 `service`

禁止把业务核心逻辑直接写在 RPC handler 里。

### 3.9 `events.rs`

只负责：

1. channel 常量
2. payload 组装
3. 发事件 helper

## 4. 命名规则

### 4.1 `Repository`

`Repository` 只表示原始存取层。

允许：

1. `get_*`
2. `set_*`
3. `clear_*`

不允许：

1. `is_valid_*`
2. `build_*_snapshot`
3. `resolve_*_level`

### 4.2 `Policy`

`Policy` 只表示纯判定或派生逻辑。

允许：

1. validity 判定
2. snapshot 计算
3. 等级推导

不允许：

1. IO
2. store key 访问

### 4.3 `Service`

`Service` 表示编排或装配层。

1. `AuthService`：应用编排器
2. `AuthPersistenceService`：持久化装配器

避免把“纯存取”或“纯判定”也命名成 `Service`。

### 4.4 `State`

`State` 只表示内存运行态。

典型内容：

1. `pending_*`
2. `is_processing_*`
3. `cooldown`
4. 当前状态快照

## 5. Auth 作为范本

当前 `auth` 模块已经是这套规范的范本：

```text
src-tauri/src/mods/auth/
  mod.rs
  service.rs
  runtime_state.rs
  persistence_service.rs
  storage_repository.rs
  token_policy.rs
  types.rs
  rpc.rs
  events.rs
```

职责边界：

1. `xbox-webapi`：Auth HTTP API
2. `xbox-auth-flow`：认证协议流程
3. `auth/service.rs`：应用编排
4. `auth/runtime_state.rs`：运行态
5. `auth/persistence_service.rs`：持久化装配
6. `auth/storage_repository.rs`：原始存取
7. `auth/token_policy.rs`：token 策略

## 6. 对后续模块的落地要求

后续新增或重构 `mods/<mod>` 时，默认按这套方式设计。

硬规则：

1. `src-tauri/src/mods/*` 不直接写 Xbox HTTP 请求
2. 协议流程优先收口到 `crates/*-flow`
3. `Repository` 只做存取
4. `Policy` 只做纯计算
5. `Service` 只做编排或装配
6. `rpc.rs` 不写核心业务逻辑
7. `events.rs` 不承载业务状态

## 7. 何时不必过度拆分

以下情况可以少建文件：

1. 模块非常小，只有 1-2 个用例
2. 没有运行态
3. 没有持久化
4. 没有纯策略逻辑

但即便裁剪，也要保持命名语义一致：

1. 不要把 policy 叫 repository
2. 不要把 runtime 状态塞进 service
3. 不要把 flow 逻辑塞回 tauri mod
