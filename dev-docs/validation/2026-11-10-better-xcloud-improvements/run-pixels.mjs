import { spawn } from 'node:child_process'
import { createRequire } from 'node:module'
import fs from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const validation = path.dirname(fileURLToPath(import.meta.url))
const root = path.resolve(validation, '../../..')
const scratch = await fs.mkdtemp(path.join(os.tmpdir(), 'xbxrc-sampling-pixels-'))
const requireVite = createRequire(import.meta.resolve('vite'))
const { build } = await import(requireVite.resolve('esbuild'))
const constants = new Map()
const tuning = await fs.readFile(path.join(root, 'src/player/infra/render/sharpening-tuning.ts'), 'utf8')
for (const [, name, expression] of tuning.matchAll(/export const (\w+) = ([\d. /]+)/g)) {
  const [numerator, denominator = 1] = expression.trim().split('/').map(Number)
  constants.set(name, numerator / denominator)
}
const sharpenSource = (source, template = false) => {
  const shader = template ? source : source.match(/gl\.shaderSource\(fShader, `([\s\S]*?)`\)/)?.[1]
  if (!shader)
    throw new Error('Production sharpen shader template missing')
  const expanded = shader.replace(/\$\{(\w+)\.toFixed\((\d+)\)\}/g, (_, name, precision) => {
    if (!constants.has(name))
      throw new Error(`Unknown shader constant: ${name}`)
    return constants.get(name).toFixed(Number(precision))
  })
  if (expanded.includes('${'))
    throw new Error('Unexpanded production shader expression')
  return expanded
}
const current = sharpenSource(await fs.readFile(path.join(root, 'src/player/infra/render/Renderers.ts'), 'utf8'))
const baseline = sharpenSource(await fs.readFile(path.join(validation, 'baseline-sharpen.frag.template.txt'), 'utf8'), true)
const oldEasu = path.join(scratch, 'baseline-easu.ts')
await fs.copyFile(path.join(validation, 'baseline-easu.txt'), oldEasu)
const imports = `
import { buildEasuFragmentSource } from ${JSON.stringify(path.join(root, 'src/player/infra/render/fsr1-reconstruction-pass.ts'))};
import { buildEasuFragmentSource as baselineEasu } from ${JSON.stringify(oldEasu)};
import { computeFsrEasuCon } from ${JSON.stringify(path.join(root, 'src/player/infra/render/fsr1-cpu.ts'))};
import { sharpenUsmPixel } from ${JSON.stringify(path.join(root, 'src/player/infra/render/sharpening-math.ts'))};
import { FULLSCREEN_VERTEX_SOURCE } from ${JSON.stringify(path.join(root, 'src/player/infra/render/fullscreen-triangle.ts'))};
const currentSharpen = ${JSON.stringify(current)};
const baselineSharpen = ${JSON.stringify(baseline)};
`
const result = await build({ stdin: { contents: imports + await fs.readFile(path.join(validation, 'fixture.js'), 'utf8'), resolveDir: root }, bundle: true, write: false, format: 'iife', platform: 'browser' })
const html = path.join(scratch, 'index.html')
await fs.writeFile(html, `<meta charset="utf-8"><pre id="result">pending</pre><script>${result.outputFiles[0].text.replaceAll('</script', '<\\/script')}</script>`)
if (process.argv.includes('--serve')) {
  const { createServer } = await import('node:http')
  const bytes = await fs.readFile(html)
  createServer((request,response) => { response.writeHead(200, {'Content-Type':'text/html'}); response.end(bytes) }).listen(1430,'127.0.0.1')
  console.log('Pixel fixture: http://127.0.0.1:1430')
  await new Promise(() => {})
}
const chrome = process.env.XBX_SAMPLING_CHROME ?? '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome'
const child = spawn(chrome, ['--headless=new', `--user-data-dir=${path.join(scratch, 'profile')}`, '--no-first-run', '--no-default-browser-check', '--disable-background-networking', '--use-angle=swiftshader', '--enable-unsafe-swiftshader', '--dump-dom', `file://${html}`], { stdio: ['ignore', 'pipe', 'pipe'] })
let stdout = ''
let stderr = ''
child.stdout.on('data', chunk => { stdout = (stdout + chunk.toString()).slice(-2000000) })
child.stderr.on('data', chunk => { stderr = (stderr + chunk.toString()).slice(-3000) })
const timer = setTimeout(() => child.kill('SIGTERM'), 20000)
const code = await new Promise((resolve, reject) => { child.once('exit', resolve); child.once('error', reject) }).finally(() => clearTimeout(timer))
await fs.writeFile(path.join(validation, 'pixel-debug.html'), stdout)
const payload = stdout.match(/<pre id="result">([\s\S]*?)<\/pre>/)?.[1]
if (code !== 0 || !payload || payload === 'pending')
  throw new Error(`Pixel probe failed (${code}): ${stderr}`)
const metrics = JSON.parse(payload.replaceAll('&quot;', '"').replaceAll('&lt;', '<').replaceAll('&gt;', '>').replaceAll('&amp;', '&'))
metrics.environment = { browser: 'headless Chromium', renderer: 'ANGLE SwiftShader', qualification: 'Actual WebGL2 shader compilation and RGBA8 readback; software GPU, performance and physical scanout require device evidence' }
await fs.writeFile(path.join(validation, 'pixels.metrics.json'), `${JSON.stringify(metrics, null, 2)}\n`)
console.log(JSON.stringify({pass:metrics.pass,cases:metrics.cases,maxCodeValue:metrics.maxCodeValue,error:metrics.error,environment:metrics.environment}, null, 2))
if (!metrics.pass)
  process.exitCode = 1
