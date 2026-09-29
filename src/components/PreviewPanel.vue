<template>
  <div ref="panelRef" class="preview-panel" :class="{ dragging }">
    <template v-if="file">
      <!-- 媒体区 -->
      <div class="media-area">
        <!-- Live/Motion 视频模式 -->
        <video
          v-if="(file.liveType || file.mediaType === 'vdo') && showVideo && file.videoUrl"
          :src="file.videoUrl"
          class="preview-media"
          autoplay
          loop
          muted
          playsinline
          controls
        />
        <!-- 图片 -->
        <img
          v-else
          :src="previewSrc"
          class="preview-media"
          draggable="false"
        />

        <!-- Live 播放切换按钮 -->
        <button
          v-if="file.liveType || file.mediaType === 'vdo'"
          class="live-toggle"
          @click="showVideo = !showVideo"
        >
          {{ showVideo ? '🖼 静图' : '▶ 动态' }}
        </button>
      </div>

      <!-- 可拖拽分隔条：调节信息区高度（双击复位） -->
      <div
        class="v-splitter"
        :class="{ dragging }"
        title="拖动调节高度，双击复位"
        @pointerdown="startDrag"
        @pointermove="onDrag"
        @pointerup="endDrag"
        @pointercancel="endDrag"
        @dblclick="resetHeight"
      />

      <!-- 文件信息 -->
      <div class="info-panel" :style="{ height: infoHeight + 'px' }">
        <div class="info-filename">{{ filename }}</div>
        <div class="info-row">
          <span class="info-label">时间</span>
          <span>{{ file.captureTime }}</span>
        </div>
        <div class="info-row">
          <span class="info-label">大小</span>
          <span>{{ formatSize(file.fileSize) }}</span>
        </div>
        <div class="info-row">
          <span class="info-label">类型</span>
          <span>{{ typeLabel }}</span>
        </div>
        <template v-for="(val, key) in file.exifInfo" :key="key">
          <div class="info-row">
            <span class="info-label">{{ key }}</span>
            <span class="info-val">{{ val }}</span>
          </div>
        </template>

        <!-- 操作按钮 -->
        <div class="actions">
          <button
            v-if="file.status === 'removed'"
            class="btn btn-restore"
            @click="store.restoreFile(store.focusedIndex)"
          >
            ↶ 恢复到工程
          </button>
          <button
            v-else
            class="btn"
            :class="file.status === 'marked' ? 'btn-active' : ''"
            @click="store.toggleMark(store.focusedIndex)"
          >
            ★ {{ file.status === 'marked' ? '已标记' : '标记' }}
          </button>
          <button
            v-if="file.status !== 'removed'"
            class="btn btn-danger"
            @click="store.removeFile(store.focusedIndex)"
          >
            ✕ 从工程移除
          </button>
        </div>
      </div>
    </template>

    <div v-else class="empty-hint">
      <svg viewBox="0 0 72 64" aria-hidden="true">
        <rect x="8" y="10" width="56" height="44" rx="4" />
        <circle cx="25" cy="25" r="4" />
        <path d="m15 47 14-13 9 8 7-7 12 12" />
      </svg>
      <strong>{{ store.files.length ? '选择一张媒体查看详情' : '暂无媒体可预览' }}</strong>
      <span>{{ store.files.length ? '点击缩略图或使用方向键选择' : '打开包含图片或视频的目录后开始整理' }}</span>
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref, computed, watch } from "vue";
import { useElementSize, useLocalStorage } from "@vueuse/core";
import { useProjectStore } from "../stores/project";

import { queueThumb, queuePreview } from "../composables/useThumbQueue";

const store = useProjectStore();
const file = computed(() => store.focusedFile);
const showVideo = ref(false);
const previewSrc = ref<string>("");

let previewRequest = 0;
watch(() => file.value?.sourcePath, (path, _previous, onCleanup) => {
  const request = ++previewRequest;
  const controller = new AbortController();
  let fullReady = false;
  showVideo.value = false;
  previewSrc.value = "";
  if (!path) return;
  const current = () => request === previewRequest && !controller.signal.aborted;
  queueThumb(path, controller.signal).then((src) => {
    if (current() && !fullReady) previewSrc.value = src;
  }).catch(() => {});
  const timer = setTimeout(() => {
    queuePreview(path, controller.signal).then((src) => {
      if (current()) { fullReady = true; previewSrc.value = src; }
    }).catch(() => { /* Retain the thumbnail if preview decoding fails. */ });
  }, 100);
  onCleanup(() => { clearTimeout(timer); controller.abort(); });
}, { immediate: true });

const filename = computed(() =>
  file.value?.sourcePath.split(/[\\/]/).pop() ?? ""
);

