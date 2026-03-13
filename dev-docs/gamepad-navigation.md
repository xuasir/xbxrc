# 手柄导航系统 (Gamepad Navigation System)

## 概述 (Overview)

本系统是一个基于**几何空间寻路 (Geometric Pathfinding)** 的现代手柄导航引擎，完全替代了传统的、需要手动维护邻居节点 (`neighbors`) 的方案。它专为 Xbox 云游戏和串流场景设计，提供了与主机原生 UI 一致的丝滑体验。

## 核心架构 (Core Architecture)

代码统一收敛在 `src/navigation/core/` 目录下：

- **`pathfinding.ts`**: 核心算法。利用 `getBoundingClientRect()` 获取 DOM 坐标，在指定方向上计算候选元素的距离和对齐度得分，选出最佳目标。
- **`engine.ts`**: 状态管理器。维护当前焦点、Scope 栈、区域记忆 (`Zone Memory`) 和自动滚动逻辑。
- **`input.ts`**: 意图分发器。作为 UI 层的事件总线，接收来自手柄监听器的标准化指令。
- **`gamepad-listener.ts`**: **唯一输入源**。订阅系统的 `gamepad.padSnapshot` 事件流，将物理按键映射为 UI 导航意图。
- **`haptics.ts`**: 反馈系统。对接 RPC 接口实现真实的手柄震动 (Rumble) 和 UI 音效。

## 使用指南 (Usage Guide)

### 1. 使元素可聚焦 (Focusable)

使用 `Focusable` 组件包装任何需要响应手柄点击的 UI 元素。

```vue
<script setup>
import { Focusable } from '@/navigation/core/vue'
</script>

<template>
  <Focusable as="button" @click="handleAction">
    确认按钮
  </Focusable>
</template>
```

- **`as`**: 指定渲染的标签（默认 `div`）。
- **`onConfirm`**: 手柄 A 键触发的回调。
- **注意**: 引擎会自动计算上下左右关系，无需手动声明 `neighbors`。

### 2. 区域隔离与管理 (Focus Scope)

使用 `FocusScope` 划分布局区域（如顶部导航、侧边栏、弹窗）。

```vue
<script setup>
import { FocusScope } from '@/navigation/core/vue'
</script>

<template>
  <!-- 弹窗场景：active 为 true 时焦点会被锁死在该区域内 -->
  <FocusScope id="modal-id" :active="isModalOpen">
    <div class="modal">
       <Focusable as="button">选项 1</Focusable>
       <Focusable as="button">选项 2</Focusable>
    </div>
  </FocusScope>
</template>
```

- **焦点记忆**: 当从区域 A 移到区域 B 再切回 A 时，引擎会自动恢复区域 A 上次的焦点。
- **自动聚焦**: Scope 激活时会自动寻找其内部标记为 `data-nav-default-focus` 的元素或第一个可聚焦元素。

### 3. 反馈配置 (Feedback Configuration)

系统对接了全局持久化配置，用户可以在设置页面控制：

- **UI Haptics**: 手柄震动开关。
- **UI Audio**: 界面音效开关。

开发者可以通过 `syncHapticsConfig` 强制同步状态，或在 `haptics.ts` 中扩展新的震动模式。

## 性能优化建议 (Performance Tips)

1. **避免频繁重建 DOM**: 引擎使用了 `MutationObserver` 缓存可聚焦元素。频繁的 DOM 销毁和重建会导致缓存失效并触发重计算。
2. **布局透明**: 默认情况下 `Focusable` 和 `FocusScope` 使用 `display: contents`，不会影响 CSS Flex/Grid 布局。如果需要自定义样式，请显式提供 `as` 属性。
3. **快速移动**: 引擎会自动检测快速连按，并将滚动模式切换为 `instant` 以保证寻路坐标的准确性。

## 维护者注意事项

- **不要** 在业务代码中直接监听 `keydown` 事件来处理导航。
- **不要** 手动计算网格索引，除非是处理极特殊的环形滚动逻辑。
- 所有的手柄逻辑应当通过 `gamepad-listener.ts` 进行中转。
