// SPDX-License-Identifier: AGPL-3.0-only

import { beforeEach, describe, expect, it } from "vitest";

import { handleCommand } from "../browserMock";

const SOURCE = "axagent-release+https://example.com/release.json";

async function expectIntegrityFailure(command: Promise<unknown>): Promise<void> {
  await expect(command).rejects.toSatisfy((error: unknown) => {
    const text = error instanceof Error ? error.message : String(error);
    try {
      const parsed = JSON.parse(text) as { code?: string; category?: string };
      return parsed.code === "PLUGIN_SOURCE_INTEGRITY_FAILED"
        && parsed.category === "validation";
    } catch {
      return false;
    }
  });
}

describe("browserMock 插件远程-only 门禁", () => {
  beforeEach(() => {
    localStorage.clear();
  });

  it("预览只接受 HTTPS release.json，并返回内部 token", async () => {
    await expectIntegrityFailure(
      handleCommand("plugin_validate_source", { source: "https://example.com/plugin.git" }),
    );
    const preview = await handleCommand<Record<string, unknown>>("plugin_validate_source", {
      source: SOURCE,
    });
    expect(preview).toMatchObject({ version: "0.0.0", capabilities: [] });
    expect(typeof preview.previewToken).toBe("string");
    expect(preview).not.toHaveProperty("expectedSha256");
  });

  it("安装需要发布地址和预览 token，安装后保持 disabled", async () => {
    await expectIntegrityFailure(handleCommand("plugin_install", { source: SOURCE }));
    await expectIntegrityFailure(
      handleCommand("plugin_install", { source: "./local-plugin", previewToken: "mock-preview.1" }),
    );
    const preview = await handleCommand<Record<string, unknown>>("plugin_validate_source", {
      source: SOURCE,
    });
    await handleCommand("plugin_install", {
      source: SOURCE,
      previewToken: preview.previewToken,
    });
    const rows = await handleCommand<Array<Record<string, unknown>>>("plugin_list", {});
    expect(rows).toHaveLength(1);
    expect(rows[0]).toMatchObject({
      source: SOURCE,
      enabled: false,
      integrityStatus: "unknown",
      pinnedHash: null,
    });
  });

  it("更新需要预览 token，且不伪造 verified 状态", async () => {
    const preview = await handleCommand<Record<string, unknown>>("plugin_validate_source", {
      source: SOURCE,
    });
    await handleCommand("plugin_install", {
      source: SOURCE,
      previewToken: preview.previewToken,
    });
    await expectIntegrityFailure(
      handleCommand("plugin_update", { pluginId: "plugin-1" }),
    );
    await expect(
      handleCommand("plugin_update", {
        pluginId: "plugin-1",
        previewToken: preview.previewToken,
      }),
    ).resolves.toMatchObject({ pluginId: "plugin-1", newVersion: "0.0.1" });
    const rows = await handleCommand<Array<Record<string, unknown>>>("plugin_list", {});
    expect(rows[0].integrityStatus).not.toBe("verified");
    expect(rows[0].pinnedHash).toBeNull();
  });

  it("无来源登记的插件不能更新", async () => {
    localStorage.setItem(
      "axagent_plugins",
      JSON.stringify([{ id: "ghost", version: "1.0.0" }]),
    );
    await expectIntegrityFailure(
      handleCommand("plugin_update", { pluginId: "ghost", previewToken: "mock-preview.1" }),
    );
  });
});
