<template>
  <Teleport to="body">
    <Transition name="dlg">
      <div v-if="store.importOpen" class="overlay" @click.self="tryClose">
        <div ref="panel" class="panel" role="dialog" aria-modal="true" aria-labelledby="device-import-title" tabindex="-1" @keydown="onDialogKey">
          <div class="dlg-head">
            <span id="device-import-title" class="dlg-title">📱 从手机导入</span>
            <button class="x" :disabled="view === 'importing'" @click="tryClose" aria-label="关闭手机导入" title="关闭 (Esc)">✕</button>
          </div>

          <!-- 设备列表：去目录浏览，选设备 → 自动定位存储根导入 -->
          <div v-if="view === 'browse'" class="dlg-body">
            <div class="crumbs">
              <span class="crumb-label">便携设备</span>
              <span class="spacer" />
              <button class="btn xs ghost" :disabled="loading || supported === false" @click="reload" aria-label="刷新设备" title="刷新">↻</button>
            </div>

            <div v-if="error" class="warn" role="alert">⚠ {{ error }}</div>
            <div v-if="!supported && !loading && !error" class="empty">手机 USB 直连目前仅支持 Windows。请先将照片复制到电脑，再用「打开目录」整理。</div>
            <div v-else-if="loading" class="muted" role="status">加载中…</div>

            <template v-else-if="!error && supported">
              <div v-if="entries.length === 0" class="empty">
                未检测到便携设备。<br />
                请用 USB 连接并解锁手机。iPhone 点「信任此电脑」，Android 选择「文件传输」，然后刷新。
              </div>
              <ul v-else class="list">
                <li v-for="e in entries" :key="e.id" class="row">
                  <span class="icon">📱</span>
                  <span class="name" :title="e.name">{{ e.name }}</span>
                  <button class="btn xs" :disabled="loading" @click="importDevice(e)">导入</button>
                </li>
              </ul>
            </template>
          </div>

          <!-- 导入进度 -->
          <div v-else class="dlg-body running">
            <div class="run-logo">📥</div>
            <div class="run-text">{{ cancelling ? '正在取消…' : progress.phase === 'counting' ? '正在统计文件…' : '正在从手机复制…' }}</div>
            <div class="progress-track" role="progressbar" aria-label="手机导入进度" :aria-valuenow="indeterminate ? undefined : percent" :aria-valuemin="0" :aria-valuemax="100">
              <div class="progress-fill" :class="{ indeterminate }" :style="{ width: indeterminate ? '35%' : percent + '%' }" />
            </div>
            <div v-if="!indeterminate" class="run-counter">
              {{ progress.done_files }} / {{ progress.total_files }} <span class="pct">({{ percent }}%)</span>
            </div>
            <div class="run-current">{{ mb(progress.done_bytes) }} · USB 传输中，请保持手机连接</div>
          </div>

          <div v-if="view === 'importing' && error" class="warn run-error" role="alert">{{ error }}</div>
          <div class="dlg-foot">
            <template v-if="view === 'browse'">
              <span class="foot-hint">将复制设备内容到本机临时目录</span>
              <button class="btn ghost" @click="tryClose">取消</button>
            </template>
            <template v-else>
              <span class="foot-hint">{{ cancelling ? '正在取消，请稍候…' : '复制完成后将自动开始整理…' }}</span>
              <button class="btn ghost" :disabled="cancelling" @click="cancelImport">
                {{ cancelling ? '取消中…' : '取消导入' }}
              </button>
            </template>
          </div>
        </div>
      </div>
    </Transition>
  </Teleport>
</template>

<script setup lang="ts">
import { ref, computed, watch, nextTick, onBeforeUnmount } from "vue";
import { invoke, Channel } from "@tauri-apps/api/core";
import { useProjectStore } from "../stores/project";

