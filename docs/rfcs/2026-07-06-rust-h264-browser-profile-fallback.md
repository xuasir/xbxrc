# RFC：Rust H264 协商浏览器对齐与兜底

## 背景

浏览器采样显示成功播放路径的 remote answer 选择 H264 `42e02a`，selected codec stats 为 `42e01f`，connected 后约 304ms 完成首帧 decode/present。Rust 失败样本选择 `4d002a`，启动期只有 non-IDR continuation，`responseObserved=0`、`decoded=0`、`cleanAnchorCommitted=0`。

## 决策

Rust-owned 协商遵循“配置决定优先级，运行时负责兜底”：

- 默认 Rust-owned profile 为 `4d`，保持高画质优先。
- 显式 `64` / `4d` / `42e` / `420` 配置按 family 排序 SDP；`high` / `main` / `browser` / `baseline` 等 alias 先归一到对应 family。
- 启动期在当前 answer 选择 `4d/64` family、receive 已进入 `remote-no-response` / `remote-continuation-only` / `remote-idr-unusable`，且本会话尚无 host present / `DisplayStable` 播出成功边时，下一次 restart 协商临时降到浏览器验证过的 `42e` family。
- fallback 状态保存触发前的原始配置 token；stop 或下一次 start 先恢复该配置，再消费新的 runtime spec。
- 当前配置已经是 `42e/420`，或 answer 已经选择 `42e` family 时，只走既有 recovery/reconnect，不触发 profile fallback。

## 边界

- fallback 只在当前会话内生效；stop 和新 start 都恢复 fallback 前配置，默认配置值为 `4d`。
- 新 start 有显式 runtime codec spec 时，spec 覆盖 fallback 前的缓存 profile。
- fallback 复用现有 reconnect / SDP policy / media backend 主线。
- fallback 不改变 codec 注册顺序、不引入平行 transport 或平行 media pipeline。

## 验证

- Rust 回归：默认 runtime profile 为 `4d`。
- Rust 回归：启动期 remote terminal + answer `4d` 会在 reconnect 前切到 `42e`。
- Rust 回归：fallback 后下一次 start 恢复 fallback 前的显式配置，例如 `64`。
- Rust 回归：fallback 后 stop 恢复原始配置 token，例如 `high`。
- Rust 回归：fallback 后下一次 start 携带显式 runtime codec spec 时，spec 覆盖缓存 profile。
- Rust 回归：配置为 `42e` 时保持 baseline 优先，不被升级成 `4d`。
- SDP policy 回归：`offer_profile=browser/42e` 时 constrained-baseline PT 排在 `4d` 前，`main` 映射到 `4d`，`high` 映射到 `64`。
- Trace gate：`python3 -B .agents/skills/analyze-runtime-logs/scripts/trace_h264_profile_fallback_gate.py --latest --require-fallback --max-age-seconds 900` 验证 fallback 观测、fallback 后 `42e*` answer 和 host present / `DisplayStable` 播出成功边。
- Fresh trace 目标：出现 `startupH264ProfileFallback` 后，下一次 answer 选择 `42e*`，并在 connected 后快速出现 host present / `DisplayStable`；`latest_video_decode_ok_time_ms` 与 `cleanAnchorCommitted` 作为解码与恢复链诊断。

## 进展

- 2026-07-07：重做 Rust fallback 状态模型，从布尔 `h264_profile_fallback_active` 改为保存 fallback 前原始配置 token 的状态；`stop` 和 `apply_execution_spec` 新 start 时先恢复原始配置，再应用 runtime spec；`high` token 映射到 `64`，`main` 映射到 `4d`，`browser/baseline` 映射到 `42e`，该映射同时进入 runtime 与 SDP policy；补齐 session policy remote terminal 测试 fixture 的 current clean anchor 事实。验证：`cargo fmt`、`cargo test -p xbxengine h264_profile --lib -- --nocapture`、`cargo test -p xbxengine transport::rtc::sdp::policy --lib -- --nocapture`、`cargo test -p xbxengine remote_terminal --lib -- --nocapture`、`cargo check -p xbxengine`、`cargo check -p xbxrc`、`PYTHONPYCACHEPREFIX=/tmp/xbxrc-pycache python3 -B -m py_compile .agents/skills/analyze-runtime-logs/scripts/trace_browser_webrtc_behavior_report.py .agents/skills/analyze-runtime-logs/scripts/trace_h264_profile_fallback_gate.py`、`git diff --check`、`git diff --cached --check`。`trace_h264_profile_fallback_gate.py --latest --require-fallback --max-age-seconds 900` 当前命中旧浏览器 trace，因 stale 且缺 Rust fallback 观测失败，等待 fresh Rust trace 验收。
- 2026-07-07：fresh Rust trace `runtime-trace-1783416446705-1.jsonl` 显示 `4d002a` answer 后已有 SPS/PPS/IDR、decode OK、`CleanAnchorCommitted` 与 `hostMailboxAccepted`，但缺 `hostFramePresented` / `DisplayStable` / 后续 statsSnapshot。runtime fallback 成功判定提升到 host present / `DisplayStable`；`startupH264ProfileFallback` summary 改写 `playout=false displayState=*`；native display trace 将 `hostMailboxAccepted`、`hostMailboxTakeDecision`、`hostFramePresented`、present tick coalesced/deferred/failed/blocked 调整为关键事实，并新增 `present_tick_immediate_requested`；trace gate 脚本同步为 host present / `DisplayStable` 成功口径，同时保留 `mediaSuccess` 兼容字段并新增 `playoutSuccess`。
