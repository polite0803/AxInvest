// SPDX-License-Identifier: AGPL-3.0-only

// Windows cwd 盘符大小写防御：规范 cwd 后再启动 vitest CLI。
// 根因（vitest#10812 / angular-cli#33559）：process.cwd() 为小写盘符（d:\）时，
// vitest module-runner 生成的模块 ID 与 runner chunk 的 canonical 路径（D:\）不一致，
// @vitest/runner 被双实例化 → 测试文件内的 describe 拿到的 runner 未初始化 →
// 首个 describe() 即崩：TypeError: Cannot read properties of undefined (reading 'config')。
// 修复：用 realpathSync.native 把 cwd 规范化为磁盘 canonical 大小写（D:\）再启动 vitest。
import { spawnSync } from "node:child_process";
import { realpathSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const projectRoot = path.resolve(here, "..");
// realpathSync.native 会把网络映射盘（D: → \\vmware-host\Shared Folders\D）解析成 UNC；
// 用 UNC 作 cwd 会让 vite 的 root 变成 `//vmware-host/...` 形态，同样收集不到用例。
// 因此只在「盘符的大小写」上做规范化，不跟随到 UNC。
const nativeReal = realpathSync.native(projectRoot);
const canonical = nativeReal.startsWith("\\\\")
  ? projectRoot.replace(/^[a-z]/, (c) => c.toUpperCase())
  : nativeReal;
if (process.cwd() !== canonical) {
  process.chdir(canonical);
}

const vitestEntry = path.join(canonical, "node_modules", "vitest", "vitest.mjs");

// 强制 `--configLoader native`：vite 8 默认用 `bundle`（rolldown）打包配置文件，
// 在 VMware 共享目录（`\\vmware-host\Shared Folders\...`）上会把配置里的
// `import.meta.dirname` / root 解析成畸形串 `Z: Folders/D/...` ⇒ 收集阶段报
// 「No test files found」（实测：0–1 个插件正常，≥2 个真实插件必现；加 `root` 也压不住）。
// native 加载器直读配置文件，不做打包 ⇒ root 正确；vite 未来版本会把它设为默认。
const passthrough = process.argv.slice(2);
const hasLoader = passthrough.some((a) => a === "--configLoader" || a.startsWith("--configLoader="));
const args = hasLoader ? passthrough : ["--configLoader", "native", ...passthrough];

const result = spawnSync(process.execPath, [vitestEntry, ...args], {
  stdio: "inherit",
  cwd: canonical,
});
process.exit(result.status ?? 1);
