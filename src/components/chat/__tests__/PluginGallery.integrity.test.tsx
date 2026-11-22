// SPDX-License-Identifier: AGPL-3.0-only

import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

const { mocks } = vi.hoisted(() => ({
  mocks: {
    validateSource: vi.fn(),
    installPlugin: vi.fn(),
    updatePlugin: vi.fn(),
  },
}));

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, opts?: Record<string, unknown>) => opts ? `${key}|${JSON.stringify(opts)}` : key,
  }),
  initReactI18next: { type: "3rdParty", init: () => {} },
}));

const { galleryState } = vi.hoisted(() => ({
  galleryState: { plugins: [] as Array<Record<string, unknown>> },
}));

vi.mock("@/hooks/usePluginGallery", () => ({
  usePluginGallery: () => ({
    plugins: galleryState.plugins,
    loading: false,
    installing: null,
    validating: false,
    loadPlugins: vi.fn(),
    validateSource: mocks.validateSource,
    installPlugin: mocks.installPlugin,
    updatePlugin: mocks.updatePlugin,
    enablePlugin: vi.fn(),
    disablePlugin: vi.fn(),
    uninstallPlugin: vi.fn(),
    error: null,
    registry: [],
    registryLoading: false,
    loadRegistry: vi.fn(),
  }),
}));

vi.mock("@/lib/toast", () => ({
  message: { error: vi.fn(), success: vi.fn(), info: vi.fn() },
}));

vi.mock("@/lib/invoke", () => ({
  isTauri: () => true,
  invoke: vi.fn(),
}));

import { PluginGallery } from "../PluginGallery";

const SOURCE = "https://example.com/demo.git";
const OLD_HASH = "a".repeat(64);
const NEW_HASH = "b".repeat(64);

function manifest() {
  return {
    name: "demo",
    version: "1.0.0",
    description: "demo plugin",
    permissions: [],
    pages: [],
    defaultEnabled: true,
    hooks: {},
    tools: [],
    mcpServers: [],
    skills: [],
    capabilities: [],
  };
}

function fillSourceAndHash(source: string, hash: string): void {
  fireEvent.change(
    screen.getByPlaceholderText("chat.plugins.marketplace.installPlaceholder"),
    { target: { value: source } },
  );
  fireEvent.change(
    screen.getByPlaceholderText("chat.plugins.marketplace.sha256Placeholder"),
    { target: { value: hash } },
  );
}

describe("PluginGallery 安装完整性", () => {
  beforeEach(() => {
    galleryState.plugins = [];
    mocks.validateSource.mockReset();
    mocks.installPlugin.mockReset();
    mocks.updatePlugin.mockReset();
    mocks.validateSource.mockResolvedValue(manifest());
    mocks.installPlugin.mockResolvedValue({
      pluginId: "demo@external",
      version: "1.0.0",
      installPath: "/mock/demo",
    });
  });

  it("远程来源未填写哈希时不能发起验证", async () => {
    render(<PluginGallery />);
    fireEvent.change(
      screen.getByPlaceholderText("chat.plugins.marketplace.installPlaceholder"),
      { target: { value: SOURCE } },
    );
    const installButton = screen.getAllByRole("button")
      .find((button) => button.textContent === "chat.plugins.marketplace.install");
    expect(installButton).toBeDefined();
    expect(installButton?.hasAttribute("disabled")).toBe(true);
    fireEvent.click(installButton!);
    await waitFor(() => expect(mocks.validateSource).not.toHaveBeenCalled());
    expect(screen.getByText("chat.plugins.marketplace.sha256Required")).toBeInTheDocument();
  });

  it("确认安装冻结验证时的哈希，而不是读取确认框打开后的输入", async () => {
    render(<PluginGallery />);
    fillSourceAndHash(SOURCE, OLD_HASH);
    fireEvent.click(screen.getByRole("button", { name: "chat.plugins.marketplace.install" }));

    await waitFor(() => expect(mocks.validateSource).toHaveBeenCalledWith(SOURCE, OLD_HASH));
    expect(await screen.findByText("chat.plugins.marketplace.installSource")).toBeInTheDocument();
    expect(screen.getByText(OLD_HASH)).toBeInTheDocument();
    expect(mocks.installPlugin).not.toHaveBeenCalled();

    // 校验成功后修改页面上的哈希输入；确认安装仍必须使用校验时冻结的旧值。
    fireEvent.change(
      screen.getByPlaceholderText("chat.plugins.marketplace.sha256Placeholder"),
      { target: { value: NEW_HASH } },
    );
    fireEvent.click(
      screen.getByRole("button", { name: "chat.plugins.marketplace.confirmInstall" }),
    );
    await waitFor(() => expect(mocks.installPlugin).toHaveBeenCalledWith(SOURCE, OLD_HASH));
  });

  it("发布地址无需确认，一次点击使用清单哈希完成安装", async () => {
    const releaseSource = "https://example.com/release.json";
    const normalizedReleaseSource = `axagent-release+${releaseSource}`;
    mocks.validateSource.mockResolvedValue({ ...manifest(), expectedSha256: OLD_HASH });
    render(<PluginGallery />);
    fireEvent.change(
      screen.getByPlaceholderText("chat.plugins.marketplace.installPlaceholder"),
      { target: { value: releaseSource } },
    );
    const installButton = screen.getAllByRole("button")
      .find((button) => button.textContent === "chat.plugins.marketplace.install");
    expect(installButton?.hasAttribute("disabled")).toBe(false);
    fireEvent.click(installButton!);

    await waitFor(() => expect(mocks.validateSource).toHaveBeenCalledWith(normalizedReleaseSource, undefined));
    await waitFor(() => expect(mocks.installPlugin).toHaveBeenCalledWith(normalizedReleaseSource, OLD_HASH));
    expect(screen.queryByRole("button", { name: "chat.plugins.marketplace.confirmInstall" })).not.toBeInTheDocument();
  });
});