interface PortableEntry { name: string; id: string; is_folder: boolean; is_filesystem: boolean }
interface ImportProgress { phase: string; done_files: number; total_files: number; done_bytes: number }
interface ImportResult { temp_dir: string; file_count: number; elapsed_ms: number; cancelled: boolean }
const store = useProjectStore();
const panel = ref<HTMLElement | null>(null);
const view = ref<"browse" | "importing">("browse");
const entries = ref<PortableEntry[]>([]);
const loading = ref(false);
const supported = ref<boolean | null>(null);
const cancelling = ref(false);
const error = ref<string | null>(null);
const progress = ref<ImportProgress>({ phase: "counting", done_files: 0, total_files: 0, done_bytes: 0 });
let generation = 0;
let returnFocus: HTMLElement | null = null;
const indeterminate = computed(() => progress.value.phase === "counting" || progress.value.total_files <= 0);
const percent = computed(() => Math.max(0, Math.min(99, Math.floor(progress.value.done_files / (progress.value.total_files || 1) * 100))));
function mb(b: number) { return (b / 1048576).toFixed(1) + " MB"; }
watch(() => store.importOpen, async (openNow) => {
  generation++;
  if (openNow) {
    returnFocus = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    view.value = "browse";
    cancelling.value = false;
    void reload();
    await nextTick();
    panel.value?.focus();
  } else {
    await nextTick();
    if (returnFocus?.isConnected) returnFocus.focus();
  }
});
watch(view, async () => {
  await nextTick();
  if (store.importOpen) panel.value?.focus();
});
onBeforeUnmount(() => generation++);
async function reload() {
  const request = ++generation;
  loading.value = true;
  entries.value = [];
  supported.value = null;
  error.value = null;
  try {
    const available = await invoke<boolean>("pd_supported");
    if (request !== generation || !store.importOpen) return;
    supported.value = available;
    if (!available) return;
    const result = await invoke<PortableEntry[]>("pd_browse", { parentId: null });
    if (request === generation && store.importOpen) entries.value = result;
  } catch (e) {
    if (request === generation && store.importOpen) error.value = String(e);
  } finally { if (request === generation) loading.value = false; }
}
async function importDevice(device: PortableEntry) {
  if (loading.value || view.value !== "browse") return;
  const request = ++generation;
  loading.value = true;
  error.value = null;
  try {
    const storages = await invoke<PortableEntry[]>("pd_browse", { parentId: device.id });
    if (request !== generation || !store.importOpen) return;
    const folders = storages.filter(s => s.is_folder);
    if (!folders.length) {
      error.value = "无法访问设备存储，请解锁手机并允许文件传输后重试。";
      return;
    }
    await startImport(folders.map(s => s.id), request);
  } catch (e) { if (request === generation) error.value = String(e); }
  finally { if (request === generation) loading.value = false; }
}
async function startImport(ids: string[], request: number) {
  view.value = "importing";
  cancelling.value = false;
  progress.value = { phase: "counting", done_files: 0, total_files: 0, done_bytes: 0 };
  await nextTick();
  panel.value?.focus();
  const channel = new Channel<ImportProgress>();
  channel.onmessage = m => { if (request === generation && view.value === "importing") progress.value = m; };
  try {
    const res = await invoke<ImportResult>("pd_import", { itemIds: ids, onProgress: channel });
    if (request !== generation) return;
    if (res.cancelled) { view.value = "browse"; error.value = null; return; }
    if (!res.file_count) { view.value = "browse"; error.value = "未复制到文件，请确认设备中有可访问的文件。"; return; }
    await store.openImportedDirectory(res.temp_dir);
  } catch (e) {
    if (request === generation) { view.value = "browse"; error.value = "导入失败：" + String(e); }
  } finally { cancelling.value = false; }
}
async function cancelImport() {
  if (cancelling.value) return;
  cancelling.value = true;
  error.value = null;
  try { await invoke("pd_cancel_import"); }
  catch (e) { cancelling.value = false; error.value = "取消失败，请重试：" + String(e); }
}
function tryClose() {
  if (view.value === "importing") return;
  generation++;
  store.closeImport();
}
function onDialogKey(e: KeyboardEvent) {
  if (e.key === "Escape") { e.preventDefault(); e.stopPropagation(); tryClose(); }
  if (e.key !== "Tab") return;
  const buttons = Array.from(panel.value?.querySelectorAll<HTMLButtonElement>("button:not(:disabled)") ?? []);
  const first = buttons[0], last = buttons[buttons.length - 1];
  if (!first || !last) { e.preventDefault(); panel.value?.focus(); return; }
  if (e.shiftKey && (document.activeElement === first || document.activeElement === panel.value)) { e.preventDefault(); last.focus(); }
  else if (!e.shiftKey && (document.activeElement === last || document.activeElement === panel.value)) { e.preventDefault(); first.focus(); }
}
</script>

