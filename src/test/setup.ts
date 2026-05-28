import "@testing-library/jest-dom";

// jsdom 销毁后 react-dom scheduler 异步回调仍可能访问 window/document。
// defineProperty(getter) 在 configurable:false 时阻止 Vitest teardown 删除属性；
// configurable:true 时 jsdom 删除属性后兜底值也丢失。
// 这里用简单赋值 + 再定义不可配置属性：赋值兼容 jsdom 初始化，
// 后续 defineProperty(writable:true, configurable:false) 防止被 delete 删除。
{
  const g = globalThis as Record<string, unknown>;
  const fallbackDoc = {
    createElement: () => ({}),
    documentElement: {},
    body: {},
    head: {},
  };
  // 确保属性存在（jsdom 在设置前 window/document 可能不存在）
  g.window ??= g;
  g.document ??= fallbackDoc;
  // 用 writable+configurable:false 的属性替代可能被删除的 getter
  let _win = g.window;
  let _doc = g.document;
  // 先尝试删除旧的（可能来自 jsdom 的 configurable:true 属性）
  try {
    delete (g as Record<string, unknown>).window;
  } catch { /* */ }
  try {
    delete (g as Record<string, unknown>).document;
  } catch { /* */ }
  Object.defineProperty(g, "window", {
    get() {
      return _win ?? g;
    },
    set(v) {
      if (v !== undefined) { _win = v; }
    },
    configurable: true,
    enumerable: true,
  });
  Object.defineProperty(g, "document", {
    get() {
      return _doc ?? fallbackDoc;
    },
    set(v) {
      if (v !== undefined) { _doc = v; }
    },
    configurable: true,
    enumerable: true,
  });
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
