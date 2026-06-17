<template>
  <div class="titlebar" data-tauri-drag-region>
    <!-- 左：上下文（首页=品牌 / 工作区=返回+目录） -->
    <div class="tb-left" data-tauri-drag-region>
      <template v-if="store.phase === 'ready'">
        <button class="tb-back" @click="store.reset()" title="返回首页">
          <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true">
            <path d="M10 3 L5 8 L10 13" fill="none" stroke="currentColor"
              stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round" />
          </svg>
          返回
        </button>
        <span class="tb-dir" data-tauri-drag-region :title="store.sourceDir ?? ''">{{ store.sourceDir }}</span>
      </template>
      <span v-else class="tb-brand" data-tauri-drag-region>
        <span class="brand-logo">🧁</span> Muffin Tidy
      </span>
    </div>

    <!-- 中：可拖动留白 + 扫描提示 -->
    <div class="tb-center" data-tauri-drag-region>
      <span v-if="store.isScanning" class="tb-scan">扫描中…</span>
    </div>

    <!-- 右：窗口控件 -->
    <div class="tb-controls">
      <button class="ctl" @click="minimize" aria-label="最小化" title="最小化">
        <svg viewBox="0 0 12 12" width="12" height="12" aria-hidden="true">
          <line x1="2.5" y1="6" x2="9.5" y2="6" stroke="currentColor" stroke-width="1" />
        </svg>
      </button>
      <button class="ctl" @click="toggleMax" :aria-label="isMax ? '还原' : '最大化'" :title="isMax ? '还原' : '最大化'">
        <svg v-if="!isMax" viewBox="0 0 12 12" width="12" height="12" aria-hidden="true">
          <rect x="2.5" y="2.5" width="7" height="7" fill="none" stroke="currentColor" stroke-width="1" />
        </svg>
        <svg v-else viewBox="0 0 12 12" width="12" height="12" aria-hidden="true">
          <rect x="2.5" y="3.5" width="6" height="6" fill="none" stroke="currentColor" stroke-width="1" />
          <path d="M4.5 3.5 V2.5 H9.5 V7.5 H8.5" fill="none" stroke="currentColor" stroke-width="1" />
        </svg>
      </button>
      <button class="ctl close" @click="closeWin" aria-label="关闭" title="关闭">
        <svg viewBox="0 0 12 12" width="12" height="12" aria-hidden="true">
          <path d="M3 3 L9 9 M9 3 L3 9" stroke="currentColor" stroke-width="1" stroke-linecap="round" />
        </svg>
      </button>
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref, onMounted, onUnmounted } from "vue";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useProjectStore } from "../stores/project";

const store = useProjectStore();
const win = getCurrentWindow();
const isMax = ref(false);
let unlisten: (() => void) | null = null;

async function refreshMax() {
  isMax.value = await win.isMaximized();
}
function minimize() {
  win.minimize();
}
async function toggleMax() {
  await win.toggleMaximize();
  refreshMax();
}
function closeWin() {
  win.close();
}

onMounted(async () => {
  await refreshMax();
  unlisten = await win.onResized(refreshMax);
});
onUnmounted(() => unlisten?.());
</script>

<style scoped>
.titlebar {
  display: flex;
  align-items: center;
  height: 36px;
  flex-shrink: 0;
  background: var(--bg-panel);
  border-bottom: 1px solid var(--border);
  padding-left: 12px;
  user-select: none;
  -webkit-user-select: none;
}

.tb-left {
  display: flex;
  align-items: center;
  gap: 10px;
  min-width: 0;
  flex: 0 1 auto;
}

.tb-brand {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 13px;
  font-weight: 600;
  color: var(--text-primary);
  white-space: nowrap;
}
.brand-logo { font-size: 15px; }

.tb-back {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  flex-shrink: 0;
  background: none;
  border: 1px solid var(--border);
  color: var(--text-secondary);
  border-radius: 5px;
  padding: 3px 9px 3px 7px;
  font-size: 12px;
  cursor: pointer;
  transition: color 0.15s, border-color 0.15s, background 0.15s;
}
.tb-back:hover {
  color: var(--text-primary);
  border-color: #555;
  background: var(--bg-card);
}

.tb-dir {
  min-width: 0;
  font-size: 12px;
  color: var(--text-secondary);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.tb-center {
  flex: 1;
  height: 100%;
  display: flex;
  align-items: center;
  justify-content: center;
}
.tb-scan { font-size: 12px; color: var(--accent); }

.tb-controls {
  display: flex;
  align-items: stretch;
  height: 100%;
  flex-shrink: 0;
}
.ctl {
  width: 44px;
  height: 100%;
  border: none;
  background: transparent;
  color: var(--text-secondary);
  cursor: pointer;
  display: flex;
  align-items: center;
  justify-content: center;
  transition: background 0.12s, color 0.12s;
}
.ctl:hover { background: var(--bg-card-hover); color: var(--text-primary); }
.ctl.close:hover { background: #e11d48; color: #fff; }
</style>
