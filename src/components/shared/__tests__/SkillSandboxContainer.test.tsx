// SPDX-License-Identifier: AGPL-3.0-only

/**
 * srcdoc 容器（`SkillSandboxContainer`）的渲染结构 / 装载时序 / 卸载回收用例（任务 #77）。
 *
 * 补的是 #64 报告 §⑦.1 取证过的既存缺陷：`loading` 初值 `true` + `if (loading) return
 * <SkillLoadingSkeleton/>` ⇒ iframe 只在 `!loading && !error` 时进 DOM，而 `loadSandbox()`
 * 在 `await` 之后读 `iframeRef.current` ⇒ 恒为 `null` ⇒ 首挂载必然落 `skill.iframeNotReady`。
 * 因此下面每条都从「渲染出来的 DOM」起步，断言打在 iframe 是否在 DOM、覆盖层是否可见、
 * 重试是否真正重挂元素、卸载后文档/监听/定时器是否回收上。
 *
 * 复用关系：`#64` 的 `PluginPageHost.test.tsx` 已锁住新容器的同一形态（常驻 iframe + 覆盖层），
 * 本文件是**srcdoc 那条通道**的对应面，两者判据同形但不共享实现（见两个容器文件头的理由）。
 */

import { act, fireEvent, render, screen } from "@testing-library/react";
import { App as AntdApp } from "antd";
import { MemoryRouter } from "react-router-dom";
import { afterEach, describe, expect, it, vi } from "vitest";

import i18n from "@/i18n";
import { SANDBOX_LOAD_TIMEOUT_MS } from "@/sdk/sandboxFrameProtocol";

vi.mock(import("@/lib/invoke"), async (importOriginal) => {
  const actual = await importOriginal();
  return { ...actual, invoke: vi.fn(), logIpcError: () => () => {} };
});

import { SkillSandboxContainer } from "@/components/shared/SkillSandboxContainer";

const SKILL_NAME = "probe-skill";
const ENTRY_HTML = '<html><body><div id="probe">hi</div></body></html>';

/** 渲染容器。`componentConfig` 在测试里保持同一对象字面量，避免 trigger 无意义的 effect 重跑。 */
const COMPONENT_CONFIG = { entry: "index.html" };

function renderContainer(loadAsset: (entry: string) => Promise<string>) {
  const view = render(
    <MemoryRouter>
      <AntdApp>
        <SkillSandboxContainer
          skillName={SKILL_NAME}
          componentId="main"
          componentConfig={COMPONENT_CONFIG}
          loadAsset={loadAsset}
        />
      </AntdApp>
    </MemoryRouter>,
  );
  return view;
}

/** 排空 promise 链（`loadAsset` 的 then 里才赋 srcdoc）。 */
async function flush(): Promise<void> {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });
}

afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
});

describe("首挂载：iframe 必须常驻 DOM（否则 ready 帧永远发不回来）", () => {
  it("mount 后 iframe 已在 DOM 里，且不落 skill.iframeNotReady，srcdoc 真正被注入", async () => {
    const loadAsset = vi.fn(async () => ENTRY_HTML);
    const { container } = renderContainer(loadAsset);

    // 同步断言：首次提交就该有 iframe —— 骨架/错误只能是覆盖层，不能把 iframe 拦在 DOM 外
    expect(container.querySelectorAll("iframe")).toHaveLength(1);
    const iframe = container.querySelector("iframe") as HTMLIFrameElement;

    await flush();

    expect(container.textContent).not.toContain(i18n.t("skill.iframeNotReady"));
    expect(iframe.getAttribute("srcdoc"), "srcdoc 没注入说明 loadSandbox 仍没读到 iframe").toBeTruthy();
  });

  it("loading 期间骨架覆盖层可见，且不遮挡后续导航（iframe 同时存在）", async () => {
    let resolveAsset: (html: string) => void = () => {};
    const loadAsset = vi.fn(
      () =>
        new Promise<string>((resolve) => {
          resolveAsset = resolve;
        }),
    );
    const { container } = renderContainer(loadAsset);
    await flush();

    expect(screen.getByText(i18n.t("skill.loading"))).toBeTruthy();
    expect(container.querySelectorAll("iframe"), "骨架期 iframe 被条件渲染摘掉了").toHaveLength(1);

    // 资产回填后仍停在 loading（等 ready 帧），iframe 一直在
    await act(async () => {
      resolveAsset(ENTRY_HTML);
      await Promise.resolve();
    });
    expect(container.querySelectorAll("iframe")).toHaveLength(1);
  });
});

