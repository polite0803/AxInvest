// SPDX-License-Identifier: AGPL-3.0-only

import { OverlayScrollbars } from "overlayscrollbars";
import { useEffect, useRef } from "react";

/**
 * Selectors for elements that should receive custom overlay scrollbars.
 *
 * `.overflow-y-auto` — Tailwind utility; covers sidebar, settings panels, etc.
 * `[data-os-scrollbar]` — explicit opt-in for containers using inline styles.
 *
 * NOTE: antd Bubble.List is excluded — it uses `flex-direction: column-reverse`
 * which inverts the scroll coordinate system.  OverlayScrollbars cannot handle
 * reversed scroll containers, so the chat area uses a separate lightweight
 * scroll indicator (`ChatScrollIndicator`) instead.
 */
const SCROLLABLE_SELECTORS = [".overflow-y-auto", "[data-os-scrollbar]"];

const OS_OPTIONS: Parameters<typeof OverlayScrollbars>[1] = {
  scrollbars: {
    theme: "os-theme-axagent",
    autoHide: "scroll",
    autoHideDelay: 600,
    autoHideSuspend: true,
    clickScroll: true,
  },
  overflow: {
    x: "hidden",
  },
};

/**
 * Global hook that automatically finds scrollable containers and initialises
 * OverlayScrollbars on them.  Uses a MutationObserver to handle elements
 * that mount later (e.g. route changes, lazy components).
 *
 * The `elements.viewport` option is passed so that OverlayScrollbars re-uses
 * each existing scrollable element as the viewport, minimising DOM
 * restructuring.
 */
export function useGlobalOverlayScrollbars() {
  const instancesRef = useRef(
    new Map<Element, ReturnType<typeof OverlayScrollbars>>(),
  );

  useEffect(() => {
    const instances = instancesRef.current;
    /** 重算节流定时器（见 refreshExisting） */
    let refreshTimer = 0;

    function initElement(el: HTMLElement) {
      if (instances.has(el)) {
        return;
      }
      if (OverlayScrollbars.valid(el)) {
        return;
      }

      try {
        const inst = OverlayScrollbars(
          { target: el, elements: { viewport: el } },
          OS_OPTIONS,
        );
        instances.set(el, inst);
      } catch {
        // Element may have been removed before init completed
      }
    }

    /**
     * 内容到位后强制重算（关键修复 — 2026-09-14）。
     *
     * OverlayScrollbars 只在「初始化那一刻」读一次元素的计算 overflow，并把
     * `auto` 折算成 `scroll`（需要滚动条）或 `hidden`（不需要）缓存进
     * `data-overlayscrollbars-viewport` 属性，随后直接以该结果改写元素的
     * overflow 样式。React 场景下「容器先挂载、内容后渲染」是常态：
     * 初始化时容器里还没有内容 → OS 判为「无需滚动」→ 写入 overflowYHidden，
     * 元素自身 overflow 变成 `hidden`。此后容器高度不再变化，OS 永远不会重算，
     * 于是内容再高也被硬裁，页面上永远出不来垂直滚动条（投资中心工作区
     * `flex-1 min-h-0 overflow-y-auto` 即为此例：内容 2664px 被裁在 281px）。
     *
     * 因此每轮 DOM 变化（内容渲染、懒加载、路由切换）后都对已存在实例强制
     * update(true)（120ms 节流），让 OS 按真实内容尺寸重算 scroll / hidden。
     */
    function refreshExisting() {
      // 节流：聊天流式输出等场景 DOM 每帧都在变，若每次变化都强制重算，
      // 会退化成「每帧对全部滚动容器做一次强制布局」。改为变化后静默 120ms
      // 统一重算一次 —— 期间无论变化多少次，读到的都是最新 DOM，最终一致。
      if (refreshTimer !== 0) {
        return;
      }
      refreshTimer = window.setTimeout(() => {
        refreshTimer = 0;
        instances.forEach((inst, el) => {
          if (!document.contains(el)) {
            return;
          }
          try {
            inst.update(true);
          } catch {
            // 实例可能已销毁，忽略
          }
        });
      }, 120);
    }

    function scanAndInit() {
      const selector = SCROLLABLE_SELECTORS.join(",");
      document.querySelectorAll<HTMLElement>(selector).forEach(initElement);
      // 新建实例的首次尺寸同样可能早于内容，故紧随其后统一重算一轮
      refreshExisting();
    }

    function cleanup() {
      instances.forEach((inst, el) => {
        if (!document.contains(el)) {
          inst.destroy();
          instances.delete(el);
        }
      });
    }

    // Initial scan
    scanAndInit();

    // Watch DOM mutations (debounced)
    let rafId = 0;
    const observer = new MutationObserver(() => {
      cancelAnimationFrame(rafId);
      rafId = requestAnimationFrame(() => {
        scanAndInit();
        cleanup();
      });
    });

    observer.observe(document.body, { childList: true, subtree: true });

    return () => {
      observer.disconnect();
      cancelAnimationFrame(rafId);
      if (refreshTimer !== 0) {
        clearTimeout(refreshTimer);
        refreshTimer = 0;
      }
      instances.forEach((inst) => inst.destroy());
      instances.clear();
    };
  }, []);
}
