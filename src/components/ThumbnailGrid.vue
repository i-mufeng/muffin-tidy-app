<template>
  <div ref="containerRef" class="grid-container" tabindex="-1" @wheel="onWheel">
    <div v-if="rows.length === 0" class="grid-empty">
      <div class="empty-art" aria-hidden="true">
        <svg v-if="store.viewFilter === 'removed'" viewBox="0 0 120 92" role="presentation">
          <path class="art-muted" d="M35 27h50l-4 52H39l-4-52Z" />
          <path class="art-muted" d="M30 27h60M48 27v-7h24v7M51 40v25M60 40v25M69 40v25" />
          <path class="art-accent" d="m24 18 72 62" />
        </svg>
        <svg v-else-if="store.viewFilter === 'marked'" viewBox="0 0 120 92" role="presentation">
          <path class="art-muted" d="M19 19h82v56H19z" />
          <path class="art-muted" d="m28 64 18-19 13 12 10-10 17 17" />
          <path class="art-accent fill" d="m60 15 5.4 11 12.1 1.8-8.8 8.6 2.1 12.1L60 42.8l-10.8 5.7 2.1-12.1-8.8-8.6L54.6 26 60 15Z" />
        </svg>
        <svg v-else viewBox="0 0 120 92" role="presentation">
          <path class="art-accent" d="M22 36h30l8 8h38v30H22z" />
          <path class="art-muted" d="M22 36v-8h29l7 8M33 53h38M33 62h24" />
          <circle class="art-dot" cx="87" cy="64" r="7" />
          <path class="art-check" d="m83 64 3 3 6-7" />
        </svg>
      </div>
      <h2>{{ emptyTitle }}</h2>
      <p>{{ emptyCopy }}</p>
      <div class="empty-actions">
        <button v-if="store.viewFilter !== 'all'" class="empty-btn secondary" @click="store.setFilter('all')">查看全部</button>
        <button class="empty-btn" @click="store.refreshDirectory()">刷新目录</button>
      </div>
    </div>
    <VList v-else ref="listRef" :data="rows" style="height: 100%">
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

const emptyTitle = computed(() => {
  if (store.viewFilter === "marked") return "还没有标记的媒体";
  if (store.viewFilter === "removed") return "没有已移除的媒体";
  return "该目录暂无媒体文件";
});
const emptyCopy = computed(() =>
  store.viewFilter === "all" ? "选择其他目录，或刷新后重新扫描当前目录" : "切换筛选范围，继续整理你的媒体"
);

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

.grid-empty {
  height: 100%;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 10px;
  color: var(--text-secondary);
  text-align: center;
  padding: 24px;
}
.empty-art { width: 120px; height: 92px; opacity: 0.95; }
.empty-art svg { display: block; width: 100%; height: 100%; overflow: visible; }
.empty-art path, .empty-art circle { fill: none; stroke: #858585; stroke-width: 2; stroke-linecap: round; stroke-linejoin: round; }
.empty-art .art-muted { stroke: #666; }
.empty-art .art-accent { stroke: var(--accent); }
.empty-art .art-accent.fill { fill: var(--accent); stroke: var(--accent); }
.empty-art .art-dot { fill: var(--accent-dim); stroke: var(--accent); }
.empty-art .art-check { stroke: var(--accent); stroke-width: 2.5; }
.grid-empty h2 { margin: 2px 0 0; color: var(--text-primary); font-size: 16px; font-weight: 600; }
.grid-empty p { margin: 0; font-size: 12px; }
.empty-actions { display: flex; gap: 8px; margin-top: 8px; }
.empty-btn {
  border: 1px solid var(--accent);
  border-radius: 5px;
  background: var(--accent-dim);
  color: var(--accent);
  font-size: 12px;
  padding: 7px 12px;
  cursor: pointer;
}
.empty-btn:hover { background: var(--accent); color: #111; }
.empty-btn.secondary { border-color: var(--border); background: var(--bg-card); color: var(--text-secondary); }
.empty-btn.secondary:hover { color: var(--text-primary); background: var(--bg-card-hover); }

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
