#!/usr/bin/env node
import { execFileSync } from 'node:child_process'
import { readReleaseIdentity, repositoryRoot } from './release-edition.mjs'

if (readReleaseIdentity().edition !== 'native-wip')
  throw new Error('原生 feature 门禁归属 rust / Native WIP')
for (const features of ['', 'native-media', 'native-gamepad', 'native-wip']) {
  const args = ['tree', '-p', 'xbxrc', '--no-default-features', '--locked', '--offline', '--edges', 'normal', '--prefix', 'none']
  if (features) args.push('--features', features)
  const tree = execFileSync('cargo', args, { cwd: repositoryRoot, encoding: 'utf8' })
  const media = /^(?:xbxengine|ffmpeg-sys-next|wgpu|cpal|rtc)\s/m
  const gamepad = /^ohmygamepad-sdl3\s/m
  const expectsMedia = features === 'native-media' || features === 'native-wip'
  const expectsGamepad = features === 'native-gamepad' || features === 'native-wip'
  if (media.test(tree) !== expectsMedia || gamepad.test(tree) !== expectsGamepad)
    throw new Error(`原生依赖归属错误: ${features || 'none'}`)
  if (!expectsMedia && /^(?:ffmpeg\S*|wgpu\S*|cpal|opus\S*|rtc(?:-\S*)?)\s/m.test(tree))
    throw new Error(`${features || 'none'} 引入媒体专用依赖`)
  if (!expectsGamepad && /^sdl\S*\s/m.test(tree))
    throw new Error(`${features || 'none'} 引入手柄专用依赖`)
  console.log(`ok ${features || 'none'}: media=${expectsMedia}, gamepad=${expectsGamepad}`)
}
