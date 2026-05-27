import "@testing-library/jest-dom";

// React 19 scheduler 在 vitest 销毁 jsdom 后通过 setImmediate 访问 window
// 导致 ReferenceError。用 getter 固化 window，阻止 jsdom teardown 回收。
let _win: any = globalThis;
Object.defineProperty(globalThis, "window", {
  get() {
    return _win;
  },
  set(v: any) {
    if (v != null && v !== globalThis) { _win = v; }
  },
  configurable: true,
  enumerable: true,
});

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
