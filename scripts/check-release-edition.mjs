#!/usr/bin/env node
import { readReleaseIdentity, sourceCommit, verifyBrowserSource, verifyReleaseSource } from './release-edition.mjs'

try {
  const identity = readReleaseIdentity()
  if (process.argv[2] && process.argv[2] !== identity.edition)
    throw new Error(`预期 edition=${process.argv[2]}，实际=${identity.edition}`)
  if (!process.argv.includes('--dependencies-only'))
    verifyReleaseSource(identity)
  const commit = sourceCommit()
  if (identity.edition === 'browser' && !process.argv.includes('--source-only'))
    verifyBrowserSource()
  console.log(JSON.stringify({ ...identity, sourceCommit: commit }, null, 2))
}
catch (error) {
  console.error(error instanceof Error ? error.message : error)
  process.exitCode = 1
}
