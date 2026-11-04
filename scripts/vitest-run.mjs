// SPDX-License-Identifier: AGPL-3.0-only

// Windows cwd 盘符大小写防御：规范 cwd 后再启动 vitest CLI。
// 根因（vitest#10812 / angular-cli#33559）：process.cwd() 为小写盘符（d:\）时，
// vitest module-runner 生成的模块 ID 与 runner chunk 的 canonical 路径（D:\）不一致，
// @vitest/runner 被双实例化 → 测试文件内的 describe 拿到的 runner 未初始化 →
// 首个 describe() 即崩：TypeError: Cannot read properties of undefined (reading 'config')。
// 修复：用 realpathSync.native 把 cwd 规范化为磁盘 canonical 大小写（D:\）再启动 vitest。
//
// 任务 #137（2026-11-04）：从 UNC 坐标（`\\vmware-host\Shared Folders\...`）调用时，
// ① 位置过滤参数按相对 root 匹配，UNC root 与映射盘 root 不是同一串 ⇒ 恒 0 命中
//    （vitest 只报「No test files found」，不告诉你问题出在坐标）；
// ② 即使 root 对了，网络盘上并行 fork 也起不来（worker 60 s 超时）。
// 两条都在本文件里处理：UNC → 映射回盘符（并排除「项目目录就是盘根」的塌缩别名）；
// 网络 root → 默认 --no-file-parallelism。
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

/** 去掉 Windows 长路径前缀，统一成 `\\host\share\...` / `X:\...` 形态。 */
function stripLongPath(p) {
  return p.replace(/^\\\\\?\\UNC\\/i, "\\\\").replace(/^\\\\\?\\/i, "");
}

