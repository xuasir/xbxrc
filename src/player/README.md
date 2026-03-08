# Player Architecture

This directory contains the low-level local stream runtime implementation used by the renderer.
The active streaming feature now composes it via `src/renderer/src/streaming/runtime`.

## Entry Points

- `index.ts`: public export surface
- `api/PlayerClient.ts`: low-level local endpoint facade

## Layers

- `domain/`: pure models and runtime configs
- `protocol/`: channel handlers and packet encoders
- `infra/`: browser, WebRTC, rendering, and platform adapters
- `app/`: orchestration services for input, media, session, and stats
- `api/`: typed events and public facade

## Validation

- `pnpm exec vue-tsc --noEmit`
- `pnpm exec tsc -p tsconfig.node.json --noEmit`
