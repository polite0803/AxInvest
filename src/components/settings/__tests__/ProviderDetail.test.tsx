// SPDX-License-Identifier: AGPL-3.0-only

import type { ProviderKey } from "@/types";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { App } from "antd";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { ProviderDetail } from "../ProviderDetail";

const toggleProvider = vi.fn();
const updateProvider = vi.fn();
const deleteProvider = vi.fn();
const addProviderKey = vi.fn();
const deleteProviderKey = vi.fn();
const toggleProviderKey = vi.fn();
const validateProviderKey = vi.fn();
const toggleModel = vi.fn();
const updateModelParams = vi.fn();
const fetchRemoteModels = vi.fn();
const saveModels = vi.fn();

let provider = {
  id: "provider-1",
  name: "OpenAI",
  providerType: "openai",
  apiHost: "https://api.openai.com",
  apiPath: "/v1/chat/completions",
  enabled: true,
  models: [
    {
      providerId: "provider-1",
      modelId: "gpt-5.4",
      name: "GPT 5.4",
      groupName: "gpt-5.4",
      modelType: "Chat",
      capabilities: ["TextChat"],
      maxTokens: null,
      enabled: true,
      paramOverrides: null,
    },
  ],
  keys: [] as ProviderKey[],
  proxyConfig: null,
  sortOrder: 0,
  createdAt: 0,
  updatedAt: 0,
};

const zh: Record<string, string> = {
  "settings.addModel": "添加模型",
  "settings.addModelToGroup": "添加到当前分组",
  "settings.model_id": "模型标识",
  "settings.modelName": "模型名称",
  "settings.modelGroup": "模型分组",
  "settings.modelType.title": "模型类型",
  "common.confirm": "确认",
  "common.cancel": "取消",
  "common.enabled": "已启用",
  "common.disabled": "已禁用",
  "common.noData": "暂无数据",
  "common.copySuccess": "复制成功",
  "common.collapseAll": "全部收起",
  "common.expandAll": "全部展开",
  "common.errorDetail": "错误详情",
  "common.failed": "失败",
  "error.saveFailed": "保存失败",
  "error.loadFailed": "加载失败",
  "error.unknown": "未知错误",
  "settings.showKey": "显示",
  "settings.hideKey": "隐藏",
  "settings.syncModels": "同步模型",
  "settings.fetchAllModels": "获取全部模型",
  "settings.noActiveKeyError": "操作失败：请先添加并启用 API 密钥",
  "error.PROVIDER_KEY_DECRYPT_FAILED": "解密提供商密钥失败",
};

// errorI18n 走的是 i18n 单例（不是 useTranslation），组件测试里同样用字典替身：
// 未收录的 key 返回 key 原文 ⇒ 翻译未命中时回落到 detail，与真 i18next 行为一致。
vi.mock("@/i18n", () => ({
  default: {
    t: (key: string) => zh[key] ?? key,
    language: "zh",
    exists: () => true,
    changeLanguage: vi.fn(),
  },
}));

vi.mock("react-i18next", () => ({
  initReactI18next: { type: "3rdParty", init: vi.fn() },
  useTranslation: () => ({
    t: (key: string, options?: Record<string, unknown>) => {
      if (zh[key]) { return zh[key]; }
      if (typeof options === "string") { return options; }
      return key;
    },
    i18n: { language: "zh" },
  }),
}));

vi.mock("@lobehub/icons", () => ({
  ProviderIcon: () => <div>provider-icon</div>,
  ModelIcon: () => <div>model-icon</div>,
  providerMappings: [],
  modelMappings: [],
}));

vi.mock("@tanstack/react-virtual", () => ({
  useVirtualizer: ({ count }: { count: number }) => ({
    getTotalSize: () => count * 40,
    getVirtualItems: () =>
      Array.from({ length: count }, (_, i) => ({
        index: i,
        key: `virtual-${i}`,
        start: i * 40,
        size: 40,
      })),
    measureElement: () => {},
  }),
}));

vi.mock("../IconPickerModal", () => ({
  IconPickerModal: () => null,
  default: () => null,
}));

const setSelectedProviderId = vi.fn();

vi.mock("@/lib/invoke", () => ({
  invoke: vi.fn(async (cmd: string) => cmd === "reveal_provider_key" ? "sk-full-plain-key-000" : ""),
  logIpcError: () => {},
}));

vi.mock("@/stores", () => ({
  useProviderStore: (selector: (state: Record<string, unknown>) => unknown) =>
    selector({
      providers: [provider],
      toggleProvider,
      updateProvider,
      deleteProvider,
      addProviderKey,
      deleteProviderKey,
      toggleProviderKey,
      validateProviderKey,
      toggleModel,
      updateModelParams,
      fetchRemoteModels,
      saveModels,
    }),
  useUIStore: (selector: (state: Record<string, unknown>) => unknown) =>
    selector({
      setSelectedProviderId,
    }),
  useSettingsStore: (selector: (state: Record<string, unknown>) => unknown) =>
    selector({
      settings: { theme_mode: "light" },
      saveSettings: vi.fn(),
    }),
}));

