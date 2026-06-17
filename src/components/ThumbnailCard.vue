<template>
  <div
    v-if="file"
    class="thumb-card"
    :class="{
      'is-focused': focused,
      'is-marked': file.status === 'marked',
      'is-removed': file.status === 'removed',
    }"
    @click="$emit('focus')"
    @dblclick="$emit('preview')"
    @mouseenter="onMouseEnter"
    @mouseleave="onMouseLeave"
  >
    <!-- 视频悬停播放（Live Photo / Motion Photo / 普通视频） -->
    <video
      v-if="(file.liveType || file.mediaType === 'vdo') && hovering && file.videoUrl"
      :src="file.videoUrl"
      class="thumb-media"
      autoplay
      loop
      muted
      playsinline
    />
    <!-- 图片 / 占位 -->
    <img
      v-else-if="thumbSrc"
      :src="thumbSrc"
      class="thumb-media"
      draggable="false"
    />
    <div v-else class="thumb-placeholder" />

    <!-- 状态角标 -->
    <div v-if="file.status === 'marked'" class="badge badge-mark">★</div>
    <div v-if="file.status === 'removed'" class="badge badge-remove">✕</div>

    <!-- Live/Motion 类型徽章 -->
    <div v-if="file.liveType" class="live-badge">
      {{ file.liveType === 'android' ? 'MOTION' : 'LIVE' }}
    </div>

    <!-- 视频类型徽章（参考 Live 徽章样式，带时长） -->
    <div v-else-if="file.mediaType === 'vdo'" class="live-badge video-badge">
      <span class="tri" />{{ file.duration != null ? fmtDuration(file.duration) : 'VIDEO' }}
    </div>

    <!-- 焦点指示边框 -->
    <div v-if="focused" class="focus-ring" />
  </div>
</template>

<script setup lang="ts">
import { ref, watch, onMounted } from "vue";
import type { ProjectFile } from "../stores/project";
import { queueThumb } from "../composables/useThumbQueue";

const props = defineProps<{
  file: ProjectFile | null;
  focused: boolean;
}>();

defineEmits<{
  focus: [];
  preview: [];
}>();

const hovering = ref(false);
const thumbSrc = ref<string | null>(null);
let hoverTimer: ReturnType<typeof setTimeout> | null = null;

async function loadThumb(file: ProjectFile | null) {
  if (!file) { thumbSrc.value = null; return; }
  const path = file.sourcePath;
  thumbSrc.value = null;
  try {
    const src = await queueThumb(path);
    // 丢弃 virtua 回收后已切换的卡片结果
    if (props.file?.sourcePath === path) thumbSrc.value = src;
  } catch {
    // 保持 null → 显示条纹占位
  }
}

onMounted(() => loadThumb(props.file));
watch(() => props.file, loadThumb);

function fmtDuration(secs: number): string {
  const t = Math.round(secs);
  const h = Math.floor(t / 3600);
  const m = Math.floor((t % 3600) / 60);
  const s = t % 60;
  const pad = (n: number) => String(n).padStart(2, "0");
  return h > 0 ? `${h}:${pad(m)}:${pad(s)}` : `${m}:${pad(s)}`;
}

function onMouseEnter() {
  // Live Photo / Motion Photo / 普通视频 都支持悬停播放
  if (!props.file?.liveType && props.file?.mediaType !== "vdo") return;
  hoverTimer = setTimeout(() => { hovering.value = true; }, 300);
}
function onMouseLeave() {
  if (hoverTimer) clearTimeout(hoverTimer);
  hovering.value = false;
}
</script>

<style scoped>
.thumb-card {
  position: relative;
  aspect-ratio: 1;
  background: var(--bg-card);
  border-radius: 4px;
  overflow: hidden;
  cursor: pointer;
  transition: opacity 0.15s;
}
.thumb-card:hover { background: var(--bg-card-hover); }
.thumb-card.is-removed { opacity: 0.3; }
.thumb-card.is-marked { box-shadow: inset 0 0 0 2px var(--marked); }

.thumb-media {
  width: 100%;
  height: 100%;
  object-fit: cover;
  display: block;
}

.thumb-placeholder {
  width: 100%;
  height: 100%;
  background: repeating-linear-gradient(
    45deg,
    var(--bg-card) 0px,
    var(--bg-card) 8px,
    var(--bg-card-hover) 8px,
    var(--bg-card-hover) 16px
  );
}

.badge {
  position: absolute;
  top: 4px;
  right: 4px;
  width: 20px;
  height: 20px;
  border-radius: 50%;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 11px;
  font-weight: 700;
  line-height: 1;
}
.badge-mark  { background: var(--marked); color: #fff; }
.badge-remove { background: #555; color: #ccc; }

.live-badge {
  position: absolute;
  bottom: 4px;
  left: 4px;
  background: rgba(0,0,0,0.6);
  color: #fff;
  font-size: 9px;
  font-weight: 700;
  letter-spacing: 0.05em;
  padding: 2px 5px;
  border-radius: 3px;
}

.video-badge {
  display: flex;
  align-items: center;
  gap: 4px;
}
.video-badge .tri {
  width: 0;
  height: 0;
  border-style: solid;
  border-width: 4px 0 4px 6px;
  border-color: transparent transparent transparent #fff;
}

.focus-ring {
  position: absolute;
  inset: 0;
  border: 2px solid var(--accent);
  border-radius: 4px;
  pointer-events: none;
}
</style>
