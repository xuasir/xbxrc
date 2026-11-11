# 云目录公开接口分析证据

日期：2026-11-11。对应 [分析文档](/Users/guo.xu/Documents/code/games/xbxrc/docs/isu/2026-11-11-cloud-catalog-api-and-ux.md)。

[public-api-probe.json](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-11-cloud-catalog-ux-analysis/public-api-probe.json) 保存公开请求合同、栏目 product ID 顺序、5 商品的 hydration 字段覆盖、响应体大小、简繁名称对照及部分详情字段。

核查范围：

- GET `catalog.gamepass.com/sigls/v2`：当前通用 newest、云专用 newest、云热门、即将离开与原生键鼠栏目。
- POST `catalog.gamepass.com/v3/products`：同 5 商品、`US/zh-TW` 下的 `RemoteLowJade0` 与 `RemoteHighSapphire0`；`US/zh-CN` 下的轻量商品资料。
- 请求采用 JSON、`ms-cv: 1.0` 和当前代码的 calling-app headers。公开查询具有独立读取性质。
- 带账户认证的 live titles、MRU、定向 title、inputconfigs 和 waittime 使用本地源码证据；有效账户及原生运行效果进入实机验收。

这批商品选择自此次通用 newest 返回的前 5 项。体积是所保存 JSON 响应体的字节数，服务延迟和用户网络传输性能分别需要专门测量。栏目 ID 的完整值、查询参数和产品样本保存在 JSON 中，可使用相同请求合同复核。

验证：证据 JSON 解析通过；分析文档的本地链接和行号范围检查通过。业务代码状态由当前工作区源码审计确认。
