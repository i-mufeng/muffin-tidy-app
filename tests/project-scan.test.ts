import { beforeEach, describe, expect, mock, test } from "bun:test";
import { createPinia, setActivePinia } from "pinia";

class TestChannel<T = unknown> {
  onmessage: (message: T) => void = () => {};
}

interface PendingCall {
  command: string;
  args: Record<string, any>;
  resolve: (value: any) => void;
  reject: (error: unknown) => void;
}

let calls: PendingCall[] = [];
mock.module("@tauri-apps/api/core", () => ({
  Channel: TestChannel,
  convertFileSrc: (path: string) => `asset://${path}`,
  invoke: (command: string, args: Record<string, any> = {}) => {
    if (command === "cancel_scan") return Promise.resolve();
    return new Promise((resolve, reject) => calls.push({ command, args, reject, resolve: (value) => {
      if (command === "scan_directory" && Array.isArray(value)) {
        // Match the native bounded Channel contract, including the empty final batch.
        for (let offset = 0; offset < Math.max(value.length, 1); offset += 256) {
          args.onFiles.onmessage({ files: value.slice(offset, offset + 256), total: value.length, done: offset + 256 >= value.length });
        }
        resolve(undefined);
      } else resolve(value);
    } }));
  },
}));

const { useProjectStore } = await import("../src/stores/project");

function media(index: number, directory = "/photos") {
  return {
    id: String(index), source_path: `${directory}/${index}.jpg`, media_type: "img",
    capture_time: "2026-09-29T12:00:00", file_size: 1024,
    live_type: null, video_path: null, duration: null, exif_info: {},
  };
}

function lastCall(command: string) {
  const call = calls.filter((candidate) => candidate.command === command).at(-1);
  expect(call).toBeDefined();
  return call!;
}

async function openReady(store: ReturnType<typeof useProjectStore>) {
  const opened = store.openDirectory("/photos");
  lastCall("scan_directory").resolve([media(1), media(2)]);
  expect(await opened).toBe(true);
  lastCall("preload_thumbnails").args.onProgress.onmessage({ done: 2, total: 2 });
  expect(store.phase).toBe("ready");
}

beforeEach(() => {
  calls = [];
  setActivePinia(createPinia());
});