<style scoped>
.overlay {
  position: fixed;
  inset: 0;
  z-index: 1100;
  display: flex;
  align-items: center;
  justify-content: center;
  background: rgba(8, 8, 8, 0.66);
  backdrop-filter: blur(4px);
}
.panel {
  width: 480px;
  max-width: calc(100vw - 24px);
  max-height: calc(100dvh - 32px);
  display: flex;
  flex-direction: column;
  background: var(--bg-panel);
  border: 1px solid var(--border);
  border-radius: 12px;
  overflow: hidden;
  box-shadow: 0 24px 60px rgba(0, 0, 0, 0.5);
}
.dlg-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 14px 16px;
  border-bottom: 1px solid var(--border);
}
.dlg-title { font-size: 14px; font-weight: 700; color: var(--text-primary); }
.x {
  background: none; border: none; color: var(--text-secondary);
  font-size: 14px; cursor: pointer; padding: 2px 6px; border-radius: 4px;
}
.x:hover:not(:disabled) { background: var(--bg-card); color: var(--text-primary); }
.x:disabled { opacity: 0.3; cursor: default; }

.dlg-body { padding: 16px; overflow-y: auto; display: flex; flex-direction: column; gap: 12px; }

.crumbs { display: flex; align-items: center; flex-wrap: wrap; gap: 4px; }
.crumb-label { font-size: 12px; font-weight: 600; color: var(--text-primary); }
.spacer { flex: 1; }

.warn { font-size: 12px; color: var(--accent); word-break: break-all; }
.muted { color: var(--text-secondary); font-size: 12px; padding: 8px 2px; }
.empty {
  color: var(--text-secondary); font-size: 12px; line-height: 1.7;
  padding: 24px 12px; text-align: center;
  background: var(--bg-base); border: 1px dashed var(--border); border-radius: 8px;
}

.list { list-style: none; margin: 0; padding: 0; max-height: 320px; overflow-y: auto; }
.row {
  display: flex; align-items: center; gap: 8px;
  padding: 6px 2px; border-bottom: 1px solid var(--border); font-size: 13px;
}
.icon { width: 18px; text-align: center; }
.name { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--text-primary); }

/* 进度 */
.running { align-items: center; text-align: center; padding: 28px 16px; gap: 14px; }
.run-logo { font-size: 40px; }
.run-text { font-size: 15px; font-weight: 600; color: var(--text-primary); }
.progress-track { width: 100%; height: 8px; background: var(--bg-card); border-radius: 4px; overflow: hidden; }
.progress-fill { height: 100%; background: var(--accent); border-radius: 4px; transition: width 0.2s ease; }
.run-counter { font-size: 13px; color: var(--text-primary); font-variant-numeric: tabular-nums; }
.pct { color: var(--text-secondary); }
.run-current { font-size: 11px; color: var(--text-secondary); }

.dlg-foot {
  display: flex; justify-content: flex-end; align-items: center; gap: 8px;
  flex-wrap: wrap; padding: 12px 16px; border-top: 1px solid var(--border);
}
.foot-hint { font-size: 12px; color: var(--text-secondary); margin-right: auto; }

.btn {
  padding: 8px 16px; border-radius: 6px; font-size: 13px; cursor: pointer;
  border: 1px solid var(--border); background: var(--bg-card); color: var(--text-primary);
  transition: all 0.15s; white-space: nowrap;
}
.btn:hover:not(:disabled) { filter: brightness(1.15); }
.btn:disabled { opacity: 0.4; cursor: default; }
.btn.ghost { background: none; color: var(--text-secondary); }
.btn.ghost:hover:not(:disabled) { color: var(--text-primary); }
.btn.xs { padding: 3px 10px; font-size: 12px; }
.btn.xs:not(.ghost) { background: var(--accent); color: #000; border-color: var(--accent); font-weight: 600; }

.dlg-enter-active, .dlg-leave-active { transition: opacity 0.18s ease; }
.dlg-enter-from, .dlg-leave-to { opacity: 0; }
.dlg-enter-active .panel, .dlg-leave-active .panel { transition: transform 0.2s var(--ease-out); }
.dlg-enter-from .panel, .dlg-leave-to .panel { transform: scale(0.96) translateY(8px); }
button:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
.x, .btn.xs { min-height: 32px; min-width: 32px; }
.run-error { padding: 0 16px 12px; }
.indeterminate { animation: transfer 1.3s ease-in-out infinite alternate; }
@keyframes transfer { to { transform: translateX(185%); } }
</style>
