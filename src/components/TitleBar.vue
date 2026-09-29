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
        <nav class="breadcrumbs" aria-label="当前目录">
          <button v-for="crumb in breadcrumbs" :key="crumb.path" class="crumb" :disabled="crumb.current || store.isScanning" @click="navigateTo(crumb.path)" :title="crumb.current ? '当前目录' : crumb.path">{{ crumb.label }}</button>
        </nav>
        <span class="tb-count">{{ store.exportableFiles.length }} 个媒体</span>
        <button class="tb-refresh" :class="{ active: store.isRefreshing }" @click="handleRefresh" :title="store.isRefreshing ? '取消刷新' : '刷新目录'" :aria-label="store.isRefreshing ? '取消刷新' : '刷新目录'">
          <svg v-if="store.isRefreshing" viewBox="0 0 16 16" width="13" height="13" aria-hidden="true"><path d="m4.5 4.5 7 7m0-7-7 7" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round"/></svg>
          <svg v-else viewBox="0 0 16 16" width="13" height="13" aria-hidden="true"><path d="M13 5.5A5.5 5.5 0 1 0 13.5 9" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round"/><path d="M10.5 5.5H13V3" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round"/></svg>
        </button>
      </template>
      <span v-else class="tb-brand" data-tauri-drag-region>
        <span class="brand-logo">🧁</span> Muffin Tidy
      </span>
    </div>

    <!-- 中：可拖动留白 + 扫描提示 -->
    <div class="tb-center" data-tauri-drag-region>
      <span v-if="store.isRefreshing" class="tb-scan">正在后台刷新…</span>
      <span v-else-if="store.isScanning" class="tb-scan">扫描中…</span>
    </div>

    <!-- 右：窗口控件 -->
    <div class="tb-controls">
      <button class="ctl help" @click="helpOpen = true" aria-label="使用帮助" title="使用帮助">
        <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true">
          <circle cx="8" cy="8" r="5.5" fill="none" stroke="currentColor" stroke-width="1.2" />
          <path d="M6.5 6.3a1.6 1.6 0 0 1 3.1.5c0 1.5-1.6 1.6-1.6 2.7" fill="none" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" />
          <circle cx="8" cy="11.7" r=".7" fill="currentColor" />
        </svg>
      </button>
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

    <Teleport to="body">
      <Transition name="help-dialog">
        <div v-if="helpOpen" class="help-overlay" @click.self="helpOpen = false">
          <section class="help-panel" role="dialog" aria-modal="true" aria-labelledby="help-title">
            <header>
              <h2 id="help-title">使用帮助</h2>
              <button class="help-close" @click="helpOpen = false" aria-label="关闭" title="关闭">×</button>
            </header>
            <div class="help-list">
              <div><kbd>双击</kbd><span>打开大图预览</span></div>
              <div><kbd>Space</kbd><span>标记或取消标记</span></div>
              <div><kbd>D</kbd><span>从工程移除，不删除原文件</span></div>
              <div><kbd>↑ ↓ ← →</kbd><span>切换当前媒体</span></div>
              <div><kbd>Ctrl / ⌘ + Z</kbd><span>撤销上一步操作</span></div>
              <div><kbd>Ctrl / ⌘ + 滚轮</kbd><span>调整缩略图大小</span></div>
              <div><kbd>E</kbd><span>打开导出面板</span></div>
            </div>
          </section>
        </div>
      </Transition>
    </Teleport>
  </div>
</template>

<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted } from "vue";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useProjectStore } from "../stores/project";

const store = useProjectStore();
const win = "__TAURI_INTERNALS__" in window ? getCurrentWindow() : null;
const isMax = ref(false);
const helpOpen = ref(false);
const breadcrumbs = computed(() => {
  const raw = store.sourceDir;
  if (!raw) return [];
  const normalized = raw.replace(/\\/g, "/");
  const isAbsolute = normalized.startsWith("/");
  const parts = normalized.split("/").filter(Boolean);
  let acc = isAbsolute ? "" : "";
  return parts.map((label, index) => {
    acc += (index === 0 && isAbsolute ? "/" : index === 0 ? "" : "/") + label;
    return { label, path: acc, current: index === parts.length - 1 };
  });
});
let unlisten: (() => void) | null = null;