describe("PluginGallery 更新完整性", () => {
  const installedPlugin = {
    id: "demo@external",
    name: "demo",
    version: "1.0.0",
    description: "demo plugin",
    kind: "external" as const,
    enabled: true,
    tools: [],
    mcpServers: [],
    skills: [],
    source: SOURCE,
    permissions: [],
    pages: [],
    integrityStatus: "verified",
    pinnedHash: OLD_HASH,
    commands: [],
    eventStats: null,
  };

  beforeEach(() => {
    galleryState.plugins = [installedPlugin];
    mocks.validateSource.mockReset();
    mocks.updatePlugin.mockReset();
    mocks.validateSource.mockResolvedValue(manifest());
    mocks.updatePlugin.mockResolvedValue({
      pluginId: installedPlugin.id,
      oldVersion: "1.0.0",
      newVersion: "1.1.0",
      installPath: "/mock/demo",
    });
  });

  it("远程来源未填写哈希时不能发起更新验证", async () => {
    render(<PluginGallery />);
    fireEvent.click(screen.getByRole("button", { name: "chat.plugins.marketplace.update" }));
    const validateButton = screen.getAllByRole("button")
      .find((button) => button.textContent === "chat.plugins.marketplace.validate");
    expect(validateButton?.hasAttribute("disabled")).toBe(true);
    fireEvent.click(validateButton!);
    await waitFor(() => expect(mocks.validateSource).not.toHaveBeenCalled());
  });

  it("发布地址更新无需手填哈希，并使用预览返回的清单哈希", async () => {
    galleryState.plugins = [{
      ...installedPlugin,
      source: "axagent-release+https://example.com/plugin-release.json",
    }];
    const releaseSource = String(galleryState.plugins[0].source);
    mocks.validateSource.mockResolvedValue({ ...manifest(), expectedSha256: OLD_HASH });
    render(<PluginGallery />);
    fireEvent.click(screen.getByRole("button", { name: "chat.plugins.marketplace.update" }));

    const validateButton = screen.getAllByRole("button")
      .find((button) => button.textContent === "chat.plugins.marketplace.validate");
    expect(validateButton?.hasAttribute("disabled")).toBe(false);
    fireEvent.click(validateButton!);

    await waitFor(() => expect(mocks.validateSource).toHaveBeenCalledWith(releaseSource, undefined));
    const confirmButton = await screen.findByRole(
      "button",
      { name: "chat.plugins.marketplace.confirmUpdate" },
    );
    await waitFor(() => expect(screen.getAllByText(OLD_HASH).length).toBeGreaterThan(0));
    fireEvent.click(confirmButton);
    await waitFor(() => expect(mocks.updatePlugin).toHaveBeenCalledWith(installedPlugin.id, OLD_HASH));
  });

  it("确认更新冻结验证时的哈希，而不是读取确认框打开后的输入", async () => {
    render(<PluginGallery />);
    fireEvent.click(screen.getByRole("button", { name: "chat.plugins.marketplace.update" }));
    const updateDialog = await screen.findByRole("dialog");
    fireEvent.change(
      within(updateDialog).getByPlaceholderText("chat.plugins.marketplace.sha256Required"),
      { target: { value: OLD_HASH } },
    );
    fireEvent.click(
      within(updateDialog).getByRole("button", { name: "chat.plugins.marketplace.validate" }),
    );
    await waitFor(() => expect(mocks.validateSource).toHaveBeenCalledWith(SOURCE, OLD_HASH));
    const confirmButton = await within(updateDialog).findByRole("button", {
      name: "chat.plugins.marketplace.confirmUpdate",
    });
    await waitFor(() => expect(screen.getAllByText(OLD_HASH).length).toBeGreaterThan(0));

    // 校验后修改输入框不能影响确认更新使用的哈希快照。
    fireEvent.change(screen.getByPlaceholderText("chat.plugins.marketplace.sha256Placeholder"), {
      target: { value: NEW_HASH },
    });
    fireEvent.click(confirmButton);
    await waitFor(() => expect(mocks.updatePlugin).toHaveBeenCalledWith(installedPlugin.id, OLD_HASH));
  });
});
