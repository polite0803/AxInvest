// SPDX-License-Identifier: AGPL-3.0-only

/**
 * 中性缓存失效广播。
 *
 * 背景（B8-4a / `PLAN-shared-primitives-neutrality.md` §4.2）：设置页保存 vendor 配置后，
 * 需要让股票域 `vendorCheck` 的「已启用 vendor 集合」缓存失效。原先由中性宿主
 * `src/components/stock-analysis/DataVendorsTab.tsx` 直接动态 import 股票域的 `vendorCheck`
 * 并调其 `clearVendorCheckCache()` —— 调用方语义（保存配置 → 失效缓存）与股票无关，
 * 却让宿主反向依赖股票域文件。
 *
 * 改为「谁拥有缓存谁订阅」：缓存持有方（域侧）在模块加载时 `onInvalidate(topic, fn)` 订阅，
 * 宿主只 `invalidate(topic)` 广播，不认识任何域模块。
 *
 * 订阅时序：域模块是懒加载的（面板首次 `checkVendorEnabled` 才 import 它）。未加载时
 * 域侧本就没有缓存可失效，广播落空是安全的；一旦加载即完成订阅，后续广播都能收到。
 */

/** 失效主题。新增主题在此并列登记，避免字符串散落到调用点。 */
export type InvalidationTopic = "vendor-config";

type Listener = () => void;

const listeners = new Map<InvalidationTopic, Set<Listener>>();

/** 订阅某主题的失效通知，返回取消订阅函数。 */
export function onInvalidate(topic: InvalidationTopic, fn: Listener): () => void {
  const set = listeners.get(topic) ?? new Set<Listener>();
  listeners.set(topic, set);
  set.add(fn);
  return () => {
    set.delete(fn);
    if (set.size === 0 && listeners.get(topic) === set) { listeners.delete(topic); }
  };
}

/** 广播失效：同步逐个调用订阅者；单个订阅者抛错不影响其余订阅者。 */
export function invalidate(topic: InvalidationTopic): void {
  const set = listeners.get(topic);
  if (!set) { return; }
  for (const fn of Array.from(set)) {
    try {
      fn();
    } catch (e) {
      console.error(`[cacheInvalidation] 主题 ${topic} 的订阅者执行失败:`, e);
    }
  }
}