async function refreshMax() {
  if (win) isMax.value = await win.isMaximized();
}
function minimize() {
  win?.minimize();
}
async function toggleMax() {
  if (!win) return;
  await win.toggleMaximize();
  refreshMax();
}
function closeWin() {
  win?.close();
}
function onKeydown(event: KeyboardEvent) {
  if (event.key === "Escape") helpOpen.value = false;
}
async function navigateTo(path: string) {
  if (store.isScanning || path === store.sourceDir) return;
  try {
    await store.openDirectory(path);
  } catch (error) {
    console.error("打开目录失败", error);
  }
}
async function handleRefresh() {
  if (store.isRefreshing) {
    store.cancelRefresh();
    return;
  }
  try {
    await store.refreshDirectory();
  } catch (error) {
    console.error("刷新目录失败", error);
  }
}

onMounted(async () => {
  window.addEventListener("keydown", onKeydown);
  if (!win) return;
  await refreshMax();
  unlisten = await win.onResized(refreshMax);
});
onUnmounted(() => {
  window.removeEventListener("keydown", onKeydown);
  unlisten?.();
});
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

.breadcrumbs { display: flex; align-items: center; min-width: 0; overflow: hidden; }
.crumb {
  max-width: 160px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  border: 0;
  background: none;
  color: var(--text-secondary);
  font-size: 12px;
  padding: 3px 4px;
  cursor: pointer;
}
.crumb + .crumb::before { content: "/"; color: #444; margin-right: 8px; }
.crumb:hover { color: var(--text-primary); }
.crumb:disabled { color: var(--text-primary); cursor: default; }
.tb-count { flex-shrink: 0; color: #666; font-size: 10px; white-space: nowrap; }
.tb-refresh {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 24px;
  height: 24px;
  margin-left: 2px;
  border: 1px solid transparent;
  border-radius: 4px;
  background: transparent;
  color: var(--text-secondary);
  cursor: pointer;
}
.tb-refresh:hover:not(:disabled) { color: var(--accent); background: var(--bg-card); border-color: var(--border); }
.tb-refresh.active { color: var(--accent); border-color: rgba(245, 158, 11, .35); background: var(--accent-dim); }
.tb-refresh:disabled { opacity: .35; cursor: default; }

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
.ctl.help { width: 36px; }

.help-overlay {
  position: fixed;
  inset: 0;
  z-index: 1300;
  display: flex;
  align-items: center;
  justify-content: center;
  background: rgba(8, 8, 8, .68);
  backdrop-filter: blur(4px);
}
.help-panel {
  width: 390px;
  max-width: calc(100vw - 40px);
  border: 1px solid var(--border);
  border-radius: 8px;
  background: var(--bg-panel);
  box-shadow: 0 20px 56px rgba(0,0,0,.5);
}
.help-panel header { display: flex; align-items: center; justify-content: space-between; padding: 13px 16px; border-bottom: 1px solid var(--border); }
.help-panel h2 { margin: 0; font-size: 14px; font-weight: 650; color: var(--text-primary); }
.help-close { width: 26px; height: 26px; border: 0; border-radius: 4px; background: transparent; color: var(--text-secondary); font-size: 18px; cursor: pointer; }
.help-close:hover { color: var(--text-primary); background: var(--bg-card); }
.help-list { padding: 10px 16px 14px; }
.help-list > div { display: grid; grid-template-columns: 126px 1fr; align-items: center; min-height: 34px; border-bottom: 1px solid rgba(255,255,255,.045); }
.help-list > div:last-child { border-bottom: 0; }
.help-list kbd { justify-self: start; border: 1px solid #444; border-bottom-color: #555; border-radius: 4px; background: var(--bg-card); color: var(--text-primary); padding: 2px 7px; font: 11px ui-monospace, monospace; }
.help-list span { color: var(--text-secondary); font-size: 12px; }
.help-dialog-enter-active, .help-dialog-leave-active { transition: opacity .16s; }
.help-dialog-enter-from, .help-dialog-leave-to { opacity: 0; }
.help-dialog-enter-active .help-panel, .help-dialog-leave-active .help-panel { transition: transform .18s var(--ease-out); }
.help-dialog-enter-from .help-panel, .help-dialog-leave-to .help-panel { transform: translateY(6px) scale(.98); }
</style>
