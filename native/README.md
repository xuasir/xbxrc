# Native Workspace

当前根级 Rust 工程采用 Cargo workspace，多包拆分如下：

- `input-dto`
  - Rust 与 TypeScript 共享的输入 DTO 定义
- `input-core`
  - 手柄输入内核骨架，承接发现、采样、映射、过滤、路由
- `input-bridge`
  - 连接宿主桥接层的命令/事件转换骨架

当前阶段先把 DTO 与包边界稳定下来，具体桌面后端实现后续再接入。
