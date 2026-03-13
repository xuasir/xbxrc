<script setup lang="ts">
interface Props {
  label?: string
  size?: 'md' | 'lg' | 'xl'
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
      
      <!-- 旋转进度环 (Xbox 风格：带有渐变尾迹的圆环) -->
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
  gap: 24px;
  user-select: none;
}

/* 尺寸定义 */
.branded-loading--md { --visual-size: 80px; --logo-scale: 0.5; --font-size: 14px; }
.branded-loading--lg { --visual-size: 120px; --logo-scale: 0.55; --font-size: 16px; }
.branded-loading--xl { --visual-size: 180px; --logo-scale: 0.6; --font-size: 18px; }

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
    rgba(16, 124, 16, 0.15) 0%,
    rgba(16, 124, 16, 0.05) 50%,
    transparent 70%
  );
  animation: aura-pulse 3s ease-in-out infinite;
}

/* 旋转进度环：使用 conic-gradient 模拟流动感 */
.branded-loading__ring {
  position: absolute;
  inset: 0;
  border-radius: 50%;
  padding: 3px; /* 环的粗细 */
  background: conic-gradient(
    from 0deg,
    rgba(255, 255, 255, 0.8) 0deg,
    rgba(255, 255, 255, 0.4) 60deg,
    rgba(255, 255, 255, 0.1) 120deg,
    transparent 240deg
  );
  -webkit-mask: 
    linear-gradient(#fff 0 0) content-box, 
    linear-gradient(#fff 0 0);
  mask: 
    linear-gradient(#fff 0 0) content-box, 
    linear-gradient(#fff 0 0);
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
  background-color: #ffffff;
  mask: url('../../assets/nav/xbox-logo.svg') center / contain no-repeat;
  -webkit-mask: url('../../assets/nav/xbox-logo.svg') center / contain no-repeat;
  animation: logo-breathe 2.5s ease-in-out infinite;
  filter: drop-shadow(0 0 12px rgba(255, 255, 255, 0.4));
}

.branded-loading__label {
  color: var(--color-text-secondary);
  font-size: var(--font-size);
  font-weight: 600;
  letter-spacing: 0.05em;
  text-transform: uppercase;
  opacity: 0.8;
  animation: text-fade 2.5s ease-in-out infinite;
}

/* 动画定义 */

@keyframes ring-rotate {
  from { transform: rotate(0deg); }
  to { transform: rotate(360deg); }
}

@keyframes logo-breathe {
  0%, 100% {
    transform: scale(1);
    filter: drop-shadow(0 0 8px rgba(255, 255, 255, 0.3));
    opacity: 0.85;
  }
  50% {
    transform: scale(1.05);
    filter: drop-shadow(0 0 20px rgba(255, 255, 255, 0.6));
    opacity: 1;
  }
}

@keyframes aura-pulse {
  0%, 100% { transform: scale(1); opacity: 0.5; }
  50% { transform: scale(1.2); opacity: 1; }
}

@keyframes text-fade {
  0%, 100% { opacity: 0.6; }
  50% { opacity: 0.9; }
}
</style>
