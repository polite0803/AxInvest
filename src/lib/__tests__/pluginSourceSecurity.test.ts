// SPDX-License-Identifier: AGPL-3.0-only

import { describe, expect, it } from "vitest";

import {
  isPluginSha256,
  isReleasePluginSource,
  isRemotePluginSource,
  normalizePluginSource,
} from "../pluginSourceSecurity";

describe("pluginSourceSecurity", () => {
  // 与 Rust manager.rs 的 parse_install_source 逐形态对齐：Git / npm / OpenClaw 必须带哈希，
  // 其余形态由 resolve_local_source 当本地路径解析，可省略哈希。
  const remoteCases: ReadonlyArray<readonly [string, string]> = [
    ["https://example.com/plugin.git", "https Git URL"],
    ["http://example.com/plugin", "http Git URL"],
    ["git@github.com:org/plugin.git", "SSH Git URL"],
    ["example.com/plugin.git", "相对 .git URL"],
    ["openclaw:demo", "OpenClaw 包"],
    ["@scope/name", "scoped npm 包"],
    ["name@1.2.3", "带版本 npm 包"],
    ["plain-package", "裸 npm 包"],
  ];
  const localCases: ReadonlyArray<readonly [string, string]> = [
    ["/tmp/plugin", "POSIX 绝对路径"],
    ["./plugin", "当前目录相对路径"],
    ["../plugin", "上级目录相对路径"],
    [".\\plugin", "Windows 当前目录相对路径"],
    ["..\\plugin", "Windows 上级目录相对路径"],
    ["~", "用户主目录"],
    ["~/plugin", "用户主目录相对路径"],
    ["~\\plugin", "Windows 用户主目录相对路径"],
    ["C:\\plugins\\demo", "Windows 盘符路径"],
    ["D:/plugins/demo", "Windows 正斜杠盘符路径"],
    ["\\\\server\\share\\plugin", "UNC 路径"],
    ["/tmp/pkg@1", "含 @ 的本地绝对路径"],
    ["relative/path", "含路径分隔符的相对路径"],
  ];

  it("识别远程 Git / npm / OpenClaw 来源", () => {
    for (const [source, label] of remoteCases) {
      expect(isRemotePluginSource(source), label).toBe(true);
    }
  });

  it("识别本地目录，并避免把类似 npm 的本地路径误判为远程", () => {
    for (const [source, label] of localCases) {
      expect(isRemotePluginSource(source), label).toBe(false);
    }
  });

  it("前后空白不影响来源判定，空串不当成远程", () => {
    expect(isRemotePluginSource("  @scope/name  ")).toBe(true);
    expect(isRemotePluginSource("  ./plugin  ")).toBe(false);
    expect(isRemotePluginSource("   ")).toBe(false);
  });

  it("发布地址是独立来源，不混同为普通远程 Git/npm", () => {
    const release = "axagent-release+https://example.com/release.json";
    const plainRelease = "https://example.com/plugins/wiki/release.json";
    expect(isReleasePluginSource(release)).toBe(true);
    expect(isReleasePluginSource(plainRelease)).toBe(true);
    expect(isRemotePluginSource(release)).toBe(false);
    expect(isRemotePluginSource(plainRelease)).toBe(false);
    expect(isReleasePluginSource("axagent-release+http://example.com/release.json")).toBe(false);
    expect(isReleasePluginSource("https://example.com/plugins/wiki/manifest.json")).toBe(false);
    expect(isReleasePluginSource("http://example.com/plugins/wiki/release.json")).toBe(false);
    expect(isReleasePluginSource("https://example.com/plugin.git")).toBe(false);
    expect(isReleasePluginSource("")).toBe(false);
  });

  it("直接粘贴发布清单地址时只补发布协议，不误伤普通 Git URL", () => {
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

  it("SHA-256 必须恰好是 64 个十六进制字符", () => {
    expect(isPluginSha256("a".repeat(64))).toBe(true);
    expect(isPluginSha256(" A".repeat(32).trim())).toBe(false);
    expect(isPluginSha256("g".repeat(64))).toBe(false);
    expect(isPluginSha256("a".repeat(63))).toBe(false);
    expect(isPluginSha256("a".repeat(65))).toBe(false);
    expect(isPluginSha256("")).toBe(false);
  });
});
