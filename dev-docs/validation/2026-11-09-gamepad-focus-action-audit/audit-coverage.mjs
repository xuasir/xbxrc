import fs from 'node:fs'
import path from 'node:path'
import process from 'node:process'
import { fileURLToPath } from 'node:url'
import { parse } from 'vue/compiler-sfc'

const auditRoot = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(auditRoot, '../../..')
const output = { vueFiles: 0, focusableSites: 0, defaultContentsSites: [], nativeSelectSites: [], rawInteractiveSites: [], rawActivationSites: [], pointerBackdropSites: [] }

function prop(node, name) {
  const item = node.props.find(item => (item.type === 6 && item.name === name) || (item.type === 7 && item.name === 'bind' && item.arg?.content === name))
  return item?.value?.content ?? item?.exp?.content
}
function on(node, name) {
  return node.props.some(item => item.type === 7 && item.name === 'on' && item.arg?.content === name)
}
function walk(node, file, owners = []) {
  if (node.type === 1) {
    const site = { file, line: node.loc.start.line, tag: node.tag, as: prop(node, 'as'), id: prop(node, 'id') }
    const focusable = node.tag === 'Focusable'
    if (focusable) {
      output.focusableSites++
      if (!site.as)
        output.defaultContentsSites.push(site)
      if (site.as === 'select')
        output.nativeSelectSites.push({ ...site, navAction: on(node, 'nav-action'), change: on(node, 'change') })
    }
    if (['button', 'input', 'select', 'textarea', 'a', 'video'].includes(node.tag)) {
      output.rawInteractiveSites.push({ ...site, directMarker: prop(node, 'data-focusable'), focusableOwner: owners.at(-1)?.id, focusSync: on(node, 'focus'), navAction: on(node, 'nav-action'), controls: node.props.some(item => item.type === 6 && item.name === 'controls'), class: prop(node, 'class') })
    }
    const activations = ['click', 'dblclick', 'pointerdown', 'pointerup', 'mousedown', 'touchstart', 'touchend'].filter(event => on(node, event))
    if (node.tagType === 0 && activations.length)
      output.rawActivationSites.push({ ...site, activations, directMarker: prop(node, 'data-focusable'), focusableOwner: owners.at(-1)?.id, class: prop(node, 'class') })
    if (['div', 'button'].includes(node.tag) && on(node, 'click') && /backdrop|modal-shell/.test(prop(node, 'class') ?? ''))
      output.pointerBackdropSites.push(site)
    if (focusable)
      owners = [...owners, site]
  }
  for (const child of node.children ?? [])
    walk(child, file, owners)
}
function scan(folder) {
  for (const entry of fs.readdirSync(folder, { withFileTypes: true })) {
    const full = path.join(folder, entry.name)
    if (entry.isDirectory()) {
      scan(full)
    }
    else if (entry.name.endsWith('.vue')) {
      output.vueFiles++
      const { descriptor, errors } = parse(fs.readFileSync(full, 'utf8'))
      if (errors.length)
        throw new Error(`${full}: ${errors.join(', ')}`)
      if (descriptor.template?.ast)
        walk(descriptor.template.ast, path.relative(repoRoot, full))
    }
  }
}
scan(path.join(repoRoot, 'src'))
fs.writeFileSync(path.join(auditRoot, 'coverage.json'), `${JSON.stringify(output, null, 2)}\n`)
process.stdout.write(`${JSON.stringify(output, null, 2)}\n`)
