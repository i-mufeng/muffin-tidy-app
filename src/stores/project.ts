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

const PRELOAD_LIMIT = 48;

interface ScanBatch { files: ScanResult[]; total: number; done: boolean }

export interface ScanProgress {
  stage: "discovering" | "metadata" | "pairing" | "loading";
  current: number;
  total: number;
}

export const useProjectStore = defineStore("project", () => {
  const sourceDir = ref<string | null>(null);
  const files = ref<ProjectFile[]>([]);
  const focusedIndex = ref(0);
  const isScanning = ref(false);
  const isRefreshing = ref(false);
  const scanProgress = ref<ScanProgress>({ stage: "discovering", current: 0, total: 0 });
  const scanError = ref<string | null>(null);

  // 加载阶段状态机：idle → scanning → preloading → ready
  const phase = ref<"idle" | "scanning" | "preloading" | "ready">("idle");
  const preload = ref({ done: 0, total: 0 });

  // history for undo
  const history = ref<Array<{ index: number; prevStatus: FileStatus }>>([]);

  // 大图浏览器开关（打开时由 Lightbox 接管键盘）
  const viewerOpen = ref(false);

  // 导出对话框开关（打开时挂起网格键盘）
  const exportOpen = ref(false);
  const importOpen = ref(false);
  // 成功复制的文件保留在本地，重置工作集不删除尚未导出的照片。
  const importTempDir = ref<string | null>(null);
  const viewFilter = ref<"all" | "marked" | "removed">("all");
  const toast = ref<string | null>(null);
  let toastTimer: ReturnType<typeof setTimeout> | null = null;
  let scanRequestId = 0;

  const visibleFiles = computed(() =>
    viewFilter.value === "removed"
      ? files.value.filter((f) => f.status === "removed")
      : viewFilter.value === "marked"
        ? files.value.filter((f) => f.status === "marked")
        : files.value.filter((f) => f.status !== "removed")
  );

  const exportableFiles = computed(() => files.value.filter((f) => f.status !== "removed"));

  const stats = computed(() => ({
    total: files.value.length,
    marked: files.value.filter((f) => f.status === "marked").length,
    removed: files.value.filter((f) => f.status === "removed").length,
    normal: files.value.filter((f) => f.status === "normal").length,
  }));

  const focusedFile = computed(() => visibleFiles.value[focusedIndex.value] ?? null);

  async function appendScanResults(results: ScanResult[], requestId: number, mapped: ProjectFile[], total: number) {
    for (let offset = 0; offset < results.length; offset += 500) {
      // Allow painting and cancellation between batches, including before the first batch.
      await new Promise<void>((resolve) => setTimeout(resolve, 0));
      if (requestId !== scanRequestId) return null;
      mapped.push(...results.slice(offset, offset + 500).map((r): ProjectFile => ({
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
      })));
      scanProgress.value = { stage: "loading", current: mapped.length, total };
    }
    return mapped;
  }

  function scanChannel(requestId: number) {
    const channel = new Channel<ScanProgress>();
    channel.onmessage = (progress) => {
      if (requestId === scanRequestId && isScanning.value && scanProgress.value.stage !== "loading") {
        scanProgress.value = progress;
      }
    };
    return channel;
  }

  async function scanFiles(path: string, requestId: number): Promise<ProjectFile[] | null> {
    const mapped: ProjectFile[] = [];
    let processing = Promise.resolve();
    let finish!: () => void;
    let fail!: (error: unknown) => void;
    const delivered = new Promise<void>((resolve, reject) => { finish = resolve; fail = reject; });
    const onFiles = new Channel<ScanBatch>();
    onFiles.onmessage = (batch) => {
      processing = processing.then(async () => {
        if (requestId !== scanRequestId) return;
        scanProgress.value = { stage: "loading", current: mapped.length, total: batch.total };
        await appendScanResults(batch.files, requestId, mapped, batch.total);
      });
      processing.catch(fail);
      if (batch.done) processing.then(finish, fail);
    };
    // Command completion and Channel delivery can arrive in different orders.
    await Promise.all([
      invoke<void>("scan_directory", { path, onProgress: scanChannel(requestId), onFiles }),
      delivered,
    ]);
    return requestId === scanRequestId ? mapped : null;
  }

  async function openDirectory(path: string) {
    if (isScanning.value) return false;
    const requestId = ++scanRequestId;
    sourceDir.value = path;
    if (path !== importTempDir.value) importTempDir.value = null;
    phase.value = "scanning";
    isScanning.value = true;
    isRefreshing.value = false;
    scanProgress.value = { stage: "discovering", current: 0, total: 0 };
    scanError.value = null;
    files.value = [];
    history.value = [];
    focusedIndex.value = 0;
    viewFilter.value = "all";
    preload.value = { done: 0, total: 0 };

    let mapped: ProjectFile[] | null;
    try {
      mapped = await scanFiles(path, requestId);
    } catch (e) {
      if (requestId !== scanRequestId) return false;
      ++scanRequestId; // Discard any already queued batches from a failed transfer.
      scanError.value = String(e);
      // 扫描失败 → 退回首页并把错误抛给调用方
      phase.value = "idle";
      sourceDir.value = null;
      isScanning.value = false;
      throw e;
    }
    if (requestId !== scanRequestId) return false;
    if (!mapped || requestId !== scanRequestId) return false;
    files.value = mapped;
    isScanning.value = false;

    // 空目录直接就绪；否则进入受控预热阶段（带进度条）
    if (files.value.length === 0) {
      phase.value = "ready";
      return true;
    }
    phase.value = "preloading";
    preload.value = { done: 0, total: Math.min(PRELOAD_LIMIT, files.value.length) };
    startPreload(requestId);
    return true;
  }

  async function refreshDirectory() {
    if (!sourceDir.value || isScanning.value) return false;
    const path = sourceDir.value;
    const requestId = ++scanRequestId;
    isScanning.value = true;
    isRefreshing.value = true;
    scanProgress.value = { stage: "discovering", current: 0, total: 0 };
    scanError.value = null;

    let mapped: ProjectFile[] | null;
    try {
      mapped = await scanFiles(path, requestId);
    } catch (e) {
      if (requestId !== scanRequestId) return false;
      ++scanRequestId; // Discard any already queued batches from a failed transfer.
      isScanning.value = false;
      isRefreshing.value = false;
      scanError.value = String(e);
      throw e;
    }
    if (requestId !== scanRequestId) return false;

    if (!mapped || requestId !== scanRequestId) return false;
    // Read the latest statuses so edits made while refreshing are preserved.
    const previousStatuses = new Map(files.value.map((file) => [file.sourcePath, file.status]));
    for (const file of mapped) file.status = previousStatuses.get(file.sourcePath) ?? "normal";
    files.value = mapped;
    focusedIndex.value = 0;
    history.value = [];
    isScanning.value = false;
    isRefreshing.value = false;
    toast.value = `已刷新，共 ${files.value.length} 个媒体`;
    if (toastTimer) clearTimeout(toastTimer);
    toastTimer = setTimeout(() => (toast.value = null), 1800);
    return true;
  }

  function cancelRefresh() {
    if (!isRefreshing.value) return;
    ++scanRequestId;
    invoke("cancel_scan").catch(() => {});
    isScanning.value = false;
    isRefreshing.value = false;
    toast.value = "已取消刷新";
    if (toastTimer) clearTimeout(toastTimer);
    toastTimer = setTimeout(() => (toast.value = null), 1800);
  }

  function setFilter(filter: "all" | "marked" | "removed") {
    viewFilter.value = filter;
    focusedIndex.value = 0;
  }

  // 受控预热：Rust 端 2 线程生成缩略图缓存，Channel 回传进度
  function startPreload(requestId: number) {
    const channel = new Channel<{ done: number; total: number }>();
    channel.onmessage = (msg) => {
      if (requestId !== scanRequestId) return;
      preload.value = msg;
      if (msg.done >= msg.total && phase.value === "preloading") {
        phase.value = "ready";
      }
    };
    const paths = files.value.slice(0, PRELOAD_LIMIT).map((f) => f.sourcePath);
    invoke("preload_thumbnails", { paths, onProgress: channel }).catch(() => {
      // 预热失败也允许进入，缩略图会按需懒加载
      if (requestId === scanRequestId && phase.value === "preloading") phase.value = "ready";
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

  function restoreFile(index: number) {
    const file = visibleFiles.value[index];
    if (!file || file.status !== "removed") return;
    const realIndex = files.value.indexOf(file);
    history.value.push({ index: realIndex, prevStatus: "removed" });
    files.value[realIndex].status = "normal";
    if (focusedIndex.value >= visibleFiles.value.length) {
      focusedIndex.value = Math.max(0, visibleFiles.value.length - 1);
    }
  }

  function undoLast() {
    const last = history.value.pop();
    if (!last) return;
    const currentStatus = files.value[last.index].status;
    files.value[last.index].status = last.prevStatus;
    toast.value = currentStatus === "removed" ? "已撤销移除" : currentStatus === "marked" ? "已撤销标记" : "已撤销操作";
    if (toastTimer) clearTimeout(toastTimer);
    toastTimer = setTimeout(() => (toast.value = null), 1800);
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

  function openImport() {
    if (isScanning.value || phase.value === "preloading" || exportOpen.value || viewerOpen.value) return;
    importOpen.value = true;
  }

  function closeImport() {
    importOpen.value = false;
  }

  async function openImportedDirectory(path: string): Promise<boolean> {
    if (!path.trim()) throw new Error("手机导入未返回有效的本地目录");
    if (isScanning.value) throw new Error("请等待当前扫描完成后再导入");
    importTempDir.value = path;
    // 转入现有扫描页面，失败由首页 scanError 展示，避免重开弹窗覆盖错误。
    importOpen.value = false;
    return await openDirectory(path);
  }

  function reset() {
    const shouldCancelScan = sourceDir.value !== null;
    ++scanRequestId;
    if (shouldCancelScan) invoke("cancel_scan").catch(() => {});
    sourceDir.value = null;
    scanError.value = null;
    scanProgress.value = { stage: "discovering", current: 0, total: 0 };
    files.value = [];
    focusedIndex.value = 0;
    history.value = [];
    phase.value = "idle";
    preload.value = { done: 0, total: 0 };
    viewerOpen.value = false;
    exportOpen.value = false;
    importOpen.value = false;
    importTempDir.value = null;
    viewFilter.value = "all";
    toast.value = null;
    isScanning.value = false;
    isRefreshing.value = false;
  }

  return {
    sourceDir, files, focusedIndex, isScanning, isRefreshing, scanProgress, scanError, phase, preload, viewerOpen, exportOpen, importOpen, importTempDir,
    viewFilter, toast, visibleFiles, exportableFiles, stats, focusedFile,
    openDirectory, refreshDirectory, cancelRefresh, setFilter, skipPreload, toggleMark, removeFile, restoreFile, undoLast, moveFocus, setFocus,
    openViewer, closeViewer, openExport, closeExport, openImport, closeImport, openImportedDirectory, reset,
  };
});
