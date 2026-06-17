<template>
  <Teleport to="body">
    <Transition name="dlg">
      <div v-if="store.exportOpen" class="overlay" @click.self="tryClose">
        <div class="panel">
          <!-- 头部 -->
          <div class="dlg-head">
            <span class="dlg-title">导出整理结果</span>
            <button class="x" :disabled="view === 'running'" @click="tryClose" title="关闭 (Esc)">✕</button>
          </div>

          <!-- ① 配置 -->
          <div v-if="view === 'config'" class="dlg-body">
            <!-- 目标目录 -->
            <div class="field">
              <label class="flabel">目标目录</label>
              <div class="target-row">
                <div class="target-box" :class="{ placeholder: !targetDir }" :title="targetDir ?? ''">
                  {{ targetDir ?? '尚未选择…' }}
                </div>
                <button class="btn" @click="pickTarget">选择…</button>
              </div>
              <div v-if="targetError" class="warn">⚠ {{ targetError }}</div>
            </div>

            <!-- 范围 -->
            <div class="field">
              <label class="flabel">导出范围</label>
              <div class="seg">
                <button class="seg-btn" :class="{ on: scope === 'all' }" @click="scope = 'all'">
                  全部保留 <b>{{ store.visibleFiles.length }}</b>
                </button>
                <button class="seg-btn" :class="{ on: scope === 'marked' }" @click="scope = 'marked'">
                  仅已标记 <b>{{ markedCount }}</b>
                </button>
              </div>
            </div>

            <!-- 选项 -->
            <div class="field">
              <label class="flabel">选项</label>
              <label class="opt">
                <input type="checkbox" v-model="organizeByDate" />
                <span>
                  <b>按日期归档并重命名</b>
                  <i>目标/年/月/{{ '类型-时间戳-序号' }}.ext；关闭则平铺保留原文件名</i>
                </span>
              </label>
              <label class="opt">
                <input type="checkbox" v-model="dedup" />
                <span>
                  <b>智能去重</b>
                  <i>按内容哈希跳过完全相同的文件（批内重复 + 目标已存在）</i>
                </span>
              </label>
              <label class="opt">
                <input type="checkbox" v-model="dryRun" />
                <span>
                  <b>预演（不写文件）</b>
                  <i>只统计将要导出的文件，验证计划无误后再正式执行</i>
                </span>
              </label>
            </div>

            <!-- 安全说明 -->
            <div class="safety">
              🔒 只复制、不移动；绝不修改源文件，完整保留 EXIF 等文件头信息。
            </div>
          </div>

          <!-- ② 进行中 -->
          <div v-else-if="view === 'running'" class="dlg-body running">
            <div class="run-logo">{{ dryRun ? '🧮' : '📤' }}</div>
            <div class="run-text">{{ dryRun ? '正在统计…' : '正在导出…' }}</div>
            <div class="progress-track">
              <div class="progress-fill" :style="{ width: percent + '%' }" />
            </div>
            <div class="run-counter">
              {{ progress.done }} / {{ progress.total }} <span class="pct">({{ percent }}%)</span>
            </div>
            <div class="run-current">{{ progress.current }}</div>
          </div>

          <!-- ③ 完成 -->
          <div v-else class="dlg-body">
            <div class="done-head">
              <span class="done-logo">{{ summary && summary.skipped_error > 0 ? '⚠️' : '✅' }}</span>
              <span class="done-title">{{ dryRunDone ? '预演完成' : '导出完成' }}</span>
            </div>
            <div v-if="summary" class="result">
              <div class="rrow rrow-main">
                <span>{{ dryRunDone ? '计划导出' : '成功导出' }}</span>
                <b>{{ totalExported }} 项 · {{ summary.copied_files }} 个文件</b>
              </div>
              <div class="rsub">
                <span>🖼 图片 {{ summary.exported_img }}</span>
                <span>🎬 视频 {{ summary.exported_vdo }}</span>
                <span>✨ 实况 {{ summary.exported_lpo }}</span>
              </div>
              <div class="rdiv" />
              <div class="rrow"><span>♻ 内容重复跳过</span><b>{{ summary.skipped_dedup }}</b></div>
              <div class="rrow"><span>⚠ 目标已存在跳过</span><b>{{ summary.skipped_conflict }}</b></div>
              <div class="rrow" :class="{ err: summary.skipped_error > 0 }">
                <span>✗ 失败</span><b>{{ summary.skipped_error }}</b>
              </div>
              <details v-if="summary.errors.length" class="errbox">
                <summary>错误详情（{{ summary.errors.length }}）</summary>
                <div v-for="(e, i) in summary.errors" :key="i" class="erritem">{{ e }}</div>
              </details>

              <button v-if="summary.log_path" class="logrow" @click="openLog" :title="summary.log_path">
                📄 已保存导出日志，点击查看
              </button>
            </div>
          </div>

          <!-- 底部操作 -->
          <div class="dlg-foot">
            <template v-if="view === 'config'">
              <button class="btn ghost" @click="tryClose">取消</button>
              <button class="btn primary" :disabled="!canStart" @click="start">
                {{ dryRun ? '开始预演' : '开始导出' }}
              </button>
            </template>
            <template v-else-if="view === 'running'">
              <span class="foot-hint">导出进行中，请稍候…</span>
            </template>
            <template v-else>
              <button v-if="!dryRunDone && targetDir" class="btn ghost" @click="reveal">打开目标目录</button>
              <button v-if="dryRunDone" class="btn ghost" @click="backToConfig">返回设置</button>
              <button class="btn primary" @click="store.closeExport()">完成</button>
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
import { open } from "@tauri-apps/plugin-dialog";
import { openPath } from "@tauri-apps/plugin-opener";
import { useEventListener } from "@vueuse/core";
import { useProjectStore } from "../stores/project";

