import assert from 'node:assert/strict'
import { readFileSync, writeFileSync } from 'node:fs'
import { createRequire } from 'node:module'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { runInNewContext } from 'node:vm'
import os from 'node:os'
const folder = path.dirname(fileURLToPath(import.meta.url))
const root = path.resolve(folder, '../../..')
const require = createRequire(path.join(root, 'package.json'))
const ts = require('typescript')
const modules = new Map()
function load(filename) {
  if (modules.has(filename)) return modules.get(filename)
  const code = ts.transpileModule(readFileSync(filename,'utf8'), { compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 } }).outputText
  const output = { exports: {} }
  runInNewContext(code, { module: output, exports: output.exports, performance, setTimeout, clearTimeout, require: id => load(path.resolve(path.dirname(filename), `${id}.ts`)) }, { filename })
  modules.set(filename, output.exports)
  return output.exports
}
const before = load(path.join(folder,'catalog-model-before.ts'))
const after = load(path.join(root,'src/pages/xcloud/catalog-index.ts'))
const windows = load(path.join(root,'src/pages/xcloud/catalog-window.ts'))
const results = []
for (const size of [500,2000,10000]) {
  const titles = Array.from({ length: size }, (_,i)=> ({ id:`P${i}`, productId:`P${i}`, titleId:`stream${i}`, name:`云游戏 ${String(i).padStart(5,'0')}`, aliases:i===1?['Halo Infinite']:[], publisherName:'Xbox Studios', description:('探索、协作与冒险世界。完整描述供多词和分类搜索。 '.repeat(24))+i, categories:i%2?['角色扮演']:['动作冒险'], tileImageUrl:`https://store-images.s-microsoft.com/image/cover${i}`, posterImageUrl:'', supportedInputTypes:['Controller'], hasEntitlement:true, entitlementKnown:true, recentRank:i<25?i:null, newRank:i>=25&&i<55?i-25:null, popularRank:i>=55&&i<85?i-55:null, leavingRank:i>=85&&i<99?i-85:null, isRecentlyPlayed:i<25, isNew:i>=25&&i<55 }))
  const baseline = []
  for (let attempt=0; attempt<3; attempt++) { const start=performance.now(); before.queryTitles(titles,''); baseline.push(performance.now()-start) }
  const index = await after.buildCatalogIndex(titles)
  const queryTimes = []
  for (const query of ['', 'Halo infinite', '探索 协作']) {
    const start=performance.now()
    const result=await after.queryCatalogIndex(index,query)
    queryTimes.push({ query, ms:performance.now()-start })
    assert.deepEqual(Array.from(result,item=>item.productId), Array.from(before.queryTitles(titles,query),item=>item.productId))
  }
  const changed = [...titles]
  for (let i=0; i<75; i++) changed[i]={...titles[i], description: `${titles[i].description} 更新`}
  const incremental = await after.buildCatalogIndex(changed,index)
  assert.equal(incremental.metrics.normalized,75)
  const warm=await after.queryCatalogIndex(index,'')
  assert.equal(warm,index.orders.name)
  const fullBytes=Buffer.byteLength(JSON.stringify({ scopeKey:'anonymous:US:zh',titles,contentRevision:'r0',updatedAt:1,refreshing:true }))
  const patchBytes=Buffer.byteLength(JSON.stringify({ scopeKey:'anonymous:US:zh',titles:[],contentRevision:'r1',updatedAt:1,refreshing:true,patch:{baseRevision:'r0',upserts:changed.slice(0,75),removedProductIds:[]} }))
  const range=windows.catalogWindowRange(Math.ceil(size/6),190,2000,2450)
  results.push({ size, baselineEmptyQueryMs:baseline, index:index.metrics, incremental:incremental.metrics, queryTimes, warmArrayReused:warm===index.orders.name, fullEventBytes:fullBytes, patch75Bytes:patchBytes, gridWindowCards:windows.catalogWindowIndices(size,6,range,[size-2]).length, baselineVisitedGridCards:Math.min(size,2001) })
}
const result={ observedDate:'2026-11-12', environment:{ node:process.version,os:os.platform(),arch:os.arch(),cpu:os.cpus()[0].model }, method:'生产索引/查询/窗口模型与已保存旧模型；Node 时间片与 JSON 字节，DOM/原生窗口独立采样',results }
writeFileSync(path.join(folder,'scale-results.json'),`${JSON.stringify(result,null,2)}\n`)
process.stdout.write(`${JSON.stringify(result,null,2)}\n`)
