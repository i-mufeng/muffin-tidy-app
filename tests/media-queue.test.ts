import { describe, expect, test } from "bun:test";
import { createMediaQueue } from "../src/composables/mediaQueue";

function harness(concurrency = 2, maxPending = 4) {
  const started: string[] = [];
  const tasks = new Map<string, { resolve: (value: string) => void; reject: (error: Error) => void }>();
  const queue = createMediaQueue((path) => {
    started.push(path);
    return new Promise<string>((resolve, reject) => tasks.set(path, { resolve, reject }));
  }, concurrency, maxPending);
  return { queue, started, tasks };
}
const tick = () => new Promise((resolve) => setTimeout(resolve, 0));

describe("bounded media request queue", () => {
  test("limits active decodes and drains waiting requests", async () => {
    const { queue, started, tasks } = harness();
    const a = queue("a"), b = queue("b"), c = queue("c");
    expect(started).toEqual(["a", "b"]);
    tasks.get("a")!.resolve("image-a");
    expect(await a).toBe("image-a");
    expect(started).toEqual(["a", "b", "c"]);
    tasks.get("b")!.resolve("image-b");
    tasks.get("c")!.resolve("image-c");
    expect(await Promise.all([b, c])).toEqual(["image-b", "image-c"]);
  });

  test("deduplicates paths without cancelling another subscriber", async () => {
    const { queue, started, tasks } = harness();
    const controller = new AbortController();
    const cancelled = queue("same", controller.signal).catch((error) => error.name);
    const retained = queue("same");
    controller.abort();
    expect(await cancelled).toBe("AbortError");
    expect(started).toEqual(["same"]);
    tasks.get("same")!.resolve("shared-image");
    expect(await retained).toBe("shared-image");
  });

  test("unmounted queued consumers never start native work", async () => {
    const { queue, started, tasks } = harness(1);
    const first = queue("first");
    const controller = new AbortController();
    const stale = queue("offscreen", controller.signal).catch((error) => error.name);
    const visible = queue("visible");
    controller.abort();
    expect(await stale).toBe("AbortError");
    tasks.get("first")!.resolve("first");
    await first;
    expect(started).toEqual(["first", "visible"]);
    tasks.get("visible")!.resolve("visible");
    await visible;
  });

  test("bounds pending backlog by dropping the oldest unstarted work", async () => {
    const { queue, started, tasks } = harness(1, 2);
    const first = queue("active");
    const old = queue("old").catch((error) => error.name);
    const next = queue("next");
    const newest = queue("newest");
    expect(await old).toBe("AbortError");
    tasks.get("active")!.resolve("active");
    await first;
    expect(started).toEqual(["active", "next"]);
    tasks.get("next")!.resolve("next");
    await next;
    tasks.get("newest")!.resolve("newest");
    await newest;
    expect(started).toEqual(["active", "next", "newest"]);
  });

  test("a failed decode releases its slot and can be retried", async () => {
    const { queue, started, tasks } = harness(1);
    const first = queue("bad").catch((error) => error.message);
    const second = queue("good");
    tasks.get("bad")!.reject(new Error("decode failed"));
    expect(await first).toBe("decode failed");
    expect(started).toEqual(["bad", "good"]);
    tasks.get("good")!.resolve("good");
    await second;
    const retry = queue("bad");
    tasks.get("bad")!.resolve("recovered");
    expect(await retry).toBe("recovered");
  });

  test("already aborted requests do no work and running cancellations stay bounded", async () => {
    const { queue, started, tasks } = harness(1);
    const aborted = new AbortController();
    aborted.abort();
    expect(await queue("never", aborted.signal).catch((error) => error.name)).toBe("AbortError");
    expect(started).toEqual([]);
    const controller = new AbortController();
    const first = queue("running", controller.signal).catch((error) => error.name);
    controller.abort();
    expect(await first).toBe("AbortError");
    const next = queue("next");
    expect(started).toEqual(["running"]);
    tasks.get("running")!.resolve("discarded");
    await tick();
    expect(started).toEqual(["running", "next"]);
    tasks.get("next")!.resolve("next");
    await next;
  });
});
