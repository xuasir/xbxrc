import crypto from 'node:crypto'
import fs from 'node:fs'
import path from 'node:path'
import ts from 'typescript'

const baselinePath = process.argv[2] ?? 'dev-docs/validation/browser-runtime-optimization-baseline.json'
const baseline = JSON.parse(fs.readFileSync(baselinePath, 'utf8'))

function collectFiles(root) {
  return fs.readdirSync(root, { withFileTypes: true }).flatMap(entry => {
    const file = path.join(root, entry.name)
    return entry.isDirectory() ? collectFiles(file) : file.endsWith('.ts') && !file.endsWith('.test.ts') ? [file] : []
  })
}

function measure(file) {
  const source = fs.readFileSync(file, 'utf8')
  const ast = ts.createSourceFile(file, source, ts.ScriptTarget.Latest, true)
  let statements = 0
  let branches = 0
  let runtimeDirectMutableBindings = 0
  const visit = (node) => {
    if (ts.isStatement(node)) statements++
    if (ts.isIfStatement(node) || ts.isConditionalExpression(node) || ts.isCaseClause(node)
      || ts.isForStatement(node) || ts.isWhileStatement(node) || ts.isDoStatement(node)) branches++
    if (ts.isFunctionDeclaration(node) && node.name?.text === 'createBrowserRuntime') {
      runtimeDirectMutableBindings = node.body.statements.reduce((count, statement) => count
        + (ts.isVariableStatement(statement) && (statement.declarationList.flags & ts.NodeFlags.Let) ? statement.declarationList.declarations.length : 0), 0)
    }
    ts.forEachChild(node, visit)
  }
  visit(ast)
  return { path: file, lines: source.split('\n').length - (source.endsWith('\n') ? 1 : 0), statements, branches, runtimeDirectMutableBindings, sha256: crypto.createHash('sha256').update(source).digest('hex') }
}

function totals(files) {
  return files.reduce((result, file) => ({ files: result.files + 1, lines: result.lines + file.lines, statements: result.statements + file.statements, branches: result.branches + file.branches }), { files: 0, lines: 0, statements: 0, branches: 0 })
}

const files = [...new Set([...baseline.files.map(file => file.path), ...collectFiles('src/player'), ...collectFiles('src/streaming/runtime')])].filter(file => fs.existsSync(file)).sort().map(measure)
const report = { baselineRevision: baseline.revision, measurement: 'TypeScript AST statements; branches = if/conditional/case/classic loops; scope includes all added production files', baseline: totals(baseline.files), current: totals(files), files }
if (process.argv[3]) fs.writeFileSync(process.argv[3], `${JSON.stringify(report, null, 2)}\n`)
console.log(JSON.stringify({ baseline: report.baseline, current: report.current, runtimeDirectMutableBindings: files.find(file => file.path.endsWith('/browser-runtime.ts'))?.runtimeDirectMutableBindings }, null, 2))