interface ExportProgress {
  done: number;
  total: number;
  current: string;
}
// 字段为 serde 默认 snake_case（与 Rust ExportSummary 对齐）
interface ExportSummary {
  exported_img: number;
  exported_vdo: number;
  exported_lpo: number;
  copied_files: number;
  skipped_dedup: number;
  skipped_conflict: number;
  skipped_error: number;
  errors: string[];
  log_path: string | null;
}

const store = useProjectStore();

const view = ref<"config" | "running" | "done">("config");
const targetDir = ref<string | null>(null);
const scope = ref<"all" | "marked">("all");
const organizeByDate = ref(true);
const dedup = ref(true);
const dryRun = ref(false);

const progress = ref<ExportProgress>({ done: 0, total: 0, current: "" });
const summary = ref<ExportSummary | null>(null);
const dryRunDone = ref(false);

// 每次打开对话框复位到配置态
watch(
  () => store.exportOpen,
  (openNow) => {
    if (openNow) {
      view.value = "config";
      summary.value = null;
      progress.value = { done: 0, total: 0, current: "" };
    }
  }
);

const markedCount = computed(
  () => store.visibleFiles.filter((f) => f.status === "marked").length
);

const items = computed(() =>
  scope.value === "marked"
    ? store.visibleFiles.filter((f) => f.status === "marked")
    : store.visibleFiles
);

const percent = computed(() => {
  const { done, total } = progress.value;
  return total === 0 ? 0 : Math.round((done / total) * 100);
});

const totalExported = computed(() =>
  summary.value
    ? summary.value.exported_img + summary.value.exported_vdo + summary.value.exported_lpo
    : 0
);

// 前端预校验目标路径（后端为权威，这里仅即时提示）
function normPath(p: string): string {
  return p.replace(/[\\/]+/g, "/").replace(/\/+$/, "").toLowerCase();
}
const targetError = computed(() => {
  if (!targetDir.value || !store.sourceDir) return null;
  const t = normPath(targetDir.value);
  const s = normPath(store.sourceDir);
  if (t === s) return "目标目录不能与源目录相同";
  if (t.startsWith(s + "/")) return "目标目录不能位于源目录内部";
  if (s.startsWith(t + "/")) return "源目录不能位于目标目录内部";
  return null;
});

const canStart = computed(
  () => !!targetDir.value && !targetError.value && items.value.length > 0
);

