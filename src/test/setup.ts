// SPDX-License-Identifier: AGPL-3.0-only

import "@testing-library/jest-dom";

// Node 24 自带惰性 webstorage getter（未提供 --localstorage-file ⇒ 恒返回 undefined），
// 而 vitest 4 的 populateGlobal 对「globalThis 上已存在的键」只转发其白名单 KEYS，
// localStorage / sessionStorage 不在名单 ⇒ jsdom 的真 Storage 到不了测试全局
// （347 个测试因此挂：localStorage.clear() 直接抛 TypeError，zustand persist 同源崩）。
// vitest 的 jsdom 环境在本文件执行前已把真实窗口挂在 globalThis.jsdom ⇒ 在这里重绑。
{
  const realWindow = (
    globalThis as unknown as {
      jsdom?: { window?: { localStorage?: Storage; sessionStorage?: Storage } };
    }
  ).jsdom?.window;
  if (realWindow?.localStorage && realWindow?.sessionStorage) {
    const g = globalThis as unknown as Record<string, unknown>;
    Object.defineProperty(g, "localStorage", {
      value: realWindow.localStorage,
      configurable: true,
      writable: true,
    });
    Object.defineProperty(g, "sessionStorage", {
      value: realWindow.sessionStorage,
      configurable: true,
      writable: true,
    });
  }
}

// jsdom 销毁后 react-dom scheduler 异步回调仍可能访问 window/document，
// 用 getter 防御 ReferenceError（不可删除的兜底值）
{
  const g = globalThis as Record<string, unknown>;
  const fallbackDoc = {
    createElement: () => ({}),
    documentElement: {},
    body: {},
    head: {},
  };
  let _win: unknown = g.window;
  let _doc: unknown = g.document;
  Object.defineProperty(g, "window", {
    get() {
      return _win ?? g;
    },
    set(v) {
      _win = v;
    },
    configurable: true,
    enumerable: true,
  });
  Object.defineProperty(g, "document", {
    get() {
      return _doc ?? fallbackDoc;
    },
    set(v) {
      _doc = v;
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

// antd 的 useDelayState / raf 在测试结束后仍会 flush 一帧并调用 setState；
// 单文件隔离跑时 16ms 落在 jsdom 生命周期内，全量并行时同一帧落到 teardown 之后，
// react-dom 直接读裸 `window` 抛 ReferenceError（CI 实测 8 条 unhandled error，
// 来源均指向先跑完的 BotRoutineFormModal.test.tsx）。在测试环境把取消帧的
// 延时压到 0，让待处理更新紧跟组件生命周期 flush，避免跨文件收尾竞态。
if (typeof window !== "undefined" && typeof window.requestAnimationFrame === "function") {
  Object.defineProperty(window, "requestAnimationFrame", {
    configurable: true,
    writable: true,
    value: (callback: FrameRequestCallback) =>
      setTimeout(() => {
        // jsdom 已在测试收尾销毁，帧回调仍触发 antd 通知计时器的 setState；
        // react-dom 在裸 `window` 上取更新优先级会抛 ReferenceError。
        // 这里与 setImmediate 兜底同口径：丢弃 teardown 后的帧回调。
        if (typeof window === "undefined") {
          return;
        }
        callback(0);
      }, 0) as unknown as number,
  });
  Object.defineProperty(window, "cancelAnimationFrame", {
    configurable: true,
    writable: true,
    value: (handle: number) => clearTimeout(handle as unknown as ReturnType<typeof setTimeout>),
  });
}
