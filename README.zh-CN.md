# xbxrc

[English README](./README.md)

`xbxrc` 是一个基于 Electron、Vue 3 与 TypeScript 的桌面端 Xbox 串流客户端。当前活跃代码库集中在 `src/main`、`src/preload`、`src/renderer`，后续开发以这三个目录为准。

项目围绕 Xbox Remote Play 与 xCloud 场景持续演进，重点完善认证、设备发现、云游戏目录、串流播放、输入适配与配置管理，同时持续提升代码质量与可维护性。

## 当前能力

- Microsoft / Xbox 账号认证与会话状态管理
- Xbox 主机发现与 Remote Play 入口流程
- xCloud 游戏目录拉取、预热与列表浏览
- 统一的串流播放页与会话控制流程
- 键盘、手柄与空间导航支持
- 码率、分辨率、音频、震动、键位映射等本地配置
- Electron 主进程、预加载层、渲染层的分层架构

## 技术栈

- Electron
- Vue 3
- TypeScript
- electron-vite
- vue-router
- vue-i18n
- `@spatial-navigation/vue`
- `xbox-webapi`

## 目录结构

```text
src/
  main/      Electron 主进程、认证、数据、串流、配置等模块
  preload/   向渲染层安全暴露的桥接 API
  renderer/  Vue 3 界面、页面、组件、播放器与交互逻辑
  shared/    RPC 协议、事件与共享类型
docs/
  project-task.md   当前活跃任务跟踪单
```

## 本地开发

### 环境要求

- Node.js `22.19.1`
- pnpm `10.30.2`

推荐版本通过 `package.json` 中的 `volta` 字段固定。

### 安装依赖

```bash
pnpm install
```

### 常用命令

```bash
pnpm dev
pnpm lint
pnpm typecheck
pnpm build
```

- `pnpm dev`：启动 Electron 与渲染层开发环境
- `pnpm lint`：执行 ESLint 检查
- `pnpm typecheck`：分别检查 node 与 web 目标的 TypeScript 类型
- `pnpm build`：先执行类型检查，再构建生产包

## 开发说明

- 活跃开发目录仅限 `src/main`、`src/preload`、`src/renderer`
- 任务跟踪统一维护在 [docs/project-task.md](./docs/project-task.md)
- 优先保证代码可读性、模块边界与长期可维护性
- 代码中的轻量注释以中文为主

## 致谢

本项目参考并致敬以下开源项目与作者的工作：

- [Geocld/XStreamingDesktop](https://github.com/Geocld/XStreamingDesktop)
- [unknownskl/greenlight](https://github.com/unknownskl/greenlight)
- [unknownskl/xbox-webapi-node](https://github.com/unknownskl/xbox-webapi-node)

这些项目在 Xbox 桌面串流实现、协议探索与生态工具建设方面提供了重要参考。具体版权与许可证说明请以上游仓库为准。

## License

本仓库采用 MIT License，详见 [LICENSE](./LICENSE)。