async function pickTarget() {
  const picked = await open({ directory: true, multiple: false });
  if (typeof picked === "string") targetDir.value = picked;
}

async function start() {
  if (!canStart.value || !targetDir.value || !store.sourceDir) return;
  view.value = "running";
  dryRunDone.value = dryRun.value;
  progress.value = { done: 0, total: items.value.length, current: "" };

  const channel = new Channel<ExportProgress>();
  channel.onmessage = (m) => {
    progress.value = m;
  };

  const payload = items.value.map((f) => ({
    sourcePath: f.sourcePath,
    mediaType: f.mediaType,
    captureTime: f.captureTime,
    videoPath: f.videoPath,
  }));

  try {
    summary.value = await invoke<ExportSummary>("export_files", {
      items: payload,
      options: {
        target: targetDir.value,
        sourceDir: store.sourceDir,
        organizeByDate: organizeByDate.value,
        dedup: dedup.value,
        dryRun: dryRun.value,
      },
      onProgress: channel,
    });
    view.value = "done";
  } catch (e) {
    // 后端安全校验失败等：退回配置态并提示
    view.value = "config";
    alert(`导出失败：${e}`);
  }
}

function backToConfig() {
  view.value = "config";
  summary.value = null;
}

async function reveal() {
  if (targetDir.value) {
    try {
      await openPath(targetDir.value);
    } catch {
      /* 打开失败忽略 */
    }
  }
}

async function openLog() {
  if (summary.value?.log_path) {
    try {
      await openPath(summary.value.log_path);
    } catch {
      /* 打开失败忽略 */
    }
  }
}

function tryClose() {
  if (view.value === "running") return; // 运行中不允许关闭
  store.closeExport();
}

