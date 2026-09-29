interface Subscriber {
  resolve: (value: string) => void;
  reject: (error: unknown) => void;
  signal?: AbortSignal;
  abort: () => void;
}
interface Job {
  key: string;
  started: boolean;
  subscribers: Set<Subscriber>;
}

/** A bounded queue with shared in-flight work and independent caller cancellation. */
export function createMediaQueue(run: (key: string) => Promise<string>, concurrency = 4, maxPending = 256) {
  const jobs = new Map<string, Job>();
  const pending: Job[] = [];
  let running = 0;
  const cancelled = () => new DOMException("Media request cancelled", "AbortError");

  function settle(job: Job, value?: string, error?: unknown) {
    jobs.delete(job.key);
    for (const subscriber of job.subscribers) {
      subscriber.signal?.removeEventListener("abort", subscriber.abort);
      if (error !== undefined) subscriber.reject(error);
      else subscriber.resolve(value!);
    }
    job.subscribers.clear();
  }

  function drain() {
    while (running < concurrency && pending.length) {
      const job = pending.shift()!;
      job.started = true;
      running++;
      void (async () => {
        try { settle(job, await run(job.key)); }
        catch (error) { settle(job, undefined, error); }
        finally { running--; drain(); }
      })();
    }
  }

  return (key: string, signal?: AbortSignal): Promise<string> => {
    if (signal?.aborted) return Promise.reject(cancelled());
    let job = jobs.get(key);
    if (!job) {
      // Prioritize newly visible media when a caller does not cancel stale work.
      if (pending.length >= maxPending) settle(pending.shift()!, undefined, cancelled());
      job = { key, started: false, subscribers: new Set() };
      jobs.set(key, job);
      pending.push(job);
    }
    const target = job;
    const promise = new Promise<string>((resolve, reject) => {
      const subscriber: Subscriber = {
        resolve, reject, signal,
        abort: () => {
          signal?.removeEventListener("abort", subscriber.abort);
          target.subscribers.delete(subscriber);
          reject(cancelled());
          if (!target.started && target.subscribers.size === 0) {
            pending.splice(pending.indexOf(target), 1);
            jobs.delete(key);
          }
        },
      };
      target.subscribers.add(subscriber);
      signal?.addEventListener("abort", subscriber.abort, { once: true });
    });
    drain();
    return promise;
  };
}
