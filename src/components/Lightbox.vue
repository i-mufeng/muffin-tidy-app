<template>
  <Teleport to="body">
    <Transition name="viewer">
      <div v-if="store.viewerOpen && file" class="viewer">
        <!-- 顶部条 -->
        <div class="v-top">
          <span class="counter">{{ store.focusedIndex + 1 }} / {{ len }}</span>
          <div class="top-actions">
            <button v-if="file.liveType" class="tbtn" @click="showVideo = !showVideo">
              {{ showVideo ? '🖼 静图' : '▶ 动态' }}
            </button>
            <button
              class="tbtn"
              :class="{ active: file.status === 'marked' }"
              @click="toggleMark"
              title="标记 (Space)"
            >★</button>
            <button class="tbtn" @click="close" title="关闭 (Esc)">✕</button>
          </div>
        </div>

        <!-- 主舞台：点击左右半区翻页 -->
        <div class="stage">
          <div class="stage-inner" @click="onStageClick">
            <Transition :name="dir >= 0 ? 'frame-next' : 'frame-prev'">
              <div class="frame" :key="store.focusedIndex">
                <!-- 缩略图垫底：瞬时可见，切换无空帧 -->
                <img
                  v-if="thumbAt(store.focusedIndex)"
                  class="layer base"
                  :src="thumbAt(store.focusedIndex)"
                  draggable="false"
                />
                <!-- 动态视频 -->
                <video
                  v-if="file.liveType && showVideo && file.videoUrl"
                  class="layer full ready"
                  :src="file.videoUrl"
                  autoplay
                  loop
                  muted
                  playsinline
                  controls
                />
                <!-- 原图：加载完成后淡入覆盖缩略图 -->
                <img
                  v-else-if="fullAt(store.focusedIndex)"
                  class="layer full"
                  :src="fullAt(store.focusedIndex)"
                  draggable="false"
                  @load="markReady"
                />
              </div>
            </Transition>

            <!-- 常驻标记角标 -->
            <Transition name="flag">
              <div v-if="file.status === 'marked'" class="mark-flag">★ 已标记</div>
            </Transition>

            <!-- 标记切换爆发动效 -->
            <div v-if="burst" :key="burst.key" class="mark-burst" :class="{ off: !burst.on }">
              <span class="burst-icon">{{ burst.on ? '★' : '☆' }}</span>
            </div>

            <!-- 左右翻页提示（悬停浮现，不拦截点击，到头隐藏） -->
            <div v-if="store.focusedIndex > 0" class="edge-hint left" aria-hidden="true">‹</div>
            <div v-if="store.focusedIndex < len - 1" class="edge-hint right" aria-hidden="true">›</div>
          </div>
        </div>

        <!-- 底部胶片条：以当前为中心滑动 -->
        <div ref="stripViewport" class="filmstrip">
          <div class="strip-track" :style="trackStyle">
            <button
              v-for="i in windowIndices"
              :key="visible[i].id"
              class="tile"
              :class="{ active: i === store.focusedIndex, marked: visible[i].status === 'marked' }"
              @click="goto(i)"
            >
              <img v-if="thumbAt(i)" :src="thumbAt(i)" draggable="false" />
              <span v-else class="tile-ph" />
            </button>
          </div>
        </div>

        <div class="caption">{{ filename }}</div>
      </div>
    </Transition>
  </Teleport>
</template>

<script setup lang="ts">
import { ref, computed, reactive, watch } from "vue";
import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { useElementSize, useEventListener } from "@vueuse/core";
import { useProjectStore, needsRustConvert } from "../stores/project";
import { queueThumb } from "../composables/useThumbQueue";

const store = useProjectStore();
const visible = computed(() => store.visibleFiles);
const len = computed(() => visible.value.length);
const file = computed(() => visible.value[store.focusedIndex] ?? null);
const filename = computed(() => file.value?.sourcePath.split(/[\\/]/).pop() ?? "");

const showVideo = ref(false);
const dir = ref(1);

// 资源缓存按路径键入，移除/重排后仍正确
const thumbByPath = reactive<Record<string, string>>({});
const fullByPath = reactive<Record<string, string>>({});

function pathAt(i: number): string | undefined {
  return visible.value[i]?.sourcePath;
}
function thumbAt(i: number): string {
  const p = pathAt(i);
  return p ? thumbByPath[p] ?? "" : "";
}
function fullAt(i: number): string {
  const p = pathAt(i);
  return p ? fullByPath[p] ?? "" : "";
}

async function ensureThumb(i: number) {
  const p = pathAt(i);
  if (!p || thumbByPath[p]) return;
  try {
    thumbByPath[p] = await queueThumb(p);
  } catch {
    /* 占位条纹 */
  }
}
async function ensureFull(i: number) {
  const p = pathAt(i);
  if (!p || fullByPath[p]) return;
  try {
    fullByPath[p] = needsRustConvert(p)
      ? await invoke<string>("get_preview", { path: p })
      : convertFileSrc(p);
  } catch {
    /* 保底用缩略图垫底 */
  }
}

