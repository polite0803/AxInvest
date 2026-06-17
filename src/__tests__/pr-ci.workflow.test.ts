// 集成测试: pr-ci.yml playwright 安装步骤不能使用 --with-deps
//
// 触发链:
//  1. GitHub Actions macos-latest runner 执行
//     `npx --yes playwright install chromium --with-deps`
//  2. download 完成后,--with-deps 在 macOS 上调用 `sudo brew install` 安装系统依赖
//  3. 在 GitHub-hosted CI 环境里:
//       - 没有交互式 sudo,brew 卡在密码提示
//       - 或 brew auto-update 在拉取 homebrew-core 时超时
//     整体 step 永远不退出,CI 卡死(用户报告 100% 后无响应)
//
// 修复要求:
//   1. 移除 --with-deps(macOS 镜像已经预装 Playwright 所需依赖)
//   2. 设置 HOMEBREW_NO_AUTO_UPDATE=1 防止 brew 自更新阻塞
//   3. 给 step 加 timeout-minutes,避免再次卡死时让 job 直接失败

import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

function loadPrCi(): string {
  const projectRoot = process.cwd();
  const ymlPath = resolve(projectRoot, ".github/workflows/pr-ci.yml");
  return readFileSync(ymlPath, "utf8");
}

describe(".github/workflows/pr-ci.yml — Playwright 安装步骤", () => {
  it("playwright install 命令不应带 --with-deps(macOS 会触发 brew install 阻塞 CI)", () => {
    const yml = loadPrCi();
    // 提取所有 npx playwright install 行
    const installLineMatches = [...yml.matchAll(/run:\s*npx[^\n]*playwright\s+install[^\n]*/g)];
    expect(installLineMatches.length).toBeGreaterThan(0);
    for (const m of installLineMatches) {
      expect(m[0]).not.toMatch(/--with-deps/);
    }
  });

  it("应设置 HOMEBREW_NO_AUTO_UPDATE=1 防止 brew 自更新阻塞 macOS runner", () => {
    const yml = loadPrCi();
    expect(yml).toMatch(/HOMEBREW_NO_AUTO_UPDATE:\s*["']?1["']?/);
  });

  it("E2E job 应有 timeout-minutes 保护(job 级别或 step 级别均可),避免再次卡死时无超时", () => {
    const yml = loadPrCi();
    // 定位 test-e2e job 块
    const jobIdx = yml.indexOf("test-e2e:");
    expect(jobIdx).toBeGreaterThan(0);
    // job 块:从 jobIdx 到下一个 "^  [a-zA-Z][a-zA-Z0-9_-]*:" 之前
    const after = yml.slice(jobIdx);
    const nextJobMatch = after.slice(1).search(/\n {2}[a-zA-Z][a-zA-Z0-9_-]*:/);
    const end = nextJobMatch > 0 ? jobIdx + 1 + nextJobMatch : yml.length;
    const jobBlock = yml.slice(jobIdx, end);
    expect(jobBlock).toMatch(/timeout-minutes:\s*\d+/);
  });
});