describe("asynchronous directory scanning", () => {
  test("reports scan progress and proceeds through thumbnail preload", async () => {
    const store = useProjectStore();
    const opened = store.openDirectory("/photos");
    const scan = lastCall("scan_directory");
    expect(store.phase).toBe("scanning");
    scan.args.onProgress.onmessage({ stage: "metadata", current: 1, total: 2 });
    expect(store.scanProgress).toEqual({ stage: "metadata", current: 1, total: 2 });
    scan.resolve([media(1), media(2)]);
    expect(await opened).toBe(true);
    expect(store.files).toHaveLength(2);
    expect(store.phase).toBe("preloading");
    lastCall("preload_thumbnails").args.onProgress.onmessage({ done: 2, total: 2 });
    expect(store.phase).toBe("ready");
    expect(store.isScanning).toBe(false);
  });

  test("empty directory becomes ready without starting thumbnail work", async () => {
    const store = useProjectStore();
    const opened = store.openDirectory("/empty");
    lastCall("scan_directory").resolve([]);
    expect(await opened).toBe(true);
    expect(store.phase).toBe("ready");
    expect(store.files).toHaveLength(0);
    expect(calls.some((call) => call.command === "preload_thumbnails")).toBe(false);
  });

  test("cancelled scan cannot update a replacement scan with late progress or results", async () => {
    const store = useProjectStore();
    const first = store.openDirectory("/first");
    const oldScan = lastCall("scan_directory");
    store.reset();
    const second = store.openDirectory("/second");
    const newScan = lastCall("scan_directory");
    newScan.args.onProgress.onmessage({ stage: "metadata", current: 1, total: 10 });
    oldScan.args.onProgress.onmessage({ stage: "pairing", current: 50, total: 50 });
    oldScan.resolve([media(1, "/first")]);
    expect(await first).toBe(false);
    expect(store.sourceDir).toBe("/second");
    expect(store.scanProgress).toEqual({ stage: "metadata", current: 1, total: 10 });
    expect(store.files).toHaveLength(0);
    newScan.resolve([]);
    expect(await second).toBe(true);
  });

  test("old thumbnail progress and errors cannot complete a new directory preload", async () => {
    const store = useProjectStore();
    const first = store.openDirectory("/first");
    lastCall("scan_directory").resolve([media(1, "/first")]);
    await first;
    const oldPreload = lastCall("preload_thumbnails");
    store.reset();
    const second = store.openDirectory("/second");
    lastCall("scan_directory").resolve([media(2, "/second"), media(3, "/second")]);
    await second;
    oldPreload.args.onProgress.onmessage({ done: 1, total: 1 });
    oldPreload.reject(new Error("old preload failed"));
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(store.phase).toBe("preloading");
    expect(store.preload).toEqual({ done: 0, total: 2 });
    lastCall("preload_thumbnails").args.onProgress.onmessage({ done: 2, total: 2 });
    expect(store.phase).toBe("ready");
  });

  test("refresh preserves edits made while the scan is running", async () => {
    const store = useProjectStore();
    await openReady(store);
    const refreshed = store.refreshDirectory();
    store.toggleMark(0);
    store.removeFile(1);
    lastCall("scan_directory").resolve([media(1), media(2), media(3)]);
    expect(await refreshed).toBe(true);
    expect(store.files.map((file) => file.status)).toEqual(["marked", "removed", "normal"]);
    expect(store.isRefreshing).toBe(false);
  });

  test("cancelled refresh retains existing files and ignores late responses", async () => {
    const store = useProjectStore();
    await openReady(store);
    store.toggleMark(0);
    const refreshed = store.refreshDirectory();
    const scan = lastCall("scan_directory");
    store.cancelRefresh();
    scan.args.onProgress.onmessage({ stage: "metadata", current: 99, total: 100 });
    scan.resolve([media(3)]);
    expect(await refreshed).toBe(false);
    expect(store.files.map((file) => file.id)).toEqual(["1", "2"]);
    expect(store.files[0].status).toBe("marked");
    expect(store.isScanning).toBe(false);
    expect(store.scanProgress.current).not.toBe(99);
  });

  test("large result mapping yields to cancellation before publishing files", async () => {
    const store = useProjectStore();
    const opened = store.openDirectory("/photos");
    lastCall("scan_directory").resolve(Array.from({ length: 5000 }, (_, index) => media(index)));
    const cancellation = new Promise<void>((resolve) => setTimeout(() => {
      expect(store.isScanning).toBe(true);
      store.reset();
      resolve();
    }, 0));
    expect(await opened).toBe(false);
    await cancellation;
    expect(store.phase).toBe("idle");
    expect(store.files).toHaveLength(0);
    expect(calls.some((call) => call.command === "preload_thumbnails")).toBe(false);
  });

  test("large library preloads only 48 items and receives bounded batches", async () => {
    const store = useProjectStore();
    const opened = store.openDirectory("/photos");
    lastCall("scan_directory").resolve(Array.from({ length: 20000 }, (_, index) => media(index)));
    expect(await opened).toBe(true);
    expect(store.files).toHaveLength(20000);
    expect(lastCall("preload_thumbnails").args.paths).toHaveLength(48);
    expect(store.preload.total).toBe(48);
    expect(store.scanProgress).toEqual({ stage: "loading", current: 20000, total: 20000 });
  });

  test("command completion waits for the final result batch", async () => {
    const store = useProjectStore();
    const opened = store.openDirectory("/photos");
    const scan = lastCall("scan_directory");
    scan.resolve(undefined);
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(store.isScanning).toBe(true);
    scan.args.onFiles.onmessage({ files: [media(1)], total: 1, done: true });
    expect(await opened).toBe(true);
    expect(store.files).toHaveLength(1);
  });

  test("failed partial transfer ignores already queued and late batches", async () => {
    const store = useProjectStore();
    const opened = store.openDirectory("/photos");
    const scan = lastCall("scan_directory");
    scan.args.onFiles.onmessage({ files: [media(1)], total: 2, done: false });
    scan.reject("transfer failed");
    await expect(opened).rejects.toBe("transfer failed");
    const progress = { ...store.scanProgress };
    scan.args.onFiles.onmessage({ files: [media(2)], total: 2, done: true });
    await new Promise((resolve) => setTimeout(resolve, 5));
    expect(store.files).toHaveLength(0);
    expect(store.scanProgress).toEqual(progress);
    expect(store.phase).toBe("idle");
  });

  test("scan failure is visible and reset clears the error", async () => {
    const store = useProjectStore();
    const opened = store.openDirectory("/missing");
    lastCall("scan_directory").reject(new Error("directory unavailable"));
    await opened.catch(() => {});
    expect(store.phase).toBe("idle");
    expect(String(store.scanError)).toContain("directory unavailable");
    expect(store.isScanning).toBe(false);
    store.reset();
    expect(store.scanError).toBeFalsy();
  });

  test("failed opening can be retried successfully", async () => {
    const store = useProjectStore();
    const failed = store.openDirectory("/offline");
    lastCall("scan_directory").reject("directory unavailable");
    await expect(failed).rejects.toBe("directory unavailable");
    expect(store.scanError).toBeTruthy();
    const retry = store.openDirectory("/photos");
    expect(store.scanError).toBeFalsy();
    expect(store.isScanning).toBe(true);
    lastCall("scan_directory").resolve([]);
    expect(await retry).toBe(true);
    expect(store.phase).toBe("ready");
  });

  test("failed refresh preserves workspace and allows a successful retry", async () => {
    const store = useProjectStore();
    await openReady(store);
    store.toggleMark(0);
    const failed = store.refreshDirectory();
    lastCall("scan_directory").reject("drive disconnected");
    await expect(failed).rejects.toBe("drive disconnected");
    expect(store.scanError).toBe("drive disconnected");
    expect(store.phase).toBe("ready");
    expect(store.isScanning).toBe(false);
    expect(store.isRefreshing).toBe(false);
    expect(store.files).toHaveLength(2);
    expect(store.files[0].status).toBe("marked");
    const retry = store.refreshDirectory();
    expect(store.scanError).toBeFalsy();
    lastCall("scan_directory").resolve([media(1), media(2), media(3)]);
    expect(await retry).toBe(true);
    expect(store.files).toHaveLength(3);
    expect(store.files[0].status).toBe("marked");
  });
});


