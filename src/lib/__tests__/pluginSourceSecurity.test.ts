// SPDX-License-Identifier: AGPL-3.0-only

import { describe, expect, it } from "vitest";

import { isReleasePluginSource, isRemotePluginSource, normalizePluginSource } from "../pluginSourceSecurity";

describe("pluginSourceSecurity", () => {
  const rejectedCases: ReadonlyArray<readonly [string, string]> = [
    ["https://example.com/plugin.git", "HTTPS Git URL"],
    ["http://example.com/plugin", "HTTP URL"],
    ["git@github.com:org/plugin.git", "SSH Git URL"],
    ["openclaw:demo", "OpenClaw 包"],
    ["@scope/name", "scoped npm 包"],
    ["name@1.2.3", "带版本 npm 包"],
    ["plain-package", "裸 npm 包"],
    ["/tmp/plugin", "POSIX 绝对路径"],
    ["./plugin", "当前目录相对路径"],
    ["C:\\plugins\\demo", "Windows 盘符路径"],
    ["\\\\server\\share\\plugin", "UNC 路径"],
  ];

  it("只把 HTTPS release.json 视为合法远程来源", () => {
    for (
      const source of [
        "axagent-release+https://example.com/release.json",
        "https://example.com/plugins/wiki/release.json",
      ]
    ) {
      expect(isReleasePluginSource(source)).toBe(true);
      expect(isRemotePluginSource(source)).toBe(true);
    }
    for (const [source, label] of rejectedCases) {
      expect(isRemotePluginSource(source), label).toBe(false);
      expect(isReleasePluginSource(source), label).toBe(false);
    }
  });

  it("前后空白不影响来源判定，空串不当成远程", () => {
    expect(isRemotePluginSource("  https://example.com/release.json  ")).toBe(true);
    expect(isRemotePluginSource("  ./plugin  ")).toBe(false);
    expect(isRemotePluginSource("   ")).toBe(false);
  });

  it("归一化只补发布协议，不误伤其它 URL", () => {
    expect(normalizePluginSource("https://example.com/plugins/wiki/release.json")).toBe(
      "axagent-release+https://example.com/plugins/wiki/release.json",
    );
    expect(
      normalizePluginSource("axagent-release+https://example.com/plugins/wiki/release.json"),
    ).toBe("axagent-release+https://example.com/plugins/wiki/release.json");
    expect(normalizePluginSource(" https://example.com/plugin.git ")).toBe(
      "https://example.com/plugin.git",
    );
    expect(normalizePluginSource("./release.json")).toBe("./release.json");
  });
});
