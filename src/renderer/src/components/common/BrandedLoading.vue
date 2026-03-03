<script setup lang="ts">
defineProps<{
  label: string
}>()
</script>

<template>
  <div class="branded-loading" role="status" aria-live="polite">
    <span class="branded-loading__visual" aria-hidden="true">
      <span class="branded-loading__orbit"></span>
      <span class="branded-loading__logo"></span>
    </span>
    <span class="branded-loading__label">{{ label }}</span>
  </div>
</template>

<style scoped>
.branded-loading {
  display: inline-flex;
  flex-direction: column;
  align-items: center;
  gap: var(--ui-loading-gap);
  width: min(100%, var(--ui-loading-width));
  height: auto;
  font-size: var(--ui-loading-font-size);
  font-weight: var(--ui-font-weight-semibold);
  color: var(--ui-page-text);
}

.branded-loading__visual {
  position: relative;
  width: var(--ui-loading-visual-size);
  height: var(--ui-loading-visual-size);
  display: inline-flex;
  align-items: center;
  justify-content: center;
}

.branded-loading__orbit {
  position: absolute;
  inset: 0;
  border-radius: 50%;
  background: radial-gradient(
    circle,
    transparent 52%,
    color-mix(in srgb, var(--ui-login-button-bg) 10%, transparent) 64%,
    color-mix(in srgb, var(--ui-login-button-bg) 52%, transparent) 72%,
    color-mix(in srgb, var(--ui-login-button-bg) 18%, transparent) 82%,
    transparent 100%
  );
}

.branded-loading__logo {
  position: relative;
  z-index: 1;
  width: 50%;
  height: 50%;
  background-color: transparent;
  background: var(--ui-page-text);
  mask: url('../../assets/nav/xbox-logo.svg') center / contain no-repeat;
  -webkit-mask: url('../../assets/nav/xbox-logo.svg') center / contain no-repeat;
  animation: branded-loading-logo 1.8s ease-in-out infinite;
  filter: drop-shadow(0 0 calc(var(--ui-space-sm) * 1.1) rgba(255, 255, 255, 0.22))
    drop-shadow(
      0 0 calc(var(--ui-space-sm) * 2.2)
        color-mix(in srgb, var(--ui-login-button-bg) 18%, transparent)
    );
}

.branded-loading__label {
  text-align: center;
  line-height: var(--ui-line-height-tight);
}

@keyframes branded-loading-logo {
  0%,
  100% {
    transform: scale(1);
    opacity: 0.92;
  }

  50% {
    transform: scale(1.1);
    opacity: 1;
  }
}
</style>