useEventListener(window, "keydown", (e: KeyboardEvent) => {
  if (!store.exportOpen) return;
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
.dlg-title {
  font-size: 14px;
  font-weight: 700;
  color: var(--text-primary);
}
.x {
  background: none;
  border: none;
  color: var(--text-secondary);
  font-size: 14px;
  cursor: pointer;
  padding: 2px 6px;
  border-radius: 4px;
}
.x:hover:not(:disabled) { background: var(--bg-card); color: var(--text-primary); }
.x:disabled { opacity: 0.3; cursor: default; }

.dlg-body {
  padding: 16px;
  overflow-y: auto;
  display: flex;
  flex-direction: column;
  gap: 16px;
}

.field { display: flex; flex-direction: column; gap: 7px; }
.flabel { font-size: 12px; color: var(--text-secondary); font-weight: 600; }

.target-row { display: flex; gap: 8px; }
.target-box {
  flex: 1;
  min-width: 0;
  padding: 8px 10px;
  background: var(--bg-base);
  border: 1px solid var(--border);
  border-radius: 6px;
  font-size: 12px;
  color: var(--text-primary);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  direction: rtl;
  text-align: left;
}
.target-box.placeholder { color: var(--text-secondary); direction: ltr; }

.warn { font-size: 12px; color: #f59e0b; }

.seg { display: flex; gap: 8px; }
.seg-btn {
  flex: 1;
  padding: 9px;
  background: var(--bg-card);
  border: 1px solid var(--border);
  border-radius: 6px;
  color: var(--text-secondary);
  font-size: 12px;
  cursor: pointer;
  transition: all 0.15s;
}
.seg-btn b { color: var(--text-primary); margin-left: 4px; }
.seg-btn:hover { background: var(--bg-card-hover); }
.seg-btn.on {
  border-color: var(--accent);
  color: var(--accent);
  background: var(--accent-dim);
}
.seg-btn.on b { color: var(--accent); }

.opt {
  display: flex;
  gap: 9px;
  align-items: flex-start;
  cursor: pointer;
  padding: 2px 0;
}
.opt input { margin-top: 2px; accent-color: var(--accent); cursor: pointer; }
.opt span { display: flex; flex-direction: column; gap: 1px; }
.opt b { font-size: 13px; color: var(--text-primary); font-weight: 500; }
.opt i { font-size: 11px; color: var(--text-secondary); font-style: normal; }

.safety {
  font-size: 11px;
  color: var(--text-secondary);
  background: var(--bg-base);
  border: 1px solid var(--border);
  border-radius: 6px;
  padding: 8px 10px;
  line-height: 1.5;
}

/* 进行中 */
.running { align-items: center; text-align: center; padding: 28px 16px; }
.run-logo { font-size: 40px; }
.run-text { font-size: 15px; font-weight: 600; color: var(--text-primary); }
.progress-track {
  width: 100%;
  height: 8px;
  background: var(--bg-card);
  border-radius: 4px;
  overflow: hidden;
}
.progress-fill {
  height: 100%;
  background: var(--accent);
  border-radius: 4px;
  transition: width 0.2s ease;
}
.run-counter { font-size: 13px; color: var(--text-primary); font-variant-numeric: tabular-nums; }
.pct { color: var(--text-secondary); }
.run-current {
  font-size: 11px;
  color: var(--text-secondary);
  max-width: 100%;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  min-height: 14px;
}

/* 完成 */
.done-head { display: flex; align-items: center; gap: 8px; }
.done-logo { font-size: 20px; }
.done-title { font-size: 15px; font-weight: 700; color: var(--text-primary); }
.result { display: flex; flex-direction: column; gap: 7px; }
.rrow {
  display: flex;
  align-items: center;
  justify-content: space-between;
  font-size: 13px;
  color: var(--text-secondary);
}
.rrow b { color: var(--text-primary); font-variant-numeric: tabular-nums; }
.rrow-main { font-size: 14px; }
.rrow-main span { color: var(--text-primary); font-weight: 600; }
.rrow-main b { color: var(--accent); }
.rrow.err b { color: #ef4444; }
.rsub {
  display: flex;
  gap: 14px;
  font-size: 12px;
  color: var(--text-secondary);
  padding-left: 2px;
}
.rdiv { height: 1px; background: var(--border); margin: 3px 0; }
.errbox {
  margin-top: 4px;
  font-size: 11px;
  color: var(--text-secondary);
}
.errbox summary { cursor: pointer; color: #ef4444; }
.erritem {
  padding: 3px 0;
  border-bottom: 1px solid var(--border);
  word-break: break-all;
}

.logrow {
  margin-top: 6px;
  width: 100%;
  text-align: left;
  background: var(--bg-base);
  border: 1px solid var(--border);
  border-radius: 6px;
  padding: 7px 10px;
  font-size: 12px;
  color: var(--text-secondary);
  cursor: pointer;
  transition: color 0.15s, border-color 0.15s;
}
.logrow:hover { color: var(--text-primary); border-color: #555; }

/* 底部 */
.dlg-foot {
  display: flex;
  justify-content: flex-end;
  align-items: center;
  gap: 8px;
  padding: 12px 16px;
  border-top: 1px solid var(--border);
}
.foot-hint { font-size: 12px; color: var(--text-secondary); }

.btn {
  padding: 8px 16px;
  border-radius: 6px;
  font-size: 13px;
  cursor: pointer;
  border: 1px solid var(--border);
  background: var(--bg-card);
  color: var(--text-primary);
  transition: all 0.15s;
}
.btn:hover { background: var(--bg-card-hover); }
.btn.ghost { background: none; color: var(--text-secondary); }
.btn.ghost:hover { color: var(--text-primary); }
.btn.primary {
  background: var(--accent);
  color: #000;
  border-color: var(--accent);
  font-weight: 600;
}
.btn.primary:hover { filter: brightness(1.1); background: var(--accent); }
.btn.primary:disabled { opacity: 0.4; cursor: default; filter: none; }

/* 过渡 */
.dlg-enter-active, .dlg-leave-active { transition: opacity 0.18s ease; }
.dlg-enter-from, .dlg-leave-to { opacity: 0; }
.dlg-enter-active .panel, .dlg-leave-active .panel { transition: transform 0.2s var(--ease-out); }
.dlg-enter-from .panel, .dlg-leave-to .panel { transform: scale(0.96) translateY(8px); }
</style>
