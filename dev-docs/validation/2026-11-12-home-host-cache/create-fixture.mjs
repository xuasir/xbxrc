import { readFileSync, writeFileSync } from 'node:fs'

const base = new URL('../2026-11-12-cloud-menu-performance-implementation/browser-fixture.js', import.meta.url)
let source = readFileSync(base, 'utf8')
function replace(before, after) {
  if (source.split(before).length !== 2) throw new Error(`expected one fixture location: ${before}`)
  source = source.replace(before, after)
}
source = `import { createHostsFixture, readHomeEvidence } from './host-fixture.js'\n${source}`
replace("    route: location.hash,", "    route: location.hash,\n    home: readHomeEvidence(),\n    hostFixture: hostFixture.state(),")
replace("const games = Array.from", "const hostFixture = createHostsFixture({ emit, record, counters })\nconst games = Array.from")
replace("      case 'data.getHosts': case 'data.getRemoteConsoles': return hosts", "      case 'data.getRemoteConsoles': return hosts\n      case 'data.getHosts': case 'data.getHostsSnapshot': case 'data.refreshHosts': case 'data.powerOnConsole': case 'data.powerOffConsole': return await hostFixture.invoke(method, params)")
writeFileSync(new URL('./browser-fixture.js', import.meta.url), source)
writeFileSync(new URL('./cover.svg', import.meta.url), readFileSync(new URL('../2026-11-12-cloud-menu-performance-implementation/cover.svg', import.meta.url)))
writeFileSync(new URL('./index.html', import.meta.url), '<!doctype html><html lang="zh"><head><meta charset="UTF-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>主机缓存验证</title></head><body data-navigation-test-fixture="true"><div id="app"></div><pre id="project-validation-results" hidden></pre><script type="module">import "./browser-fixture.js"; location.hash = location.hash || "/xhome"; await import("/src/main.ts");</script></body></html>\n')