// 胶片条窗口：当前 ±6（多缓冲以保证滑动连续），可视区域约 ±3
const WIN = 6;
const windowIndices = computed(() => {
  const c = store.focusedIndex;
  const start = Math.max(0, c - WIN);
  const end = Math.min(len.value - 1, c + WIN);
  const arr: number[] = [];
  for (let i = start; i <= end; i++) arr.push(i);
  return arr;
});

const stripViewport = ref<HTMLElement | null>(null);
const { width: stripW } = useElementSize(stripViewport);
const TILE = 60;
const TGAP = 8;
const STRIDE = TILE + TGAP;
const trackStyle = computed(() => {
  const start = windowIndices.value[0] ?? 0;
  const activePos = store.focusedIndex - start;
  const x = stripW.value / 2 - TILE / 2 - activePos * STRIDE;
  return { transform: `translateX(${x}px)` };
});

function markReady(e: Event) {
  (e.target as HTMLElement).classList.add("ready");
}

function go(d: number) {
  const target = store.focusedIndex + d;
  if (target < 0 || target >= len.value) return;
  dir.value = d >= 0 ? 1 : -1;
  store.setFocus(target);
}
function goto(i: number) {
  if (i === store.focusedIndex) return;
  dir.value = i > store.focusedIndex ? 1 : -1;
  store.setFocus(i);
}
// 点击图片左/右半区 → 上一张 / 下一张
function onStageClick(e: MouseEvent) {
  const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
  if (e.clientX < rect.left + rect.width / 2) go(-1);
  else go(1);
}
function close() {
  store.closeViewer();
}
function onRemove() {
  store.removeFile(store.focusedIndex);
  if (len.value === 0) close();
}

// 标记切换：触发中心爆发动效（标记/取消用不同图标与配色）
let burstKey = 0;
let burstTimer: ReturnType<typeof setTimeout> | null = null;
const burst = ref<{ key: number; on: boolean } | null>(null);
function toggleMark() {
  store.toggleMark(store.focusedIndex);
  burst.value = { key: ++burstKey, on: file.value?.status === "marked" };
  if (burstTimer) clearTimeout(burstTimer);
  burstTimer = setTimeout(() => (burst.value = null), 600);
}

// 切换图片复位为静图
watch(() => store.focusedIndex, () => {
  showVideo.value = false;
});

// 打开 / 切换时预取窗口缩略图 + 当前及相邻原图
watch(
  () => [store.viewerOpen, store.focusedIndex] as const,
  ([open]) => {
    if (!open) return;
    windowIndices.value.forEach(ensureThumb);
    ensureFull(store.focusedIndex);
    ensureFull(store.focusedIndex - 1);
    ensureFull(store.focusedIndex + 1);
  },
  { immediate: true }
);

useEventListener(window, "keydown", (e: KeyboardEvent) => {
  if (!store.viewerOpen) return;
  switch (e.key) {
    case "ArrowRight":
    case "ArrowDown": e.preventDefault(); go(1); break;
    case "ArrowLeft":
    case "ArrowUp": e.preventDefault(); go(-1); break;
    case "Escape": e.preventDefault(); close(); break;
    case " ": e.preventDefault(); toggleMark(); break;
    case "d":
    case "D":
    case "Delete": e.preventDefault(); onRemove(); break;
    case "z":
    case "Z": if (e.ctrlKey || e.metaKey) { e.preventDefault(); store.undoLast(); } break;
  }
});
</script>

<style scoped>
.viewer {
  position: fixed;
  inset: 0;
  z-index: 1000;
  display: flex;
  flex-direction: column;
  background: rgba(10, 10, 10, 0.96);
  backdrop-filter: blur(6px);
}

/* 顶部条 */
.v-top {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 10px 14px;
  flex-shrink: 0;
}
.counter {
  font-size: 13px;
  color: var(--text-secondary);
  font-variant-numeric: tabular-nums;
}
.top-actions { display: flex; gap: 8px; }
.tbtn {
  background: rgba(255, 255, 255, 0.08);
  color: #fff;
  border: none;
  border-radius: 6px;
  padding: 6px 11px;
  font-size: 13px;
  cursor: pointer;
  transition: background 0.15s, color 0.15s;
}
.tbtn:hover { background: rgba(255, 255, 255, 0.18); }
.tbtn.active { color: var(--accent); }

/* 主舞台 */
.stage {
  flex: 1;
  min-height: 0;
  display: flex;
  padding: 0 8px;
}
.stage-inner {
  position: relative;
  flex: 1;
  height: 100%;
  overflow: hidden;
  cursor: pointer;
}
.frame {
  position: absolute;
  inset: 0;
}
.layer {
  position: absolute;
  inset: 0;
  margin: auto;
  max-width: 100%;
  max-height: 100%;
  object-fit: contain;
}
.layer.base { z-index: 1; }
.layer.full {
  z-index: 2;
  opacity: 0;
  transition: opacity 0.25s ease;
}
.layer.full.ready { opacity: 1; }

