# PC 串流执行域可行性与计划交付

- 日期：2026-11-26，Asia/Shanghai。
- 本轮完成：可行性分析与工程计划。
- 工程状态：planned；M0–M4等待工程执行。
- [RFC](../rfcs/2026-11-26-pc-streaming-execution-domains.md) · [规划证据](../../dev-docs/validation/2026-11-26-pc-streaming-execution-domains-plan/README.md)。

推荐保留独立Reader Worker，新增Render Worker，以MessageChannel直接传递原生VideoFrame。Frame路径与现有实现同为一次transfer、一次ACK；新增控制/统计/反馈成本全量计入M0。Reader持续更新latest，GL同步调用只占用Render执行域，Main保留Gamepad/DOM/peer与首批输入发送。

结构实现具备清晰路径，宿主默认资格取决于真实OffscreenCanvas/WebGL2/VideoFrame组合测试。TypeScript接口声明、已有Reader/帧transfer实机证据和待验证的新组合分别保留。独立Input Worker受DataChannel实际转移及握手/关闭能力约束，作为M4增量。

| 阶段 | 推荐交付 |
| --- | --- |
| M0 | 当前Main基线、双Worker端口/离屏原型及平台成本门 |
| M1 | 同一DOM无关WebGL核心与scope/frame/配置/截图协议 |
| M2 | Reader→Render直连、单一渲染状态、尺寸/context/截图/停止 |
| M3 | 数值batch/1Hz观察、按成本加入聚合Worker、现行局部恢复接续 |
| M4 | 按宿主能力迁移input channel与同一个InputCore，输出节拍及neutral/关闭合同 |

性能门规划初值：同档read→submit P95增量≤1ms、P99增量≤2ms，保持≥98%基线吞吐；受控Render繁忙场景至少一个输入/主线程调度长尾改善≥20%。帧待处理队列≤1，应用帧引用峰值≤3；VideoFrame导入、配置与截图结果通过同档像素及资源序列核验。阈值在M0根据真实基线及计时精度固定。

本轮已核对当前源码和接口声明，复用新完成的PC反馈、音频和局部恢复合同，单窗口完成RFC与一次规划冒烟。运行时代码沿现有工作树实现，M0性能结果仍待工程阶段采集；本轮临时产物创建数为0。

建议先执行M0，按宿主结果决定M1–M3开放顺序，M4作为独立能力增量接续。
