# iOS 信息优先级修正

主机图独立展示；存储摘要限制在主机信息侧的 320pt 内，显示已用、可用、总计和每设备短进度条。排序与存储筛选保持单行；相册媒体类型采用第一层直接选择的下拉菜单。游戏概览移除重复 Tab 入口，长简介默认收起；玩家页三个记录入口同行，游戏相册空状态在剩余内容区居中。

[最新 iPad 预览](preview.jpg)包含主机与局部存储、媒体类型下拉和空状态。[来源](manifest.json)对应固定测试账户截图。

验证证据：

- 最新批注方案：手机、iPad 各 4 项 UI 通过，覆盖图片/存储边界、单行筛选、菜单直接选择、简介展开/收起、玩家入口、相册空态、返回恢复。结果包为 `/tmp/xbxrc-ios-priority-revised-phone.xcresult`、`/tmp/xbxrc-ios-priority-revised-ipad.xcresult`。
- 首轮手机 6 项 UI 和 84 张双语/多尺寸布局绘制通过，结果包 `/tmp/xbxrc-ios-priority-phone.xcresult`；最新主机图与相册下拉的全尺寸复验尚待完成。
- 存储汇总的完整容量、未知容量、可用值收敛和溢出保护：2 项单元测试通过，结果包 `/tmp/xbxrc-ios-priority-ipad.xcresult`。
- Swift 格式、pnpm lint:fix、串流边界及暂存/未暂存 diff 检查通过。Swift 格式保留既有 ForEach 改写建议，返回码为 0。
- 最新 iPhoneOS 无签名构建、全尺寸复验、手机截图导出被自动审批服务的 Azure 资源归属错误阻断；单独重试构建仍失败，命令尚未执行。修复审批服务后接续验证。