/* 左右翻页提示 */
.edge-hint {
  position: absolute;
  top: 50%;
  transform: translateY(-50%);
  z-index: 4;
  font-size: 44px;
  line-height: 1;
  color: #fff;
  opacity: 0;
  pointer-events: none;
  text-shadow: 0 2px 14px rgba(0, 0, 0, 0.6);
  transition: opacity 0.2s ease;
}
.edge-hint.left { left: 18px; }
.edge-hint.right { right: 18px; }
.stage-inner:hover .edge-hint { opacity: 0.5; }

/* 常驻标记角标 */
.mark-flag {
  position: absolute;
  top: 12px;
  left: 12px;
  z-index: 5;
  display: flex;
  align-items: center;
  gap: 4px;
  padding: 4px 10px;
  background: var(--accent);
  color: #000;
  font-size: 12px;
  font-weight: 700;
  border-radius: 6px;
  pointer-events: none;
}
.flag-enter-active,
.flag-leave-active { transition: opacity 0.2s ease, transform 0.2s ease; }
.flag-enter-from,
.flag-leave-to { opacity: 0; transform: translateY(-6px); }

/* 标记切换中心爆发 */
.mark-burst {
  position: absolute;
  inset: 0;
  z-index: 6;
  display: flex;
  align-items: center;
  justify-content: center;
  pointer-events: none;
}
.burst-icon {
  font-size: 128px;
  line-height: 1;
  color: var(--accent);
  text-shadow: 0 6px 30px rgba(0, 0, 0, 0.55);
  animation: markBurst 0.55s cubic-bezier(0.22, 0.61, 0.36, 1) forwards;
}
.mark-burst.off .burst-icon { color: var(--text-secondary); }
@keyframes markBurst {
  0% { transform: scale(0.5); opacity: 0; }
  22% { transform: scale(1.08); opacity: 0.95; }
  55% { transform: scale(0.98); opacity: 0.9; }
  100% { transform: scale(1.18); opacity: 0; }
}

/* 方向感切换动画 */
.frame-next-enter-active,
.frame-next-leave-active,
.frame-prev-enter-active,
.frame-prev-leave-active {
  transition: transform 0.34s cubic-bezier(0.22, 0.61, 0.36, 1), opacity 0.34s ease;
}
.frame-next-enter-from { transform: translateX(36px); opacity: 0; }
.frame-next-leave-to { transform: translateX(-36px); opacity: 0; }
.frame-prev-enter-from { transform: translateX(-36px); opacity: 0; }
.frame-prev-leave-to { transform: translateX(36px); opacity: 0; }

/* 胶片条 */
.filmstrip {
  flex-shrink: 0;
  height: 80px;
  overflow: hidden;
  padding: 8px 0;
  -webkit-mask: linear-gradient(90deg, transparent, #000 9%, #000 91%, transparent);
  mask: linear-gradient(90deg, transparent, #000 9%, #000 91%, transparent);
}
.strip-track {
  display: flex;
  gap: 8px;
  height: 100%;
  align-items: center;
  will-change: transform;
  transition: transform 0.32s cubic-bezier(0.22, 0.61, 0.36, 1);
}
.tile {
  position: relative;
  flex: 0 0 60px;
  width: 60px;
  height: 60px;
  padding: 0;
  border: 2px solid transparent;
  border-radius: 6px;
  overflow: hidden;
  background: var(--bg-card);
  cursor: pointer;
  opacity: 0.55;
  transition: transform 0.22s cubic-bezier(0.22, 0.61, 0.36, 1), opacity 0.22s ease, border-color 0.15s ease;
}
.tile:hover { opacity: 0.85; }
.tile img { width: 100%; height: 100%; object-fit: cover; display: block; }
.tile-ph {
  display: block;
  width: 100%;
  height: 100%;
  background: repeating-linear-gradient(45deg,
    var(--bg-card) 0, var(--bg-card) 6px, var(--bg-card-hover) 6px, var(--bg-card-hover) 12px);
}
.tile.marked { border-color: var(--marked); }
.tile.active {
  opacity: 1;
  transform: scale(1.12);
  border-color: var(--accent);
}

.caption {
  flex-shrink: 0;
  text-align: center;
  font-size: 12px;
  color: var(--text-secondary);
  padding: 4px 16px 14px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

/* 打开 / 关闭过渡 */
.viewer-enter-active,
.viewer-leave-active { transition: opacity 0.22s ease; }
.viewer-enter-from,
.viewer-leave-to { opacity: 0; }
.viewer-enter-active .stage,
.viewer-leave-active .stage { transition: transform 0.24s cubic-bezier(0.22, 0.61, 0.36, 1); }
.viewer-enter-from .stage,
.viewer-leave-to .stage { transform: scale(0.97); }
</style>
