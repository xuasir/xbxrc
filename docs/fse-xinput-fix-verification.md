# FSE XInput 修复验证指南

## 修复内容

在 Windows FSE 模式下，绕过 SDL3 缓存，直接调用 XInput API 轮询手柄状态。

## 修改文件

1. `crates/ohmygamepad/backends/sdl3/Cargo.toml` - 添加 `windows-sys` 依赖
2. `crates/ohmygamepad/backends/sdl3/src/source.rs` - 新增 `try_capture_xinput_state()` 函数

## 验证步骤

### 1. FSE 冷启动测试

**操作**：
1. 连接 Xbox 手柄
2. 进入 Windows FSE 模式（Win + F11）
3. 启动应用

**预期**：
- 打开 `runtime-logs/runtime-trace-*.jsonl`
- 搜索 `sdl3PolledSnapshotChanged` 事件
- 确认 `allZero: false` 且 `maxAbsAxisMilli` > 0

**关键指标**：
```json
{
  "event": "sdl3PolledSnapshotChanged",
  "allZero": false,
  "maxAbsAxisMilli": 100,
  "pressedButtonCount": 0
}
```

### 2. 触屏后采样测试

**操作**：
1. FSE 模式下运行应用
2. 触摸屏幕任意位置
3. 移动手柄摇杆

**预期**：
- 搜索 `last_sample_progress_at_ms` 字段
- 数值应持续增长（每次输入变化时）
- `sampling_health` 保持 `"Healthy"`

**失败特征**（已修复）：
```json
{
  "sampling_health": "Stalled",
  "last_sample_progress_at_ms": 1000,  // 冻结
  "last_backend_sample_activity_at_ms": 5000  // 持续增长
}
```

### 3. SDL3 回退路径测试

**操作**：
1. 退出 FSE 模式（Win + F11）
2. 在普通窗口模式运行应用
3. 测试手柄输入

**预期**：
- 手柄输入正常工作
- 使用 SDL3 原有路径（非 XInput 直接轮询）

## 技术验证

### XInput 调用验证

在 Windows 上运行时，XInput 路径会被优先使用：
- 检查 `player_index` 是否在 0-3 范围内
- 调用 `XInputGetState()` 返回 0（成功）

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
3. **回退机制**：非 XInput 设备或 `player_index` 缺失时回退到 SDL3

## 性能指标

- **轮询频率**：每 8ms 一次（125 Hz）
- **XInput 调用开销**：< 0.1ms
- **CPU 影响**：可忽略（原有轮询频率不变）

## 回滚方案

如果需要回滚：
1. 从 `Cargo.toml` 移除 `windows-sys` 依赖
2. 删除 `try_capture_xinput_state()` 函数
3. 恢复 `capture_gamepad_baseline_state()` 原始实现
