<template>
  <div ref="containerRef" class="grid-container" tabindex="-1" @wheel="onWheel">
    <VList ref="listRef" :data="rows" style="height: 100%">
      <template #default="{ item: row, index: rowIndex }">
        <div class="grid-row" :style="rowStyle">
          <ThumbnailCard
            v-for="(file, col) in row"
            :key="file ? file.id : `empty-${rowIndex}-${col}`"
            v-bind="file ? {
              file,
              focused: store.focusedIndex === rowIndex * cols + col,
            } : { file: null as any, focused: false }"
            @focus="file && store.setFocus(rowIndex * cols + col)"
            @preview="file && $emit('preview', rowIndex * cols + col)"
          />
        </div>
      </template>
    </VList>

    <!-- 缩放档位提示（缩放后短暂浮现再淡出） -->
    <Transition name="zoom-pill">
      <div v-if="showPill" class="zoom-pill">{{ cols }} 列 · {{ thumbSize }}px</div>
    </Transition>
  </div>
</template>

<script setup lang="ts">
import { ref, computed, watch, onUnmounted } from "vue";
import { VList } from "virtua/vue";
import { useElementSize, useLocalStorage } from "@vueuse/core";
import { useProjectStore } from "../stores/project";
import ThumbnailCard from "./ThumbnailCard.vue";
import type { ProjectFile } from "../stores/project";

defineEmits<{ preview: [index: number] }>();

const store = useProjectStore();
const containerRef = ref<HTMLElement | null>(null);
const listRef = ref<InstanceType<typeof VList> | null>(null);
const { width } = useElementSize(containerRef);

const GAP = 4;
const MIN_THUMB = 96;
const MAX_THUMB = 320;

// 缩略图边长（持久化），决定列数；连续可变 → 缩放平滑
const thumbSize = useLocalStorage("mtidy.thumbSize", 160);
thumbSize.value = clamp(thumbSize.value);

function clamp(v: number) {
  return Math.min(MAX_THUMB, Math.max(MIN_THUMB, Math.round(v)));
}

const cols = computed(() =>
  Math.max(2, Math.floor((width.value + GAP) / (thumbSize.value + GAP)))
);

// 按列数分组为行（虚拟滚动以行为单位）
const rows = computed<(ProjectFile | null)[][]>(() => {
  const result: (ProjectFile | null)[][] = [];
  const all = store.visibleFiles;
  const c = cols.value;
  for (let i = 0; i < all.length; i += c) {
    const row: (ProjectFile | null)[] = all.slice(i, i + c);
    while (row.length < c) row.push(null);
    result.push(row);
  }
  return result;
});

// 固定 px 单元格 + 居中：缩放时单元格连续放大/缩小，不再整体跳档
const rowStyle = computed(() => ({
  display: "grid",
  gridTemplateColumns: `repeat(${cols.value}, ${thumbSize.value}px)`,
  justifyContent: "center",
  gap: `${GAP}px`,
  padding: `0 ${GAP}px`,
  marginBottom: `${GAP}px`,
}));

function scrollFocusedIntoView() {
  const rowIdx = Math.floor(store.focusedIndex / cols.value);
  listRef.value?.scrollToIndex?.(rowIdx, { align: "nearest" });
}

watch(() => store.focusedIndex, scrollFocusedIntoView);

// —— Ctrl/⌘ + 滚轮 连续缩放（同时支持触控板捏合） ——
let pendingDelta = 0;
let rafId = 0;
const showPill = ref(false);
let pillTimer: ReturnType<typeof setTimeout> | null = null;

function applyZoom() {
  rafId = 0;
  if (pendingDelta === 0) return;
  // deltaY 向上为负 → 放大
  thumbSize.value = clamp(thumbSize.value - pendingDelta * 0.2);
  pendingDelta = 0;
  scrollFocusedIntoView();
  showPill.value = true;
  if (pillTimer) clearTimeout(pillTimer);
  pillTimer = setTimeout(() => (showPill.value = false), 900);
}

function onWheel(e: WheelEvent) {
  if (!(e.ctrlKey || e.metaKey)) return; // 普通滚动交给 VList
  e.preventDefault();
  pendingDelta += e.deltaY;
  if (!rafId) rafId = requestAnimationFrame(applyZoom);
}

onUnmounted(() => {
  if (rafId) cancelAnimationFrame(rafId);
  if (pillTimer) clearTimeout(pillTimer);
});

defineExpose({ getCols: () => cols.value });
</script>

<style scoped>
.grid-container {
  position: relative;
  width: 100%;
  height: 100%;
  outline: none;
  overflow: hidden;
}

.zoom-pill {
  position: absolute;
  right: 12px;
  bottom: 12px;
  padding: 4px 10px;
  background: rgba(0, 0, 0, 0.72);
  border: 1px solid var(--border);
  border-radius: 6px;
  font-size: 12px;
  color: var(--text-primary);
  font-variant-numeric: tabular-nums;
  pointer-events: none;
  backdrop-filter: blur(4px);
}
.zoom-pill-enter-active,
.zoom-pill-leave-active {
  transition: opacity 0.2s ease, transform 0.2s ease;
}
.zoom-pill-enter-from,
.zoom-pill-leave-to {
  opacity: 0;
  transform: translateY(4px);
}
</style>
