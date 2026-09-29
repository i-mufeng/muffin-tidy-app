<template>
  <Teleport to="body">
    <Transition name="dlg">
      <div v-if="store.importOpen" class="overlay" @click.self="tryClose">
        <div class="panel">
          <div class="dlg-head">
            <span class="dlg-title">📱 从手机导入</span>
            <button class="x" :disabled="view === 'importing'" @click="tryClose" title="关闭 (Esc)">✕</button>
          </div>

          <!-- 设备列表：去目录浏览，选设备 → 自动定位存储根导入 -->
          <div v-if="view === 'browse'" class="dlg-body">
            <div class="crumbs">
              <span class="crumb-label">便携设备</span>
              <span class="spacer" />
              <button class="btn xs ghost" :disabled="loading" @click="reload" title="刷新">↻</button>
            </div>

            <div v-if="error" class="warn">⚠ {{ error }}</div>
            <div v-if="loading" class="muted">加载中…</div>

            <template v-else>
              <div v-if="entries.length === 0" class="empty">
                未检测到便携设备。<br />
                请用 USB 连接手机，iPhone 需解锁并点「信任此电脑」，然后点 ↻ 刷新。
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
            <div class="progress-track">
              <div class="progress-fill" :style="{ width: percent + '%' }" />
            </div>
            <div class="run-counter">
              {{ progress.done_files }} / {{ progress.total_files }} <span class="pct">({{ percent }}%)</span>
            </div>
            <div class="run-current">{{ mb(progress.done_bytes) }} · MTP 传输较慢，请耐心等待</div>
          </div>

          <div class="dlg-foot">
            <template v-if="view === 'browse'">
              <span class="foot-hint">选择设备后将自动导入其全部照片</span>
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
import { ref, computed, watch } from "vue";
import { invoke, Channel } from "@tauri-apps/api/core";
import { useEventListener } from "@vueuse/core";
import { useProjectStore } from "../stores/project";

interface PortableEntry {
  name: string;
  id: string;
  is_folder: boolean;
  is_filesystem: boolean;
}
interface ImportProgress {
  phase: string;
  done_files: number;
  total_files: number;
  done_bytes: number;
}
interface ImportResult {
  temp_dir: string;
  file_count: number;
  elapsed_ms: number;
  cancelled: boolean;
}

const store = useProjectStore();

const view = ref<"browse" | "importing">("browse");
const entries = ref<PortableEntry[]>([]);
const loading = ref(false);
const cancelling = ref(false);
const error = ref<string | null>(null);
const progress = ref<ImportProgress>({ phase: "counting", done_files: 0, total_files: 0, done_bytes: 0 });

// 每次打开复位并枚举设备
watch(
  () => store.importOpen,
  (openNow) => {
    if (openNow) {
      view.value = "browse";
      error.value = null;
      load(null);
    }
  }
);

const percent = computed(() => {
  const { done_files, total_files } = progress.value;
  return total_files === 0 ? 0 : Math.min(100, Math.round((done_files / total_files) * 100));
});
function mb(b: number) {
  return (b / 1048576).toFixed(1) + " MB";
}

async function load(parentId: string | null) {
  loading.value = true;
  error.value = null;
  try {
    entries.value = await invoke<PortableEntry[]>("pd_browse", { parentId });
  } catch (e) {
    error.value = String(e);
    entries.value = [];
  } finally {
    loading.value = false;
  }
}
function reload() {
  load(null);
}

// 选设备 → 自动下钻一层到存储根再导入。
// 设备根（Apple iPhone）本身不是可复制的文件系统对象，CopyItem 会拷 0；
// 其直接子节点即存储根（Internal Storage），可被 IFileOperation 整体递归复制。
async function importDevice(device: PortableEntry) {
  loading.value = true;
  error.value = null;
  try {
    const storages = await invoke<PortableEntry[]>("pd_browse", { parentId: device.id });
    if (storages.length === 0) {
      error.value = "无法访问该设备存储：请确认手机已解锁，并在手机上点「信任此电脑」后重试";
      return;
    }
    startImport(storages.map((s) => s.id));
  } catch (e) {
    error.value = String(e);
  } finally {
    loading.value = false;
  }
}

async function startImport(ids: string[]) {
  view.value = "importing";
  cancelling.value = false;
  progress.value = { phase: "counting", done_files: 0, total_files: 0, done_bytes: 0 };
  const channel = new Channel<ImportProgress>();
  channel.onmessage = (m) => {
    progress.value = m;
  };
  try {
    const res = await invoke<ImportResult>("pd_import", { itemIds: ids, onProgress: channel });
    if (res.cancelled) {
      // 用户取消：后端已清理临时目录，静默回到浏览态
      view.value = "browse";
      return;
    }
    if (res.file_count === 0) {
      view.value = "browse";
      error.value = "未复制到任何文件（该文件夹可能为空）";
      return;
    }
    // 关闭对话框 + 记录临时目录 + 进入现有扫描链路
    await store.openImportedDirectory(res.temp_dir);
  } catch (e) {
    view.value = "browse";
    error.value = "导入失败：" + String(e);
  } finally {
    cancelling.value = false;
  }
}

// 请求取消：后端在下一个文件边界中止，pd_import 随后以 cancelled=true 返回收尾
async function cancelImport() {
  if (cancelling.value) return;
  cancelling.value = true;
  try {
    await invoke("pd_cancel_import");
  } catch {
    /* 取消请求失败不阻塞，忽略 */
  }
}

function tryClose() {
  if (view.value === "importing") return; // 复制中不允许关闭
  store.closeImport();
}

useEventListener(window, "keydown", (e: KeyboardEvent) => {
  if (!store.importOpen) return;
  if (e.key === "Escape") {
    e.preventDefault();
    tryClose();
  }
});
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
  max-width: calc(100vw - 48px);
  max-height: calc(100vh - 80px);
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

.warn { font-size: 12px; color: #f59e0b; word-break: break-all; }
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
  padding: 12px 16px; border-top: 1px solid var(--border);
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
</style>
