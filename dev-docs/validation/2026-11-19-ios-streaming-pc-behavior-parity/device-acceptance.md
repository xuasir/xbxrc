# iOS 串流对齐实机验收

当前状态为`Validation Pending`，执行责任人为用户。代码、公共事实回放和指定自动回归已交付；实机按同分辨率、码率、网络环境和连续观察窗口，与PC main的Browser会话进行对照。

## 场景

- [ ] xCloud启动与设置：分别核对Auto/Custom及0值、四档分辨率和H.264档位；记录请求/协商与实际源尺寸、codec和码率。常规/分辨率保持两模式都观察Metadata，关联原始值、wire值和源尺寸变化。
- [ ] xHome启动与路径：待机主机分别使用唤醒开关；记录发现区域、授权区域与实际origin。核对直连优先、15秒初始连接期限、一次TURN回退及自定义TURN；记录selected pair的地址族、协议、host/srflx/relay和回退原因。
- [ ] 首帧与恢复：观察4秒首帧guard、短暂断网后的WebRTC自恢复、failed/closed后的恢复仲裁、关键帧/播放资源动作预算与120秒期限；以连续新解码/呈现帧收口，记录improved/neutral/degraded/unknown及统计缺值。
- [ ] 输入与菜单：双手柄选择/切换/断开、键盘组合、映射/死区/Y反转、600ms快捷键、触屏短按/多指/轴和四路震动。核对输入、公告和震动来自同一设备，菜单/失焦/后台后输入与触觉释放；在中英与深浅色下核对串流菜单、诊断、文本、Home及远端操作的状态和失败提示。
- [ ] 音频与生命周期：初始静音、菜单静音/音量控制、麦克风授权/拒绝/取消/关闭、同Peer聊天协商与上行bytes/packets。观察音频路由、Peer重建和前台恢复后的静音、麦克风、有效码率、新帧与进入前idle timer状态。
- [ ] 显示与长测：逐项核对比例、颜色/锐化、兼容显示和MetalFX能力/回退；保存参数切换前后截图，采集稳定段与异常前后窗。后台、锁屏和返回分别观察远端terminal、凭据续期、输入恢复和系统调度。

## 六维证据

| 维度 | 每份fresh xCloud/xHome trace应给出的事实 |
| --- | --- |
| 网络健康 | selected pair/路径、RTT、jitter、丢包、NACK/PLI/FIR、入包进度与连续覆盖 |
| 恢复链健康 | 触发事实、代次、仲裁、动作/预算、执行结果、后续新帧及效果窗口 |
| media supply | receive/decode/mailbox/submit/GPU完成计数、帧率及观测来源 |
| steady supply | 连续15秒窗口覆盖、健康段供给与压力后的稳定回归 |
| submit/present长尾 | submit-to-GPU-complete的P95/P99与frame interval；PC保留renderer/rVFC计时边界 |
| 帧供给delta | 原始累计计数的段间增量、丢弃/覆盖原因，以及decode/submit/完成供给差 |

iOS按decoder与GPU完成边界记录，PC按Browser/renderer边界记录；物理上屏和glass-to-glass由设备观测确认。结果关联源码哈希、目标、档位、设备、观察时段和trace采集时间，归档后回写RFC中的实机验收项及任务状态。
