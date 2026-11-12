<script setup lang="ts">
import type { CloudTitle } from '@/shared/rpc/cloud'
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import GameCard from '@/components/common/GameCard.vue'
import { cloudNode, entitlement } from './catalog-model'

const props = defineProps<{ title: CloudTitle, section: string, favorite: boolean, available: boolean, playBusy: boolean, meta: string, imageSize: number }>()
const emit = defineEmits<{ select: [title: CloudTitle], play: [title: CloudTitle], favorite: [title: CloudTitle] }>()
const { t } = useI18n()
const imageSources = computed(() => [props.title.posterImageUrl])
const badge = computed(() => !props.available ? t('xcloudUx.catalogMissing') : entitlement(props.title) === 'denied' ? t('xcloudUx.accessRequired') : '')
function select() {
  emit('select', props.title)
}
function play() {
  emit('play', props.title)
}
function toggleFavorite() {
  emit('favorite', props.title)
}
</script>

<template>
  <GameCard :id="cloudNode(section, title.productId)" scope-id="app.shell" :data-cloud-product="title.productId" :title="title.name" :image-url="title.tileImageUrl || title.posterImageUrl" :image-sources="imageSources" :image-size="imageSize" :actions="true" :favorite="favorite" :play-busy="playBusy" :play-disabled="entitlement(title) === 'denied'" :play-label="t('xcloudPage.actions.playSelected', { name: title.name })" :favorite-label="t(favorite ? 'xcloudUx.unfavorite' : 'xcloudUx.favorite')" :meta="meta" :badge="badge" :aria-label="t('xcloudUx.detailsFor', { name: title.name })" @select="select" @play="play" @favorite="toggleFavorite" />
</template>
