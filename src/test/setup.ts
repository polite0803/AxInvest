import "@testing-library/jest-dom";
import { afterEach } from "vitest";

// React 19 scheduler 在 vitest 销毁 jsdom 后通过 setImmediate 回调访问 window
// 导致 ReferenceError。在每个测试后清空 scheduler 队列。
afterEach(async () => {
  try {
    const ReactDOM = await import("react-dom");
    if (typeof (ReactDOM as any).flushSync === "function") {
      try {
        (ReactDOM as any).flushSync(() => {});
      } catch { /* */ }
    }
  } catch { /* */ }
  // 等一个 microtask 让 pending setImmediate 有机会执行
  await new Promise((r) => setTimeout(r, 0));
});

if (typeof window === "undefined") {
  // @ts-expect-error
  globalThis.window = globalThis;
}

if (typeof window !== "undefined" && !window.matchMedia) {
  Object.defineProperty(window, "matchMedia", {
    writable: true,
    value: (query: string) => ({
      matches: false,
      media: query,
      onchange: null,
      addListener: () => {},
      removeListener: () => {},
      addEventListener: () => {},
      removeEventListener: () => {},
      dispatchEvent: () => false,
    }),
  });
}

if (typeof globalThis.ResizeObserver === "undefined") {
  class ResizeObserver {
    observe() {}
    unobserve() {}
    disconnect() {}
  }
  globalThis.ResizeObserver = ResizeObserver;
}

// 用 setTimeout 替代 setImmediate — React scheduler 在 jsdom 销毁后
// 仍可能通过 setImmediate 触发回调，导致 "window is not defined"
// setTimeout 确保回调在 jsdom 生命周期内执行完毕
if (typeof globalThis.setImmediate !== "undefined") {
  const pending = new Map<number, ReturnType<typeof setTimeout>>();
  let nextId = 1;
  (globalThis as any).setImmediate = (fn: (...args: any[]) => void, ...args: any[]) => {
    const id = nextId++;
    pending.set(
      id,
      setTimeout(() => {
        pending.delete(id);
        // jsdom 销毁后 React scheduler 回调仍可能执行，window 已变为 undefined
        try {
          fn(...args);
        } catch {
          // React performWorkOnRootViaSchedulerTask — silently discard post-teardown errors
        }
      }, 0),
    );
    return id;
  };
  (globalThis as any).clearImmediate = (id: number) => {
    const timer = pending.get(id);
    if (timer) {
      clearTimeout(timer);
      pending.delete(id);
    }
  };
}
