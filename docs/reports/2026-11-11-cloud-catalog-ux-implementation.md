# 云目录与交互增强 1–6 项 Report

## Summary

已完成计划 1–6 项、卡片播放/收藏图标按钮、云游戏详情重做，以及用户指出的搜索双层焦点修复。开发、适用自动回归和真实组件模拟 RPC 验证已收口；真实账户与原生窗口验收由用户执行。

- [RFC](../rfcs/2026-11-11-cloud-catalog-ux-implementation.md)
- [接口分析与 better-xcloud 对照](../isu/2026-11-11-cloud-catalog-api-and-ux.md)
- [验证证据](../../dev-docs/validation/2026-11-11-cloud-catalog-ux-implementation/README.md)

## Delivered

| 任务 | 交付行为 |
| --- | --- |
| 1. 目录数据与可用性合同 | 保留栏目顺序，权益显式未知，分开商品/串流/Xbox 标识；合法空快照与每商品资料时间生效，刷新具有范围和终态，局部来源失败保留最近成功内容 |
| 2. 浏览稳定与搜索 | 稳定商品节点与已浏览窗口、返回焦点/搜索恢复，NFKC 与多词 AND 搜索、名称优先/别名匹配、本地分类排序、Tile→Poster 回退、目录来源状态 |
| 3. 详情与再次游玩 | 独立商品详情路由、完整商品按需资料、Hero 和明确游玩操作、简介/功能/截图/语言/游戏信息，复用成就、时长和相册入口 |
| 4. 官方发现与等待 | 云专用新入库、热门与离库栏目；稳定聚焦 500ms 后查询，60 秒缓存、按 key 去重、最多两路、离开取消与迟到结果隔离 |
| 5. 浏览与账户分层 | 公开/缓存目录可浏览，公开播放引导登录；Cloud 访问与输入配置使用正确范围，启动前核验当前账户资格 |
| 6. 收藏与栏目记忆 | 账户范围本地收藏、置顶、分类/排序/栏目偏好；冷读与串行保存保护已有收藏，目录暂缺条目保留可见资料 |

## Changes

Rust 继续负责接口、缓存、访问与持久化，新增 `data/cloud_queries.rs` 收纳详情、定向标题、输入与等待查询。Vue 使用统一 Cloud RPC 类型、目录纯模型和两个页面组合函数，导航继续接入现有几何引擎。共享 flow 的权益字段和 IceCandidate 字段兼容 iOS 调用方，workspace 编译覆盖这些入口。

卡片主体、播放图标、收藏图标分别注册焦点，DOM 中嵌套按钮数量为 0。详情页采用统一横幅、留白、工具栏和信息层级，截图弹层支持前后切换与 Escape 返回。搜索控件的内层轮廓归属可由 `externalFocus` 交给外层；共享飞行焦点动画也定位到外层搜索栏，修复截图中的双框。

最终空目录验证补充修复：成功加载后刷新保留页面、收藏和刷新按钮焦点；刷新成功显示更新时间，降级缓存显示缓存来源。

## Validation

| 验证 | 结果 |
| --- | --- |
| `cargo fmt` / `cargo check --workspace` | 通过；现有 dead_code warnings 保留 |
| WebAPI / Cloud flow / Tauri data 回归 | 分别 70 / 10 / 43 项通过 |
| 云目录相关 Vue 回归 | 4 文件、12 项通过 |
| `pnpm lint:fix` / `pnpm typecheck` / `pnpm build` | 最终代码通过 |
| `pnpm test:navigation-coverage` | 209 处交互入口，0 违规 |
| 最终 `pnpm test` | 105 文件，941 项通过、2 项失败 |
| 浏览器真实组件 + 模拟 RPC | 2,000 项目录、卡片收藏/详情入口、名称归一化、返回搜索/焦点、图片回退、截图切换与关闭、公开播放登录引导、刷新失败、空目录刷新、详情失败重试、深浅色和 480px 窄窗口通过 |
| `git diff --check` | 通过 |

两项全量失败均在云目录任务范围外，并在最终复跑中保持相同断言结果：

- `src/pages/settings/setting-validation.test.ts:38`：`force_region_ip` 断言为 `nextConnection`，当前地区配置合同返回 `nextAuthorization`。
- `src/streaming/runtime/browser-runtime.test.ts:1607`：开发诊断健康采样断言为 0，当前结果为 27。

详情失败证据记录 `data.getCloudDetail` 请求数从首次加载到重试达到 2；页面保留基础资料、个人记录与游玩入口，窄窗口无横向溢出。搜索证据记录内层保持普通 field-shell、输入阴影为 `none`，整条搜索栏拥有 `data-ui-focus-flow=true`。

## Risks

- 定向 title、Cloud inputconfigs、waittime 与启动资格使用源码及模拟服务验证；真实账户的订阅、购买、offering 和地区组合仍需实测。
- 语言支持来自商品资料，实际云运行版本与资料可能存在差异；未知值在页面显式展示。
- 物理手柄、Tauri 原生窗口和实际串流启动依照仓库边界由用户实机验收。

## Follow-up

实机依次验证登录后目录、播放资格/输入配置、等待时间、收藏跨重启与退出后的账户隔离，再验证手柄进入详情、截图弹层返回和实际云串流启动。两项全量失败分别归地区设置与串流诊断任务处理。
