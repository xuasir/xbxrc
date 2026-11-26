# 定向修正记录

## 保存与资源策略

首轮 iOS 单元 15/16 通过。`testMissingImagesAndDeferredActionsKeepRunnableFallback` 已获得可运行的编译结果，保存时触发 `SavedTouchLayoutsStore` 中的 `choice.issues.isEmpty` 门禁。统一保存、卡片与串流选择器的判定后，该用例成功保存并重新加载通用回退布局。

Rust 本机库首轮 14/16 通过。原始快照损坏与装饰图片损坏的两处断言误置；修正后分别验证来源完整性失败和图片降级成功，并保留同版本重存修复共享资源的验证。两项定向补验通过。

## 旋转与退出

首轮和第一次 UI 补验均未完成保存状态断言。UI 录像显示预览仍在显示，底层卡片的 accessibility existence 已可查询；该条件不足以证明预览退出。保留[诊断截图](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-26-ios-tak-protocol-engine/failure-preview-return.png)。

用例改为等待正式 Surface 的方向状态，横屏验收后回到竖屏核对返回按钮，并验证预览关闭按钮消失。随后滚动至社区卡片，核对“已保存到本机”和“重新保存”。UI 定向轮次通过，最终协议轮次再次通过。

## 合法数值被收窄

首份整库诊断显示 23 处数值回退：22 处 `actionThreshold` 为 2 或 2.5，1 处 `deadzone.threshold` 为 -0.3。官方 schema 规定 `ActionThreshold` 具有非负下限，`DeadzoneThreshold` 为数值；冻结运行时缺省动作阈值为 0，并对带符号死区执行偏移公式。

将这些输入数值交由生成合同解释，同时保留手柄输出的有限值检查与限幅。新增 Rust/Swift 能力用例及 7 项官方原方法对照通过。最终整库恢复这 23 处原始语义，完整兼容数量由 52 增至 70，85 份布局持续可运行。
