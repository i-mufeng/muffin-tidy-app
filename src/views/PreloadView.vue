<template>
  <div class="preload">
    <div class="card">
      <div class="dir">{{ store.sourceDir }}</div>

      <!-- 扫描阶段：尚不知道总数 -->
      <template v-if="store.phase === 'scanning'">
        <div class="logo spin">🔍</div>
        <div class="phase-text">正在扫描目录…</div>
        <div class="sub">识别图片 / 视频 / 实况照片</div>
      </template>

      <!-- 预热阶段：生成缩略图缓存，带进度条 -->
      <template v-else>
        <div class="logo">🖼️</div>
        <div class="phase-text">正在生成预览缓存</div>
        <div class="progress-track">
          <div class="progress-fill" :style="{ width: percent + '%' }" />
        </div>
        <div class="counter">
          {{ store.preload.done }} / {{ store.preload.total }}
          <span class="pct">({{ percent }}%)</span>
        </div>
        <div class="sub">已缓存的图片进入后可秒开</div>

        <div class="actions">
          <button class="btn-ghost" @click="store.reset()">← 返回</button>
          <button class="btn-skip" @click="store.skipPreload()">跳过，直接浏览</button>
        </div>
      </template>
    </div>
  </div>
</template>

<script setup lang="ts">
import { computed } from "vue";
import { useProjectStore } from "../stores/project";

const store = useProjectStore();

const percent = computed(() => {
  const { done, total } = store.preload;
  if (total === 0) return 0;
  return Math.round((done / total) * 100);
});
</script>

<style scoped>
.preload {
  display: flex;
  align-items: center;
  justify-content: center;
  height: 100%;
  background: var(--bg-base);
}

.card {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 14px;
  padding: 40px 48px;
  background: var(--bg-panel);
  border: 1px solid var(--border);
  border-radius: 12px;
  min-width: 420px;
}

.dir {
  font-size: 12px;
  color: var(--text-secondary);
  max-width: 380px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.logo { font-size: 44px; }
.spin { animation: spin 1.4s linear infinite; }
@keyframes spin { to { transform: rotate(360deg); } }

.phase-text {
  font-size: 16px;
  font-weight: 600;
  color: var(--text-primary);
}

.progress-track {
  width: 100%;
  height: 8px;
  background: var(--bg-card);
  border-radius: 4px;
  overflow: hidden;
}
.progress-fill {
  height: 100%;
  background: var(--accent);
  border-radius: 4px;
  transition: width 0.2s ease;
}

.counter {
  font-size: 13px;
  color: var(--text-primary);
  font-variant-numeric: tabular-nums;
}
.pct { color: var(--text-secondary); }

.sub {
  font-size: 12px;
  color: var(--text-secondary);
}

.actions {
  display: flex;
  gap: 10px;
  margin-top: 8px;
}
.btn-ghost, .btn-skip {
  padding: 7px 16px;
  border-radius: 6px;
  font-size: 13px;
  cursor: pointer;
  border: 1px solid var(--border);
}
.btn-ghost {
  background: none;
  color: var(--text-secondary);
}
.btn-ghost:hover { color: var(--text-primary); }
.btn-skip {
  background: var(--accent);
  color: #000;
  border-color: var(--accent);
  font-weight: 600;
}
.btn-skip:hover { filter: brightness(1.1); }
</style>
