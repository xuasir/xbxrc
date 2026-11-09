import fs from 'node:fs'
import path from 'node:path'
import process from 'node:process'
import { fileURLToPath } from 'node:url'
import { parse } from 'vue/compiler-sfc'

const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const nativeControls = new Set(['button', 'input', 'textarea', 'select', 'summary', 'details'])
const activationEvents = new Set(['click', 'dblclick', 'pointerdown', 'pointerup', 'mousedown', 'mouseup', 'touchstart', 'touchend'])
const componentActions = new Set([...activationEvents, 'select', 'refresh', 'back', 'submit', 'change', 'input', 'tabChange', 'rowConfirm', 'rowSelect', 'actionClick', 'toggle'])
const interactiveRoles = new Set(['button', 'link', 'checkbox', 'radio', 'switch', 'slider', 'spinbutton', 'combobox', 'tab', 'menuitem', 'menuitemcheckbox', 'menuitemradio'])
const components = new Map()
const violations = []
const sites = []
const normalize = tag => tag.replace(/-/g, '').toLowerCase()
function attr(node, name) {
  return node.props?.find(prop => (prop.type === 6 && prop.name === name) || (prop.type === 7 && prop.name === 'bind' && prop.arg?.content === name))
}
function value(node, name) {
  const prop = attr(node, name)
  return prop?.value?.content ?? prop?.exp?.content ?? (prop ? '' : undefined)
}
function events(node) {
  return node.props?.filter(prop => prop.type === 7 && prop.name === 'on').map(prop => ({ name: prop.arg?.content, handler: prop.exp?.content ?? '' })) ?? []
}
function walk(node, visitor, owners = []) {
  if (node.type === 1) {
    visitor(node, owners)
    owners = [...owners, node]
  }
  for (const child of node.children ?? []) walk(child, visitor, owners)
}
function scan(folder) {
  for (const entry of fs.readdirSync(folder, { withFileTypes: true })) {
    const full = path.join(folder, entry.name)
    if (entry.isDirectory()) scan(full)
    else if (entry.name.endsWith('.vue')) {
      const source = fs.readFileSync(full, 'utf8')
      const { descriptor, errors } = parse(source, { filename: full })
      if (errors.length) throw new Error(`${full}: ${errors.join(', ')}`)
      const nodes = []
      if (descriptor.template?.ast) walk(descriptor.template.ast, (node, owners) => nodes.push({ node, owners }))
      components.set(normalize(entry.name.slice(0, -4)), { full, source, nodes })
    }
  }
}
scan(path.join(repo, 'src'))
function containsNavigation(component, seen = new Set()) {
  if (!component || seen.has(component.full)) return false
  seen.add(component.full)
  return component.nodes.some(({ node }) => node.tag === 'Focusable' || containsNavigation(components.get(normalize(node.tag)), seen))
}
function issue(site, message) {
  violations.push({ ...site, message })
}
for (const component of components.values()) {
  const focusables = component.nodes.filter(({ node }) => node.tag === 'Focusable').map(({ node }) => node)
  for (const { node, owners } of component.nodes) {
    const handlers = events(node)
    const semantic = nativeControls.has(node.tag)
      || (node.tag === 'a' && attr(node, 'href'))
      || (['video', 'audio'].includes(node.tag) && attr(node, 'controls'))
      || interactiveRoles.has(value(node, 'role'))
      || (attr(node, 'tabindex') && value(node, 'tabindex') !== '-1')
      || (attr(node, 'contenteditable') && value(node, 'contenteditable') !== 'false')
    const activation = handlers.some(handler => (node.tagType === 0 ? activationEvents : componentActions).has(handler.name))
    if (node.tag !== 'Focusable' && !semantic && !activation) continue
    const site = { file: path.relative(repo, component.full), line: node.loc.start.line, tag: node.tag, id: value(node, 'id'), as: value(node, 'as') }
    if (node.tag === 'Focusable') {
      site.navigation = 'Focusable'
      if (!site.id) issue(site, '导航入口需要稳定 ID')
      if (!site.as) issue(site, '导航入口需要有几何边界的 as 元素')
      if (site.as === 'select') issue(site, '选择控件需要可确认的导航弹层')
      if (['input', 'textarea'].includes(site.as) && !handlers.some(handler => handler.name === 'nav-action'))
        issue(site, '输入入口需要 A 确认的编辑合同')
    }
    else if (attr(node, 'data-focusable') || owners.some(owner => owner.tag === 'Focusable')) {
      site.navigation = '共享导航节点'
    }
    else if (node.tagType === 1 && containsNavigation(components.get(normalize(node.tag)))) {
      site.navigation = '组件内 Focusable'
    }
    else if (/backdrop|^setting-modal-shell$/.test(value(node, 'class') ?? '')) {
      // 遮罩关闭共用弹层 B 返回和显式关闭按钮，避免将整屏遮罩加入几何候选。
      const closing = focusables.filter(focusable => {
        const back = value(focusable, 'on-back')
        return back && handlers.some(handler => handler.handler.includes(back))
      })
      if (closing.length) {
        site.navigation = 'B 返回与显式关闭入口'
        site.targets = closing.map(focusable => value(focusable, 'id'))
      }
      else issue(site, '遮罩操作需要对应的 B 返回或关闭入口')
    }
    else if (node.tag === 'main' && handlers.every(handler => ['touchstart', 'touchend'].includes(handler.name)) && component.source.includes('navigationEngine.onPageSwitch')) {
      site.navigation = 'LB/RB 切页与顶栏入口'
    }
    else issue(site, '交互元素需要几何导航入口')
    if (['video', 'audio'].includes(node.tag) && attr(node, 'controls'))
      issue(site, '媒体操作需要显式几何导航控件')
    sites.push(site)
  }
}
const report = { vueFiles: components.size, interactiveSites: sites.length, focusableSites: sites.filter(site => site.tag === 'Focusable').length, supportedSites: sites.filter(site => site.navigation).length, violations, sites }
const outputIndex = process.argv.indexOf('--output')
if (outputIndex >= 0) fs.writeFileSync(path.resolve(process.argv[outputIndex + 1]), `${JSON.stringify(report, null, 2)}\n`)
process.stdout.write(`${JSON.stringify({ ...report, sites: undefined }, null, 2)}\n`)
if (violations.length) process.exitCode = 1
