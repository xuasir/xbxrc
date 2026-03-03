---
name: setting-up
description: Project setup files including .gitignore and VS Code extensions. Use when initializing new projects or adding CI/editor config.
---

# Project Setup

## .gitignore

Create when `.gitignore` is not present:

```
*.log
*.tgz
.cache
.DS_Store
.eslintcache
.idea
.env
.nuxt
.temp
.output
.turbo
cache
coverage
dist
lib-cov
logs
node_modules
temp
```

## VS Code Extensions

Configure in `.vscode/extensions.json`:

```json
{
  "recommendations": [
    "dbaeumer.vscode-eslint",
    "antfu.pnpm-catalog-lens",
    "vue.volar"
  ]
}
```

| Extension | Description |
|-----------|-------------|
| `dbaeumer.vscode-eslint` | ESLint integration for linting and formatting |
| `antfu.pnpm-catalog-lens` | Shows pnpm catalog version hints inline |
| `vue.volar` | Vue Language Features |
