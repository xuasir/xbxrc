---
name: monorepo
description: Monorepo setup with pnpm workspaces, centralized aliases, and Turborepo. Use when creating or managing multi-package repositories.
---

# Monorepo Setup

## Workspace Layout

Use pnpm workspaces for monorepo management:

```yaml
# pnpm-workspace.yaml
packages:
  - 'packages/*'
  - 'docs'
  - 'playground/*'
```

## Tooling Stack

- Package management and workspaces: `pnpm` workspace via `pnpm-workspace.yaml`
- Dependency version mapping: `catalogs` in `pnpm-workspace.yaml`
- Build and run: `tsdown` for build, `tsx` for local execution
- Testing: `vitest`
- Quality and conventions: `eslint` + `lint-staged`
- Git hooks: `simple-git-hooks`
- Versioning and release: `bumpp` + `pnpm publish`
- Docs: `vitepress` under `docs`

## Scripts Convention

Have scripts in each package, and use `-r` (recursive) at root. Enable ESLint cache for faster linting.

```json
// root package.json
{
  "scripts": {
    "build": "pnpm -r --filter \"./packages/*\" build",
    "test": "vitest",
    "lint": "eslint . --cache --concurrency=auto",
    "typecheck": "tsc --noEmit",
    "docs": "pnpm -C docs run docs:dev",
    "docs:build": "pnpm -C docs run docs:build",
    "version": "bumpp -r",
    "release": "pnpm -r --filter \"./packages/*\" publish",
  }
}
```

In each package's `package.json`, add the scripts and exports.

```json
// packages/*/package.json
{
  "main": "./dist/index.mjs",
  "module": "./dist/index.mjs",
  "types": "./dist/index.d.mts",
  "files": ["dist"],
  "scripts": {
    "dev": "tsdown --watch",
    "build": "tsdown",
    "prepack": "pnpm build"
  }
}
```

## Core Workflows

Development:
1. `pnpm dev` (recursive `dev`, typically `tsdown --watch`)
2. `pnpm -C docs run docs:dev` (docs site dev)

Debug:
1. `pnpm -C packages/<pkg> run dev` (run with `watch` mode)

Test and checks:
1. `pnpm test` (`vitest`)
2. `pnpm lint` (`eslint . --cache --concurrency=auto`)
3. `pnpm typecheck` (`tsc --noEmit`)

Build:
1. `pnpm build` (recursive `build`)
2. `pnpm docs:build` (docs build)

Release:
1. `pnpm version` (`bumpp -r` for unified version bump)
2. `pnpm release` (recursive `publish`)

## Tool Roles

- `pnpm`: workspace management, recursive execution, dependency hoisting and lockfile; use `-r`, `-C`, and `--filter`
- `catalogs`: centralize dependency versions to avoid drift
- `tsdown`: package build entry; output `dist` ESM artifacts
- `tsx`: local run/debug entry for CLI/tooling packages
- `vitest`: test runner with TS/ESM compatibility
- `eslint`: static checks; `lint-staged` auto-fix pre-commit
- `simple-git-hooks`: lightweight Git hooks, currently `pre-commit`
- `bumpp`: unified version bump across packages
- `vite`: dev/build foundation used by `tsdown` and `vitepress`
- `vitepress`: docs dev/build/preview

## Notes

- Recursive runs: `pnpm -r` runs in topological order to respect dependencies
- Filtered runs: `pnpm --filter` targets a package or subset to reduce cost
- Multi-package dev: prefer `pnpm dev` to keep all packages in watch mode
- Publish order: rely on `pnpm -r` to handle dependency order
- CI gates: run `lint`, `typecheck`, `test`, then `build`

## ESLint Cache

```json
{
  "scripts": {
    "lint": "eslint . --cache --concurrency=auto"
  }
}
```

## Turborepo (Optional)

For monorepos with many packages or long build times, use Turborepo for task orchestration and caching.

See the dedicated Turborepo skill for detailed configuration.

## Centralized Alias

For better DX across Vite, Nuxt, Vitest configs, create a centralized `alias.ts` at project root:

```ts
// alias.ts
import fs from 'node:fs'
import { fileURLToPath } from 'node:url'
import { join, relative } from 'pathe'

const root = fileURLToPath(new URL('.', import.meta.url))
const r = (path: string) => fileURLToPath(new URL(`./packages/${path}`, import.meta.url))

export const alias = {
  '@myorg/core': r('core/src/index.ts'),
  '@myorg/utils': r('utils/src/index.ts'),
  '@myorg/ui': r('ui/src/index.ts'),
  // Add more aliases as needed
}

// Auto-update tsconfig.alias.json paths
const raw = fs.readFileSync(join(root, 'tsconfig.alias.json'), 'utf-8').trim()
const tsconfig = JSON.parse(raw)
tsconfig.compilerOptions.paths = Object.fromEntries(
  Object.entries(alias).map(([key, value]) => [key, [`./${relative(root, value)}`]]),
)
const newRaw = JSON.stringify(tsconfig, null, 2)
if (newRaw !== raw)
  fs.writeFileSync(join(root, 'tsconfig.alias.json'), `${newRaw}\n`, 'utf-8')
```

Then update the `tsconfig.json` to use the alias file:

```json
{
  "extends": [
    "./tsconfig.alias.json"
  ]
}
```

### Using Alias in Configs

Reference the centralized alias in all config files:

```ts
// vite.config.ts
import { alias } from './alias'

export default defineConfig({
  resolve: { alias },
})
```

```ts
// nuxt.config.ts
import { alias } from './alias'

export default defineNuxtConfig({
  alias,
})
```
