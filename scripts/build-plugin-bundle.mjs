#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-only
/**
 * 把股票插件 worker 打包成「可投放的插件目录」：`<out>/plugin.json` + `<out>/bin/<worker>`。
 *
 * 为什么要这个脚本（PLAN §74 未做 1）：宿主的 `PluginWorkerDecl.program` 只接受
 * **相对插件安装目录**的路径，而 `cargo build` 的产物在 target 目录 ⇒ 中间缺一步
 * 「把产物摆进插件目录」。本脚本就是那一步，产物落在 git 忽略的 `dist/plugins/`。
 *
 * 用法：
 *   node scripts/build-plugin-bundle.mjs              # 构建（debug）+ 打包 + 打印路径
 *   node scripts/build-plugin-bundle.mjs --release    # 构建 release
 *   node scripts/build-plugin-bundle.mjs --check      # 打包后真起一次 worker 做握手 + 停机回执
 *   node scripts/build-plugin-bundle.mjs --out <dir>  # 自定义输出目录
 *
 * `--check` 不造假：它 spawn 的是**打包出来的那个文件**，按 `axagent-plugin-proto`
 * 的长度前缀 JSON 帧发 `describe` / `shutdown`，并核对 integrity 的 sha256 与盘上一致。
 */
import { spawn, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import fs from "node:fs";
import path from "node:path";

const repoRoot = path.resolve(import.meta.dirname, "..");
const workspaceRoot = path.join(repoRoot, "axagent-plugins");
const argv = process.argv.slice(2);
const release = argv.includes("--release");
const check = argv.includes("--check");
const outFlag = argv.indexOf("--out");
const outRoot = path.resolve(
  outFlag >= 0 && argv[outFlag + 1] ? argv[outFlag + 1] : path.join(repoRoot, "dist", "plugins", "stock"),
);

// 前端贡献的源目录：`web/` 下的文件按**相对路径原样**进产物，`pages[].entry` 即这些相对路径。
// 目前只有一页通道探针（B8-0）；真实股票 UI 随 B8-1..B8-5 才落进这里。
const webRoot = path.join(workspaceRoot, "crates", "plugin-stock", "web");
// 与宿主 `PANEL_ASSET_ALLOWED_EXTS`（axagent-plugins/crates/plugins/src/manager.rs）同口径：
// 清单校验会拒掉白名单外的扩展名，产物自检必须在**打包期**就报出来，而不是等安装时才失败。
const PAGE_ASSET_ALLOWED_EXTS = new Set(["html", "htm", "js", "css"]);
const PAGE_ENTRY = "pages/index.html";
// UI 贡献声明（任务 #149 第 3 步）：`ui.json` 与 `plugin.json` **同级**落在安装根，
// 任务源的 `detailSchema` 只能引用它里面已声明的 key —— 宿主 `build_manifest_task_sources`
// 安装期硬校验，「声明了却不在 ui.json 里」等于装不上。故它是清单的一部分，必须由本脚本
// 一起产出（前端资产目录 `web/` 只收 html/js/css，扩展名白名单会拒掉 .json）。
const UI_SCHEMA_SOURCE = path.join(workspaceRoot, "crates", "plugin-stock", "ui.json");

function fail(message) {
  console.error(`[plugin-bundle] ${message}`);
  process.exit(1);
}

function run(command, args, options = {}) {
  const result = spawnSync(command, args, { stdio: "inherit", windowsHide: true, ...options });
  if (result.error) fail(`启动 ${command} 失败：${result.error.message}`);
  if (result.status !== 0) fail(`${command} ${args.join(" ")} 退出码 ${result.status}`);
  return result;
}

// 1) 构建 worker（宿主与打包用同一份源码，不另存产物）
run("cargo", ["build", "-p", "axagent-plugin-stock", ...(release ? ["--release"] : [])], {
  cwd: workspaceRoot,
});

// 2) 从 cargo 自己的视角取 target 目录与版本号 —— 不猜 `target/` 的位置
//    （本机把 CARGO_TARGET_DIR 指到了 `~/.cargo/target-axagent`）。
const metadataRaw = spawnSync("cargo", ["metadata", "--format-version", "1", "--no-deps"], {
  cwd: workspaceRoot,
  encoding: "utf8",
  windowsHide: true,
});
if (metadataRaw.status !== 0) {
  fail(`cargo metadata 退出码 ${metadataRaw.status}：${metadataRaw.stderr.trim()}`);
}
const metadata = JSON.parse(metadataRaw.stdout);
const workerPackage = metadata.packages.find((p) => p.name === "axagent-plugin-stock");
if (!workerPackage) fail("cargo metadata 里找不到 axagent-plugin-stock");

const targetDir = metadata.target_directory;
const profileDir = release ? "release" : "debug";
const binaryName = process.platform === "win32" ? "axagent-plugin-stock.exe" : "axagent-plugin-stock";
const builtBinary = path.join(targetDir, profileDir, binaryName);
if (!fs.existsSync(builtBinary)) fail(`构建产物不存在：${builtBinary}`);

// 3) 摆放插件目录：worker 路径**必须**相对安装目录（宿主 resolve_worker_program 会拒绝
//    绝对路径与 `..` 逃逸），所以这里用 `bin/<name>` 而不是复制到根。
const binDir = path.join(outRoot, "bin");
fs.mkdirSync(binDir, { recursive: true });
const stagedBinary = path.join(binDir, binaryName);
fs.copyFileSync(builtBinary, stagedBinary);

const sha256 = createHash("sha256").update(fs.readFileSync(stagedBinary)).digest("hex");

// 3.5) 前端资产：源目录整树拷进安装根。
//      先按源目录的**顶层子目录**清一遍同名目录 —— 否则源里删掉的页面会作为孤儿留在产物里，
//      宿主按目录哈希钉锚也看不出「这份资产没有任何声明指向它」。
if (!fs.existsSync(webRoot)) fail(`前端源目录不存在：${webRoot}`);
for (const top of fs.readdirSync(webRoot, { withFileTypes: true })) {
  if (top.isDirectory()) fs.rmSync(path.join(outRoot, top.name), { recursive: true, force: true });
}
const webFiles = listFilesRecursive(webRoot);
if (webFiles.length === 0) fail(`前端源目录为空：${webRoot}`);
for (const file of webFiles) {
  const rel = path.relative(webRoot, file).split(path.sep).join("/");
  const ext = path.extname(rel).slice(1).toLowerCase();
  if (!PAGE_ASSET_ALLOWED_EXTS.has(ext)) {
    fail(`前端资产扩展名不在白名单（${[...PAGE_ASSET_ALLOWED_EXTS].join(", ")}）：${rel}`);
  }
  const target = path.join(outRoot, ...rel.split("/"));
  fs.mkdirSync(path.dirname(target), { recursive: true });
  fs.copyFileSync(file, target);
}
if (!fs.existsSync(path.join(outRoot, ...PAGE_ENTRY.split("/")))) {
  fail(`清单要声明的页面资产不在源目录里：${path.join(webRoot, ...PAGE_ENTRY.split("/"))}`);
}

// 3.6) UI 声明：原样拷到安装根（不做任何改写）。key 集合在这里先解析一次 ——
//      `--check` 与清单里的 `detailSchema` 都要它，解析失败必须**在打包期**报错，
//      而不是等宿主安装时才说「ui.json 不合法」。
if (!fs.existsSync(UI_SCHEMA_SOURCE)) fail(`UI 声明文件不存在：${UI_SCHEMA_SOURCE}`);
const uiSchemaRaw = fs.readFileSync(UI_SCHEMA_SOURCE, "utf8");
let uiSchemaKeys;
try {
  uiSchemaKeys = new Set((JSON.parse(uiSchemaRaw).schemas ?? []).map((s) => s.key));
} catch (error) {
  fail(`UI 声明不是合法 JSON（${UI_SCHEMA_SOURCE}）：${error.message}`);
}
fs.copyFileSync(UI_SCHEMA_SOURCE, path.join(outRoot, "ui.json"));

const manifest = {
  name: "stock",
  version: workerPackage.version,
  description:
    "股票域功能包：行情工具 get_stock_quote + 5 条命名命令（行情/意图解析/证据权重/在跑执行/分析记录）+ 1 个前端页面与 2 个任务中心任务源",
  // worker 由宿主 spawn ⇒ 必须声明 `subprocess_execution`，否则 enable 时
  // `check_subprocess_permission` 直接拒绝（沙箱是从 manifest.permissions 推出来的，
  // 不是 `PluginWorkerConfig::new` 的 permissive 默认值）。
  permissions: ["subprocess_execution"],
  mcp_servers: [],
  skills: [],
  agents: [],
  // 默认启用：前端 `StockSearchBar` 的 NL 意图解析已改走本插件的命名命令，
  // 宿主同名命令（B4-j 后删除）不再兜底 ⇒ 内置包不默认启用就等于该功能静默失效。
  defaultEnabled: true,
  worker: { program: path.posix.join("bin", binaryName) },
  // 前端页面声明：`entry` 相对安装目录（扩展名走宿主资产白名单），宿主
  // `PluginManager::resolve_page_asset` 把它解析成安装根内的绝对路径；
  // 前端只传 `(pluginId, pageId)`，不得自己拼路径。
  pages: [{ id: "stock-home", title: "股票", entry: PAGE_ENTRY, loading: "bundle" }],
  // `command` 用 `worker://<名>` 占位：worker 通道不读它（按 `name` 发帧），但若意外
  // 退回 spawn 通道，这个不存在的程序名会当场失败而不是静默跑别的东西。
  commands: [
    {
      name: "stock_get_quote",
      description: "获取 A 股个股实时行情（经宿主 market.quote 取数）",
      command: "worker://stock_get_quote",
      inputSchema: {
        type: "object",
        properties: { stock_code: { type: "string", description: "6 位股票代码，如 600519" } },
        required: ["stock_code"],
      },
    },
    {
      name: "parse_analysis_intent",
      description: "解析自然语言分析意图（纯函数，跑 axagent-stock-core 的权威实现）",
      command: "worker://parse_analysis_intent",
      inputSchema: {
        type: "object",
        properties: { input: { type: "string", description: "如「调研茅台短线」" } },
        required: ["input"],
      },
    },
    {
      // B9-1：宿主同名 Tauri 命令在双声明期仍保留；前端已切到本条，
      // 删除宿主实现与计数收口走 B9-2（避免同批做「改路由 + 重生成注册表」两件不可回滚的事）。
      name: "compute_evidence_weights",
      description: "按市场环境/投资周期/分析师输入计算证据质量权重与共识（纯函数，跑 analysis-engine 权威实现）",
      command: "worker://compute_evidence_weights",
      inputSchema: {
        type: "object",
        properties: {
          marketRegime: {
            type: "object",
            properties: {
              regime: { type: "string", description: "bull | bear | sideways | volatile" },
              confidence: { type: "number", description: "0-1" },
              volatility: { type: "string", description: "high | low | normal" },
              description: { type: "string" },
              volatilityPct: { type: "number" },
              consecutiveUp: { type: "integer" },
              consecutiveDown: { type: "integer" },
            },
            required: ["regime", "confidence", "volatility", "description"],
          },
          timeHorizon: { type: "string", description: "ultra_short | short | mid | long" },
          analysts: {
            type: "array",
            items: {
              type: "object",
              properties: {
                analystId: { type: "string" },
                reportText: { type: "string" },
                stance: { type: "string" },
                bullScore: { type: "number" },
                bearScore: { type: "number" },
                positionPct: { type: "number" },
                untrusted: { type: "boolean" },
              },
              required: ["analystId"],
            },
          },
          historicalWeights: { type: "object", additionalProperties: { type: "number" } },
        },
        required: ["marketRegime", "timeHorizon", "analysts"],
      },
    },
    {
      // B8-1b 建通道、#150 换取数口：行数据全部来自宿主 `workflow.run` 的 `list_recent`
      // （在跑 + 已结束），插件只做状态词表翻译（引擎六态 → 中心七态）与形状收敛。
      name: "list_recent_workflow_runs",
      description: "列出本插件近期工作流执行（在跑与已结束，经宿主 workflow.run 接缝，返回任务中心行数组）",
      command: "worker://list_recent_workflow_runs",
      inputSchema: {
        type: "object",
        properties: {
          limit: { type: "integer", description: "取数上限，宿主夹在 1..=200，缺省 50" },
          workflowId: { type: "string", description: "只看该 workflow 的执行；缺省 = 全部" },
        },
      },
    },
    {
      // #149 第 1 步建通道、第 3 步才进清单：行数据来自宿主 `analysis.record` 的 `list`
      // （宿主发起的单股分析记录也在内），插件只做状态词表翻译与形状收敛。
      // `name` 必须与 worker 的 `COMMAND_ANALYSIS_RECORDS` 逐字一致（`list_stock_analysis_records`）。
      name: "list_stock_analysis_records",
      description: "列出单股分析记录（经宿主 analysis.record 接缝，只交根记录，返回任务中心行数组）",
      command: "worker://list_stock_analysis_records",
      inputSchema: {
        type: "object",
        properties: {
          limit: { type: "integer", description: "取数上限，宿主夹在 1..=200，缺省 50" },
          offset: { type: "integer", description: "分页偏移，缺省 0" },
        },
      },
    },
  ],
  // 任务中心任务源声明（PLAN §97.4）：只给「源标识 + 显示名 + 取数命令」，
  // 不给渲染。`command` 必须是上面 commands[] 里已声明的名字 —— 宿主清单校验会硬拦，
  // 这里 `--check` 再自证一次，免得产物装不上才发现。
  taskSources: [
    { id: "stock-workflow-runs", title: "股票工作流执行", command: "list_recent_workflow_runs", color: "volcano" },
    {
      // 与宿主原 `stockAnalysis.adapter.ts` **同 id**（#149 第 3 步删掉宿主那份后不再重名）：
      // 插件源的注册是「宿主优先、重名忽略」，同 id 才谈得上「插件源顶替宿主源」。
      id: "stock-analysis",
      title: "单股分析",
      command: "list_stock_analysis_records",
      color: "gold",
      // 详情槽指向本插件 `ui.json` 的 key（任务 #149 第 2 步的通道）：
      // 只写 key，行 id `plugin.<pluginId>.<key>` 由宿主拼（插件拼得出来就能引用别人的 schema）。
      detailSchema: "stock-analysis-detail",
    },
  ],
  integrity: { algorithm: "sha256", hash: sha256 },
};
const manifestPath = path.join(outRoot, "plugin.json");
fs.writeFileSync(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`);

console.log(`[plugin-bundle] 产物目录：${outRoot}`);
console.log(`[plugin-bundle] worker   ：${stagedBinary}`);
console.log(`[plugin-bundle] 前端资产 ：${webFiles.length} 个文件（页面入口 ${PAGE_ENTRY}）`);
console.log(`[plugin-bundle] UI 声明  ：ui.json（${uiSchemaKeys.size} 个 schema key）`);
console.log(`[plugin-bundle] 清单     ：${manifestPath}`);
console.log(`[plugin-bundle] sha256   ：${sha256}`);

if (check) {
  await checkStagedBundle(manifest, outRoot);
}

/** 起一次成品 worker：describe 必须回合法声明，shutdown 必须回执并以 0 退出。 */
async function checkStagedBundle(declared, root) {
  checkDeclaredPages(declared, root);
  checkDeclaredTaskSources(declared, root);

  const program = path.join(root, declared.worker.program);
  if (!fs.existsSync(program)) fail(`--check：清单声明的 worker 不存在：${program}`);
  const onDisk = createHash("sha256").update(fs.readFileSync(program)).digest("hex");
  if (onDisk !== declared.integrity.hash) {
    fail(`--check：integrity 不匹配（清单 ${declared.integrity.hash} / 盘上 ${onDisk}）`);
  }

  const child = spawn(program, [], { stdio: ["pipe", "pipe", "inherit"], windowsHide: true });
  // 任何提前 `fail()` 都要带走子进程：`fail` 是 `process.exit(1)`，不会回收它。
  process.on("exit", () => child.kill());
  const reader = frameReader(child.stdout);

  const describeId = "bundle-check-describe";
  writeFrame(child.stdin, { op: "describe", args: { proto_version: 1 }, request_id: describeId });
  const describe = await reader.next();
  if (describe.kind !== "success") fail(`--check：describe 失败：${JSON.stringify(describe)}`);
  if (describe.request_id !== describeId) fail(`--check：describe 未回填 request_id：${JSON.stringify(describe)}`);
  if (describe.value?.proto_version !== 1) {
    fail(`--check：声明协议版本不符：${JSON.stringify(describe.value)}`);
  }

  writeFrame(child.stdin, { op: "shutdown", args: {} });
  const shutdown = await reader.next();
  if (shutdown.kind !== "success") fail(`--check：shutdown 回执失败：${JSON.stringify(shutdown)}`);

  // `shutdown` 在协议里只是**回执**，真正的停机信号是 stdin EOF（worker 的 `serve` 循环
  // 靠对端关管道结束）。少这一步，自检会永远挂在 `child.on("exit")` 上 —— 2026-11-04 实测
  // （worker 收到回执后仍在等下一帧，脚本却在等它退出，双方互等）。
  child.stdin.end();
  let timer;
  const exitCode = await Promise.race([
    new Promise((resolve) => child.on("exit", resolve)),
    new Promise((resolve) => {
      timer = setTimeout(() => resolve(null), 10000);
    }),
  ]);
  clearTimeout(timer);
  if (exitCode === null) fail("--check：stdin EOF 后 10 s 内未退出");
  if (exitCode !== 0) fail(`--check：worker 退出码 ${exitCode}（应为 0）`);

  console.log("[plugin-bundle] --check ✅ describe 握手 + shutdown 回执 + 退出码 0 + sha256 一致");
}

/**
 * 任务源声明的自证（B8-1b）：与宿主 `build_manifest_task_sources` 同判据，
 * 目的是让「装不上才发现」提前到打包期。宿主侧是权威，这里只是提前一遍 ——
 * 判据漂了会以 e2e 安装测试的形式暴露，不靠本函数兜底。
 */
function checkDeclaredTaskSources(declared, root) {
  const sources = declared.taskSources ?? [];
  const declaredCommands = new Set((declared.commands ?? []).map((c) => c.name));
  // `detailSchema` 的归属：只能是**本包 `ui.json`** 的 key（#149 第 3 步）。
  // 与宿主 `build_manifest_task_sources` 同判据：拼别人的 key / 写全行 id 都不许，
  // 宿主那边还会再按库里现存 schema 复核一次。
  const uiSchemaPath = path.join(root, "ui.json");
  let uiKeys = new Set();
  if (fs.existsSync(uiSchemaPath)) {
    try {
      uiKeys = new Set((JSON.parse(fs.readFileSync(uiSchemaPath, "utf8")).schemas ?? []).map((s) => s.key));
    } catch (error) {
      fail(`--check：ui.json 不是合法 JSON：${error.message}`);
    }
  }
  const seenIds = new Set();
  for (const source of sources) {
    if (!/^[a-z0-9][a-z0-9-]{1,63}$/.test(source.id ?? "")) {
      fail(`--check：任务源 id 形状非法（小写字母数字与 -）：${JSON.stringify(source.id)}`);
    }
    if (seenIds.has(source.id)) fail(`--check：任务源 id 在本包内重复：${source.id}`);
    seenIds.add(source.id);
    if (!source.title || !source.title.trim()) fail(`--check：任务源 ${source.id} 没有 title`);
    if (!declaredCommands.has(source.command)) {
      fail(`--check：任务源 ${source.id} 的 command「${source.command}」不在 commands[] 里`);
    }
    if (source.detailSchema !== undefined && source.detailSchema !== null && `${source.detailSchema}`.trim() !== "") {
      if (!/^[a-z0-9][a-z0-9-]{1,63}$/.test(source.detailSchema)) {
        fail(`--check：任务源 ${source.id} 的 detailSchema 必须是 key（不是行 id）：${source.detailSchema}`);
      }
      if (!uiKeys.has(source.detailSchema)) {
        fail(`--check：任务源 ${source.id} 的 detailSchema「${source.detailSchema}」不在 ui.json 里`);
      }
    }
  }
  console.log(`[plugin-bundle] --check ✅ ${sources.length} 个任务源声明自洽（command ∈ commands[]，detailSchema ∈ ui.json）`);
}

/**
 * 产物自检的前端一侧：`pages[]` 必须真能指向安装根内的既有资产。
 *
 * 校验口径照宿主 `PluginManager::resolve_page_asset` 的载入期防线，但**不替代**它 ——
 * 宿主那边还有 registry 哈希锚与 canonicalize（符号链接逃逸）。这里只保证
 * 「打包脚本写出的清单与盘上文件一致」，把那类缺陷挡在装机之前。
 */
function checkDeclaredPages(declared, root) {
  const pages = declared.pages ?? [];
  if (pages.length === 0) fail("--check：清单没有 pages[]，前端 bundle 通道无从验证");
  let bundlePages = 0;
  for (const page of pages) {
    // `schema` 页的 entry 是动态 UI Schema 的 id，没有文件资产可查（宿主同样跳过形状校验）。
    if (page.loading !== "bundle") continue;
    bundlePages += 1;
    if (!page.entry) fail(`--check：页面 ${page.id} 没有 entry`);
    if (path.isAbsolute(page.entry) || page.entry.startsWith("/") || page.entry.startsWith("\\")) {
      fail(`--check：页面 ${page.id} 的 entry 必须是相对安装目录的路径：${page.entry}`);
    }
    if (page.entry.split(/[\\/]/).includes("..")) {
      fail(`--check：页面 ${page.id} 的 entry 不得逃出安装目录：${page.entry}`);
    }
    const ext = path.extname(page.entry).slice(1).toLowerCase();
    if (!PAGE_ASSET_ALLOWED_EXTS.has(ext)) {
      fail(`--check：页面 ${page.id} 的 entry 扩展名不在白名单：${page.entry}`);
    }
    const absolute = path.resolve(root, page.entry);
    const relative = path.relative(root, absolute);
    if (relative.startsWith("..") || path.isAbsolute(relative)) {
      fail(`--check：页面 ${page.id} 的资产落在安装根之外：${absolute}`);
    }
    if (!fs.existsSync(absolute) || !fs.statSync(absolute).isFile()) {
      fail(`--check：清单声明的页面资产不存在：${absolute}`);
    }
  }
  if (bundlePages === 0) fail("--check：pages[] 里没有任何 bundle 页，通道未被验证");
  console.log(`[plugin-bundle] --check ✅ ${bundlePages} 个 bundle 页的 entry 均存在且在白名单内`);
}

/** 递归列出目录下的所有文件（保持不排序；调用方只按相对路径逐个处理）。 */
function listFilesRecursive(dir) {
  const files = [];
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) files.push(...listFilesRecursive(full));
    else if (entry.isFile()) files.push(full);
  }
  return files;
}

function writeFrame(stream, value) {
  const payload = Buffer.from(JSON.stringify(value), "utf8");
  const header = Buffer.alloc(4);
  header.writeUInt32BE(payload.length, 0);
  stream.write(Buffer.concat([header, payload]));
}

/** 长度前缀 JSON 帧读取器：返回 `next()`，每次给出一帧反序列化后的对象。 */
function frameReader(stream) {
  let buffered = Buffer.alloc(0);
  /** 等待中的读取者：`{ resolve, reject, timer }`。 */
  const waiting = [];
  const pending = [];
  stream.on("data", (chunk) => {
    buffered = Buffer.concat([buffered, chunk]);
    for (;;) {
      if (buffered.length < 4) break;
      const length = buffered.readUInt32BE(0);
      if (buffered.length < 4 + length) break;
      const payload = buffered.subarray(4, 4 + length);
      buffered = buffered.subarray(4 + length);
      const parsed = JSON.parse(payload.toString("utf8"));
      const waiter = waiting.shift();
      if (waiter) {
        clearTimeout(waiter.timer);
        waiter.resolve(parsed);
      } else {
        pending.push(parsed);
      }
    }
  });
  stream.on("error", (error) => {
    for (const waiter of waiting.splice(0)) {
      clearTimeout(waiter.timer);
      waiter.reject(error);
    }
  });
  return {
    next() {
      if (pending.length > 0) return Promise.resolve(pending.shift());
      return new Promise((resolve, reject) => {
        const waiter = {
          resolve,
          reject,
          timer: setTimeout(() => {
            const index = waiting.indexOf(waiter);
            if (index >= 0) waiting.splice(index, 1);
            reject(new Error("等回帧超时（5 s）"));
          }, 5000),
        };
        waiting.push(waiter);
      });
    },
  };
}
