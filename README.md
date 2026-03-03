# xbxrc

[中文版本](/Users/guo.xu/Documents/code/games/XStreamingDesktop/README.zh-CN.md)

`xbxrc` is a desktop Xbox streaming client built with Electron, Vue 3, and TypeScript. The active codebase lives in `src/main`, `src/preload`, and `src/renderer`, and ongoing development is limited to those directories.

The project focuses on Xbox Remote Play and xCloud scenarios, with continued investment in authentication, device discovery, cloud catalog browsing, streaming playback, input adaptation, and configuration management, while improving code quality and maintainability.

## Current Capabilities

- Microsoft / Xbox account authentication and session state management
- Xbox console discovery and Remote Play entry flow
- xCloud title fetching, warmup, and catalog browsing
- Unified streaming page and session control flow
- Keyboard, gamepad, and spatial navigation support
- Local settings for bitrate, resolution, audio, rumble, and input mapping
- Layered architecture across Electron main, preload, and renderer processes

## Tech Stack

- Electron
- Vue 3
- TypeScript
- electron-vite
- vue-router
- vue-i18n
- `@spatial-navigation/vue`
- `xbox-webapi`

## Repository Layout

```text
src/
  main/      Electron main process, auth, data, streaming, and config modules
  preload/   Secure bridge APIs exposed to the renderer
  renderer/  Vue 3 UI, pages, components, player, and interaction logic
  shared/    RPC contracts, events, and shared types
docs/
  project-task.md   Active task tracker
```

## Local Development

### Requirements

- Node.js `22.19.1`
- pnpm `10.30.2`

The recommended versions are pinned through the `volta` field in `package.json`.

### Install Dependencies

```bash
pnpm install
```

### Common Commands

```bash
pnpm dev
pnpm lint
pnpm typecheck
pnpm build
```

- `pnpm dev`: start Electron and the renderer in development mode
- `pnpm lint`: run ESLint checks
- `pnpm typecheck`: run TypeScript checks for both node and web targets
- `pnpm build`: run type checks and build the production bundle

## Development Notes

- Active development is limited to `src/main`, `src/preload`, and `src/renderer`
- Task tracking is maintained in [docs/project-task.md](/Users/guo.xu/Documents/code/games/XStreamingDesktop/docs/project-task.md)
- Readability, module boundaries, and maintainability take priority
- Light code comments are expected to be written in Chinese

## Acknowledgements

This project pays tribute to and has been informed by the following open source work:

- [Geocld/XStreamingDesktop](https://github.com/Geocld/XStreamingDesktop)
- [unknownskl/greenlight](https://github.com/unknownskl/greenlight)
- [unknownskl/xbox-webapi-node](https://github.com/unknownskl/xbox-webapi-node)

These projects provided important reference points for Xbox desktop streaming implementations, protocol exploration, and ecosystem tooling. Please refer to each upstream repository for its own license terms and notices.

## License

This repository is released under the MIT License. See [LICENSE](/Users/guo.xu/Documents/code/games/XStreamingDesktop/LICENSE) for details.