describe("phone import workspace", () => {
  test("imported directory enters the existing scan/preload pipeline", async () => {
    const store = useProjectStore();
    store.openImport();
    expect(store.importOpen).toBe(true);
    const opened = store.openImportedDirectory("/cache/phone-session");
    expect(store.importOpen).toBe(false);
    expect(store.importTempDir).toBe("/cache/phone-session");
    expect(lastCall("scan_directory").args.path).toBe("/cache/phone-session");
    lastCall("scan_directory").resolve([media(1, "/cache/phone-session")]);
    expect(await opened).toBe(true);
    expect(store.phase).toBe("preloading");
    expect(store.files[0].sourcePath).toBe("/cache/phone-session/1.jpg");
    store.reset();
    expect(store.importTempDir).toBeNull();
    expect(calls.some(call => call.command === "pd_cleanup")).toBe(false);
  });

  test("failed imported scan exposes the scan error and preserves the cache location", async () => {
    const store = useProjectStore();
    const opened = store.openImportedDirectory("/cache/phone-session");
    lastCall("scan_directory").reject("read failed");
    await expect(opened).rejects.toBe("read failed");
    expect(store.importOpen).toBe(false);
    expect(store.importTempDir).toBe("/cache/phone-session");
    expect(store.scanError).toBe("read failed");
  });

  test("cancelled import scan never reopens the dialog or overwrites a new directory", async () => {
    const store = useProjectStore();
    const opened = store.openImportedDirectory("/cache/phone-session");
    const oldScan = lastCall("scan_directory");
    store.reset();
    const newer = store.openDirectory("/new");
    oldScan.reject("cancelled");
    expect(await opened).toBe(false);
    expect(store.importOpen).toBe(false);
    expect(store.sourceDir).toBe("/new");
    expect(store.importTempDir).toBeNull();
    lastCall("scan_directory").resolve([]);
    expect(await newer).toBe(true);
  });

  test("rejects empty paths and does not interrupt an existing scan", async () => {
    const store = useProjectStore();
    await expect(store.openImportedDirectory(" ")).rejects.toThrow("有效");
    expect(calls).toHaveLength(0);
    const opened = store.openDirectory("/photos");
    store.openImport();
    expect(store.importOpen).toBe(false);
    await expect(store.openImportedDirectory("/cache/session")).rejects.toThrow("扫描");
    expect(store.sourceDir).toBe("/photos");
    lastCall("scan_directory").resolve([]);
    await opened;
  });
});
