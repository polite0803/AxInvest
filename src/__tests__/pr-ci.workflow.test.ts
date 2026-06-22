// 集成测试: pr-ci.yml playwright 安装步骤
//
// macOS runner 上 playwright 需要 --with-deps 来安装系统依赖（brew）。
// 当 cache miss 时 brew install 可能卡死（brew update 慢），
// 但这是上游设定的正确配置，保持与上游一致。
// 缓存命中时 brew install 跳过，速度正常。
// cache key 包含 package-lock.json hash，lock 不变时走 cache。

import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

function loadPrCi(): string {
  const projectRoot = process.cwd();
  const ymlPath = resolve(projectRoot, ".github/workflows/pr-ci.yml");
  return readFileSync(ymlPath, "utf8");
}

describe(".github/workflows/pr-ci.yml — Playwright 安装步骤", () => {
  it("playwright install 应仅在有缓存 miss 时执行", () => {
    const yml = loadPrCi();
    const installLines = [...yml.matchAll(/run:\s*npx[^\n]*playwright\s+install[^\n]*/g)];
    expect(installLines.length).toBeGreaterThan(0);
  });

  it("E2E job 应与上游一致（macOS + 无 HOMEBREW_NO_AUTO_UPDATE，无 timeout-minutes）", () => {
    const yml = loadPrCi();
    const jobIdx = yml.indexOf("test-e2e:");
    expect(jobIdx).toBeGreaterThan(0);
    const after = yml.slice(jobIdx);
    const nextJobMatch = after.slice(1).search(/\n {2}[a-zA-Z][a-zA-Z0-9_-]*:/);
    const end = nextJobMatch > 0 ? jobIdx + 1 + nextJobMatch : yml.length;
    const jobBlock = yml.slice(jobIdx, end);
    expect(jobBlock).toMatch(/macOS/);
    expect(jobBlock).not.toMatch(/HOMEBREW_NO_AUTO_UPDATE/);
    expect(jobBlock).not.toMatch(/timeout-minutes/);
  });
});