describe("ProviderDetail", () => {
  vi.setConfig({ testTimeout: 20000 });
  beforeEach(() => {
    vi.clearAllMocks();
    provider = {
      id: "provider-1",
      name: "OpenAI",
      providerType: "openai",
      apiHost: "https://api.openai.com",
      apiPath: "/v1/chat/completions",
      enabled: true,
      models: [
        {
          providerId: "provider-1",
          modelId: "gpt-5.4",
          name: "GPT 5.4",
          groupName: "gpt-5.4",
          modelType: "Chat",
          capabilities: ["TextChat"],
          maxTokens: null,
          enabled: true,
          paramOverrides: null,
        },
      ],
      keys: [] as ProviderKey[],
      proxyConfig: null,
      sortOrder: 0,
      createdAt: 0,
      updatedAt: 0,
    };
    saveModels.mockResolvedValue(undefined);

    Object.defineProperty(window, "matchMedia", {
      writable: true,
      value: vi.fn().mockImplementation((query: string) => ({
        matches: false,
        media: query,
        onchange: null,
        addListener: vi.fn(),
        removeListener: vi.fn(),
        addEventListener: vi.fn(),
        removeEventListener: vi.fn(),
        dispatchEvent: vi.fn(),
      })),
    });
  });

  it("adds a model from the card-level action and derives the default group from the model id", async () => {
    render(
      <App>
        <ProviderDetail providerId="provider-1" />
      </App>,
    );

    await userEvent.click(screen.getByRole("button", { name: "添加模型" }));

    const dialog = await screen.findByRole("dialog");
    const inputs = within(dialog).getAllByRole("textbox");
    await userEvent.type(inputs[0], "gpt-5.4-think");
    await userEvent.clear(inputs[1]);
    await userEvent.type(inputs[1], "GPT 5.4 Think");

    await userEvent.click(
      within(dialog).getByRole("button", { name: "添加模型" }),
    );

    expect(saveModels).toHaveBeenCalledWith(
      "provider-1",
      expect.arrayContaining([
        expect.objectContaining({
          modelId: "gpt-5.4-think",
          name: "GPT 5.4 Think",
          groupName: "gpt-5.4",
          modelType: "Chat",
        }),
      ]),
    );
  });

  it("prefills the current group when adding a model from a group header", async () => {
    render(
      <App>
        <ProviderDetail providerId="provider-1" />
      </App>,
    );

    await userEvent.click(
      screen.getByRole("button", { name: "添加到当前分组" }),
    );

    const dialog = await screen.findByRole("dialog");
    const comboboxes = within(dialog).getAllByRole("combobox");
    expect(comboboxes[0]).toHaveValue("gpt-5.4");
  });

  it("reveals and hides the full API key via the eye button", async () => {
    provider = {
      ...provider,
      keys: [
        {
          id: "key-1",
          providerId: "provider-1",
          keyEncrypted: "enc",
          keyPrefix: "abcd1234...",
          enabled: true,
          lastValidatedAt: null,
          lastError: null,
          rotationIndex: 0,
          createdAt: 0,
        },
      ],
    };
    render(
      <App>
        <ProviderDetail providerId="provider-1" />
      </App>,
    );

    // 默认掩码展示
    expect(screen.getByText("abcd1234...••••••••")).toBeTruthy();
    expect(screen.queryByText("sk-full-plain-key-000")).toBeNull();

    // 点击「显示」→ 明文密钥可见
    await userEvent.click(screen.getByTitle("显示"));
    expect(await screen.findByText("sk-full-plain-key-000")).toBeTruthy();

    // 点击「隐藏」→ 回到掩码
    await userEvent.click(screen.getByTitle("隐藏"));
    expect(screen.queryByText("sk-full-plain-key-000")).toBeNull();
    expect(screen.getByText("abcd1234...••••••••")).toBeTruthy();
  });

  // 同步模型失败提示按**后端错误码**分支。旧写法是 `String(e).includes("key")` 子串嗅探
  // + `t("error.loadFailed") + ": " + String(e)`，于是「密钥解不开」在界面上糊成一坨 JSON
  // （2026-10-31 用户实测），而任何 detail 里带 "key" 的无关错误又会被误报成「请先添加密钥」。
  it("密钥解密失败时提示指到凭证，不把后端 JSON 糊在界面上", async () => {
    fetchRemoteModels.mockRejectedValue(
      JSON.stringify({
        code: "PROVIDER_KEY_DECRYPT_FAILED",
        category: "unrecoverable",
        detail: "密钥行 key-1 解密失败: Crypto error: Decryption failed: aead::Error",
      }),
    );
    render(
      <App>
        <ProviderDetail providerId="provider-1" />
      </App>,
    );

    await userEvent.click(screen.getByRole("button", { name: "同步模型" }));

    expect(await screen.findByText("解密提供商密钥失败")).toBeTruthy();
    expect(screen.queryByText(/aead::Error/)).toBeNull();
    expect(screen.queryByText(/"code":/)).toBeNull();
  });

  it("供应商没有启用密钥时提示去添加密钥", async () => {
    fetchRemoteModels.mockRejectedValue(
      JSON.stringify({
        code: "PROVIDER_NO_ACTIVE_KEY",
        category: "unrecoverable",
        detail: "NotFound: No active key for provider provider-1",
      }),
    );
    render(
      <App>
        <ProviderDetail providerId="provider-1" />
      </App>,
    );

    await userEvent.click(screen.getByRole("button", { name: "同步模型" }));

    expect(await screen.findByText("操作失败：请先添加并启用 API 密钥")).toBeTruthy();
  });

  it("负控：detail 里含 key 字样的其它失败不得被报成「请先添加密钥」", async () => {
    fetchRemoteModels.mockRejectedValue(
      JSON.stringify({
        code: "UNKNOWN_FOR_TEST",
        category: "unrecoverable",
        detail: "upstream rejected the key header",
      }),
    );
    render(
      <App>
        <ProviderDetail providerId="provider-1" />
      </App>,
    );

    await userEvent.click(screen.getByRole("button", { name: "同步模型" }));

    expect(await screen.findByText(/upstream rejected the key header/)).toBeTruthy();
    expect(screen.queryByText("操作失败：请先添加并启用 API 密钥")).toBeNull();
  });
});