describe("错误覆盖层与重试", () => {
  it("加载失败显示错误覆盖层；点重试换新 iframe（key 变化）并重新导航", async () => {
    const loadAsset = vi.fn()
      .mockRejectedValueOnce(new Error("asset boom"))
      .mockImplementation(async () => ENTRY_HTML);
    const { container } = renderContainer(loadAsset);
    await flush();

    const first = container.querySelector("iframe") as HTMLIFrameElement;
    expect(first).toBeTruthy();
    expect(container.textContent).toContain(i18n.t("skill.loadFailed", { name: SKILL_NAME }));
    expect(loadAsset).toHaveBeenCalledTimes(1);

    await act(async () => {
      // antd Button 会在两个汉字间插一个空格（渲染成「重 试」），按 role + 正则取
      fireEvent.click(screen.getByRole("button", { name: /重\s*试/ }));
    });
    await flush();

    const second = container.querySelector("iframe") as HTMLIFrameElement;
    expect(second, "重试没有重挂 iframe —— 同一元素重复赋 srcdoc 不会重新导航").not.toBe(first);
    expect(first.isConnected).toBe(false);
    expect(loadAsset).toHaveBeenCalledTimes(2);
    expect(second.getAttribute("srcdoc"), "重试后没有重新注入 srcdoc").toBeTruthy();
  });

  it("30 s 闸保留：未收到 ready 帧仍落 skill.timeoutError，且 iframe 在 DOM 里", async () => {
    vi.useFakeTimers();
    const loadAsset = vi.fn(async () => ENTRY_HTML);
    const { container } = renderContainer(loadAsset);
    await flush();

    expect(screen.queryByText(i18n.t("skill.timeoutError"))).toBeNull();
    await act(async () => {
      vi.advanceTimersByTime(SANDBOX_LOAD_TIMEOUT_MS + 1);
    });

    expect(screen.getByText(i18n.t("skill.timeoutError"))).toBeTruthy();
    expect(container.querySelectorAll("iframe")).toHaveLength(1);
  });
});

describe("卸载回收", () => {
  it("卸载后：沙箱文档被清空、message 监听配平、无残留定时器", async () => {
    vi.useFakeTimers();
    const addSpy = vi.spyOn(window, "addEventListener");
    const removeSpy = vi.spyOn(window, "removeEventListener");
    const baselineTimers = vi.getTimerCount();

    const loadAsset = vi.fn(async () => ENTRY_HTML);
    const { container, unmount } = renderContainer(loadAsset);
    await flush();

    const iframe = container.querySelector("iframe") as HTMLIFrameElement;
    expect(iframe.getAttribute("srcdoc")).toBeTruthy();
    expect(vi.getTimerCount(), "30 s 闸没挂上").toBeGreaterThan(baselineTimers);

    const added = addSpy.mock.calls
      .filter(([type]) => type === "message")
      .map(([, handler]) => handler);
    unmount();
    const removed = removeSpy.mock.calls
      .filter(([type]) => type === "message")
      .map(([, handler]) => handler);

    expect(added, "加载期没挂 message 监听").toHaveLength(1);
    for (const handler of added) {
      // 按 handler 身份比对：只数次数会放过「摘了一个从未挂上的、留下的那个还在」
      expect(removed, "卸载后没摘掉加载期挂上的 message 监听").toContain(handler);
    }
    expect(vi.getTimerCount(), "卸载后仍有定时器在跑").toBe(baselineTimers);
    expect(iframe.hasAttribute("srcdoc"), "卸载后沙箱文档没被回收").toBe(false);
  });
});
