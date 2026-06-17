import { invoke } from "@tauri-apps/api/core";

const MAX = 4;
let running = 0;
const queue: Array<() => void> = [];

function drain() {
  while (running < MAX && queue.length > 0) {
    running++;
    queue.shift()!();
  }
}

/** 限流版 get_thumbnail：全局最多 4 个并发 invoke */
export function queueThumb(path: string): Promise<string> {
  return new Promise((resolve, reject) => {
    queue.push(async () => {
      try {
        resolve(await invoke<string>("get_thumbnail", { path }));
      } catch (e) {
        reject(e);
      } finally {
        running--;
        drain();
      }
    });
    drain();
  });
}
