# PC 执行域隔离规划证据

本轮完成源码可行性与具体实施计划，工程状态为planned。主线为Main控制/采集、独立Reader、独立Render和直接MessagePort帧链；诊断与Input Worker按成本/宿主能力分阶段接入。

- [RFC](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-11-26-pc-streaming-execution-domains.md)。
- [规划Report](/Users/guo.xu/Documents/code/games/xbxrc/docs/reports/2026-11-26-pc-streaming-execution-domains-plan.md)。
- 前序依据：[iOS启发评估](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-26-pc-ios-threading-assessment/README.md)、[完整上传恢复](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-26-pc-teximage-frame-reuse/README.md)。

成本评估明确区分当前Reader→Window与计划Reader→Render，两者帧链均为1次transfer/1次ACK；控制、统计和必要反馈的额外消息纳入完整成本账。VideoFrame移交对象所有权，浏览器内部与GPU导入的实际成本由M0测量。

已有实机证据覆盖Reader Worker、轨道和VideoFrame转移以及主线程texImage2D导入。当前类型库声明MessagePort、OffscreenCanvas和Worker RTCDataChannel；新跨Worker/WebGL组合及目标宿主channel转移仍需M0/M4实测。规划冒烟验证源证据、合同与文档引用，结果见 `checks.json`；源码身份见 `source-manifest.json`。

M0–M4的工程模块、所有权、帧上限、stop/context/截图、成本门和默认开放条件已记录。现有PC局部恢复/音频/反馈代码与iOS共享合同按实施时的当前工作树接续；工程性能与实际上屏资格保持各自能力口径。

本轮仅交付规划文档与检查摘要，临时文件创建数为0。建议先执行M0的同档基线与平台原型。
