# 布局请求的 Cloud 租约并发修复

2026-11-25，接续布局请求失败与已安装游戏详情入口任务。用户补充官方下载时跳转失败，并提供诊断 `358D1785`。当前代码基于 `release/test`，前轮入口、来源独立和下载阶段追踪改动继续保留。

最新诊断共有 56 行、一个 production 会话，覆盖北京时间 09:15:11.461–09:15:16.951。结构、顺序、隐私和预算检查通过，压力通知为零；全流程 gate 的两项失败来自 seq 36 目录刷新与 seq 42 第二个 Cloud 授权缺少终结事件。导出为聚合日志，物理保留数量由原始滚动文件核验。[分析](diagnostic-analysis.json)、[原始锚点](diagnostic-anchors.json)和[输入指纹](input-fingerprint.json)保留实际覆盖范围。

seq 37、42 先后开始同账户 Cloud 准备；seq 50 首次返回租约代次 1，seq 51 授权成功；seq 52 目录请求开始，seq 55 第二次准备返回代次 2；seq 56 目录请求经过 2ms 返回 `streamAccessRevoked`。Rust 注册表 `StreamAccessRegistry::insert` 对同账户同目标的新代次撤销旧租约，代码与此故障链对应。

本轮在现有 AuthStore 内合并同账户、同地区的在途 Cloud 查询授权。每个等待者继续记录开始及终结事件；单个取消立即结束该等待者，最后一个取消才终止底层准备。串流 startup 沿既有专用生命周期准备。CloudLibraryStore 在目录提交前公布已准备授权，使等待时间和官方布局查询共享当前租约；过时代次回收以当前持有的句柄判断归属。

Mac 公开社区版本接口的首响应为 HTTP 200；微软公开 SDK 摘录 SHA256 与前轮一致，官方仍使用 `download?noRedirect=true` 的文本地址，再独立获取二进制包。公共 Rust 客户端默认自动跳转。这些结果的范围为公开接口与协议核对；最新诊断覆盖启动和授权，官方资源响应与社区布局下载留待包含 `touch-layouts` 事件的新记录确认。[公开探针摘要](public-probe-summary.json)。

本轮五项定向回归全部通过，只执行一轮：并发授权仅准备一次、取消一个等待者保留另一个租约、退出登录释放延迟准备的句柄、目录提交前布局查询复用租约、清空页面等待延迟缓存提交。[测试摘要](test-summary.json)与[逐项时长](test-cases.json)保留实际范围。Simulator 编译为 0 error、7 warning，[编译摘要](build-summary.json)保留既有警告；`pnpm lint:fix` 通过。变更行格式检查通过，完整文件范围的其他格式诊断保存在[原始输出](format-lint-raw.json)，范围分类见[检查结果](format-lint.json)。前轮已完成完整与商品身份两条安装详情入口的一条 UI 冒烟，复用[前轮证据](../2026-11-24-ios-touch-layout-request-followup/README.md)。

本轮临时 DerivedData、SourcePackages、原始 xcresult、公开响应正文与一次性检查记录按[创建清单](temporary-artifact-plan.json)清理；[清理记录](cleanup.json)保存进程结束、释放量与路径核验。用户两份原始日志、前轮证据、共享 Rust/SDK 缓存和模拟器应用数据保留。

使用新构建进入“已安装游戏 → 详情 → 查看游戏布局”，确认官方和社区布局的下载、预览及保存，并在失败后立即导出诊断。
