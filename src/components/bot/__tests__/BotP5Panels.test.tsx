// SPDX-License-Identifier: AGPL-3.0-only

/**
 * P5 B4 + C1-4 前端回归锁：
 * - 用量面板：四类 token / 估算标识 / 超限徽标 / 计价来源 / 配额保存与预校验 / scope 过滤
 * - 技能面板：来源 / 遮蔽 / 缺失 / 本 Bot 启停 / preset 导入回报
 * - 详情页签与部门弹窗用量页签接线
 */

import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { App } from "antd";
import { beforeEach, describe, expect, it, vi } from "vitest";

import i18n from "@/i18n";
import { useBotStore } from "@/stores/feature/botStore";
import type { Bot, BotCostQuota, BotDepartment, BotSkill, BotUsageEntry, BotUsageSummary } from "@/types";
import { BotDepartmentModal } from "../BotDepartmentModal";
import { BotDetail } from "../BotDetail";
import { BotSkillsPanel } from "../BotSkillsPanel";
import { BotUsagePanel } from "../BotUsagePanel";

function makeQuota(over: Partial<BotCostQuota> = {}): BotCostQuota {
  return {
    scope: "bot",
    scopeId: "bot-1",
    maxTotalTokens: 10_000,
    maxCostUsd: 1.5,
    windowHours: 168,
    isDefault: false,
    updatedAt: 1_700_000_000_000,
    ...over,
  };
}

function makeSummary(over: Partial<BotUsageSummary> = {}): BotUsageSummary {
  return {
    scope: "bot",
    scopeId: "bot-1",
    windowHours: 168,
    inputTokens: 100,
    outputTokens: 50,
    cacheCreationTokens: 20,
    cacheReadTokens: 10,
    totalTokens: 180,
    estimatedCostUsd: 0.0123,
    entryCount: 1,
    breached: true,
    pricingNote: "本地价格表估算",
    ...over,
  };
}

function makeEntry(over: Partial<BotUsageEntry> = {}): BotUsageEntry {
  return {
    id: "entry-1",
    botId: "bot-1",
    departmentId: null,
    conversationId: "conv-1",
    executionId: null,
    workflowRunId: null,
    assignmentId: null,
    modelId: "model-a",
    origin: "interactive",
    inputTokens: 100,
    outputTokens: 50,
    cacheCreationTokens: 20,
    cacheReadTokens: 10,
    totalTokens: 180,
    estimatedCostUsd: 0.0123,
    pricingSource: "model",
    createdAt: 1_700_000_000_000,
    ...over,
  };
}

function makeSkills(): BotSkill[] {
  return [
    {
      name: "alpha-global",
      description: "全局技能",
      source: "global",
      enabled: true,
      shadowedGlobal: false,
      missing: false,
      presetId: null,
      updatedAt: null,
    },
    {
      name: "beta-bot",
      description: "Bot 技能",
      source: "bot",
      enabled: true,
      shadowedGlobal: true,
      missing: false,
      presetId: "preset-1",
      updatedAt: 1_700_000_000_000,
    },
    {
      name: "gamma-missing",
      description: "",
      source: "bot",
      enabled: false,
      shadowedGlobal: false,
      missing: true,
      presetId: "preset-1",
      updatedAt: 1_700_000_000_000,
    },
  ];
}

function makeBot(): Bot {
  return {
    id: "bot-1",
    name: "测试 Bot",
    title: null,
    description: null,
    soul: "",
    soulHash: "hash",
    soulDrift: false,
    icon: "🤖",
    color: null,
    section: "default",
    modelId: null,
    providerId: null,
    effort: null,
    workdir: null,
    approvalMode: "auto",
    agentProfileId: null,
    isEnabled: true,
    sortOrder: 0,
    createdAt: 1_700_000_000,
    updatedAt: 1_700_000_000,
  };
}

function renderInApp(ui: React.ReactElement) {
  return render(<App>{ui}</App>);
}

beforeEach(async () => {
  await i18n.changeLanguage("zh-CN");
  useBotStore.setState({
    bots: [],
    costQuotaSaving: false,
    botSkillsSaving: false,
    getBotCostQuota: vi.fn().mockResolvedValue(makeQuota()) as never,
    setBotCostQuota: vi.fn().mockResolvedValue(makeQuota()) as never,
    resetBotCostQuota: vi.fn().mockResolvedValue(undefined) as never,
    getBotUsageSummary: vi.fn().mockResolvedValue(makeSummary()) as never,
    listBotUsage: vi.fn().mockResolvedValue([makeEntry()]) as never,
    listBotSkills: vi.fn().mockResolvedValue(makeSkills()) as never,
    setBotSkillEnabled: vi.fn().mockResolvedValue(makeSkills()[1]) as never,
    importBotSkillPreset: vi.fn().mockResolvedValue({
      presetId: "preset-2",
      imported: ["new-skill"],
      skipped: [],
    }) as never,
    listThreads: vi.fn().mockResolvedValue([]) as never,
  });
});