const typeLabel = computed(() => {
  if (!file.value) return "";
  const map = { img: "图片", vdo: "视频", lpo: "实况照片" };
  const live = file.value.liveType
    ? `（${file.value.liveType === "android" ? "Android Motion" : file.value.liveType === "apple" ? "iOS Live" : "动态"}）`
    : "";
  return map[file.value.mediaType] + live;
});

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

// —— 信息区可拖拽调高 ——
const DEFAULT_H = 360;
const MIN_H = 140;
const panelRef = ref<HTMLElement | null>(null);
const { height: panelH } = useElementSize(panelRef);
const infoHeight = useLocalStorage("mtidy.infoHeight", DEFAULT_H);

// 上限随面板自适应：媒体区至少保留 160px（panelH 未测量时用兜底值）
function maxH() {
  return Math.max(MIN_H, (panelH.value || 760) - 160);
}
function clampH(v: number) {
  return Math.min(maxH(), Math.max(MIN_H, Math.round(v)));
}
infoHeight.value = clampH(infoHeight.value);

const dragging = ref(false);
let startY = 0;
let startH = 0;

function startDrag(e: PointerEvent) {
  dragging.value = true;
  startY = e.clientY;
  startH = infoHeight.value;
  (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
  e.preventDefault();
}
function onDrag(e: PointerEvent) {
  if (!dragging.value) return;
  // 向上拖 = 加高
  infoHeight.value = clampH(startH - (e.clientY - startY));
}
function endDrag(e: PointerEvent) {
  if (!dragging.value) return;
  dragging.value = false;
  (e.currentTarget as HTMLElement).releasePointerCapture?.(e.pointerId);
}
function resetHeight() {
  infoHeight.value = clampH(DEFAULT_H);
}

// 面板高度变化（窗口缩放 / 拖宽）时回收超界高度
watch(panelH, () => {
  infoHeight.value = clampH(infoHeight.value);
});
</script>

<style scoped>
.preview-panel {
  display: flex;
  flex-direction: column;
  height: 100%;
  background: var(--bg-panel);
  overflow: hidden;
}
.preview-panel.dragging { cursor: ns-resize; }
.preview-panel.dragging .media-area,
.preview-panel.dragging .info-panel { pointer-events: none; }

.media-area {
  position: relative;
  flex: 1;
  min-height: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  background: #000;
}

.empty-hint {
  height: 100%;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 8px;
  padding: 24px;
  text-align: center;
  color: var(--text-secondary);
}
.empty-hint svg { width: 72px; height: 64px; margin-bottom: 4px; opacity: 0.7; }
.empty-hint svg rect, .empty-hint svg path, .empty-hint svg circle { fill: none; stroke: #666; stroke-width: 2; stroke-linecap: round; stroke-linejoin: round; }
.empty-hint strong { color: var(--text-primary); font-size: 13px; font-weight: 600; }
.empty-hint span { font-size: 11px; line-height: 1.45; }

.preview-media {
  max-width: 100%;
  max-height: 100%;
  object-fit: contain;
  display: block;
}

.live-toggle {
  position: absolute;
  bottom: 8px;
  right: 8px;
  background: rgba(0,0,0,0.6);
  color: #fff;
  border: none;
  border-radius: 4px;
  padding: 4px 10px;
  font-size: 12px;
  cursor: pointer;
}
.live-toggle:hover { background: rgba(0,0,0,0.85); }

/* 横向分隔条：1px 实线 + 7px 透明命中区 */
.v-splitter {
  position: relative;
  height: 1px;
  flex-shrink: 0;
  background: var(--border);
  cursor: ns-resize;
  touch-action: none;
  transition: background 0.15s;
}
.v-splitter::after {
  content: "";
  position: absolute;
  left: 0;
  right: 0;
  top: -3px;
  bottom: -3px;
  z-index: 5;
}
.v-splitter:hover,
.v-splitter.dragging { background: var(--accent); }

.info-panel {
  flex-shrink: 0;
  padding: 12px;
  font-size: 12px;
  color: var(--text-secondary);
  overflow-y: auto;
}

.info-filename {
  color: var(--text-primary);
  font-size: 13px;
  font-weight: 600;
  margin-bottom: 8px;
  word-break: break-all;
}

.info-row {
  display: flex;
  gap: 8px;
  padding: 2px 0;
}
.info-label {
  min-width: 48px;
  color: #555;
}
.info-val {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.actions {
  display: flex;
  gap: 6px;
  margin-top: 12px;
}

.btn {
  flex: 1;
  padding: 6px;
  border: 1px solid var(--border);
  background: var(--bg-card);
  color: var(--text-secondary);
  border-radius: 4px;
  font-size: 12px;
  cursor: pointer;
}
.btn:hover { background: var(--bg-card-hover); color: var(--text-primary); }
.btn-active { border-color: var(--marked); color: var(--marked); }
.btn-restore { border-color: var(--accent); color: var(--accent); }
.btn-danger:hover { border-color: #ef4444; color: #ef4444; }
.btn:disabled { opacity: 0.3; cursor: default; }

</style>
