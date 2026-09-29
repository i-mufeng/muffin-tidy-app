<template>
  <div class="status-bar">
    <span class="item">共 {{ stats.total }} 张</span>
    <span class="sep">·</span>
    <span class="item marked">★ 已标记 {{ stats.marked }}</span>
    <span class="sep">·</span>
    <span class="item removed">✕ 已移除 {{ stats.removed }}</span>
    <div class="filters" role="tablist" aria-label="媒体筛选">
      <button v-for="tab in tabs" :key="tab.key" class="filter-tab" :class="{ active: store.viewFilter === tab.key }" @click="store.setFilter(tab.key)">
        {{ tab.label }} <b>{{ tab.count }}</b>
      </button>
    </div>
    <span class="spacer" />
    <span class="hint">Space 标记 · D/Del 移除 · Ctrl+Z 撤销 · E 导出</span>
    <button class="export-btn" :disabled="store.exportableFiles.length === 0" @click="store.openExport()" title="导出整理结果 (E)">
      📤 导出
    </button>
  </div>
</template>

<script setup lang="ts">
import { storeToRefs } from "pinia";
import { computed } from "vue";
import { useProjectStore } from "../stores/project";
const store = useProjectStore();
// storeToRefs 保留响应式：标记/移除后计数实时更新（直接解构会丢失响应式）
const { stats } = storeToRefs(store);
const tabs = computed(() => [
  { key: "all" as const, label: "全部", count: store.exportableFiles.length },
  { key: "marked" as const, label: "已标记", count: stats.value.marked },
  { key: "removed" as const, label: "已移除", count: stats.value.removed },
]);
</script>

<style scoped>
.status-bar {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 0 12px;
  height: 28px;
  background: var(--bg-panel);
  border-top: 1px solid var(--border);
  font-size: 11px;
  color: var(--text-secondary);
  flex-shrink: 0;
}
.sep { color: #444; }
.marked { color: var(--marked); }
.removed { color: #666; }
.spacer { flex: 1; }
.hint { color: #444; font-size: 10px; margin-right: 10px; }

.filters { display: flex; align-items: center; gap: 2px; margin-left: 10px; }
.filter-tab {
  border: 1px solid transparent;
  border-radius: 4px;
  background: transparent;
  color: #777;
  font-size: 10px;
  padding: 3px 6px;
  cursor: pointer;
}
.filter-tab b { color: #aaa; font-weight: 500; margin-left: 2px; }
.filter-tab:hover { color: var(--text-primary); background: var(--bg-card); }
.filter-tab.active { color: var(--accent); border-color: rgba(245, 158, 11, 0.35); background: var(--accent-dim); }
.filter-tab.active b { color: var(--accent); }

.export-btn {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  height: 20px;
  padding: 0 10px;
  background: var(--accent);
  color: #000;
  border: none;
  border-radius: 4px;
  font-size: 11px;
  font-weight: 600;
  cursor: pointer;
  transition: filter 0.15s;
}
.export-btn:hover { filter: brightness(1.1); }
.export-btn:disabled { opacity: 0.35; cursor: not-allowed; filter: none; }
</style>
