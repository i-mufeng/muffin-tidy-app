<template>
  <div class="workspace">
    <!-- 主区域 -->
    <div ref="mainRef" class="main-area" :class="{ dragging }">
      <ThumbnailGrid
        ref="gridRef"
        class="grid-area"
        @preview="store.openViewer($event)"
      />

      <!-- 可拖拽分隔条：调节右侧预览框宽度（双击复位） -->
      <div
        class="splitter"
        :class="{ dragging }"
        title="拖动调节宽度，双击复位"
        @pointerdown="startDrag"
        @pointermove="onDrag"
        @pointerup="endDrag"
        @pointercancel="endDrag"
        @dblclick="resetWidth"
      />

      <PreviewPanel class="preview-area" :style="{ width: previewWidth + 'px' }" />
    </div>

    <StatusBar />

    <!-- 大图浏览器（双击缩略图触发） -->
    <Lightbox />

    <!-- 导出对话框 -->
    <ExportDialog />
  </div>
</template>

<script setup lang="ts">
import { ref, watch } from "vue";
import { useElementSize, useLocalStorage } from "@vueuse/core";
import { useProjectStore } from "../stores/project";
import { useKeyboard } from "../composables/useKeyboard";
import ThumbnailGrid from "../components/ThumbnailGrid.vue";
import PreviewPanel from "../components/PreviewPanel.vue";
import StatusBar from "../components/StatusBar.vue";
import Lightbox from "../components/Lightbox.vue";
import ExportDialog from "../components/ExportDialog.vue";

const store = useProjectStore();
const gridRef = ref<InstanceType<typeof ThumbnailGrid> | null>(null);

useKeyboard(() => gridRef.value?.getCols() ?? 5);

// —— 右侧预览框可拖拽调宽 ——
const DEFAULT_W = 240;
const MIN_W = 200;
const mainRef = ref<HTMLElement | null>(null);
const { width: mainW } = useElementSize(mainRef);
const previewWidth = useLocalStorage("mtidy.previewWidth", DEFAULT_W);

// 上限随窗口自适应：网格至少保留 320px（mainW 未测量时用兜底值，避免初始误夹紧）
function maxW() {
  return Math.max(MIN_W, Math.min(600, (mainW.value || 1280) - 320));
}
function clampW(v: number) {
  return Math.min(maxW(), Math.max(MIN_W, Math.round(v)));
}
previewWidth.value = clampW(previewWidth.value);

const dragging = ref(false);
let startX = 0;
let startW = 0;

function startDrag(e: PointerEvent) {
  dragging.value = true;
  startX = e.clientX;
  startW = previewWidth.value;
  (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
  e.preventDefault();
}
function onDrag(e: PointerEvent) {
  if (!dragging.value) return;
  // 向左拖 = 加宽
  previewWidth.value = clampW(startW - (e.clientX - startX));
}
function endDrag(e: PointerEvent) {
  if (!dragging.value) return;
  dragging.value = false;
  (e.currentTarget as HTMLElement).releasePointerCapture?.(e.pointerId);
}
function resetWidth() {
  previewWidth.value = clampW(DEFAULT_W);
}

// 窗口缩小时回收超界宽度
watch(mainW, () => {
  previewWidth.value = clampW(previewWidth.value);
});
</script>

<style scoped>
.workspace {
  display: flex;
  flex-direction: column;
  height: 100%;
  overflow: hidden;
}

.main-area {
  display: flex;
  flex: 1;
  min-height: 0;
  overflow: hidden;
}
.main-area.dragging { cursor: col-resize; }
.main-area.dragging .grid-area,
.main-area.dragging .preview-area { pointer-events: none; }

.grid-area {
  flex: 1;
  min-width: 0;
  overflow: hidden;
}

/* 分隔条：1px 实线 + 7px 透明命中区 */
.splitter {
  position: relative;
  flex: 0 0 1px;
  width: 1px;
  background: var(--border);
  cursor: col-resize;
  touch-action: none;
  transition: background 0.15s;
}
.splitter::after {
  content: "";
  position: absolute;
  top: 0;
  bottom: 0;
  left: -3px;
  right: -3px;
  z-index: 5;
}
.splitter:hover,
.splitter.dragging { background: var(--accent); }

.preview-area {
  flex-shrink: 0;
}
</style>
