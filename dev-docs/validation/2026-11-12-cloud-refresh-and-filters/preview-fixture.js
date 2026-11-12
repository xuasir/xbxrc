import '/dev-docs/validation/2026-11-12-cloud-menu-performance-implementation/browser-fixture.js'
const output = document.getElementById('cloud-refresh-observations')
const observations = []
let previous = ''
function measure() {
  const page = document.querySelector('.xcloud-page')
  if (!page) return
  const rect = element => element ? { left: element.getBoundingClientRect().left, top: element.getBoundingClientRect().top, right: element.getBoundingClientRect().right, bottom: element.getBoundingClientRect().bottom, width: element.getBoundingClientRect().width } : null
  const current = {
    viewport: { width: innerWidth, height: innerHeight },
    category: rect(document.getElementById('xcloud.filter.category')),
    sort: rect(document.getElementById('xcloud.filter.sort')),
    search: rect(document.querySelector('.xcloud-page__search-shell')),
    refresh: rect(document.getElementById('xcloud.refresh')),
    refreshBusy: document.getElementById('xcloud.refresh')?.getAttribute('aria-busy'),
    categoryRail: (() => {
      const rail = document.querySelector('.xcloud-page__categories')
      return rail ? { left: rail.getBoundingClientRect().left, right: rail.getBoundingClientRect().right, scrollLeft: rail.scrollLeft, clientWidth: rail.clientWidth, scrollWidth: rail.scrollWidth, rows: [...new Set([...rail.querySelectorAll('button')].map(button => Math.round(button.getBoundingClientRect().top)))], buttons: [...rail.querySelectorAll('button')].map(button => ({id: button.id, ...rect(button)})) } : null
    })(),
    categories: [...document.querySelectorAll('.xcloud-page__jumps nav, .xcloud-page__jumps [id^="xcloud.jump."]')].map(element => ({id: element.id, label: element.textContent.trim()})),
    sections: [...page.querySelectorAll('[data-nav-section]')].map(element => element.dataset.navSection),
    overflow: document.documentElement.scrollWidth > innerWidth || page.scrollWidth > page.clientWidth,
    selectedCategory: document.getElementById('xcloud.filter.category')?.textContent.trim(),
    selectedSort: document.getElementById('xcloud.filter.sort')?.textContent.trim(),
  }
  const signature = JSON.stringify(current)
  if (signature !== previous) { observations.push({atMs: Math.round(performance.now()), ...current}); previous = signature }
  output.textContent = JSON.stringify({current, observations}, null, 2)
}
new MutationObserver(changes => { if (changes.some(change => change.target !== output && !output.contains(change.target))) requestAnimationFrame(measure) }).observe(document.body, {subtree:true, childList:true, attributes:true, attributeFilter:['aria-busy']})
addEventListener('resize', measure)
setInterval(measure, 100)
