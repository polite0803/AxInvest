import { invoke } from "@/lib/invoke";
import type { RecoIcStats } from "@/types";

/**
 * `reco_ic_stats` 的唯一取数入口（单飞 + 短 TTL 缓存）。
 *
 * 为什么需要它：同一屏的「回测 → 策略」矩阵（RecoStrategyMatrix）与荐股反思面板
 * （RecoLoopPanel）各自在挂载时取一次观测面，React StrictMode 下每次挂载还会双跑
 * ⇒ 一次页面导航会并发发 4 个同参请求。该命令要读全表校验样本 + 逐格聚合，
 * 把 4 次合并成 1 次可显著缩短首屏等待（实测慢调用日志正是 4 条同参调用）。
 */
const TTL_MS = 10_000;

const cache = new Map<string, { at: number; data: RecoIcStats }>();
const inflight = new Map<string, Promise<RecoIcStats>>();

function keyOf(generation?: string | null): string {
  return generation ?? "";
}

export async function loadRecoIcStats(generation?: string | null): Promise<RecoIcStats> {
  const key = keyOf(generation);
  const hit = cache.get(key);
  if (hit && Date.now() - hit.at < TTL_MS) { return hit.data; }
  const pending = inflight.get(key);
  if (pending) { return pending; }

  const task = (async (): Promise<RecoIcStats> => {
    try {
      const data = await invoke<RecoIcStats>("reco_ic_stats", { generation: generation ?? null });
      // 测试里 invoke 可能被 mock 成同步 undefined；空值不写缓存，避免后续测试命中脏数据
      if (data) { cache.set(key, { at: Date.now(), data }); }
      return data;
    } finally {
      inflight.delete(key);
    }
  })();
  inflight.set(key, task);
  return task;
}

/** 权重重算后必须失效，否则 TTL 内读到的还是旧权重。 */
export function invalidateRecoIcStats(): void {
  cache.clear();
}
