#!/usr/bin/env node
// scripts/typecheck-unc.mjs
// UNC 共享目录下的 TypeScript 全量类型检查（任务 #45，2026-11-01）。
//
// 建门理由（实测两证，主代理与另一会话各独立复现一次）：
//   ① `npm run typecheck`（= `tsc --noEmit`）在本机从 UNC 目录启动时，`node_modules/.bin/tsc.cmd`
//      被 cmd.exe 以「UNC 路径不支持」拒绝，随后**退化成打印 tsc 帮助**；
//   ② 绕开 cmd.exe 用 `node node_modules/typescript/bin/tsc --noEmit` 直调，exit 0，
//      但 `--listFiles` **列出 0 个文件** —— 根因是 `tsconfig.json` 的 `include: ["src"]`
//      这条目录 glob 在 VMware 共享目录上枚举不到任何输入。
//   ⇒ 结论：本仓历史上所有「typecheck 通过」的记录都不算证据（CI 跑在本地盘，所以 CI 仍然有效，
//     这也是它一直没被发现的原因）。「退 0 + 0 输入」的门等于没有门。
//
// 修法：用 node 的 fs 遍历（在 UNC 上可用）把 `src/**` 的 `.ts/.tsx/.d.ts` **逐条写进
// 临时 tsconfig 的 `files`**（不走任何目录 glob），compilerOptions 从仓内 `tsconfig.json`
// 原样继承（`paths` 按新位置重新取相对前缀、丢弃 `references`——`-p` 非 build 模式下它本就不参与），
// 再 `tsc -p <该文件> --noEmit`。
//
// ⚠ 三档 fail-closed（任何一档不过 ⇒ 非零退出，绝不 warn 化）：
//   A. 枚举输入数 == 0 ⇒ 红。这正是 UNC 静默空跑的原始形态；把它当「没什么可查」放过去，
//      就是把这条门重新变回它要修的那个缺陷。
//   B. tsc 输出命中「帮助横幅」⇒ 红（cmd.exe 劫持 / 参数丢失时的退化形态，横幅+退 0 是假绿）。
//   C. 生成的临时 tsconfig 里 `include`/`exclude` 目录 glob 必须不存在、`files` 长度必须等于
//      枚举读数 ⇒ 红（保证「跑的就是逐条清单」，而不是又落回 glob）。
//   D. 退出码**直接取 tsc 的退出码**（1=有类型错误），不做任何「0 诊断且 0 文件也算失败」之外的兜底。
//
// 用法：
//   node scripts/typecheck-unc.mjs              # 判定：枚举 → 生成 → tsc --noEmit，报文件数与诊断数
//   node scripts/typecheck-unc.mjs --selftest    # 正负对照：枚举器非空 / 0 输入必红 / 真跑 tsc 抓到植入错误 / 帮助横幅检测
//   node scripts/typecheck-unc.mjs --list        # 只打印枚举读数与生成的配置路径（不调 tsc，秒级）
//   node scripts/typecheck-unc.mjs --src-dir D   # 枚举面覆写（仅自检/复现用；空目录 ⇒ A 档必红，不是放行通道）
//
// 退出码：0 = 枚举非空且 tsc 退 0；1 = 0 输入 / 配置自检失败 / 工具链缺失 / 帮助横幅退化；
//         tsc 自身的失败码（通常 1，参数错误 2）原样透传；selftest 任一断言不过 ⇒ 1。
//
// ⚠ 为什么**没有**把 `npm run typecheck` 改成走本脚本（实测过再放弃的，不是遗漏）：
//   本机 `npm run <任何 script>` 从 UNC 起就先被 cmd.exe 拒绝（「UNC 路径不支持」⇒ 工作目录退回
//   `C:\Windows`），对照 `npm run lint`（= `tsc --noEmit`，本次未改）同样是 `EXIT=1` 并打印帮助。
//   也就是说 npm 这条路**本来就已经响亮地失败**，换成 `node scripts/typecheck-unc.mjs` 只是把
//   报错文案从「帮助横幅」换成「MODULE_NOT_FOUND」，本机收益为零；而在本地盘 / CI 上
//   `npm run typecheck`（原生 tsc）本来就是有效的，改了反而与 ci.yml 既有步骤重复跑一遍全量类型检查。
//   ⇒ 正确入口是 **`node scripts/typecheck-unc.mjs`**（直调 node，绕开 .cmd 包装器与 cmd.exe）。
//   真正会被静默骗到的形态是 `node node_modules/typescript/bin/tsc --noEmit`（退 0 + 0 文件），
//   那一条现在由本门与 CI 的对应 step 覆盖。
//
// 接线：`scripts/ci-check.mjs` 的对应 step 与 `.github/workflows/ci.yml` **末尾**对应步骤
//   命令行逐字同源（先 `--selftest` 再判定），由 `src/__tests__/ci.workflow.test.ts` 的
//   对称差集护栏自动校验，不往 ALLOWLIST 塞条目。

import { spawnSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join, relative, resolve, sep } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const args = process.argv.slice(2);
const SELFTEST = args.includes("--selftest");
const LIST_ONLY = args.includes("--list");

const SRC_DIR = join(ROOT, "src");
// `--src-dir <目录>`：**仅供自检 / 复现用的枚举面覆写**（把枚举指向空目录 ⇒ A 档必须退非零）。
// 它不构成逃逸通道 —— 这个开关的失效方向只会是「更红」（空目录 ⇒ 判失败），不会是「放行」。
const SRC_DIR_OVERRIDE = (() => {
  const i = args.indexOf("--src-dir");
  return i >= 0 && args[i + 1] ? resolve(args[i + 1]) : null;
})();
const TSCONFIG = join(ROOT, "tsconfig.json");
const TSC_BIN = join(ROOT, "node_modules", "typescript", "bin", "tsc");
// 生成的配置放在 `.tmp/` 直下（不放子目录）：`files` 与 `paths` 都以「配置文件所在目录」为基准
// 取相对路径，少一层就少一处换算。`.tmp/` 已在 audit 忽略集里，不会污染其它门禁的扫描面。
const GEN_CFG = join(ROOT, ".tmp", "typecheck-unc-tsconfig.json");
const SRC_EXT_RE = /\.(?:d\.ts|ts|tsx)$/;

/** A 档判据：枚举到 0 个输入 ⇒ 失败。纯函数，selftest 直接喂空数组验它真的会红。 */
export function assertNonEmptyInputs(files) {
  if (files.length === 0) {
    return "枚举到 0 个 .ts/.tsx/.d.ts 输入 —— 这正是 UNC 上 `include: [\"src\"]` 的静默空跑形态；"
      + "「0 输入 + 退 0」的门等于没有门，绝不按「没什么可查」放行。";
  }
  return null;
}

/** B 档判据：tsc 退化打印帮助（cmd.exe 拒绝 UNC 后 `.cmd` 包装器的既有形态）⇒ 判失败。纯函数。 */
export function looksLikeTscHelpBanner(output) {
  const text = String(output ?? "");
  if (/tsc: The TypeScript Compiler/.test(text)) { return true; }
  // 第二形态：没有版本行、但整段是 usage + OPTIONS 列举（参数没落到项目上）
  return /Provide the root directory/.test(text) && /--project, -p/.test(text);
}

/**
 * C 档判据：生成的临时配置必须「只有逐条 files、没有目录 glob」，且条数等于枚举读数。
 * 纯函数（输入是将要落盘的 JSON 对象），防的是「改着改着又把 include 加回去」⇒ 又空跑。
 */
export function checkGeneratedConfig(cfg, enumeratedCount) {
  const problems = [];
  for (const k of ["include", "exclude", "references"]) {
    if (cfg[k] !== undefined) { problems.push(`生成配置残留 "${k}"（目录 glob / 工程引用不参与 -p --noEmit，留着它只会让人误判覆盖面）`); }
  }
  if (!Array.isArray(cfg.files)) { problems.push("生成配置缺 files 数组"); }
  else if (cfg.files.length !== enumeratedCount) { problems.push(`files 条数 ${cfg.files.length} ≠ 枚举读数 ${enumeratedCount}`); }
  else if (cfg.files.length === 0) { problems.push("files 为空数组"); }
  else if (cfg.files.some((f) => /[*?]/.test(f))) { problems.push("files 里出现 glob 字符（必须逐条真实路径）"); }
  if (cfg.compilerOptions?.noEmit !== true) { problems.push("compilerOptions.noEmit 必须为 true（与 `tsc --noEmit` 同口径）"); }
  return problems;
}

/** 遍历 src/**，逐条收 .ts/.tsx/.d.ts。目录不可读 ⇒ 抛错（不静默少收）。 */
function enumerateSrc(srcDir = SRC_DIR_OVERRIDE ?? SRC_DIR) {
  const files = [];
  (function walk(dir) {
    let entries;
    try {
      entries = readdirSync(dir, { withFileTypes: true });
    } catch (e) {
      throw new Error(`无法枚举目录 ${dir}：${e.message}`);
    }
    for (const e of entries) {
      const full = join(dir, e.name);
      if (e.isDirectory()) { walk(full); }
      else if (e.isFile() && SRC_EXT_RE.test(e.name)) { files.push(full); }
    }
  })(srcDir);
  return files.sort();
}

