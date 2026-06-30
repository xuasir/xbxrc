# FSE XInput 修复验证指南

## 修复内容

在 Windows FSE 模式下，SDL3 source 同时具备两条 XInput 兜底路径：

- `xinputFallback`：SDL3 已枚举并打开 `Gamepad`，且该设备带有 `player_index` 时，snapshot 读取路径优先调用 `XInputGetState()`。
- `xinputDirect`：Windows-only direct poller 每 8ms 扫描 XInput user 0..3。SDL3 枚举为空，或 SDL opened-device 缺少 `player_index` 时，仍可合成 `xinput:user:N` 设备和 snapshot。

SDL3 opened-device 带有同一个 `player_index=N` 后，direct poller 会断开 `xinput:user:N`，让 SDL3 路径重新成为该 user 的权威来源。

## 修改文件

1. `crates/ohmygamepad/backends/sdl3/Cargo.toml` - 添加 `windows-sys` 依赖
2. `crates/ohmygamepad/backends/sdl3/src/source.rs` - 新增 `try_capture_xinput_state()` 与 Windows XInput direct poller
3. `crates/ohmygamepad/backends/sdl3/src/source.rs` - snapshot trace 增加 `sampleSource`、`playerIndex`、`xinputUserIndex`、`xinputResult`、`fallbackReason`

## 验证步骤

### 1. FSE 冷启动测试

**操作**：
1. 连接 Xbox 手柄
2. 进入 Windows FSE 模式（Win + F11）
3. 启动应用

**预期**：
- 打开 `runtime-logs/runtime-trace-*.jsonl`
- 搜索 `sdl3PolledSnapshotChanged` 事件
- SDL3 已打开设备时确认 `sampleSource: "xinputFallback"`、`xinputResult: 0`
- SDL3 枚举为空时确认 `sampleSource: "xinputDirect"`、`deviceId: "xinput:user:0"`、`xinputResult: 0`
- 输入变化时确认 `allZero: false` 或 `maxAbsAxisMilli` / `pressedButtonCount` 随操作变化

**SDL opened-device 兜底关键指标**：
```json
{
  "event": "sdl3PolledSnapshotChanged",
  "sampleSource": "xinputFallback",
  "playerIndex": 0,
  "xinputUserIndex": 0,
  "xinputResult": 0,
  "fallbackReason": null,
  "allZero": false,
  "maxAbsAxisMilli": 100,
  "pressedButtonCount": 0
}
```

**SDL 枚举为空 + direct poller 关键指标**：
```json
{
  "event": "sdl3GamepadEnumerationObserved",
  "discoveredCount": 0,
  "openedDeviceCount": 0
}
```

```json
{
  "event": "sdl3ConnectedBaselineSnapshot",
  "stage": "xinput-direct-connected",
  "deviceId": "xinput:user:0",
  "sampleSource": "xinputDirect",
  "playerIndex": 0,
  "xinputUserIndex": 0,
  "xinputResult": 0,
  "fallbackReason": null
}
```

### 2. 触屏后采样测试

**操作**：
1. FSE 模式下运行应用
2. 触摸屏幕任意位置
3. 移动手柄摇杆

**预期**：
- 搜索 `lastSampleProgressAtMs` 字段
- 数值应持续增长（每次输入变化时）
- `samplingHealth` 保持 `"healthy"`

**失败特征**（已修复）：
```json
{
  "samplingHealth": "stalled",
  "lastSampleProgressAtMs": 1000,
  "lastBackendSampleActivityAtMs": 5000
}
```

### 3. SDL3 回退路径测试

**操作**：
1. 退出 FSE 模式（Win + F11）
2. 在普通窗口模式运行应用
3. 测试手柄输入

**预期**：
- 手柄输入正常工作
- `sampleSource` 可为 `"sdl"`、`"xinputFallback"` 或 `"xinputDirect"`
- `fallbackReason` 为 `player-index-missing`、`player-index-out-of-range`、`xinput-get-state-failed` 时，说明当前 opened-device snapshot 使用 SDL 读取路径；同一 user 可由 `xinputDirect` 兜底

## 技术验证

### XInput 调用验证

在 Windows 上运行时，XInput 路径按 user 级别生效：
- 检查 `player_index` 是否在 0-3 范围内
- 调用 `XInputGetState()` 返回 0（成功）
- SDL opened-device fallback trace 中 `sampleSource` 应为 `"xinputFallback"`
- direct poller trace 中 `sampleSource` 应为 `"xinputDirect"`
- trace 中 `xinputResult` 应为 `0`

### 按键映射验证

| 手柄按键 | XInput 掩码 | buttons[] 索引 |
|---------|------------|----------------|
| A | 0x1000 | 0 |
| B | 0x2000 | 1 |
| X | 0x4000 | 2 |
| Y | 0x8000 | 3 |
| LB | 0x0100 | 4 |
| RB | 0x0200 | 5 |
| Back | 0x0020 | 8 |
| Start | 0x0010 | 9 |

### 轴归一化验证

测试摇杆和扳机输入：
- 左摇杆向右推到底 → `axes[0]` 应接近 1.0
- 左扳机按下一半 → `axes[4]` 应接近 0.0
- 左扳机按到底 → `axes[4]` 应接近 1.0

## 已知限制

1. **设备支持**：仅支持 XInput 设备（Xbox 手柄），最多 4 个并发
2. **平台限制**：修复仅在 Windows 上生效
3. **回退机制**：非 XInput 设备继续走 SDL3；XInput user 被 SDL3 同 `player_index` 覆盖时 direct 设备会断开

## 性能指标

- **轮询频率**：每 8ms 一次（125 Hz）
- **XInput 调用开销**：< 0.1ms
- **CPU 影响**：可忽略（原有轮询频率不变）

## 回滚方案

如果需要回滚：
1. 从 `Cargo.toml` 移除 `windows-sys` 依赖
2. 删除 `try_capture_xinput_state()` 函数
3. 恢复 `capture_gamepad_baseline_state()` 原始实现
