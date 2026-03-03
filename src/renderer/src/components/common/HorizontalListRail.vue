<script setup lang="ts">
import { nextTick, onBeforeUnmount, onMounted, ref } from 'vue'

interface HorizontalListRailProps {
  title?: string
  hint?: string
  ariaLabel?: string
}

const props = withDefaults(defineProps<HorizontalListRailProps>(), {
  title: '',
  hint: '',
  ariaLabel: 'Horizontal list rail'
})

const scrollerRef = ref<HTMLElement | null>(null)
let focusObserver: MutationObserver | undefined

function scrollFocusedItemIntoView(): void {
  const scrollerElement = scrollerRef.value
  if (scrollerElement === null) {
    return
  }

  const focusedElement = scrollerElement.querySelector<HTMLElement>('[data-focused="true"]')
  if (focusedElement === null) {
    return
  }

  focusedElement.scrollIntoView({
    block: 'nearest',
    inline: 'nearest',
    behavior: 'smooth'
  })
}

async function setupFocusObserver(): Promise<void> {
  await nextTick()

  if (scrollerRef.value === null) {
    return
  }

  focusObserver?.disconnect()
  focusObserver = new MutationObserver(() => {
    scrollFocusedItemIntoView()
  })
  focusObserver.observe(scrollerRef.value, {
    subtree: true,
    attributes: true,
    attributeFilter: ['data-focused']
  })
  scrollFocusedItemIntoView()
}

onMounted(() => {
  void setupFocusObserver()
})

onBeforeUnmount(() => {
  focusObserver?.disconnect()
  focusObserver = undefined
})
</script>

<template>
  <section class="horizontal-list-rail" :aria-label="props.ariaLabel">
    <header v-if="props.title || props.hint" class="horizontal-list-rail__header">
      <div class="horizontal-list-rail__copy">
        <p v-if="props.title" class="horizontal-list-rail__title">{{ props.title }}</p>
        <p v-if="props.hint" class="horizontal-list-rail__hint">{{ props.hint }}</p>
      </div>
    </header>

    <div ref="scrollerRef" class="horizontal-list-rail__viewport">
      <div class="horizontal-list-rail__scroller">
        <slot />
      </div>
    </div>
  </section>
</template>

<style scoped>
.horizontal-list-rail {
  display: flex;
  flex-direction: column;
  gap: var(--ui-rail-gap);
  min-width: 0;
}

.horizontal-list-rail__header {
  display: flex;
  align-items: flex-end;
  justify-content: space-between;
  gap: 16px;
}

.horizontal-list-rail__copy {
  display: flex;
  flex-direction: column;
  gap: var(--ui-rail-copy-gap);
  min-width: 0;
}

.horizontal-list-rail__title {
  font-size: var(--ui-rail-title-size);
  font-weight: var(--ui-font-weight-bold);
  line-height: 1.1;
  color: var(--ui-page-text);
}

.horizontal-list-rail__hint {
  font-size: var(--ui-rail-hint-size);
  line-height: 1.4;
  color: var(--ui-page-text-soft);
}

.horizontal-list-rail__viewport {
  overflow-x: auto;
  overflow-y: hidden;
  padding:
    var(--ui-rail-padding-block-start)
    var(--ui-rail-padding-inline)
    var(--ui-rail-padding-block-end);
  scroll-behavior: smooth;
  scroll-padding-inline: var(--ui-rail-padding-inline);
  scrollbar-width: none;
  -ms-overflow-style: none;
}

.horizontal-list-rail__viewport::-webkit-scrollbar {
  display: none;
}

.horizontal-list-rail__scroller {
  display: flex;
  gap: var(--ui-rail-gap);
  min-width: max-content;
}
</style>
