// SPDX-License-Identifier: AGPL-3.0-only

import { describe, expect, it } from "vitest";

import {
  buildWorkspaceTabSearch,
  CHAT_ONLY_QUERY_PARAMS,
  DEFAULT_WORKSPACE_TAB,
  GATED_WORKSPACE_TABS,
  isWorkspaceTab,
  parseWorkspaceTab,
  shouldRenderTab,
  withoutParams,
  WORKSPACE_TAB_ICON_COLORS,
  WORKSPACE_TAB_ICONS,
  WORKSPACE_TABS,
  type WorkspaceTabMeta,
} from "../workspaceTabs";

describe("workspaceTabs — Tab 元数据契约", () => {
  it("每个 Tab 都有 labelKey、图标与图标色（新增 Tab 漏配会在这里报错）", () => {
    for (const tab of WORKSPACE_TABS) {
      expect(tab.labelKey).toMatch(/^nav\./);
      expect(WORKSPACE_TAB_ICONS[tab.key]).toBeDefined();
      expect(WORKSPACE_TAB_ICON_COLORS[tab.key]).toMatch(/^#[0-9a-f]{6}$/i);
    }
  });

  it("无重复 key 或重复 labelKey", () => {
    const keys = WORKSPACE_TABS.map((t) => t.key);
    const labels = WORKSPACE_TABS.map((t) => t.labelKey);
    expect(new Set(keys).size).toBe(keys.length);
    expect(new Set(labels).size).toBe(labels.length);
  });

  it("默认 Tab 必须存在于 Tab 表内（否则 Hub 会落到 default 分支）", () => {
    expect(WORKSPACE_TABS.some((t) => t.key === DEFAULT_WORKSPACE_TAB)).toBe(true);
  });

  it("门控 Tab 必须排在表尾 —— 否则开关 showDeveloperTools 会让其余 Tab 的快捷键序号整体前移", () => {
    const lastUngatedIndex = WORKSPACE_TABS.length - GATED_WORKSPACE_TABS.length - 1;
    for (const gated of GATED_WORKSPACE_TABS) {
      expect(WORKSPACE_TABS.findIndex((t) => t.key === gated)).toBeGreaterThan(lastUngatedIndex);
    }
  });
});

describe("workspaceTabs — isWorkspaceTab 脏值防线", () => {
  it("接受合法 Tab", () => {
    for (const tab of WORKSPACE_TABS) {
      expect(isWorkspaceTab(tab.key)).toBe(true);
    }
  });

  it("拒绝非法值（旧版本持久化的已删 Tab / URL 手改 / 非字符串）", () => {
    for (const bogus of ["__bogus__", "", "Chat", "devTools", null, undefined, 8, {}]) {
      expect(isWorkspaceTab(bogus)).toBe(false);
    }
  });
});

describe("workspaceTabs — parseWorkspaceTab", () => {
  it("解析合法 ?ws=", () => {
    expect(parseWorkspaceTab("?ws=terminal")).toBe("terminal");
    expect(parseWorkspaceTab("?foo=1&ws=devtools")).toBe("devtools");
  });

  it("无 ws 或非法 ws 时返回 null（**不得**回落默认值，否则会覆盖持久化位置）", () => {
    expect(parseWorkspaceTab("")).toBeNull();
    expect(parseWorkspaceTab("?conversationId=abc")).toBeNull();
    expect(parseWorkspaceTab("?ws=__bogus__")).toBeNull();
  });
});

describe("workspaceTabs — buildWorkspaceTabSearch", () => {
  it("写入 ws 且清掉全部 chat 专属参数（不清会让 Hub 强制切回对话）", () => {
    const query = CHAT_ONLY_QUERY_PARAMS.map((k) => `${k}=v`).join("&");
    const search = buildWorkspaceTabSearch(`?${query}`, "terminal");
    const params = new URLSearchParams(search);
    expect(params.get("ws")).toBe("terminal");
    for (const key of CHAT_ONLY_QUERY_PARAMS) {
      expect(params.has(key)).toBe(false);
    }
  });

  it("保留无关参数", () => {
    const search = buildWorkspaceTabSearch("?stockCode=600519&foo=bar", "files");
    const params = new URLSearchParams(search);
    expect(params.get("stockCode")).toBe("600519");
    expect(params.get("foo")).toBe("bar");
    expect(params.get("ws")).toBe("files");
  });

  it("空查询串也能生成 ws", () => {
    expect(buildWorkspaceTabSearch("", "chat")).toBe("ws=chat");
  });

  it("离开 devtools 时清掉 sub（否则留下 /chat?ws=terminal&sub=benchmark 这类语义悬空 URL）", () => {
    const params = new URLSearchParams(buildWorkspaceTabSearch("?ws=devtools&sub=benchmark", "terminal"));
    expect(params.get("ws")).toBe("terminal");
    expect(params.has("sub")).toBe(false);
  });

  it("切到 devtools 时保留 sub（切走再切回不该丢掉子页选择）", () => {
    const params = new URLSearchParams(buildWorkspaceTabSearch("?sub=benchmark", "devtools"));
    expect(params.get("ws")).toBe("devtools");
    expect(params.get("sub")).toBe("benchmark");
  });
});

describe("workspaceTabs — withoutParams（消费完 URL 参数后回写）", () => {
  it("只剔除指定参数，**保留 ws**（全量清空会让 Tab 深链静默失效）", () => {
    const prev = new URLSearchParams("ws=chat&conversationId=c1&prompt=hi&foo=bar");
    const next = withoutParams(prev, CHAT_ONLY_QUERY_PARAMS);
    expect(next.get("ws")).toBe("chat");
    expect(next.get("foo")).toBe("bar");
    for (const key of CHAT_ONLY_QUERY_PARAMS) {
      expect(next.has(key)).toBe(false);
    }
  });

  it("不修改入参（纯函数，避免调用方手中的 URLSearchParams 被就地改动）", () => {
    const prev = new URLSearchParams("ws=chat&conversationId=c1");
    withoutParams(prev, CHAT_ONLY_QUERY_PARAMS);
    expect(prev.get("conversationId")).toBe("c1");
  });

  it("剔除不存在的参数不报错", () => {
    const next = withoutParams(new URLSearchParams("ws=files"), ["template", "domain"]);
    expect(next.get("ws")).toBe("files");
  });
});

describe("workspaceTabs — shouldRenderTab（保活判定）", () => {
  const kept: WorkspaceTabMeta = { key: "files", labelKey: "nav.files", keepAlive: true };
  const dropped: WorkspaceTabMeta = { key: "terminal", labelKey: "nav.terminal", keepAlive: false };

  it("保活页：活跃时渲染", () => {
    expect(shouldRenderTab(kept, true, [])).toBe(true);
  });

  it("保活页：曾访问过（即使不活跃）仍渲染 —— 这就是保活", () => {
    expect(shouldRenderTab(kept, false, ["files"])).toBe(true);
  });

  it("保活页：从未访问且不活跃 ⇒ 不渲染（首次访问才挂载，不是首屏全挂）", () => {
    expect(shouldRenderTab(kept, false, ["chat"])).toBe(false);
  });

  it("非保活页：始终只看是否活跃，忽略已访问集合", () => {
    expect(shouldRenderTab(dropped, true, [])).toBe(true);
    expect(shouldRenderTab(dropped, false, ["terminal"])).toBe(false);
  });

  it("真实 Tab 表当前全部保活（若将来关掉某个，此断言会失败以提醒补测）", () => {
    for (const tab of WORKSPACE_TABS) {
      expect(tab.keepAlive, `Tab "${tab.key}" 改为不保活后需同步更新本组用例`).toBe(true);
    }
  });
});
