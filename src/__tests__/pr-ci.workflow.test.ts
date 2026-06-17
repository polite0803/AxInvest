// 集成测试: pr-ci.yml playwright 安装步骤
//
// macOS GitHub runner 已预装系统库，不需要 --with-deps。
// 去掉 --with-deps 可避免 cache miss 时 brew install 卡死。
// 也不需要 HOMEBREW_NO_AUTO_UPDATE 或 timeout-minutes。

import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

function loadPrCi(): string {
  const projectRoot = process.cwd();
  const ymlPath = resolve(projectRoot, ".github/workflows/pr-ci.yml");
  return readFileSync(ymlPath, "utf8");
}

describe(".github/workflows/pr-ci.yml — Playwright 安装步骤", () => {
  it("playwright install 不应带 --with-deps（避免 cache miss 时 brew install 卡死 macOS runner）", () => {
    const yml = loadPrCi();
    const installLineMatches = [...yml.matchAll(/run:\s*npx[^\n]*playwright\s+install[^\n]*/g)];
    expect(installLineMatches.length).toBeGreaterThan(0);
    for (const m of installLineMatches) {
      expect(m[0]).not.toMatch(/--with-deps/);
    }
  });

  it("E2E job 应与上游一致（无 HOMEBREW_NO_AUTO_UPDATE，无 timeout-minutes）", () => {
    const yml = loadPrCi();
    const jobIdx = yml.indexOf("test-e2e:");
    expect(jobIdx).toBeGreaterThan(0);
    const after = yml.slice(jobIdx);
    const nextJobMatch = after.slice(1).search(/\n {2}[a-zA-Z][a-zA-Z0-9_-]*:/);
    const end = nextJobMatch > 0 ? jobIdx + 1 + nextJobMatch : yml.length;
    const jobBlock = yml.slice(jobIdx, end);
    expect(jobBlock).not.toMatch(/HOMEBREW_NO_AUTO_UPDATE/);
    expect(jobBlock).not.toMatch(/timeout-minutes/);
  });
});
