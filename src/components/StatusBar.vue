<template>
  <div class="status-bar">
    <span class="item">共 {{ stats.total }} 张</span>
    <span class="sep">·</span>
    <span class="item marked">★ 已标记 {{ stats.marked }}</span>
    <span class="sep">·</span>
    <span class="item removed">✕ 已移除 {{ stats.removed }}</span>
    <span class="spacer" />
    <span class="hint">Space 标记 · D/Del 移除 · Ctrl+Z 撤销 · E 导出</span>
    <button class="export-btn" @click="store.openExport()" title="导出整理结果 (E)">
      📤 导出
    </button>
  </div>
</template>

<script setup lang="ts">
import { storeToRefs } from "pinia";
import { useProjectStore } from "../stores/project";
const store = useProjectStore();
// storeToRefs 保留响应式：标记/移除后计数实时更新（直接解构会丢失响应式）
const { stats } = storeToRefs(store);
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
</style>
