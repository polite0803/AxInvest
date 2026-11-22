// SPDX-License-Identifier: AGPL-3.0-only

import { beforeEach, describe, expect, it } from "vitest";

import { handleCommand } from "../browserMock";

const HASH = "a".repeat(64);

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

describe("browserMock 插件来源完整性门禁", () => {
  beforeEach(() => {
    localStorage.clear();
  });

  it("预览验证允许远程源省略哈希，但拒绝非法哈希", async () => {
    await expect(
      handleCommand("plugin_validate_source", { source: "https://example.com/plugin.git" }),
    ).resolves.toMatchObject({ version: "0.0.0" });
    await expectIntegrityFailure(
      handleCommand("plugin_validate_source", {
        source: "https://example.com/plugin.git",
        expectedSha256: "bad",
      }),
    );
  });

  it("发布源预览返回确定性哈希，mock 安装保持真实能力边界", async () => {
    const preview = await handleCommand<Record<string, unknown>>("plugin_validate_source", {
      source: "axagent-release+https://example.com/plugin-release.json",
    });
    expect(preview).toMatchObject({
      expectedSha256: "0".repeat(64),
      capabilities: [],
    });

    await handleCommand("plugin_install", {
      source: "axagent-release+https://example.com/plugin-release.json",
      expectedSha256: "0".repeat(64),
    });
    const rows = await handleCommand<Array<Record<string, unknown>>>("plugin_list", {});
    expect(rows[0].integrityStatus).not.toBe("verified");
    expect(rows[0].pinnedHash).toBeNull();
  });

  it("发布源安装和更新缺少或漂移预览哈希时拒绝", async () => {
    const source = "axagent-release+https://example.com/plugin-release.json";
    await expectIntegrityFailure(handleCommand("plugin_install", { source }));
    await expectIntegrityFailure(
      handleCommand("plugin_install", { source, expectedSha256: HASH }),
    );
    await handleCommand("plugin_install", { source, expectedSha256: "0".repeat(64) });
    await expectIntegrityFailure(
      handleCommand("plugin_update", { pluginId: "plugin-1" }),
    );
    await expectIntegrityFailure(
      handleCommand("plugin_update", { pluginId: "plugin-1", expectedSha256: HASH }),
    );
    await expect(
      handleCommand("plugin_update", {
        pluginId: "plugin-1",
        expectedSha256: "0".repeat(64),
      }),
    ).resolves.toMatchObject({ pluginId: "plugin-1", newVersion: "0.0.1" });
  });

  it("远程安装省略哈希时拒绝，且不写入插件", async () => {
    await expectIntegrityFailure(
      handleCommand("plugin_install", { source: "https://example.com/plugin.git" }),
    );
    await expect(handleCommand("plugin_list", {})).resolves.toEqual([]);
  });

  it("本地安装允许省略哈希，填写哈希时仍校验格式", async () => {
    const installed = await handleCommand<Record<string, unknown>>("plugin_install", {
      source: "./local-plugin",
    });
    expect(installed).toMatchObject({
      pluginId: "plugin-1",
      version: "0.0.0",
      installPath: "/mock/plugins/plugin-1",
    });
    const rows = await handleCommand<Array<Record<string, unknown>>>("plugin_list", {});
    expect(rows).toHaveLength(1);
    expect(rows[0].source).toBe("./local-plugin");
    expect(rows[0]).toMatchObject({
      permissions: [],
      pages: [],
      integrityStatus: "unknown",
      pinnedHash: null,
    });

    await expectIntegrityFailure(
      handleCommand("plugin_install", {
        source: "./another-local-plugin",
        expectedSha256: "bad",
      }),
    );
  });

  it("本地插件更新允许省略哈希，并返回完整 UpdateOutcomeDto", async () => {
    await handleCommand("plugin_install", { source: "./local-plugin" });
    const outcome = await handleCommand<Record<string, unknown>>("plugin_update", {
      pluginId: "plugin-1",
    });
    expect(outcome).toEqual({
      pluginId: "plugin-1",
      oldVersion: "0.0.0",
      newVersion: "0.0.1",
      installPath: "/mock/plugins/plugin-1",
    });
    const rows = await handleCommand<Array<Record<string, unknown>>>("plugin_list", {});
    expect(rows[0].version).toBe("0.0.1");
  });

  it("浏览器 mock 不伪造 verified 完整性状态", async () => {
    await handleCommand("plugin_install", {
      source: "https://example.com/plugin.git",
      expectedSha256: HASH,
    });
    const rows = await handleCommand<Array<Record<string, unknown>>>("plugin_list", {});
    expect(rows[0].integrityStatus).not.toBe("verified");
    expect(rows[0].pinnedHash).toBeNull();
  });

  it("远程插件更新无哈希时拒绝，填写合法哈希后允许", async () => {
    await handleCommand("plugin_install", {
      source: "https://example.com/plugin.git",
      expectedSha256: HASH,
    });
    await expectIntegrityFailure(
      handleCommand("plugin_update", { pluginId: "plugin-1" }),
    );
    await expect(
      handleCommand("plugin_update", {
        pluginId: "plugin-1",
        expectedSha256: HASH,
      }),
    ).resolves.toMatchObject({ pluginId: "plugin-1", newVersion: "0.0.1" });
  });

  it("无来源登记的插件不能更新（不能伪造已验证升级）", async () => {
    localStorage.setItem(
      "axagent_plugins",
      JSON.stringify([{ id: "ghost", version: "1.0.0" }]),
    );
    await expectIntegrityFailure(
      handleCommand("plugin_update", { pluginId: "ghost", expectedSha256: HASH }),
    );
  });
});