describe("BotUsagePanel（B4）", () => {
  it("展示四类 token、合计、估算费用、计价来源与超限徽标", async () => {
    renderInApp(<BotUsagePanel scope="bot" scopeId="bot-1" />);
    expect(await screen.findByTestId("bot-usage-estimated")).toBeTruthy();
    expect(screen.getByTestId("bot-usage-inputTokens").textContent).toContain("100");
    expect(screen.getByTestId("bot-usage-outputTokens").textContent).toContain("50");
    expect(screen.getByTestId("bot-usage-cacheCreationTokens").textContent).toContain("20");
    expect(screen.getByTestId("bot-usage-cacheReadTokens").textContent).toContain("10");
    expect(screen.getByTestId("bot-usage-total-tokens").textContent).toContain("180");
    expect(screen.getByTestId("bot-usage-total-cost").textContent).toContain("$0.0123");
    expect(screen.getByTestId("bot-usage-estimated").textContent).toContain("估算");
    expect(screen.getByTestId("bot-usage-breached").textContent).toContain("已超限");
    expect(screen.getByTestId("bot-usage-pricing-source").textContent).toContain("model");
    expect(screen.getByText(/超限后只拦截新建的运行/)).toBeTruthy();
    expect(useBotStore.getState().getBotUsageSummary).toHaveBeenCalledWith("bot", "bot-1", 168);
  });

  it("保存时用 scope/scopeId 提交完整配额字段", async () => {
    const user = userEvent.setup();
    renderInApp(<BotUsagePanel scope="bot" scopeId="bot-1" />);
    const input = await screen.findByTestId("bot-usage-quota-windowHours");
    await user.clear(input);
    await user.type(input, "24");
    await user.click(screen.getByTestId("bot-usage-quota-save"));
    await waitFor(() => expect(useBotStore.getState().setBotCostQuota).toHaveBeenCalledTimes(1));
    expect(useBotStore.getState().setBotCostQuota).toHaveBeenCalledWith({
      scope: "bot",
      scopeId: "bot-1",
      maxTotalTokens: 10_000,
      maxCostUsd: 1.5,
      windowHours: 24,
    });
  });

  it("窗口越界被前端拦下，不发保存请求", async () => {
    const user = userEvent.setup();
    renderInApp(<BotUsagePanel scope="bot" scopeId="bot-1" />);
    const input = await screen.findByTestId("bot-usage-quota-windowHours");
    await user.clear(input);
    await user.type(input, "0");
    await user.click(screen.getByTestId("bot-usage-quota-save"));
    const err = await screen.findByTestId("bot-usage-quota-error");
    expect(err.textContent).toContain("统计窗口");
    expect(useBotStore.getState().setBotCostQuota).not.toHaveBeenCalled();
  });

  it("恢复默认二次确认后调用 reset，部门 scope 走 departmentId 过滤", async () => {
    const user = userEvent.setup();
    renderInApp(<BotUsagePanel scope="department" scopeId="dep-1" />);
    await screen.findByTestId("bot-usage-refresh");
    expect(useBotStore.getState().listBotUsage).toHaveBeenCalledWith({ departmentId: "dep-1" });
    await user.click(screen.getByTestId("bot-usage-quota-reset"));
    const confirm = await screen.findByRole("button", { name: /确\s*认/ });
    await user.click(confirm);
    await waitFor(() => expect(useBotStore.getState().resetBotCostQuota).toHaveBeenCalledWith("department", "dep-1"));
  });

  it("无账本行时显示空态", async () => {
    useBotStore.setState({ listBotUsage: vi.fn().mockResolvedValue([]) as never });
    renderInApp(<BotUsagePanel scope="bot" scopeId="bot-1" />);
    expect(await screen.findByTestId("bot-usage-empty")).toBeTruthy();
  });
});

