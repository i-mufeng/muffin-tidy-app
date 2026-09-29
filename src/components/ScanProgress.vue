<template>
  <div class="scan-progress" :class="{ compact }">
    <div class="scan-label">{{ label }}</div>
    <progress :value="progress.stage === 'discovering' ? undefined : progress.current" :max="progress.total || 1" :aria-label="label" />
    <span class="scan-count">
      <template v-if="progress.stage === 'discovering'">已发现 {{ progress.current }} 个媒体</template>
      <template v-else>{{ progress.current }} / {{ progress.total }}（{{ percent }}%）</template>
    </span>
  </div>
</template>

<script setup lang="ts">
import { computed } from "vue";
import type { ScanProgress } from "../stores/project";

const props = defineProps<{ progress: ScanProgress; compact?: boolean }>();
const label = computed(() => ({
  discovering: "正在查找媒体文件…",
  metadata: "正在读取媒体信息…",
  pairing: "正在配对实况照片…",
  loading: "正在加载媒体列表…",
})[props.progress.stage]);
const percent = computed(() => props.progress.total
  ? Math.min(100, Math.round(props.progress.current / props.progress.total * 100)) : 0);
</script>

<style scoped>
.scan-progress { display: flex; flex-direction: column; align-items: center; gap: 14px; width: 100%; }
.scan-label { font-size: 16px; font-weight: 600; color: var(--text-primary); }
.scan-count { font-size: 12px; color: var(--text-secondary); font-variant-numeric: tabular-nums; }
progress { appearance: none; width: 100%; height: 8px; border: none; border-radius: 4px; overflow: hidden; background: var(--bg-card); accent-color: var(--accent); }
progress::-webkit-progress-bar { background: var(--bg-card); border-radius: 4px; }
progress::-webkit-progress-value { background: var(--accent); border-radius: 4px; }
progress:indeterminate { background: linear-gradient(90deg, var(--bg-card) 25%, var(--accent) 50%, var(--bg-card) 75%); background-size: 200% 100%; animation: scan-slide 1.5s linear infinite; }
progress:indeterminate::-webkit-progress-bar { background: transparent; }
.compact { flex-direction: row; flex-wrap: wrap; gap: 10px; width: auto; padding: 8px 12px; background: var(--bg-panel); border-bottom: 1px solid var(--border); }
.compact .scan-label { font-size: 12px; font-weight: 400; }
.compact progress { flex: 1; min-width: 80px; max-width: 240px; }
@keyframes scan-slide { to { background-position: -200% 0; } }
@media (prefers-reduced-motion: reduce) { progress:indeterminate { animation: none; } }
</style>