/** 规范成「无前导长路径前缀 + 反斜杠 + 尾部分隔符」的可比较形态。 */
function normalizePath(p) {
  const s = stripLongPath(p).replace(/\//g, "\\");
  return s.endsWith("\\") ? s : s + "\\";
}

/**
 * UNC 根 → 盘符别名（任务 #137）：cwd 落在 UNC 时，`vitest run <文件或目录>` 的
 * 位置过滤参数按相对 root 匹配，UNC root 与映射盘 root 不是同一串 ⇒ **静默 0 命中**
 * （vitest 只报「No test files found」，不会告诉你问题出在 cwd 坐标）。
 * 做法：扫 A–Z，用 `realpathSync.native("X:\\")` 拿到该盘的 UNC 真身，取**最长**匹配
 * （Z:\ 可能映射到共享根、D:\ 映射到共享下的某个目录，两者是前缀关系 ⇒ 必须取更具体的那个）。
 * 找不到映射（真 UNC 共享、未 `net use`）⇒ 返回 null，由调用方决定怎么报。
 */
function driveAliasForUnc(uncPath) {
  const target = normalizePath(uncPath);
  const lower = target.toLowerCase();
  let best = null;
  for (let c = 65; c <= 90; c++) {
    const letter = String.fromCharCode(c) + ":\\";
    let real;
    try {
      real = realpathSync.native(letter);
    } catch {
      continue;
    }
    const root = normalizePath(real);
    if (!root.startsWith("\\\\") || !lower.startsWith(root.toLowerCase())) {
      continue;
    }
    // 项目目录本身就等于某个盘的根时（实测本机 `X:` → 本仓目录），别名会塌成 `X:\`：
    // vitest 的 root 变成 `X:/`，forks worker 60 s 起不来（同一目录换个串走另一条 SMB 路径，
    // 依赖优化被当成新 root 重跑）。⇒ 这类别名不选，宁可报「无可用盘符」。
    if (root.length === target.length) {
      continue;
    }
    if (!best || root.length > best.root.length) {
      best = { letter, root };
    }
  }
  if (!best) {
    return null;
  }
  return best.letter + target.slice(best.root.length);
}

// 坐标选择（任务 #137）：从盘符路径进来 ⇒ 沿用旧行为（只规范化盘符大小写）；
// 从 UNC 路径进来 ⇒ 先尝试映射回盘符（文件过滤器才有意义），映射不到才退回 UNC。
let canonical;
if (projectRoot.startsWith("\\\\")) {
  canonical = driveAliasForUnc(projectRoot) ?? projectRoot.replace(/^([a-z])/i, (c) => c.toUpperCase());
} else {
  canonical = nativeReal.startsWith("\\\\") ? projectRoot.replace(/^[a-z]/, (c) => c.toUpperCase()) : nativeReal;
}
if (process.cwd() !== canonical) {
  process.chdir(canonical);
}

// 映射不到盘符、又带了位置过滤参数 ⇒ 当场说清楚，不让 vitest 只吐一句
// 「No test files found」（那条消息会把「坐标不对」误导成「没有测试」，本仓已栽过一次）。
if (canonical.startsWith("\\\\")) {
  const argv = process.argv.slice(2);
  const filterAt = argv.findIndex((a, i) => i > 0 && !a.startsWith("-") && argv[i - 1] === "run");
  if (filterAt > 0) {
    console.error(
      `[vitest-run] root 仍是 UNC 坐标（${canonical}），而位置过滤参数「${argv[filterAt]}」按相对 root 匹配 ⇒ 恒 0 命中。\n` +
        "修法二选一：① 从盘符路径调用（cd D:\\OneManager\\AxInvest）；② 先 net use 把该共享映射成盘符。",
    );
    process.exit(2);
  }
}

const vitestEntry = path.join(canonical, "node_modules", "vitest", "vitest.mjs");

// 强制 `--configLoader native`：vite 8 默认用 `bundle`（rolldown）打包配置文件，
// 在 VMware 共享目录（`\\vmware-host\Shared Folders\...`）上会把配置里的
// `import.meta.dirname` / root 解析成畸形串 `Z: Folders/D/...` ⇒ 收集阶段报
// 「No test files found」（实测：0–1 个插件正常，≥2 个真实插件必现；加 `root` 也压不住）。
// native 加载器直读配置文件，不做打包 ⇒ root 正确；vite 未来版本会把它设为默认。
const passthrough = process.argv.slice(2);
const hasLoader = passthrough.some((a) => a === "--configLoader" || a.startsWith("--configLoader="));
// 网络盘上并行 fork 起不来（实测 2026-11-04，本机 SMB：4 个文件并发 ⇒ 4 个 worker 全部
// 60 s「Timeout waiting for worker to respond」；同一命令加 --no-file-parallelism ⇒
// 4 文件 / 54 tests 全过，耗时 628 s）。⇒ root 落在网络共享时默认串行；
// 调用方显式给了并行开关就尊重它（本地盘不受影响，CI 走本地盘）。
const networkRoot = realpathSync.native(canonical).startsWith("\\\\");
const hasParallelSwitch = passthrough.some((a) =>
  a === "--no-file-parallelism" || a.startsWith("--file-parallelism") || a.startsWith("--fileParallelism")
);
// 网络盘上 fork 池的**启动握手**慢到撞 vitest 内部常量（任务 #141）：`START_TIMEOUT = 6e4`
// 硬编码在 `vitest/dist/chunks/cli-api.*.js`，CLI 无开关、`poolOptions` 也够不着 ⇒ 只能换池。
// 实测 2026-11-04（本机 SMB，4 个 plugin 测试文件 = 54 tests，两次样本）：
//   forks + 串行   ⇒ 628 s / 505 s，且 505 s 那次 2 个 worker 直接启动超时；
//   threads + 串行 ⇒ **4 文件 54 tests 全过两次**（181.5 s / 199.1 s，EXIT=0）。
// 为什么不是 `--no-isolate`（同轮试过）：它把单 worker 的启动次数降到 1，但四个测试文件共享
// 模块表 ⇒ `vi.mocked(invoke)` 的 mock 跨文件互相污染，**3 文件 17 条断言当场转红**。
// 故网络 root 默认 threads 池 + 串行；调用方显式给了 --pool / 并行开关就尊重它。
// 本机 vitest 已是 4.1.12，配置注释里 4.1.4 时代的「threads 池在 Windows 上偶发崩溃」未复现。
const hasPoolSwitch = passthrough.some((a) => a === "--pool" || a.startsWith("--pool="));
const args = [
  ...(hasLoader ? [] : ["--configLoader", "native"]),
  ...(networkRoot && !hasPoolSwitch ? ["--pool=threads"] : []),
  ...(networkRoot && !hasParallelSwitch ? ["--no-file-parallelism"] : []),
  ...passthrough,
];
if (networkRoot) {
  // 自证：本机排障时一眼能看出这次到底用哪个池、是不是串行（不靠读源码猜）。
  console.error(`[vitest-run] 网络 root ⇒ ${args.filter((a) => a.startsWith("--pool") || a.includes("parallelism")).join(" ")}`);
}

const result = spawnSync(process.execPath, [vitestEntry, ...args], {
  stdio: "inherit",
  cwd: canonical,
});
process.exit(result.status ?? 1);
