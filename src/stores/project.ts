import { defineStore } from "pinia";
import { ref, computed } from "vue";
import { convertFileSrc, invoke, Channel } from "@tauri-apps/api/core";

export type FileStatus = "normal" | "marked" | "removed";
export type LiveType = "apple" | "android" | "huawei" | null;

// Extensions that need Rust-side conversion (WIC) — can't use convertFileSrc directly
export const WIC_EXTS = new Set(["heic","heif","cr2","cr3","nef","nrw","arw","srf","sr2",
  "dng","orf","rw2","raf","pef","rwl","srw"]);

export function needsRustConvert(path: string): boolean {
  const ext = path.split(".").pop()?.toLowerCase() ?? "";
  return WIC_EXTS.has(ext);
}

export interface ProjectFile {
  id: string;
  sourcePath: string;
  mediaType: "img" | "vdo" | "lpo";
  status: FileStatus;
  captureTime: string;
  fileSize: number;
  liveType: LiveType;
  videoPath: string | null;
  videoUrl: string | null;
  duration: number | null;
  exifInfo: Record<string, string>;
}

interface ScanResult {
  id: string;
  source_path: string;
  media_type: "img" | "vdo" | "lpo";
  capture_time: string;
  file_size: number;
  live_type: LiveType;
  video_path: string | null;
  duration: number | null;
  exif_info: Record<string, string>;
}

export const useProjectStore = defineStore("project", () => {
  const sourceDir = ref<string | null>(null);
  const files = ref<ProjectFile[]>([]);
  const focusedIndex = ref(0);
  const isScanning = ref(false);
  const scanProgress = ref({ current: 0, total: 0 });

  // 加载阶段状态机：idle → scanning → preloading → ready
  const phase = ref<"idle" | "scanning" | "preloading" | "ready">("idle");
  const preload = ref({ done: 0, total: 0 });

  // history for undo
  const history = ref<Array<{ index: number; prevStatus: FileStatus }>>([]);

  // 大图浏览器开关（打开时由 Lightbox 接管键盘）
  const viewerOpen = ref(false);

  // 导出对话框开关（打开时挂起网格键盘）
  const exportOpen = ref(false);

  const visibleFiles = computed(() =>
    files.value.filter((f) => f.status !== "removed")
  );

  const stats = computed(() => ({
    total: files.value.length,
    marked: files.value.filter((f) => f.status === "marked").length,
    removed: files.value.filter((f) => f.status === "removed").length,
    normal: files.value.filter((f) => f.status === "normal").length,
  }));

  const focusedFile = computed(() => visibleFiles.value[focusedIndex.value] ?? null);

  async function openDirectory(path: string) {
    sourceDir.value = path;
    phase.value = "scanning";
    isScanning.value = true;
    scanProgress.value = { current: 0, total: 0 };
    files.value = [];
    history.value = [];
    focusedIndex.value = 0;
    preload.value = { done: 0, total: 0 };

    let results: ScanResult[];
    try {
      results = await invoke("scan_directory", { path });
    } catch (e) {
      // 扫描失败 → 退回首页并把错误抛给调用方
      phase.value = "idle";
      sourceDir.value = null;
      isScanning.value = false;
      throw e;
    }
    isScanning.value = false;

    files.value = results.map((r) => ({
      id: r.id,
      sourcePath: r.source_path,
      mediaType: r.media_type,
      status: "normal",
      captureTime: r.capture_time,
      fileSize: r.file_size,
      liveType: r.live_type,
      videoPath: r.video_path,
      videoUrl: r.video_path
        ? r.live_type === "android"
          ? `mtidy-mphoto://video?path=${encodeURIComponent(r.source_path)}`
          : convertFileSrc(r.video_path)
        : r.media_type === "vdo"
          ? convertFileSrc(r.source_path)
          : null,
      duration: r.duration,
      exifInfo: r.exif_info,
    }));

    // 空目录直接就绪；否则进入受控预热阶段（带进度条）
    if (files.value.length === 0) {
      phase.value = "ready";
      return;
    }
    phase.value = "preloading";
    preload.value = { done: 0, total: files.value.length };
    startPreload();
  }

  // 受控预热：Rust 端 2 线程生成缩略图缓存，Channel 回传进度
  function startPreload() {
    const channel = new Channel<{ done: number; total: number }>();
    channel.onmessage = (msg) => {
      preload.value = msg;
      if (msg.done >= msg.total && phase.value === "preloading") {
        phase.value = "ready";
      }
    };
    const paths = files.value.map((f) => f.sourcePath);
    invoke("preload_thumbnails", { paths, onProgress: channel }).catch(() => {
      // 预热失败也允许进入，缩略图会按需懒加载
      if (phase.value === "preloading") phase.value = "ready";
    });
  }

  // 用户跳过预热，直接进入网格（后台预热仍在继续）
  function skipPreload() {
    phase.value = "ready";
  }

  function setStatus(index: number, status: FileStatus) {
    const file = visibleFiles.value[index];
    if (!file) return;
    const realIndex = files.value.indexOf(file);
    history.value.push({ index: realIndex, prevStatus: files.value[realIndex].status });
    files.value[realIndex].status = status;
  }

  function toggleMark(index: number) {
    const file = visibleFiles.value[index];
    if (!file) return;
    const realIndex = files.value.indexOf(file);
    const prev = files.value[realIndex].status;
    history.value.push({ index: realIndex, prevStatus: prev });
    files.value[realIndex].status = prev === "marked" ? "normal" : "marked";
  }

  function removeFile(index: number) {
    setStatus(index, "removed");
    // keep focus in bounds
    if (focusedIndex.value >= visibleFiles.value.length) {
      focusedIndex.value = Math.max(0, visibleFiles.value.length - 1);
    }
  }

  function undoLast() {
    const last = history.value.pop();
    if (!last) return;
    files.value[last.index].status = last.prevStatus;
  }

  function moveFocus(delta: number) {
    const len = visibleFiles.value.length;
    if (len === 0) return;
    focusedIndex.value = Math.max(0, Math.min(len - 1, focusedIndex.value + delta));
  }

  function setFocus(index: number) {
    focusedIndex.value = Math.max(0, Math.min(visibleFiles.value.length - 1, index));
  }

  function openViewer(index: number) {
    setFocus(index);
    viewerOpen.value = true;
  }
  function closeViewer() {
    viewerOpen.value = false;
  }

  function openExport() {
    exportOpen.value = true;
  }
  function closeExport() {
    exportOpen.value = false;
  }

  function reset() {
    sourceDir.value = null;
    files.value = [];
    focusedIndex.value = 0;
    history.value = [];
    phase.value = "idle";
    preload.value = { done: 0, total: 0 };
    viewerOpen.value = false;
    exportOpen.value = false;
  }

  return {
    sourceDir, files, focusedIndex, isScanning, scanProgress, phase, preload, viewerOpen, exportOpen,
    visibleFiles, stats, focusedFile,
    openDirectory, skipPreload, toggleMark, removeFile, undoLast, moveFocus, setFocus,
    openViewer, closeViewer, openExport, closeExport, reset,
  };
});
