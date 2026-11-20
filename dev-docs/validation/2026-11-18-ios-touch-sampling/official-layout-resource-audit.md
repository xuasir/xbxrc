# 官方布局资源入口核对

- 当前云目录 `crates/xbox-cloud-catalog-flow/src/lib.rs` 390–435 行分别投影 productId、streamTitleId、xboxTitleId 与 supportedInputTypes；输入能力标记用于标题筛选。
- 公共 Rust `crates/xbox-webapi/src/session_api.rs` 242 行及 `xcloud_api.rs` 61 行已有 POST `/v2/titles/inputconfigs`，请求字段为 titleIds 与 titleIdType=xboxTitleId，保留服务返回 JSON。
- 冻结官方源码 `../2026-11-12-tak-layout-and-engine/source-excerpts/tak-core-protocol.txt` 360–369 行按 supportedTabs 的 manifestVersion/layoutVersion 筛选，630–663 行逐个调用 fetchTouchAdaptationBundle(bundle.id)，再展开 TAKX。
- 历史官方宿主调用链见 `docs/isu/2026-11-12-tak-compatibility-feasibility.md`：fetchTouchAdaptationBundle(id) → client.getTouchAdaptationBundle(user,id)。当前公共层继续待补下载 API 和真实认证返回样本。
- 完整布局文件消费还需 manifest、Context/Patch、definitions/$ref、locale/DPI 图片及能力覆盖；当前直接 Layout/社区资源进入同一解释器。
- Forza 社区索引用 Xbox Title ID 2030093255，经 better-xcloud 的 title index → layout 文件读取；仓库两套布局原件继续保留。

后续执行：沿公共 Rust 输入配置取得真实 supportedTabs，核对当前认证下 bundle 下载返回形式，保存代表性原件与能力矩阵，再实现自动加载和缓存。当前采样验收覆盖已有离线布局。
