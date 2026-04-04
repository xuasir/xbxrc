<script setup lang="ts">
interface Props {
  label?: string
  size?: 'xs' | 'sm' | 'md' | 'lg' | 'xl'
}

withDefaults(defineProps<Props>(), {
  label: '',
  size: 'md',
})
</script>

<template>
  <div class="branded-loading" :class="[`branded-loading--${size}`]" role="status" aria-live="polite">
    <div class="branded-loading__visual" aria-hidden="true">
      <!-- 外圈发光环 -->
      <div class="branded-loading__aura" />
      
      <!-- 旋转进度环 -->
      <div class="branded-loading__ring" />
      
      <!-- 核心 Logo: 呼吸灯效果 -->
      <div class="branded-loading__logo-shell">
        <div class="branded-loading__logo" />
      </div>
    </div>
    <span v-if="label" class="branded-loading__label">{{ label }}</span>
  </div>
</template>

<style scoped>
.branded-loading {
  display: inline-flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  user-select: none;
}

/* 尺寸精细定义 */
.branded-loading--xs { --visual-size: 32px; --logo-scale: 0.6; --font-size: 11px; --gap: 8px; --ring-width: 2px; }
.branded-loading--sm { --visual-size: 48px; --logo-scale: 0.6; --font-size: 12px; --gap: 12px; --ring-width: 2.5px; }
.branded-loading--md { --visual-size: 64px; --logo-scale: 0.6; --font-size: 13px; --gap: 16px; --ring-width: 3px; }
.branded-loading--lg { --visual-size: 96px; --logo-scale: 0.6; --font-size: 15px; --gap: 20px; --ring-width: 4px; }
.branded-loading--xl { --visual-size: 144px; --logo-scale: 0.6; --font-size: 17px; --gap: 24px; --ring-width: 5px; }

.branded-loading {
  gap: var(--gap);
}

.branded-loading__visual {
  position: relative;
  width: var(--visual-size);
  height: var(--visual-size);
  display: flex;
  align-items: center;
  justify-content: center;
}

/* 外圈柔和氛围灯 */
.branded-loading__aura {
  position: absolute;
  inset: -15%;
  border-radius: 50%;
  background: radial-gradient(
    circle,
    color-mix(in srgb, var(--brand-primary) 22%, transparent) 0%,
    color-mix(in srgb, var(--brand-primary) 8%, transparent) 50%,
    transparent 70%
  );
  animation: aura-pulse 3s ease-in-out infinite;
}

/* 旋转进度环 */
.branded-loading__ring {
  position: absolute;
  inset: 0;
  border-radius: 50%;
  padding: var(--ring-width); 
  background: conic-gradient(
    from 0deg,
    var(--brand-primary) 0deg,
    color-mix(in srgb, var(--brand-primary) 60%, transparent) 60deg,
    color-mix(in srgb, var(--brand-primary) 20%, transparent) 120deg,
    transparent 240deg
  );
  -webkit-mask:
    linear-gradient(var(--neutral-1000) 0 0) content-box,
    linear-gradient(var(--neutral-1000) 0 0);
  mask:
    linear-gradient(var(--neutral-1000) 0 0) content-box,
    linear-gradient(var(--neutral-1000) 0 0);
  -webkit-mask-composite: xor;
  mask-composite: exclude;
  animation: ring-rotate 1.2s cubic-bezier(0.4, 0, 0.2, 1) infinite;
}

.branded-loading__logo-shell {
  position: relative;
  width: 100%;
  height: 100%;
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 2;
}

.branded-loading__logo {
  width: calc(100% * var(--logo-scale));
  height: calc(100% * var(--logo-scale));
  background-color: var(--brand-primary);
  mask: url('../../assets/nav/xbox-logo.svg') center / contain no-repeat;
  -webkit-mask: url('../../assets/nav/xbox-logo.svg') center / contain no-repeat;
  animation: logo-breathe 2.5s ease-in-out infinite;
  filter: drop-shadow(0 0 12px color-mix(in srgb, var(--brand-primary) 40%, transparent));
}

.branded-loading__label {
  color: var(--ui-page-text);
  font-size: var(--font-size);
  font-weight: 700;
  letter-spacing: 0.04em;
  text-transform: uppercase;
  opacity: 0.8;
  animation: text-fade 2.5s ease-in-out infinite;
  text-align: center;
  max-width: 200px;
}

@keyframes ring-rotate {
  from { transform: rotate(0deg); }
  to { transform: rotate(360deg); }
}

@keyframes logo-breathe {
  0%, 100% {
    transform: scale(1);
    opacity: 0.8;
  }
  50% {
    transform: scale(1.08);
    opacity: 1;
  }
}

@keyframes aura-pulse {
  0%, 100% { transform: scale(1); opacity: 0.5; }
  50% { transform: scale(1.15); opacity: 0.9; }
}

@keyframes text-fade {
  0%, 100% { opacity: 0.5; }
  50% { opacity: 0.9; }
}
</style>
