<template>
  <div class="home">
    <div class="card">
      <div class="logo">🧁</div>
      <h1 class="title">Muffin Tidy</h1>
      <p class="subtitle">选择一个目录开始整理媒体文件</p>

      <button class="btn-open" @click="pickDir" :disabled="loading">
        {{ loading ? '扫描中…' : '📁 打开目录' }}
      </button>

      <div v-if="error" class="error">{{ error }}</div>

      <div class="hints">
        <div class="hint-row"><kbd>Space</kbd> 标记 / 取消标记</div>
        <div class="hint-row"><kbd>D</kbd> / <kbd>Del</kbd> 从工程移除（不删文件）</div>
        <div class="hint-row"><kbd>↑ ↓ ← →</kbd> 导航</div>
        <div class="hint-row"><kbd>Ctrl Z</kbd> 撤销</div>
      </div>
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref } from "vue";
import { open } from "@tauri-apps/plugin-dialog";
import { useProjectStore } from "../stores/project";

const store = useProjectStore();
const loading = ref(false);
const error = ref<string | null>(null);

async function pickDir() {
  error.value = null;
  const selected = await open({ directory: true, multiple: false });
  if (!selected || typeof selected !== "string") return;

  loading.value = true;
  try {
    await store.openDirectory(selected);
  } catch (e) {
    error.value = String(e);
  } finally {
    loading.value = false;
  }
}
</script>

<style scoped>
.home {
  display: flex;
  align-items: center;
  justify-content: center;
  height: 100%;
  background: var(--bg-base);
}

.card {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 16px;
  padding: 48px;
  background: var(--bg-panel);
  border: 1px solid var(--border);
  border-radius: 12px;
  min-width: 380px;
}

.logo { font-size: 48px; }

.title {
  margin: 0;
  font-size: 24px;
  font-weight: 700;
  color: var(--text-primary);
}

.subtitle {
  margin: 0;
  font-size: 14px;
  color: var(--text-secondary);
}

.btn-open {
  width: 100%;
  padding: 12px;
  background: var(--accent);
  color: #000;
  border: none;
  border-radius: 6px;
  font-size: 14px;
  font-weight: 600;
  cursor: pointer;
  margin-top: 8px;
}
.btn-open:hover { filter: brightness(1.1); }
.btn-open:disabled { opacity: 0.5; cursor: default; }

.error {
  color: #ef4444;
  font-size: 12px;
  text-align: center;
}

.hints {
  width: 100%;
  padding-top: 8px;
  border-top: 1px solid var(--border);
  display: flex;
  flex-direction: column;
  gap: 4px;
}
.hint-row {
  font-size: 12px;
  color: var(--text-secondary);
  display: flex;
  align-items: center;
  gap: 8px;
}
kbd {
  background: var(--bg-card);
  border: 1px solid var(--border);
  border-radius: 3px;
  padding: 1px 6px;
  font-size: 11px;
  font-family: monospace;
  color: var(--text-primary);
}
</style>