describe("BotSkillsPanel（C1-4）", () => {
  it("展示来源、同名遮蔽与缺失标记", async () => {
    renderInApp(<BotSkillsPanel botId="bot-1" />);
    expect(await screen.findByTestId("bot-skill-source-alpha-global")).toBeTruthy();
    expect(screen.getByTestId("bot-skill-source-alpha-global").textContent).toContain("全局");
    expect(screen.getByTestId("bot-skill-source-beta-bot").textContent).toContain("本 Bot");
    expect(screen.getByTestId("bot-skill-shadowed-beta-bot")).toBeTruthy();
    expect(screen.getByTestId("bot-skill-missing-gamma-missing").textContent).toContain("技能文件缺失");
  });

  it("开关调用 set_bot_skill_enabled 并带 botId/skillName/enabled", async () => {
    const user = userEvent.setup();
    renderInApp(<BotSkillsPanel botId="bot-1" />);
    const toggle = await screen.findByTestId("bot-skill-enabled-beta-bot");
    await user.click(toggle);
    await waitFor(() =>
      expect(useBotStore.getState().setBotSkillEnabled).toHaveBeenCalledWith("bot-1", "beta-bot", false)
    );
  });

  it("preset 文本非 JSON 时提示格式不合法且不发导入", async () => {
    const user = userEvent.setup();
    renderInApp(<BotSkillsPanel botId="bot-1" />);
    const input = await screen.findByTestId("bot-skills-preset-input");
    fireEvent.change(input, { target: { value: "{bad json}" } });
    await user.click(screen.getByTestId("bot-skills-import"));
    const err = await screen.findByTestId("bot-skills-import-error");
    expect(err.textContent).toContain("预设格式不合法");
    expect(useBotStore.getState().importBotSkillPreset).not.toHaveBeenCalled();
  });

  it("导入合法 preset 文本并回报 imported / skipped", async () => {
    const user = userEvent.setup();
    renderInApp(<BotSkillsPanel botId="bot-1" />);
    const input = await screen.findByTestId("bot-skills-preset-input");
    fireEvent.change(input, {
      target: { value: '{"version":"bot-skill-preset.v1","name":"p","description":"","skills":[]}' },
    });
    await user.click(screen.getByTestId("bot-skills-import"));
    await waitFor(() => expect(useBotStore.getState().importBotSkillPreset).toHaveBeenCalledTimes(1));
    expect(useBotStore.getState().importBotSkillPreset).toHaveBeenCalledWith("bot-1", {
      version: "bot-skill-preset.v1",
      name: "p",
      description: "",
      skills: [],
    });
    const result = await screen.findByTestId("bot-skills-import-result");
    expect(result.textContent).toContain("已导入 1 个技能");
  });

  it("preset 跳过条目按白名单回报", async () => {
    useBotStore.setState({
      importBotSkillPreset: vi.fn().mockResolvedValue({
        presetId: "preset-3",
        imported: ["ok-skill"],
        skipped: [{ name: "../bad", reason: "not_whitelisted", detail: "非法技能名" }],
      }) as never,
    });
    const user = userEvent.setup();
    renderInApp(<BotSkillsPanel botId="bot-1" />);
    const input = await screen.findByTestId("bot-skills-preset-input");
    fireEvent.change(input, {
      target: { value: '{"version":"bot-skill-preset.v1","name":"p","description":"","skills":[]}' },
    });
    await user.click(screen.getByTestId("bot-skills-import"));
    const result = await screen.findByTestId("bot-skills-import-result");
    expect(result.textContent).toContain("已导入 1 个技能");
    expect(result.textContent).toContain("跳过 1 个技能");
    expect(result.textContent).toContain("../bad");
  });
});

describe("页签接线", () => {
  it("Bot 详情注入用量/技能页签", async () => {
    const user = userEvent.setup();
    renderInApp(
      <BotDetail
        bot={makeBot()}
        onEdit={() => {}}
        onDelete={() => {}}
        onCreateThread={() => {}}
        onOpenThread={() => {}}
        usagePanel={<div>usage-sentinel</div>}
        skillsPanel={<div>skills-sentinel</div>}
      />,
    );
    await user.click(await screen.findByRole("tab", { name: "用量与费用" }));
    expect(await screen.findByText("usage-sentinel")).toBeTruthy();
    await user.click(await screen.findByRole("tab", { name: "技能" }));
    expect(await screen.findByText("skills-sentinel")).toBeTruthy();
  });

  it("部门弹窗以 initialTab=usage 打开用量页签", async () => {
    const department: BotDepartment = {
      id: "dep-1",
      name: "研发部",
      description: "",
      chiefBotId: null,
      botIds: [],
      createdAt: 1_700_000_000,
      updatedAt: 1_700_000_000,
    };
    renderInApp(
      <BotDepartmentModal
        open
        editing={department}
        initialTab="usage"
        onClose={() => {}}
        onSaved={() => {}}
      />,
    );
    expect(await screen.findByTestId("bot-usage-panel")).toBeTruthy();
    expect(useBotStore.getState().getBotCostQuota).toHaveBeenCalledWith("department", "dep-1");
  });
});
