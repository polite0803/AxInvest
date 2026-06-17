// 集成测试: pr-ci.yml playwright 安装步骤与上游对齐
//
// 上游(macOS 镜像)使用 --with-deps 能在 7 分钟内跑完 E2E,
// 不需要 HOMEBREW_NO_AUTO_UPDATE 或 timeout-minutes。
// 本地与上游保持完全一致。

import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

function loadPrCi(): string {
  const projectRoot = process.cwd();
  const ymlPath = resolve(projectRoot, ".github/workflows/pr-ci.yml");
  return readFileSync(ymlPath, "utf8");
}

describe(".github/workflows/pr-ci.yml — Playwright 安装步骤", () => {
  it("playwright install 使用 --with-deps(与上游一致,macOS 镜像预装系统库可正常执行)", () => {
    const yml = loadPrCi();
    const installLineMatches = [...yml.matchAll(/run:\s*npx[^\n]*playwright\s+install[^\n]*/g)];
    expect(installLineMatches.length).toBeGreaterThan(0);
    for (const m of installLineMatches) {
      expect(m[0]).toMatch(/--with-deps/);
    }
  });

  it("E2E job 步骤与上游一致(无 HOMEBREW_NO_AUTO_UPDATE,无 timeout-minutes)", () => {
    const yml = loadPrCi();
    const jobIdx = yml.indexOf("test-e2e:");
    expect(jobIdx).toBeGreaterThan(0);
    const after = yml.slice(jobIdx);
    const nextJobMatch = after.slice(1).search(/\n {2}[a-zA-Z][a-zA-Z0-9_-]*:/);
    const end = nextJobMatch > 0 ? jobIdx + 1 + nextJobMatch : yml.length;
    const jobBlock = yml.slice(jobIdx, end);
    // 不应有与上游不一致的配置
    expect(jobBlock).not.toMatch(/HOMEBREW_NO_AUTO_UPDATE/);
    expect(jobBlock).not.toMatch(/timeout-minutes/);
  });
});