/** 读仓内 tsconfig.json（剥块注释；本仓那份的注释是块注释形态）。 */
function loadRepoTsconfig(path = TSCONFIG) {
  const raw = readFileSync(path, "utf8").replace(/\/\*[\s\S]*?\*\//g, "");
  return JSON.parse(raw);
}

/** 组装「逐条 files」配置：只继承 compilerOptions（原样），paths 按新目录重取相对前缀。
 *  `include` / `exclude` / `references` 一律不带 —— 前两者正是空跑的真凶，后者在 `-p`（非 `--build`）
 *  模式下本就不参与，留着只会让人误判覆盖面。 */
function buildConfig(repo, srcFiles, configDir) {
  const compilerOptions = { ...repo.compilerOptions };
  if (compilerOptions.paths) {
    const rebased = {};
    for (const [alias, targets] of Object.entries(compilerOptions.paths)) {
      rebased[alias] = targets.map((t) => toFwd(relative(configDir, resolve(ROOT, t))));
    }
    compilerOptions.paths = rebased;
  }
  const cfg = { compilerOptions, files: srcFiles.map((f) => toFwd(relative(configDir, f))) };
  if (repo.compilerOptions === undefined) { throw new Error("tsconfig.json 缺 compilerOptions —— 无口径可继承"); }
  return cfg;
}

const toFwd = (p) => p.split(sep).join("/");

/** 调 tsc：`node node_modules/typescript/bin/tsc -p <cfg> --noEmit`（直调 bin，绕开 .cmd 包装器）。 */
function runTsc(configPath, extraArgs) {
  if (!existsSync(TSC_BIN)) { return { missing: true, code: 1, stdout: "", stderr: `找不到 TypeScript 编译器：${TSC_BIN}` }; }
  const r = spawnSync(process.execPath, [TSC_BIN, "-p", configPath, "--noEmit", ...extraArgs], {
    encoding: "utf8",
    cwd: ROOT,
    maxBuffer: 256 * 1024 * 1024,
  });
  if (r.error) { return { missing: false, code: 1, stdout: "", stderr: String(r.error.message) }; }
  return { missing: false, code: r.status ?? 1, stdout: r.stdout ?? "", stderr: r.stderr ?? "" };
}

/** 正式跑。返回退出码（不 process.exit，便于 --list 与 selftest 复用）。 */
function runGate() {
  if (!existsSync(TSCONFIG)) { console.error(`FAIL: 缺 ${rel(TSCONFIG)} —— 无口径可继承，跳过 ≠ 通过。`); return 1; }
  const t0 = Date.now();
  let srcFiles;
  try {
    srcFiles = enumerateSrc();
  } catch (e) {
    console.error(`FAIL: src 枚举异常 —— ${e.message}`);
    return 1;
  }
  const empty = assertNonEmptyInputs(srcFiles);
  if (empty) { console.error(`FAIL（A 档 · 0 输入即红）：${empty}`); return 1; }

  let repo;
  try {
    repo = loadRepoTsconfig();
  } catch (e) {
    console.error(`FAIL: ${rel(TSCONFIG)} 解析失败 —— ${e.message}`);
    return 1;
  }
  mkdirSync(dirname(GEN_CFG), { recursive: true });
  const cfg = buildConfig(repo, srcFiles, dirname(GEN_CFG));
  const cfgProblems = checkGeneratedConfig(cfg, srcFiles.length);
  if (cfgProblems.length) { console.error(`FAIL（C 档 · 生成配置自检）：\n  - ${cfgProblems.join("\n  - ")}`); return 1; }
  writeFileSync(GEN_CFG, `${JSON.stringify(cfg, null, 2)}\n`, "utf8");

  if (LIST_ONLY) {
    console.log(`枚举输入 ${srcFiles.length} 个（src/**，.ts/.tsx/.d.ts，逐条进 files，不走目录 glob）`);
    console.log(`生成配置：${rel(GEN_CFG)}（未调 tsc —— --list 只报读数，不构成类型检查通过的证据）`);
    return 0;
  }

  process.stdout.write(`tsc -p ${rel(GEN_CFG)} --noEmit … `);
  const r = runTsc(GEN_CFG, []);
  const secs = ((Date.now() - t0) / 1000).toFixed(1);
  console.log(`exit=${r.code}（耗时 ${secs}s）`);
  const out = `${r.stdout}${r.stderr}`;
  const diagCount = (out.match(/error TS\d+/g) ?? []).length;

  // B 档：帮助横幅退化（退 0 但打印的是 usage ⇒ 什么都没查）
  if (looksLikeTscHelpBanner(out)) {
    console.error("FAIL（B 档 · 退化打印 tsc 帮助）：输出命中 usage 横幅 ⇒ 参数没落到项目上，本次不算检查通过。");
    console.error(out.split("\n").slice(0, 20).join("\n"));
    return 1;
  }
  if (r.missing) { console.error(`FAIL：${r.stderr}`); return 1; }

  if (out.trim()) {
    console.log(`\n── tsc 原始输出（诊断逐条印出，不折叠、不隐藏）──`);
    console.log(out.replace(/\s+$/, ""));
  }
  console.log(`\n=== UNC 安全 typecheck ===`);
  console.log(`输入文件 ${srcFiles.length} 个（src/** 逐条枚举）· tsc 诊断 ${diagCount} 条 · exit=${r.code} · 耗时 ${secs}s`);
  if (r.code !== 0) {
    console.error(`FAIL: tsc 退出码 ${r.code}（诊断 ${diagCount} 条）—— 退出码原样透传，不做兜底。`);
  }
  return r.code;
}

const rel = (p) => relative(ROOT, p).split(sep).join("/");

// ── selftest：四类正负对照（其中两条真跑 tsc，证明「不是空跑」而非「声称不是空跑」）──
function runSelftest() {
  let fail = 0;
  const check = (cond, msg) => {
    if (!cond) { console.error(`SELFTEST FAIL：${msg}`); fail++; }
  };
  const tscReady = existsSync(TSC_BIN);
  if (!tscReady) { console.error(`SELFTEST FAIL：找不到 ${rel(TSC_BIN)}（工具链缺失 ⇒ 自检无法证明本门能抓到错误，按失败处理，不静默跳过）`); return 1; }

  // 负控①（A 档核心）：枚举 0 输入必须被判失败 —— 这就是 UNC 上 include:["src"] 空跑的形态
  check(assertNonEmptyInputs([]) !== null, "负控①：枚举到 0 个文件必须返回失败（不得按「没什么可查」放行）");
  check(assertNonEmptyInputs(["a.ts"]) === null, "正控①：枚举到 1 个文件不得误判");

  // 负控②（B 档）：帮助横幅形态必须被识别
  const help = "tsc: The TypeScript Compiler - Version 7.0.2\n\nCOMMON COMMANDS\n  tsc\n  Compiles the input file...\nOPTIONS\n  --project, -p\n";
  check(looksLikeTscHelpBanner(help), "负控②：退 0 + 打印 usage 帮助必须判失败（cmd.exe 拒绝 UNC 后的退化形态）");
  check(!looksLikeTscHelpBanner("src/x.ts(1,7): error TS2322: Type 'string' is not assignable to type 'number'."), "正控②：普通诊断输出不得误判成帮助");

  // 负控③（C 档）：生成配置里若混回 include / files 与枚举数不符 ⇒ 必须拒绝落盘
  check(checkGeneratedConfig({ compilerOptions: { noEmit: true }, include: ["src"] }, 1).length >= 2, "负控③a：残留 include 且缺 files 必须报两处问题");
  check(checkGeneratedConfig({ compilerOptions: { noEmit: true }, files: ["a.ts", "b.ts"] }, 1).length === 1, "负控③b：files 条数 ≠ 枚举读数必须报");
  check(checkGeneratedConfig({ compilerOptions: { noEmit: true }, files: ["src/**"] }, 1).length >= 1, "负控③c：files 里出现 glob 字符必须报");
  check(checkGeneratedConfig({ compilerOptions: {}, files: ["a.ts"] }, 1).length === 1, "负控③d：noEmit 丢了（会写产物）必须报");
  check(checkGeneratedConfig({ compilerOptions: { noEmit: true }, files: ["a.ts"] }, 1).length === 0, "正控③：逐条 files + noEmit 的干净配置不得误判");

  // 正控④：真实 src 的枚举读数非零（本门唯一存在的理由就是它能在 UNC 上数到东西）
  let realFiles = [];
  try { realFiles = enumerateSrc(); } catch (e) { check(false, `负控④：真实 src 枚举抛错 —— ${e.message}`); }
  check(realFiles.length > 0, `正控④：真实 src 必须枚举到 > 0 个文件（实得 ${realFiles.length}）`);
  check(realFiles.length >= 1000, `正控④b：真实 src 枚举面不得莫名变小（< 1000 ⇒ 多半是遍历被剪错；实得 ${realFiles.length}）`);
  const cfgReal = buildConfig(loadRepoTsconfig(), realFiles, dirname(GEN_CFG));
  check(checkGeneratedConfig(cfgReal, realFiles.length).length === 0, "正控④c：真实读数生成的配置必须过 C 档自检");
  check(cfgReal.compilerOptions.paths["@/*"].length === 1 && cfgReal.compilerOptions.paths["@/*"][0].includes("src"), "正控④d：paths 别名必须重取到 src 的相对前缀");

  // 负控⑦（CLI 级，不只验纯函数）：把枚举面指向空目录 ⇒ **本进程入口必须非零退出**并打印 A 档措辞。
  // 为什么要单独起子进程：纯函数断言只证明「判据写对了」，不证明「判据接在了退出码上」——
  // 本仓反复出现的形态恰是「门看着在、一行没跑」，所以这条走真实命令行。
  const emptyDir = join(ROOT, ".tmp", "typecheck-unc-empty");
  rmSync(emptyDir, { recursive: true, force: true });
  mkdirSync(emptyDir, { recursive: true });
  const child = spawnSync(process.execPath, [fileURLToPath(import.meta.url), "--src-dir", emptyDir, "--list"], { encoding: "utf8", cwd: ROOT });
  const childOut = `${child.stdout ?? ""}${child.stderr ?? ""}`;
  check(child.status !== 0, `负控⑦：枚举 0 个文件必须让命令行非零退出（实得 exit=${child.status}，输出 ${JSON.stringify(childOut.slice(0, 200))}）`);
  check(/0 输入即红/.test(childOut), "负控⑦b：枚举 0 个文件必须打印 A 档判据措辞（不能只默默退码）");
  rmSync(emptyDir, { recursive: true, force: true });

  // 负控⑤⑥（端到端）：临时目录里造一处真类型错误 ⇒ tsc 必须非零并含 error TS；
  // 同一夹具去掉错误 ⇒ tsc 必须退 0 且 0 诊断。这两条一起证明本门的退出码真的连着判据，
  // 不是「tsc 在 UNC 上跑不动 ⇒ 恒 0」的第二种空跑。
  const sandbox = join(ROOT, ".tmp", "typecheck-unc-selftest");
  rmSync(sandbox, { recursive: true, force: true });
  mkdirSync(join(sandbox, "src"), { recursive: true });
  const mk = (bad) => {
    writeFileSync(join(sandbox, "src", "a.ts"), bad ? 'export const n: number = "故意的类型错误";\n' : 'export const n: number = 1;\n', "utf8");
    writeFileSync(join(sandbox, "src", "b.ts"), 'import { n } from "./a.js";\nexport const m = n + 1;\n', "utf8");
    const cfg = {
      compilerOptions: { target: "ES2022", module: "ESNext", moduleResolution: "bundler", strict: true, noEmit: true, skipLibCheck: true, types: [] },
      files: ["src/a.ts", "src/b.ts"],
    };
    writeFileSync(join(sandbox, "tsconfig.json"), `${JSON.stringify(cfg, null, 2)}\n`, "utf8");
    return join(sandbox, "tsconfig.json");
  };
  const badCfg = mk(true);
  const bad = runTsc(badCfg, []);
  check(bad.code !== 0, `负控⑤：植入类型错误后 tsc 必须非零退出（实得 exit=${bad.code}，输出 ${JSON.stringify((bad.stdout + bad.stderr).slice(0, 200))}）`);
  check(/error TS\d+/.test(bad.stdout + bad.stderr), "负控⑤b：植入错误的输出必须含 error TS（证明诊断真的被产出，不是空跑）");
  const goodCfg = mk(false);
  const good = runTsc(goodCfg, []);
  check(good.code === 0, `正控⑥：同一夹具去掉错误必须退 0（实得 exit=${good.code}，输出 ${JSON.stringify((good.stdout + good.stderr).slice(0, 200))}）`);
  check(!/error TS\d+/.test(good.stdout + good.stderr), "正控⑥b：干净夹具不得有诊断");

  rmSync(sandbox, { recursive: true, force: true });
  if (fail > 0) { return 1; }
  console.log(`SELFTEST OK：0 输入判红 · 帮助横幅判红 · 生成配置四项判红（include 残留/files 数不符/glob 字符/noEmit 丢失）· 真实 src 枚举 ${realFiles.length} 个非空 · 夹具植入错误被抓（exit≠0 且报 error TS）· 干净夹具退 0`);
  return 0;
}

const entryPath = process.argv[1];
if (entryPath && import.meta.url === pathToFileURL(entryPath).href) { process.exit(SELFTEST ? runSelftest() : runGate()); }
