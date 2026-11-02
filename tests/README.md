# 仓库级日志回归

`runtime-trace/` 统一承载仓库 Python 门禁测试，`fixtures/streaming-session-lifecycle/` 保存通过与失败场景，场景合同由 `cases.json` 指定。

```sh
python3 -B -m unittest discover -s tests/runtime-trace -p 'test_*.py'
```

- `test_browser_runtime_stability_report.py` 验证共享观测、代际隔离、资源预算、诊断成本、CLI 返回码与退出清理；调用 `scripts/browser_runtime_stability_report.py`。
- `test_streaming_session_lifecycle_gate.py` 验证启动、取消、恢复授权/效果、代际、drain、超时和资源关闭；调用 `.agents/skills/analyze-runtime-logs/scripts/trace_streaming_session_lifecycle_gate.py`。可用 `STREAMING_LIFECYCLE_GATE_SCRIPT` 指定待验证的脚本。

前端模块回归与源码共置，通过 `pnpm test` 执行；Rust 回归位于对应 crate，通过 `cargo test -p <crate>` 执行。skill 自身测试保留在 `.agents/skills/analyze-runtime-logs/tests/`。实机验证顺序见 [本地验收手册](../dev-docs/streaming-reliability-local-validation.md)。
