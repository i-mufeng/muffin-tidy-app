import { invoke } from "@tauri-apps/api/core";
import { createMediaQueue } from "./mediaQueue";

// Only visible subscribers stay queued; callers sharing a path share one decode.
export const queueThumb = createMediaQueue(
  (path) => invoke<string>("get_thumbnail", { path }), 4, 256,
);
export const queuePreview = createMediaQueue(
  (path) => invoke<string>("get_preview", { path }), 1, 8,
);
