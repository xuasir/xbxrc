# 仓库工具

在仓库根目录执行。发布和构建命令由 `package.json` 与 `.github/workflows/` 调用，各工具的内部依赖随调用链保留。

| 入口 | 用途 |
| --- | --- |
| `pnpm tauri dev` / `pnpm tauri build` | `with-ffmpeg-sdk.mjs` 装配当前 FFmpeg SDK 与本地运行库 |
| `pnpm release:check-bundle` | 检查 Tauri 包内运行库和许可证 |
| `pnpm release:check-collected` | 检查发布资产集合 |
| `pnpm release:verify-pipeline` | 校验 beta/stable 工作流与资产合同 |
| `pnpm skills:verify` / `skills:lock` / `skills:sync` | 本地技能哈希、锁文件与同步 |
| [browser_runtime_stability_report.py](browser_runtime_stability_report.py) | 浏览器共享观测、资源上限和退出清理报告 |
| [measure-browser-runtime.mjs](measure-browser-runtime.mjs) | 对固定基线计算 TypeScript AST 复杂度 |
| [official-xbox-capture.mjs](official-xbox-capture.mjs) | 官方 Xbox 网页的手动对照采样；需要本机 Chrome 与手动登录 |

`release-bundle-lib.mjs`、`release-collect-assets.mjs`、`release-set-version.mjs` 和 `release-write-latest-json.mjs` 是发布内部工具，随上述入口及 CI 使用。

```sh
python3 -B scripts/browser_runtime_stability_report.py TRACE
node scripts/measure-browser-runtime.mjs dev-docs/validation/browser-runtime-optimization-baseline.json
python3 -B -m unittest discover -s tests/runtime-trace -p 'test_*.py'
```

专项样本回放及输入说明见 [validation/](../dev-docs/validation/README.md)；常规运行日志分析见 [本地验收手册](../dev-docs/streaming-reliability-local-validation.md)。
