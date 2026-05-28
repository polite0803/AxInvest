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
