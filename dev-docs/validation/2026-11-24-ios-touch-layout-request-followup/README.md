# 布局授权请求与安装详情入口修复

2026-11-24。按现有布局功能接续修复，代码与一次最小冒烟完成；真实官方服务回包和 Home 实机应用布局继续待验。当前修改基于 `release/test` 检出，源码基线为 `a98e5457c8f9d67b099033710aa34d08d2fcd3cb`。

官方来源按需准备 Cloud 授权，社区来源通过既有 Rust 公共资源接口查询和下载。游戏已有 Xbox 标题身份时，Cloud 授权失败或单个授权等待者取消均记入官方来源失败，社区来源正常返回。标题身份暂缺时复用一次授权解析标题，再查询来源。仍有效的页面请求会显示可重试的错误，官方认证错误在布局页明确显示。

云游戏、带 Xbox 标题身份的安装详情与仅有商品身份的安装详情共用“查看游戏布局”入口。个人游戏转换优先保留真实商品 ID，布局预览、保存和 Home 串流消费沿原有实现。

用户提供的诊断有 178 行、1 个 dev 会话，覆盖北京时间 17:50:35.783–17:51:05.123。结构、顺序、隐私与预算检查通过；全流程 gate 有 2 处未完成操作配对，分别为 seq 111 的目录刷新和 seq 171 的 Cloud 授权。seq 118 → 176 的授权成功耗时 4316ms；seq 136 → 178 的授权等待者取消耗时 2904ms。导出覆盖启动、目录和授权，布局 HTTP 接口与状态码留待包含 `touch-layouts` 事件的新记录确认。[完整分析](diagnostic-analysis.json)、[原始锚点](diagnostic-anchors.json)与[输入指纹](input-fingerprint.json)保留这一区分。

查询追踪从授权准备前开始，来源错误记录 `source` 与 `stage=cloudAccess|catalog`；布局下载记录开始、成功、失败或取消及对应阶段。公共 HTTP 错误继续沿既有类型化诊断传递。Mac 公共社区版本接口返回 200，核验范围见[请求摘要](community-revision-probe.json)。

本轮只运行 2 项请求单元回归和 1 项安装详情 UI 冒烟，共 3 项通过，UI 用时 32.116 秒。覆盖授权超时/取消时的社区查询与下载、标题解析授权复用、两条详情入口与认证失败提示下的社区卡片。Simulator 编译为 0 error、7 warning；[测试摘要](test-summary.json)、[构建摘要](build-summary.json)、[验证清单](validation.json)和[工具诊断](verification-diagnostics.json)记录实际范围。

- [完整安装详情入口](installed-touch-entry-1.png)
- [商品身份安装详情入口](installed-touch-entry-2.png)
- [完整详情进入布局目录](installed-touch-catalog-1.png)
- [商品身份详情进入布局目录](installed-touch-catalog-2.png)

截图已逐张检查。临时 DerivedData、复制的 SourcePackages、原始 xcresult、重复导出截图和一次性配置按[本轮清单](temporary-artifact-plan.json)回收；[清理记录](cleanup.json)记录进程结束、释放量与路径核验。共享 Rust/SDK 缓存、现有模拟器应用数据及用户原始诊断保留。

使用新构建从“已安装游戏 → 详情 → 查看游戏布局”复测真实布局查询、下载与保存，并核对新诊断中的来源与失败阶段。

11-25 接续：用户新日志定位同账户并发 Cloud 准备导致 `streamAccessRevoked`。现有授权协调层已合并在途准备，目录提交前共享租约；五项定向回归与编译通过，实际官方下载与社区资源结果继续实机确认。[接续证据](../2026-11-25-ios-touch-layout-download-redirect/README.md)。
