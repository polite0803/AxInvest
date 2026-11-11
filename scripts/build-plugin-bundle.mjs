#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-only
/**
 * 把一个插件 worker 打包成「可投放的插件目录」：`<out>/plugin.json` + `<out>/bin/<worker>`。
 *
 * 为什么要这个脚本（PLAN §74 未做 1）：宿主的 `PluginWorkerDecl.program` 只接受
 * **相对插件安装目录**的路径，而 `cargo build` 的产物在 target 目录 ⇒ 中间缺一步
 * 「把产物摆进插件目录」。本脚本就是那一步，产物落在 git 忽略的 `dist/plugins/<插件>/`。
 *
 * 用法：
 *   node scripts/build-plugin-bundle.mjs                 # 构建（debug）+ 打包 + 打印路径（缺省 stock）
 *   node scripts/build-plugin-bundle.mjs --plugin opc     # 打包 opc 插件（#36 骨架）
 *   node scripts/build-plugin-bundle.mjs --release       # 构建 release
 *   node scripts/build-plugin-bundle.mjs --check         # 打包后真起一次 worker 做握手 + 停机回执
 *   node scripts/build-plugin-bundle.mjs --out <dir>     # 自定义输出目录
 *
 * 参数化口径（任务 #36 第 1 批）：**一个插件 = `PLUGIN_SPECS` 里一条**，条目的字段就是
 * 本脚本对插件的全部假设（cargo 包名、前端源目录、`ui.json` 源、清单构造器）。新增插件只加
 * 一条，不改脚本主体。⚠ **`--plugin stock` 的产物（路径 / 清单字段与键序 / 产物名 / sha256 /
 * 控制台行）必须与参数化之前逐字一致** —— 下游 `stage-plugin-bundles.mjs` 与既有插件 e2e
 * 都按那份形状断言，静默失配不会有任何一处报错。
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
const pluginFlag = argv.indexOf("--plugin");
const pluginName = pluginFlag >= 0 && argv[pluginFlag + 1] ? argv[pluginFlag + 1] : "stock";

/**
 * 插件规格表（键 = `--plugin` 取值）。
 *
 * - `packageName`：cargo 包名，同时决定 `cargo build -p` 与产物文件名（`<packageName>[.exe]`）；
 * - `webDir`：前端贡献源目录（相对 `axagent-plugins/`）；`null` = 无前端贡献 ⇒ 不拷资产、
 *   且清单不得声明 `pages[]`（`--check` 会拦）；
 * - `uiSchema`：`ui.json` 源（相对 `axagent-plugins/`）；`null` = 无详情槽声明；
 * - `manifest(ctx)`：清单构造器，`ctx = { version, binaryName, sha256 }`。**键序即产物键序**
 *   （`JSON.stringify` 保序），改键序等于改产物，下游比对会当漂移。
 */
const PLUGIN_SPECS = {
  stock: {
    packageName: "axagent-plugin-stock",
    // 前端贡献的源目录：`web/` 下的文件按**相对路径原样**进产物，`pages[].entry` 即这些相对路径。
    // 目前只有一页通道探针（B8-0）；真实股票 UI 随 B8-1..B8-5 才落进这里。
    webDir: path.join("crates", "plugin-stock", "web"),
    // UI 贡献声明（任务 #149 第 3 步）：`ui.json` 与 `plugin.json` **同级**落在安装根，
    // 任务源的 `detailSchema` 只能引用它里面已声明的 key —— 宿主 `build_manifest_task_sources`
    // 安装期硬校验，「声明了却不在 ui.json 里」等于装不上。故它是清单的一部分，必须由本脚本
    // 一起产出（前端资产目录 `web/` 只收 html/js/css，扩展名白名单会拒掉 .json）。
    uiSchema: path.join("crates", "plugin-stock", "ui.json"),
    manifest: stockManifest,
  },
  opc: {
    packageName: "axagent-plugin-opc",
    // #36 第 1 批只有 worker 骨架：没有前端页面，也没有 `ui.json` 详情槽 ⇒ 两者皆 null。
    // 第一批真实页面/详情槽接进来时再改这两处（不要为「以后可能用」预留空目录）。
    webDir: null,
    uiSchema: null,
    manifest: opcManifest,
  },
};

const spec = PLUGIN_SPECS[pluginName];
if (!spec) {
  fail(`未知插件 --plugin ${pluginName}（可选：${Object.keys(PLUGIN_SPECS).join(" | ")}）`);
}
const outRoot = path.resolve(
  outFlag >= 0 && argv[outFlag + 1] ? argv[outFlag + 1] : path.join(repoRoot, "dist", "plugins", pluginName),
);
// 与宿主 `PANEL_ASSET_ALLOWED_EXTS`（axagent-plugins/crates/plugins/src/manager.rs）同口径：
// 清单校验会拒掉白名单外的扩展名，产物自检必须在**打包期**就报出来，而不是等安装时才失败。
const PAGE_ASSET_ALLOWED_EXTS = new Set(["html", "htm", "js", "css"]);
const PAGE_ENTRY = "pages/index.html";
const webRoot = spec.webDir ? path.join(workspaceRoot, spec.webDir) : null;
const uiSchemaSource = spec.uiSchema ? path.join(workspaceRoot, spec.uiSchema) : null;

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
run("cargo", ["build", "-p", spec.packageName, ...(release ? ["--release"] : [])], {
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
const workerPackage = metadata.packages.find((p) => p.name === spec.packageName);
if (!workerPackage) fail(`cargo metadata 里找不到 ${spec.packageName}`);

const targetDir = metadata.target_directory;
const profileDir = release ? "release" : "debug";
const binaryName = process.platform === "win32" ? `${spec.packageName}.exe` : spec.packageName;
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
//      `webDir = null` 的插件（#36 骨架期的 opc）没有前端贡献 ⇒ 整段跳过，不建空目录。
let webFileCount = 0;
if (webRoot) {
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
  webFileCount = webFiles.length;
}

// 3.6) UI 声明：原样拷到安装根（不做任何改写）。key 集合在这里先解析一次 ——
//      `--check` 与清单里的 `detailSchema` 都要它，解析失败必须**在打包期**报错，
//      而不是等宿主安装时才说「ui.json 不合法」。无 `ui.json` 的插件整段跳过。
let uiSchemaKeyCount = 0;
if (uiSchemaSource) {
  if (!fs.existsSync(uiSchemaSource)) fail(`UI 声明文件不存在：${uiSchemaSource}`);
  const uiSchemaRaw = fs.readFileSync(uiSchemaSource, "utf8");
  let uiSchemaKeys;
  try {
    uiSchemaKeys = new Set((JSON.parse(uiSchemaRaw).schemas ?? []).map((s) => s.key));
  } catch (error) {
    fail(`UI 声明不是合法 JSON（${uiSchemaSource}）：${error.message}`);
  }
  fs.copyFileSync(uiSchemaSource, path.join(outRoot, "ui.json"));
  uiSchemaKeyCount = uiSchemaKeys.size;
}

// 4) 清单：形状归 `PLUGIN_SPECS[*].manifest`，此处只递随产物变化的三个量。
const manifest = spec.manifest({ version: workerPackage.version, binaryName, sha256 });
const manifestPath = path.join(outRoot, "plugin.json");
fs.writeFileSync(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`);

console.log(`[plugin-bundle] 产物目录：${outRoot}`);
console.log(`[plugin-bundle] worker   ：${stagedBinary}`);
if (webRoot) console.log(`[plugin-bundle] 前端资产 ：${webFileCount} 个文件（页面入口 ${PAGE_ENTRY}）`);
if (uiSchemaSource) console.log(`[plugin-bundle] UI 声明  ：ui.json（${uiSchemaKeyCount} 个 schema key）`);
console.log(`[plugin-bundle] 清单     ：${manifestPath}`);
console.log(`[plugin-bundle] sha256   ：${sha256}`);

if (check) {
  await checkStagedBundle(manifest, outRoot);
}

/** 起一次成品 worker：describe 必须回合法声明，shutdown 必须回执并以 0 退出。 */
async function checkStagedBundle(declared, root) {
  // 页面自检只在**声明了前端源**的插件上跑：无前端贡献的插件（opc 骨架）连 pages[] 都不许有，
  // 否则宿主会去解析一条不存在的资产 —— 那种清单装得上但页面永远打不开。
  if (webRoot) {
    checkDeclaredPages(declared, root);
  } else if ((declared.pages ?? []).length > 0) {
    fail("--check：本插件未声明前端源（webDir=null），清单不得声明 pages[]");
  }
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

/**
 * stock 清单：**字段、键序、取值来源与参数化之前逐字一致**（任务 #36 的硬要求）。
 * 唯一改写是把 `workerPackage.version` 换成 `ctx.version`（同一个值，经 `cargo metadata` 取）。
 */
function stockManifest({ version, binaryName, sha256 }) {
  return {
    name: "stock",
    version,
    description:
      "股票域功能包：行情工具 get_stock_quote + 24 条命名命令（行情/意图解析/证据权重/在跑执行/分析记录/行业成分股/市场主线写入/逐票因子/管道历史与详情/进化计划/DES 仿真/三族 cron 配置 CRUD）+ 1 个前端页面与 2 个任务中心任务源",
    // worker 由宿主 spawn ⇒ 必须声明 `subprocess_execution`，否则 enable 时
    // `check_subprocess_permission` 直接拒绝（沙箱是从 manifest.permissions 推出来的，
    // 不是 `PluginWorkerConfig::new` 的 permissive 默认值）。
    //
    // ⚠ `write` 是 E 批（#240）加的，**两个面都要它**，缺一条即「装得上、跑必失败」：
    // ① 插件出站：`worker.rs::required_permission_for(("stock.analysis", "batch_upsert_mainlines"))`
    //    返回 `Write`，而 `has_permission` 是**精确 contains**（不套 `implies_*`，
    //    `subprocess_execution` 顶不上）⇒ 命令在接缝调用前落 `PLUGIN_PERMISSION_DENIED`；
    // ② 命令入站：该命令 `requiredPermission: "workspace-write"`，`check_command_gate`
    //    要求插件声明 `write | file_system_write`，否则整条命令被拒。
    permissions: ["subprocess_execution", "write"],
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
        // 批 2b：入参键由 `stock_code` 改 camelCase `stockCode`（禁区 13 既存偏离收口）。
        // 接缝面的 snake_case 翻译点在插件 `market_read_route`，不在清单；宿主 LLM 工具面
        // （`stock_analysis_bridge.rs` 的 `stock_get_quote`）仍声明 `stock_code`，两条面各有调用方。
        name: "stock_get_quote",
        description: "获取 A 股个股实时行情（经宿主 market.quote 接缝，只读）",
        command: "worker://stock_get_quote",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: { stockCode: { type: "string", description: "6 位股票代码，如 600519" } },
          required: ["stockCode"],
        },
      },
      {
        // 批 2b：K 线走 `market.quote` 已实现的 `get_klines` op。
        // 不开放 `asOfDate` —— 宿主 `get_stock_kline` 有该入参，但接缝没有，多写一个键就是
        // 承诺一条取不到数的通道；`adjType` 与宿主 `adj` 同档（auto 与缺省都落未复权）。
        name: "stock_get_kline",
        description: "获取 A 股 K 线序列（经宿主 market.quote 接缝，只读）",
        command: "worker://stock_get_kline",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            stockCode: { type: "string", description: "6 位股票代码，如 600519" },
            period: { type: "string", description: "周期，如 daily / weekly / 60min" },
            limit: { type: "integer", minimum: 1, description: "取多少根，宿主缺省 100" },
            adjType: { type: "string", description: "复权档：none / forward / backward；auto 与缺省=未复权" },
          },
          required: ["stockCode", "period", "limit"],
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
      // ── E 批（#240）：三条数据面命令，取数经宿主 `stock.analysis` 接缝 ──
      // 三个工具（`get_industry_constituents` / `market_mainline_batch_upsert` /
      // `get_stock_factor`）只发命令，不再直连 `analysis-engine`（跨包口归零）。
      // `name` 必须与 worker 的三个 `COMMAND_*` 常量逐字一致。
      {
        // 只读：纯查询，宿主 `required_permission_for` 不给它登记权限（值仍是 `None`）。
        name: "stock_get_industry_constituents",
        description: "按行业名查询 A 股成分股分组（经宿主 stock.analysis 接缝，只读）",
        command: "worker://stock_get_industry_constituents",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            industries: {
              type: "array",
              items: {
                oneOf: [
                  { type: "string" },
                  {
                    type: "object",
                    properties: {
                      name: { type: "string" },
                      leaderCode: { type: "string" },
                    },
                    required: ["name"],
                  },
                ],
              },
            },
          },
          required: ["industries"],
        },
      },
      {
        // 唯一写命令：`requiredPermission` 必须是 `workspace-write`（否则按未声明=只读档，
        // 智能体面把它当 `Safe` 暴露、确认弹窗形同虚设）；`permissions` 里也必须有 `write`，
        // 否则 `check_command_gate` 直接拒（见上方 permissions 注释的两面）。
        name: "stock_market_mainline_batch_upsert",
        description: "批量写入市场主线（经宿主 stock.analysis 写面；回 inserted / updated / archived）",
        command: "worker://stock_market_mainline_batch_upsert",
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          additionalProperties: true,
          properties: {
            mainline_date: { type: "string", description: "主线日期 YYYY-MM-DD（必填，也收 mainlineDate）" },
            mainlines: {
              type: "array",
              items: { type: "object", additionalProperties: true },
            },
            archive_missing: { type: "boolean", description: "归档当日未提及主线（也收 archiveMissing）" },
            source_workflow_execution_id: { type: "string" },
          },
          required: ["mainline_date", "mainlines"],
        },
      },
      {
        // 只读：逐票 1 月涨跌幅，宿主不登记权限（`None`）。
        name: "stock_get_stock_factor",
        description: "逐票查询 1 月涨跌幅（经宿主 stock.analysis 接缝，只读；缺席票单独报）",
        command: "worker://stock_get_stock_factor",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            stock_codes: {
              description: "股票代码清单（宽容形态）：字符串数组、{\"stockCode\"|\"code\"} 对象数组、或含 stocks 分组的信封",
            },
            as_of_date: { type: "string", description: "可选截止日（YYYY-MM-DD）" },
          },
          required: ["stock_codes"],
        },
      },
      // ── T2 试点批（#248）：四条宿主命令的取数面（只读 / 纯计算），经 `stock.analysis` 接缝 ──
      // 与 E 批三条同一透传纪律：字段形状的权威在主壳 commands 层（pipeline 手拼行 /
      // dao 进化模型 / `commands/wf_des.rs` 的 WfDes DTO），插件侧改写即第二套形状。
      // 四条都只读，宿主 `required_permission_for` 不登记权限（同 E 批只读三条）。
      {
        name: "stock_get_pipeline_history",
        description: "列出管道执行历史（经宿主 stock.analysis 接缝，只读；返回手拼行数组）",
        command: "worker://stock_get_pipeline_history",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            limit: { type: "integer", description: "取数上限，宿主缺省取 20" },
          },
        },
      },
      {
        name: "stock_get_pipeline_run_detail",
        description: "查询单次管道执行详情（经宿主 stock.analysis 接缝，只读；缺席 run 由宿主报错）",
        command: "worker://stock_get_pipeline_run_detail",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            runId: { type: "string", description: "管道执行 id（接缝主键；空值由宿主 provider 拒收）" },
          },
          required: ["runId"],
        },
      },
      {
        // 该命令无参：入参非空对象会被宿主 provider 的 Validation 拒收。
        name: "stock_list_pending_evolution_plans",
        description: "列出待审批的反思进化计划（经宿主 stock.analysis 接缝，只读，无参数）",
        command: "worker://stock_list_pending_evolution_plans",
        inputSchema: { type: "object", properties: {} },
      },
      {
        // 清单不复刻 DTO（同 stock_market_mainline_batch_upsert 宽形态先例）：形状权威在
        // `commands/wf_des.rs` 的 WfDesRequest（camelCase，四字段全必填），此处只锁最外层。
        name: "stock_wf_des_integration",
        description: "Walk-Forward 回测 + DES 对比仿真（纯计算，经宿主 stock.analysis 接缝）",
        command: "worker://stock_wf_des_integration",
        inputSchema: {
          type: "object",
          additionalProperties: true,
          properties: {
            klines: { type: "array", description: "历史 K 线（Bar 数组，宿主拒空集）" },
            wfConfig: { type: "object", description: "Walk-Forward 配置（trainDays/testDays/stepDays/riskFreeAnnual）" },
            desConfig: { type: "object", description: "DES 模拟配置" },
            strategyName: { type: "string", description: "策略名（当前仅 ma_cross）" },
          },
          required: ["klines", "wfConfig", "desConfig", "strategyName"],
        },
      },
      // ── 批 1a（#35）：十二股票 cron 配置命令，经 `stock.analysis` 接缝 ──
      // 三条 create 回建好的任务、九条管理面回 `null`；配置归一与响应形状的权威在主壳
      // `commands/recommendation_cron.rs` / `commands/pool_scan.rs` 的无 State 核心。
      // ⚠ 九条写命令（三 create + 三 toggle + 三 delete）必须 `requiredPermission:
      // "workspace-write"`（否则智能体面按 `Safe` 暴露、确认弹窗形同虚设）；三条 list
      // 是读 op，宿主 `required_permission_for` 与清单都不加闸。
      {
        name: "stock_create_recommendation_cron",
        description: "创建荐股定时任务（经宿主 stock.analysis 接缝写入调度器任务表）",
        command: "worker://stock_create_recommendation_cron",
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            name: { type: "string", description: "任务名" },
            cronExpression: { type: "string", description: "cron 表达式（camelCase）" },
            periods: {
              type: "array",
              items: { type: "string", enum: ["ultra_short", "short", "mid", "long"] },
              description: "投资周期档位，不能为空",
            },
            minConfidence: { type: "integer", description: "最低置信度 0-100" },
            topN: { type: "integer", description: "每次推送只数，必须大于 0" },
          },
          required: ["name", "cronExpression", "periods", "minConfidence", "topN"],
        },
      },
      {
        // 无参：接缝 provider 的 `require_no_params` 会拒非空入参。
        name: "stock_list_recommendation_crons",
        description: "列出荐股定时任务（经宿主 stock.analysis 接缝，只读，无参数）",
        command: "worker://stock_list_recommendation_crons",
        inputSchema: { type: "object", properties: {} },
      },
      {
        name: "stock_toggle_recommendation_cron",
        description: "启停荐股定时任务（经宿主 stock.analysis 接缝）",
        command: "worker://stock_toggle_recommendation_cron",
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            id: { type: "string", description: "任务 id" },
            enabled: { type: "boolean", description: "true = 启用，false = 暂停" },
          },
          required: ["id", "enabled"],
        },
      },
      {
        name: "stock_delete_recommendation_cron",
        description: "删除荐股定时任务（经宿主 stock.analysis 接缝）",
        command: "worker://stock_delete_recommendation_cron",
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: { id: { type: "string", description: "任务 id" } },
          required: ["id"],
        },
      },
      {
        // 三参可选：缺省 = 原链 / `0 16 * * 1-5` / 启用。
        name: "stock_create_trend_screening_cron",
        description: "创建趋势智选定时任务（经宿主 stock.analysis 接缝写入调度器任务表）",
        command: "worker://stock_create_trend_screening_cron",
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            cronExpression: { type: "string", description: "cron 表达式，缺省 0 16 * * 1-5" },
            enabled: { type: "boolean", description: "缺省 true" },
            workflowId: {
              type: "string",
              description: "链 id：serenity-screening（原链，缺省）/ serenity-screening-fast（快速链）",
            },
          },
        },
      },
      {
        name: "stock_list_trend_screening_crons",
        description: "列出趋势智选定时任务（经宿主 stock.analysis 接缝，只读，无参数）",
        command: "worker://stock_list_trend_screening_crons",
        inputSchema: { type: "object", properties: {} },
      },
      {
        name: "stock_toggle_trend_screening_cron",
        description: "启停趋势智选定时任务（经宿主 stock.analysis 接缝）",
        command: "worker://stock_toggle_trend_screening_cron",
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            id: { type: "string", description: "任务 id" },
            enabled: { type: "boolean", description: "true = 启用，false = 暂停" },
          },
          required: ["id", "enabled"],
        },
      },
      {
        name: "stock_delete_trend_screening_cron",
        description: "删除趋势智选定时任务（经宿主 stock.analysis 接缝）",
        command: "worker://stock_delete_trend_screening_cron",
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: { id: { type: "string", description: "任务 id" } },
          required: ["id"],
        },
      },
      {
        // 六参可选：缺省值在宿主无 State 核心内（`PoolScanConfig::default()`）。
        name: "stock_create_pool_scan_cron",
        description: "创建候选池扫描定时任务（经宿主 stock.analysis 接缝写入调度器任务表）",
        command: "worker://stock_create_pool_scan_cron",
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            cronExpression: { type: "string", description: "cron 表达式，缺省 0 17 * * *" },
            lookbackDays: { type: "integer", description: "进池窗口天数，0 = 全池" },
            maxStocks: { type: "integer", description: "最多分析只数，0 = 不限" },
            minConfidence: { type: "integer", description: "最低置信度 0-100" },
            enabled: { type: "boolean", description: "缺省 true" },
            maxPerRun: { type: "integer", description: "单次运行硬上限" },
            runDeadlineSeconds: { type: "integer", description: "单次运行截止秒数" },
          },
        },
      },
      {
        name: "stock_list_pool_scan_crons",
        description: "列出候选池扫描定时任务（经宿主 stock.analysis 接缝，只读，无参数）",
        command: "worker://stock_list_pool_scan_crons",
        inputSchema: { type: "object", properties: {} },
      },
      {
        name: "stock_toggle_pool_scan_cron",
        description: "启停候选池扫描定时任务（经宿主 stock.analysis 接缝）",
        command: "worker://stock_toggle_pool_scan_cron",
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            id: { type: "string", description: "任务 id" },
            enabled: { type: "boolean", description: "true = 启用，false = 暂停" },
          },
          required: ["id", "enabled"],
        },
      },
      {
        name: "stock_delete_pool_scan_cron",
        description: "删除候选池扫描定时任务（经宿主 stock.analysis 接缝）",
        command: "worker://stock_delete_pool_scan_cron",
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: { id: { type: "string", description: "任务 id" } },
          required: ["id"],
        },
      },
      // ── 批 2a（#35）：十一条行情只读命令，一律经宿主 market.quote 接缝 ──
      // 不声明 requiredPermission ⇒ 投影 Safe，与宿主同名命令的 `safety = Safe` 逐条等阶。
      // 入参是 camelCase（禁区 13），插件路由表翻成接缝要求的 snake_case。
      {
        name: "stock_get_research_reports",
        description: "获取研究报告（经宿主 market.quote 接缝，只读）",
        command: "worker://stock_get_research_reports",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: { stockCode: { type: "string", description: "6 位股票代码，如 600519" } },
          required: ["stockCode"],
        },
      },
      {
        name: "stock_get_concept_blocks",
        description: "获取个股所属概念板块（经宿主 market.quote 接缝，只读）",
        command: "worker://stock_get_concept_blocks",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: { stockCode: { type: "string", description: "6 位股票代码，如 600519" } },
          required: ["stockCode"],
        },
      },
      {
        name: "stock_get_announcements",
        description: "获取公司公告（经宿主 market.quote 接缝，只读）",
        command: "worker://stock_get_announcements",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: { stockCode: { type: "string", description: "6 位股票代码，如 600519" } },
          required: ["stockCode"],
        },
      },
      {
        name: "stock_get_earnings_calendar",
        description: "获取财报披露日历（经宿主 market.quote 接缝，只读）",
        command: "worker://stock_get_earnings_calendar",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: { stockCode: { type: "string", description: "6 位股票代码，如 600519" } },
          required: ["stockCode"],
        },
      },
      {
        name: "stock_get_concept_board_members",
        description: "获取概念板块成分股（经宿主 market.quote 接缝，只读；入参是板块代码）",
        command: "worker://stock_get_concept_board_members",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: { boardCode: { type: "string", description: "板块代码，如 BK0475" } },
          required: ["boardCode"],
        },
      },
      {
        name: "stock_get_cls_flash",
        description: "获取财联社快讯（经宿主 market.quote 接缝，只读；无入参）",
        command: "worker://stock_get_cls_flash",
        inputSchema: { type: "object", additionalProperties: false, properties: {} },
      },
      {
        name: "stock_get_peers",
        description: "获取同行业对比（经宿主 market.quote 接缝，只读）",
        command: "worker://stock_get_peers",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: { stockCode: { type: "string", description: "6 位股票代码，如 600519" } },
          required: ["stockCode"],
        },
      },
      {
        name: "stock_get_option_pcr",
        description: "获取期权 PCR（经宿主 market.quote 接缝，只读）",
        command: "worker://stock_get_option_pcr",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: { stockCode: { type: "string", description: "6 位股票代码，如 600519" } },
          required: ["stockCode"],
        },
      },
      {
        name: "stock_get_lockup_schedule",
        description: "获取限售解禁时间表（经宿主 market.quote 接缝，只读）",
        command: "worker://stock_get_lockup_schedule",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: { stockCode: { type: "string", description: "6 位股票代码，如 600519" } },
          required: ["stockCode"],
        },
      },
      {
        name: "stock_get_dividend_records",
        description: "获取分红记录（经宿主 market.quote 接缝，只读）",
        command: "worker://stock_get_dividend_records",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: { stockCode: { type: "string", description: "6 位股票代码，如 600519" } },
          required: ["stockCode"],
        },
      },
      {
        name: "stock_get_financials",
        description: "获取股票财务数据（经宿主 market.quote 接缝，只读；对比面板使用）",
        command: "worker://stock_get_financials",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: { stockCode: { type: "string", description: "6 位股票代码，如 600519" } },
          required: ["stockCode"],
        },
      },
      // ── 批 2c（#262）：五条全市场榜单/盘面只读命令，零入参，同走 market.quote 接缝 ──
      // `stock_get_index_quotes` 的接缝 op 是 `get_index_quotes_outcome`（返回 DataOutcome
      // 信封，与宿主命令同形）；清单面命令名与宿主 `get_index_quotes` 对齐。
      {
        name: "stock_get_hot_stocks",
        description: "获取热门股票榜（经宿主 market.quote 接缝，只读；无入参）",
        command: "worker://stock_get_hot_stocks",
        inputSchema: { type: "object", additionalProperties: false, properties: {} },
      },
      {
        name: "stock_get_industry_ranking",
        description: "获取行业排名榜（经宿主 market.quote 接缝，只读；无入参）",
        command: "worker://stock_get_industry_ranking",
        inputSchema: { type: "object", additionalProperties: false, properties: {} },
      },
      {
        name: "stock_get_index_quotes",
        description: "获取指数行情（经宿主 market.quote 接缝，只读；无入参，返回 DataOutcome 信封）",
        command: "worker://stock_get_index_quotes",
        inputSchema: { type: "object", additionalProperties: false, properties: {} },
      },
      {
        name: "stock_get_market_dragon_tiger",
        description: "获取全市场龙虎榜（经宿主 market.quote 接缝，只读；无入参）",
        command: "worker://stock_get_market_dragon_tiger",
        inputSchema: { type: "object", additionalProperties: false, properties: {} },
      },
      {
        name: "stock_get_north_bound_flow",
        description: "获取北向资金（经宿主 market.quote 接缝，只读；无入参）",
        command: "worker://stock_get_north_bound_flow",
        inputSchema: { type: "object", additionalProperties: false, properties: {} },
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
}

/**
 * opc 清单（#36 第 1 批只有 worker；第 4 批接进首条命名命令）。
 *
 * `defaultEnabled: true` 的理由与 stock 同一条（第 402 行那段注释）：前端 OPC 站点页 /
 * 仪表盘的博客列表已改走本插件的命名命令，宿主同名命令已删 ⇒ 内置包不默认启用就等于
 * 该功能静默失效（`resolvePluginForCommand` 解析不到插件）。
 * 仍**不写** `pages` / `taskSources` / `uiSchema`：OPC 的页面贡献属后续批。
 */

// 清单命令的智能体面授权档位（`requiredPermission`，见 `PluginCommandManifest`）：
// **未声明 = 只读档** ⇒ 写命令在智能体面按 `Safe` 暴露（`plugin_command_safety(None)`
// 的测试锁定），用户确认弹窗形同虚设。写命令名单以宿主
// `axagent-plugins/crates/plugins/src/worker.rs` 的
// `required_permission_for(("opc.store", …)) => Write` 为准 —— **两处必须同批改**
// （那边加一条写 op，这里就要加一条命令）。
// ⚠ 删除类（delete_blog_post / delete_content_asset / delete_customer / delete_invoice）
// 的宿主等价命令是 `Dangerous`，但清单标 `danger-full-access` 会被
// `PluginManager::check_command_gate` 拒（该档要求插件声明 `execute`）；为不扩张插件
// 权限面，本表一律 `workspace-write`（= Caution）。升 Dangerous 需先裁定 execute 面
// （插件整体获得该声明），属安全面变更，另立评审。
const OPC_WRITE_COMMANDS = new Set([
  "opc_add_milestone",
  "opc_cancel_publish_schedule",
  "opc_complete_milestone",
  "opc_create_blog_post",
  "opc_create_content_asset",
  "opc_create_customer",
  "opc_create_invoice",
  "opc_create_landing_page",
  "opc_create_project",
  "opc_create_publish_schedule",
  "opc_delete_blog_post",
  "opc_delete_content_asset",
  "opc_delete_customer",
  "opc_delete_invoice",
  "opc_delete_landing_page",
  "opc_delete_project",
  "opc_mark_contact_read",
  "opc_process_due_schedules",
  "opc_publish_blog_post",
  "opc_publish_landing_page",
  "opc_transition_invoice",
  "opc_update_content_asset",
  "opc_update_customer",
  "opc_update_project",
]);

function withRequiredPermission(commands) {
  return commands.map((command) => ({
    ...command,
    requiredPermission: OPC_WRITE_COMMANDS.has(command.name) ? "workspace-write" : "read-only",
  }));
}

function opcManifest({ version, binaryName, sha256 }) {
  return {
    name: "opc",
    version,
    description:
      "OPC 域功能包（#36）：worker 通道 + 33 条命名命令（读 12 条 · 写 21 条）",
    // 同 stock：worker 由宿主 spawn ⇒ 不声明 `subprocess_execution` 会被 enable 直接拒绝。
    // ⚠ `write` 是第二批加的：宿主分派层 `worker.rs` 的 `required_permission_for`
    // 给 `("opc.store","publish_blog_post" | "delete_blog_post" | "create_blog_post")`
    // 三个写 op 登记了 `Write`，而 `has_permission` 是**精确
    // `contains`**（刻意不套 `implies_*`，免得 `subprocess_execution` 这类宽权限悄悄补齐写面
    // 准入）⇒ 清单缺这一条时登记即拒：命令会在接缝调用前落 `PLUGIN_PERMISSION_DENIED`，
    // 表现为「插件装得上、describe 正常、发布恒失败」。权限与 op 登记必须同批。
    permissions: ["subprocess_execution", "write"],
    mcp_servers: [],
    skills: [],
    agents: [],
    defaultEnabled: true,
    worker: { program: path.posix.join("bin", binaryName) },
    // `command` 用 `worker://<名>` 占位（同 stock）：worker 通道不读它，但按名发帧的
    // `name` 必须与 worker 里的 `COMMAND_LIST_BLOG_POSTS` / `COMMAND_PUBLISH_BLOG_POST` 逐字一致。
    commands: withRequiredPermission([
      {
        name: "opc_list_blog_posts",
        description: "列出 OPC 博客文章（经宿主 opc.store 接缝取数，不含正文；不传分页则取尽整表）",
        command: "worker://opc_list_blog_posts",
        inputSchema: {
          type: "object",
          properties: {
            limit: { type: "number", description: "单页条数（越界由宿主按契约区间拒收）" },
            offset: { type: "number", description: "分页偏移；与 limit 都缺省 = 取尽整表" },
          },
        },
      },
      {
        // 先例 #144 的双声明已在 #36 blog 刀收口：宿主 `commands/opc.rs` 的同名 Tauri 命令
        // 已删，本条是发布博客的唯一通道，回宿主的两态枚举串。
        name: "opc_publish_blog_post",
        description: "发布一篇 OPC 博客（经宿主 opc.store 写面；回 published / not_found 两态）",
        command: "worker://opc_publish_blog_post",
        inputSchema: {
          type: "object",
          properties: {
            id: { type: "string", description: "文章 id（接缝主键；空值由宿主 provider 拒收）" },
          },
          required: ["id"],
        },
      },
      {
        // 删除：入参同 publish（平铺 `id`），出参是三态枚举串逐字透传 —— 命令层与宿主
        // provider 各判一半（形状 / 语义），前端对 not_found / protected 自行翻提示。
        name: "opc_delete_blog_post",
        description: "删除一篇 OPC 博客（经宿主 opc.store 写面；回 removed / not_found / protected 三态）",
        command: "worker://opc_delete_blog_post",
        inputSchema: {
          type: "object",
          properties: {
            id: { type: "string", description: "文章 id（接缝主键；空值由宿主 provider 拒收）" },
          },
          required: ["id"],
        },
      },
      {
        // 新建：入参按清单 `inputSchema` 收平铺五字段（形状判在 worker 命令层，非空 /
        // 字节预算判在宿主 provider）；出参 = 创建行的列裁剪视图（含 id 句柄）。
        name: "opc_create_blog_post",
        description: "新建一篇 OPC 博客（经宿主 opc.store 写面；回创建行的列裁剪视图，含 id）",
        command: "worker://opc_create_blog_post",
        inputSchema: {
          type: "object",
          properties: {
            title: { type: "string", description: "标题" },
            slug: { type: "string", description: "URL 标识（唯一性由宿主 provider 判）" },
            excerpt: { type: "string", description: "摘要" },
            content: { type: "string", description: "正文（字节预算由宿主按契约常量判）" },
            tags: { type: "array", items: { type: "string" }, description: "标签" },
          },
          required: ["title", "slug", "excerpt", "content", "tags"],
        },
      },
      // ── #36 批 2：4 条写命令（全部走 opc.store 写面 ⇒ 宿主 requireWrite 闸）──
      // 出参一律是契约的外部标记枚举（`{"transitioned"…}` / `{"updated"…}` / `"removed"` /
      // `"not_found"`）；「缺行」是正常读数，前端按取值翻提示，不按错误码分派。
      {
        name: "opc_transition_invoice",
        description: "变更 OPC 发票状态（经宿主 opc.store 写面；回 transitioned / not_found 两态）",
        command: "worker://opc_transition_invoice",
        inputSchema: {
          type: "object",
          properties: {
            id: { type: "string", description: "发票 id（接缝主键；空值由宿主 provider 拒收）" },
            status: {
              type: "string",
              enum: ["draft", "sent", "paid", "overdue", "cancelled", "refunded"],
              description: "目标状态（合法性由宿主 provider 判）"
            },
          },
          required: ["id", "status"],
        },
      },
      {
        name: "opc_update_content_asset",
        description: "更新 OPC 内容资产（经宿主 opc.store 写面；回 updated / not_found 两态）",
        command: "worker://opc_update_content_asset",
        inputSchema: {
          type: "object",
          properties: {
            id: { type: "string", description: "内容资产 id（接缝主键；空值由宿主 provider 拒收）" },
            title: { type: "string", description: "新标题（缺省 = 该列不动）" },
            contentType: { type: "string", description: "新内容类型（缺省 = 该列不动）" },
            body: { type: "string", description: "新正文（缺省 = 该列不动）" },
            tags: { type: "array", items: { type: "string" }, description: "新标签（缺省 = 该列不动）" },
            status: { type: "string", description: "新状态（缺省 = 该列不动）" },
          },
          required: ["id"],
        },
      },
      {
        name: "opc_delete_content_asset",
        description: "删除 OPC 内容资产（经宿主 opc.store 写面；回 removed / not_found 两态）",
        command: "worker://opc_delete_content_asset",
        inputSchema: {
          type: "object",
          properties: {
            id: { type: "string", description: "内容资产 id（接缝主键；空值由宿主 provider 拒收）" },
          },
          required: ["id"],
        },
      },
      {
        name: "opc_cancel_publish_schedule",
        description: "取消/删除 OPC 发布计划（硬删；经宿主 opc.store 写面，回 removed / not_found 两态）",
        command: "worker://opc_cancel_publish_schedule",
        inputSchema: {
          type: "object",
          properties: {
            id: { type: "string", description: "发布计划 id（接缝主键；空值由宿主 provider 拒收）" },
          },
          required: ["id"],
        },
      },
      // ── #36 批 3：8 条写命令 + 1 条财务报表查询（前 8 条走 opc.store 写面 ⇒ 宿主
      //    requireWrite 闸；`opc_get_financial_report` 是读 op，与八个 list op 同档无闸）──
      // 入参一律 camelCase（与契约 `*Request` DTO 的 serde 键逐字一致，禁区 13）；
      // 数组元素内部字段类型的错由宿主 serde 拒收，命令层只判「是不是对象数组」。
      {
        name: "opc_create_invoice",
        description: "创建 OPC 发票（经宿主 opc.store 写面；回创建行的列裁剪视图，含 id）",
        command: "worker://opc_create_invoice",
        inputSchema: {
          type: "object",
          properties: {
            customerId: { type: "string", description: "客户 id" },
            lineItems: {
              type: "array",
              description: "行项目；行金额由宿主按 quantity×unitPrice×(1+taxRate) 现算",
              items: {
                type: "object",
                properties: {
                  description: { type: "string", description: "项目描述" },
                  quantity: { type: "number", description: "数量（缺省 1）" },
                  unitPrice: { type: "number", description: "单价（缺省 0）" },
                  taxRate: { type: "number", description: "税率，如 0.13（缺省 0）" },
                },
              },
            },
            currency: { type: "string", description: "货币代码（缺省 CNY）" },
            dueAt: { type: "number", description: "到期时间戳（epoch 秒）" },
            notes: { type: "string", description: "备注" },
          },
          required: ["customerId", "lineItems"],
        },
      },
      {
        name: "opc_create_customer",
        description: "创建 OPC 客户（经宿主 opc.store 写面；回创建行的列裁剪视图，含 id）",
        command: "worker://opc_create_customer",
        inputSchema: {
          type: "object",
          properties: {
            name: { type: "string", description: "客户姓名" },
            email: { type: "string", description: "客户邮箱" },
            phone: { type: "string", description: "电话（可选）" },
            company: { type: "string", description: "公司（可选）" },
            customerType: {
              type: "string",
              description: "客户类型（consumer / business；其余值由宿主落 Unknown）",
            },
            country: { type: "string", description: "国家（可选）" },
            region: { type: "string", description: "地区（可选）" },
            city: { type: "string", description: "城市（可选）" },
            address: { type: "string", description: "地址（可选）" },
            latitude: { type: "number", description: "纬度（可选）" },
            longitude: { type: "number", description: "经度（可选）" },
            source: {
              type: "string",
              description: "来源（Referral / Website / SocialMedia / Marketplace / Direct；其余值由宿主落 Other）",
            },
            notes: { type: "string", description: "备注（可选）" },
          },
          required: ["name", "email"],
        },
      },
      // ── #32 批 A：三条写命令（客户更新/客户删除/发票删除，均走 opc.store 写面 ⇒
      //    宿主 requireWrite 闸）。update 出参 = 更新后的列裁剪视图；删除出参 = 无载荷。
      //    可选字段「缺省 = 该列不动」；⚠ 清空在本命令线格式上不可表达（与宿主同名
      //    Tauri 命令行为一致）。opc_delete_invoice 删的是域包发票 —— 宿主同名 Tauri
      //    命令删的是交付域发票，两套体系同名不同义。
      {
        name: "opc_update_customer",
        description: "更新 OPC 客户（经宿主 opc.store 写面；回更新后的列裁剪视图）",
        command: "worker://opc_update_customer",
        inputSchema: {
          type: "object",
          properties: {
            id: { type: "string", description: "客户 id（接缝主键；空值由宿主 provider 拒收）" },
            name: { type: "string", description: "新姓名（缺省 = 该列不动）" },
            email: { type: "string", description: "新邮箱（缺省 = 该列不动）" },
            phone: { type: "string", description: "新电话（缺省 = 该列不动）" },
            company: { type: "string", description: "新公司（缺省 = 该列不动）" },
            customerType: {
              type: "string",
              description: "新客户类型（consumer / business；其余值由宿主落 Unknown）",
            },
            country: { type: "string", description: "新国家（缺省 = 该列不动）" },
            region: { type: "string", description: "新地区（缺省 = 该列不动）" },
            city: { type: "string", description: "新城市（缺省 = 该列不动）" },
            address: { type: "string", description: "新地址（缺省 = 该列不动）" },
            latitude: { type: "number", description: "新纬度（缺省 = 该列不动）" },
            longitude: { type: "number", description: "新经度（缺省 = 该列不动）" },
            notes: { type: "string", description: "新备注（缺省 = 该列不动）" },
          },
          required: ["id"],
        },
      },
      {
        name: "opc_delete_customer",
        description: "删除 OPC 客户（经宿主 opc.store 写面；成功无载荷）",
        command: "worker://opc_delete_customer",
        inputSchema: {
          type: "object",
          properties: {
            id: { type: "string", description: "客户 id（接缝主键；空值由宿主 provider 拒收）" },
          },
          required: ["id"],
        },
      },
      {
        name: "opc_delete_invoice",
        description: "删除 OPC 发票（域包发票；经宿主 opc.store 写面，成功无载荷）",
        command: "worker://opc_delete_invoice",
        inputSchema: {
          type: "object",
          properties: {
            id: { type: "string", description: "发票 id（接缝主键；空值由宿主 provider 拒收）" },
          },
          required: ["id"],
        },
      },
      // ── #32 批 B：四条项目命令（详情读 1 + 写 3，均走 opc.store ⇒ 写命令由
      //    `withRequiredPermission` 按上面 `OPC_WRITE_COMMANDS` 自动挂 workspace-write）。
      //    详情出参 = 摘要列 + 描述 / 备注 / 里程碑明细（一个扁平对象，无内层壳键）。
      {
        name: "opc_get_project",
        description: "获取 OPC 项目详情（经宿主 opc.store 只读接缝；含描述 / 备注 / 里程碑明细）",
        command: "worker://opc_get_project",
        inputSchema: {
          type: "object",
          properties: {
            id: { type: "string", description: "项目 id（接缝主键；空值由宿主 provider 拒收）" },
          },
          required: ["id"],
        },
      },
      {
        name: "opc_update_project",
        description: "更新 OPC 项目（经宿主 opc.store 写面；回更新后的详情视图）",
        command: "worker://opc_update_project",
        inputSchema: {
          type: "object",
          properties: {
            id: { type: "string", description: "项目 id（接缝主键；空值由宿主 provider 拒收）" },
            title: { type: "string", description: "新标题（缺省 = 该列不动）" },
            description: { type: "string", description: "新描述（缺省 = 该列不动）" },
            budget: { type: "number", description: "新预算（缺省 = 该列不动）" },
            deadline: { type: "number", description: "新截止时间戳（epoch 秒，缺省 = 不动）" },
            notes: { type: "string", description: "新备注（缺省 = 该列不动）" },
          },
          required: ["id"],
        },
      },
      {
        name: "opc_delete_project",
        description: "删除 OPC 项目（经宿主 opc.store 写面；成功无载荷）",
        command: "worker://opc_delete_project",
        inputSchema: {
          type: "object",
          properties: {
            id: { type: "string", description: "项目 id（接缝主键；空值由宿主 provider 拒收）" },
          },
          required: ["id"],
        },
      },
      {
        name: "opc_complete_milestone",
        description: "完成 OPC 项目的里程碑（经宿主 opc.store 写面；回完成后的详情视图）",
        command: "worker://opc_complete_milestone",
        inputSchema: {
          type: "object",
          properties: {
            projectId: { type: "string", description: "项目 id" },
            milestoneId: { type: "string", description: "里程碑 id（须在该项目明细里）" },
          },
          required: ["projectId", "milestoneId"],
        },
      },
      {
        name: "opc_create_project",
        description: "创建 OPC 项目（经宿主 opc.store 写面；回创建行的列裁剪视图，含 id）",
        command: "worker://opc_create_project",
        inputSchema: {
          type: "object",
          properties: {
            customerId: { type: "string", description: "关联客户 id（可选）" },
            title: { type: "string", description: "项目标题" },
            description: { type: "string", description: "项目描述（可选）" },
            budget: { type: "number", description: "预算（可选）" },
            currency: { type: "string", description: "货币代码（缺省 CNY）" },
            deadline: { type: "number", description: "截止时间戳（epoch 秒，可选）" },
            notes: { type: "string", description: "备注（可选）" },
          },
          required: ["title"],
        },
      },
      {
        name: "opc_add_milestone",
        description: "给 OPC 项目添加里程碑（经宿主 opc.store 写面；回项目视图，含里程碑计数）",
        command: "worker://opc_add_milestone",
        inputSchema: {
          type: "object",
          properties: {
            projectId: { type: "string", description: "项目 id" },
            title: { type: "string", description: "里程碑标题" },
            description: { type: "string", description: "描述（可选）" },
            dueAt: { type: "number", description: "到期时间戳（epoch 秒，可选）" },
          },
          required: ["projectId", "title"],
        },
      },
      {
        name: "opc_create_landing_page",
        description: "创建 OPC 落地页（经宿主 opc.store 写面；创建后为草稿态，回列裁剪视图）",
        command: "worker://opc_create_landing_page",
        inputSchema: {
          type: "object",
          properties: {
            title: { type: "string", description: "标题" },
            slug: { type: "string", description: "URL 标识" },
            description: { type: "string", description: "页面描述（可选）" },
            content: { type: "string", description: "正文（Markdown）" },
          },
          required: ["title", "slug", "content"],
        },
      },
      {
        name: "opc_publish_landing_page",
        description: "发布 OPC 落地页（经宿主 opc.store 写面；回发布后的列裁剪视图）",
        command: "worker://opc_publish_landing_page",
        inputSchema: {
          type: "object",
          properties: {
            id: { type: "string", description: "落地页 id（接缝主键；空值由宿主 provider 拒收）" },
          },
          required: ["id"],
        },
      },
      {
        name: "opc_delete_landing_page",
        description: "删除 OPC 落地页（经宿主 opc.store 写面；成功无载荷）",
        command: "worker://opc_delete_landing_page",
        inputSchema: {
          type: "object",
          properties: {
            id: { type: "string", description: "落地页 id（接缝主键；空值由宿主 provider 拒收）" },
          },
          required: ["id"],
        },
      },
      {
        name: "opc_mark_contact_read",
        description: "把 OPC 联系提交标成已读（经宿主 opc.store 写面；成功无载荷）",
        command: "worker://opc_mark_contact_read",
        inputSchema: {
          type: "object",
          properties: {
            id: { type: "string", description: "联系提交 id（接缝主键；空值由宿主 provider 拒收）" },
          },
          required: ["id"],
        },
      },
      {
        name: "opc_get_financial_report",
        description: "生成 OPC 财务报表并附投资建议（只读 op：按期间统计收入 / 利润 / 可投资金额）",
        command: "worker://opc_get_financial_report",
        inputSchema: {
          type: "object",
          properties: {
            period: { type: "string", description: "期间，如 '2026-07'（月度）或 '2026-Q2'（季度）" },
          },
          required: ["period"],
        },
      },
      {
        name: "opc_create_content_asset",
        description: "创建 OPC 内容资产（经宿主 opc.store 写面；创建后为草稿态，回列裁剪视图）",
        command: "worker://opc_create_content_asset",
        inputSchema: {
          type: "object",
          properties: {
            title: { type: "string", description: "标题" },
            contentType: { type: "string", description: "内容类型（article / video / image）" },
            body: { type: "string", description: "正文（Markdown）" },
            tags: { type: "array", items: { type: "string" }, description: "标签（可选）" },
          },
          required: ["title", "contentType", "body"],
        },
      },
      {
        name: "opc_create_publish_schedule",
        description: "创建 OPC 发布计划（经宿主 opc.store 写面；回创建行的列裁剪视图，含 id）",
        command: "worker://opc_create_publish_schedule",
        inputSchema: {
          type: "object",
          properties: {
            contentRefType: { type: "string", description: "内容类型（blog_post / content_asset）" },
            contentRefId: { type: "string", description: "被发布内容的主键（不是本行 id）" },
            scheduledAt: { type: "number", description: "计划发布时间戳（epoch 秒）" },
          },
          required: ["contentRefType", "contentRefId", "scheduledAt"],
        },
      },
      {
        name: "opc_process_due_schedules",
        description: "处理全部到期的 OPC 发布计划（写 op；回本次处理过的行视图数组，空数组 = 无到期计划）",
        command: "worker://opc_process_due_schedules",
        inputSchema: { type: "object", properties: {} },
      },
      // ── #36 批 1：8 条只读查询 + 1 条看板（全部走 opc.store 只读 op，无必填字段）──
      // 属性名一律 camelCase（与契约 `*ListQuery` 的 serde 键逐字一致）；不写 `required`：
      // 缺键即「不过滤 / 宿主取缺省」，不是错误。
      {
        name: "opc_list_invoices",
        description: "列出 OPC 发票（经宿主 opc.store 只读接缝取数）",
        command: "worker://opc_list_invoices",
        inputSchema: {
          type: "object",
          properties: {
            status: { type: "string", description: "状态过滤（非法值按不过滤处理）" },
            customerId: { type: "string", description: "所属客户 id" },
            dateFrom: { type: "number", description: "开票时间下界（epoch 秒）" },
            dateTo: { type: "number", description: "开票时间上界（epoch 秒）" },
            limit: { type: "number", description: "返回条数上限" },
          },
        },
      },
      {
        name: "opc_list_customers",
        description: "列出 OPC 客户（经宿主 opc.store 只读接缝取数）",
        command: "worker://opc_list_customers",
        inputSchema: {
          type: "object",
          properties: {
            status: { type: "string", description: "状态过滤" },
            customerType: { type: "string", description: "客户类型过滤" },
            search: { type: "string", description: "名称 / 邮箱等关键词" },
            limit: { type: "number", description: "返回条数上限" },
          },
        },
      },
      {
        name: "opc_list_projects",
        description: "列出 OPC 项目（经宿主 opc.store 只读接缝取数）",
        command: "worker://opc_list_projects",
        inputSchema: {
          type: "object",
          properties: {
            status: { type: "string", description: "状态过滤" },
            customerId: { type: "string", description: "所属客户 id" },
            search: { type: "string", description: "标题关键词" },
            limit: { type: "number", description: "返回条数上限" },
          },
        },
      },
      {
        name: "opc_list_landing_pages",
        description: "列出 OPC 落地页（经宿主 opc.store 只读接缝取数）",
        command: "worker://opc_list_landing_pages",
        inputSchema: { type: "object", properties: {} },
      },
      {
        name: "opc_list_contacts",
        description: "列出 OPC 联系表单提交（经宿主 opc.store 只读接缝取数）",
        command: "worker://opc_list_contacts",
        inputSchema: { type: "object", properties: {} },
      },
      {
        name: "opc_list_kpis",
        description: "列出 OPC KPI 记录（经宿主 opc.store 只读接缝取数）",
        command: "worker://opc_list_kpis",
        inputSchema: {
          type: "object",
          properties: {
            period: { type: "string", description: "统计周期过滤（空 = 不过滤）" },
            limit: { type: "number", description: "返回条数上限" },
          },
        },
      },
      {
        name: "opc_list_content_assets",
        description: "列出 OPC 内容资产（经宿主 opc.store 只读接缝取数，不含正文）",
        command: "worker://opc_list_content_assets",
        inputSchema: { type: "object", properties: {} },
      },
      {
        name: "opc_list_publish_schedules",
        description: "列出 OPC 发布计划（经宿主 opc.store 只读接缝取数）",
        command: "worker://opc_list_publish_schedules",
        inputSchema: { type: "object", properties: {} },
      },
      {
        // 看板：宿主没有聚合 op，由插件进程组合三次列表查询现算六项读数。
        name: "opc_get_dashboard",
        description: "获取 OPC 仪表盘汇总（收入 / 待处理发票 / 活跃项目与客户计数）",
        command: "worker://opc_get_dashboard",
        inputSchema: { type: "object", properties: {} },
      },
      {
        // 看板总览：走宿主 `dashboard_summary` op（整表聚合，与上面「首页现算」不同口径）。
        name: "opc_get_dashboard_summary",
        description: "获取 OPC 看板总览（收入合计 / 发票 / 项目 / 客户计数 + KPI 与收入趋势明细）",
        command: "worker://opc_get_dashboard_summary",
        inputSchema: { type: "object", properties: {} },
      },
      // ── #264 批 1：交付域五命令（两读三写）──
      // 与域包发票命令同名不同义：交付账本只列 `lead_id IS NOT NULL` 的来源行。
      // 属性名一律 camelCase（命令面）。两条读命令缺键即「不过滤」故不写 `required`；
      // 三条写命令把必填 id 列进 `required`，缺键在宿主侧就会被拒。
      {
        name: "opc_list_delivery_invoices",
        description: "列出交付发票（won 线索开票账本，可按状态过滤）",
        command: "worker://opc_list_delivery_invoices",
        inputSchema: {
          type: "object",
          properties: {
            status: { type: "string", description: "状态过滤（draft / sent / paid）" },
          },
        },
      },
      {
        name: "opc_create_invoice_from_lead",
        description: "把 won 线索开成交付发票（宿主幂等：已有发票直接返回）",
        command: "worker://opc_create_invoice_from_lead",
        inputSchema: {
          type: "object",
          properties: {
            leadId: { type: "string", description: "需求线索 id（必须为 won 状态）" },
            input: {
              type: "object",
              description: "缺省字段从线索元数据自动填充（title / amount / currency / notes）",
            },
          },
          required: ["leadId"],
        },
      },
      {
        name: "opc_update_invoice_status",
        description: "推进交付发票状态机（draft → sent → paid，逆向报错）",
        command: "worker://opc_update_invoice_status",
        inputSchema: {
          type: "object",
          properties: {
            invoiceId: { type: "string", description: "交付发票 id" },
            status: { type: "string", description: "目标状态（draft / sent / paid）" },
          },
          required: ["invoiceId", "status"],
        },
      },
      {
        name: "opc_delete_delivery_invoice",
        description: "删除交付发票（作废用删除代替，不可逆）",
        command: "worker://opc_delete_delivery_invoice",
        inputSchema: {
          type: "object",
          properties: {
            invoiceId: { type: "string", description: "交付发票 id" },
          },
          required: ["invoiceId"],
        },
      },
      {
        name: "opc_get_delivery_summary",
        description: "交付汇总（won 数 / 发票数 / 回款按币种 / 转化率）",
        command: "worker://opc_get_delivery_summary",
        inputSchema: { type: "object", properties: {} },
      },
      // ── #264 批 2：需求发现八命令（五写三读）──
      // 属性名 camelCase（命令面）。三条写命令把必填对象/字符串列进 `required`；
      // 两条读命令零入参，`list_leads` 三个过滤项均可缺省（缺键即不过滤）。
      {
        name: "opc_list_platforms",
        description: "列出需求平台配置（表空时自动填充内置默认平台）",
        command: "worker://opc_list_platforms",
        inputSchema: { type: "object", properties: {} },
      },
      {
        name: "opc_save_platform",
        description: "保存（新增或更新）需求平台配置",
        command: "worker://opc_save_platform",
        inputSchema: {
          type: "object",
          properties: {
            input: { type: "object", description: "平台配置（字段同 DemandPlatform）" },
          },
          required: ["input"],
        },
      },
      {
        name: "opc_delete_platform",
        description: "删除需求平台配置",
        command: "worker://opc_delete_platform",
        inputSchema: {
          type: "object",
          properties: { id: { type: "string", description: "平台标识" } },
          required: ["id"],
        },
      },
      {
        name: "opc_list_leads",
        description: "列出需求线索（按商业价值分降序，可按状态过滤）",
        command: "worker://opc_list_leads",
        inputSchema: {
          type: "object",
          properties: {
            limit: { type: "integer", description: "返回条数上限（缺省 100，宿主钳到 500）" },
            minScore: { type: "number", description: "最低商业价值分（缺省不过滤）" },
            status: { type: "string", description: "生命周期状态过滤（new / evaluated / ...）" },
          },
        },
      },
      {
        name: "opc_get_scan_policy",
        description: "读取需求扫描策略（缺键时返回默认策略）",
        command: "worker://opc_get_scan_policy",
        inputSchema: { type: "object", properties: {} },
      },
      {
        name: "opc_save_scan_policy",
        description: "保存需求扫描策略（宿主侧做范围钳制）",
        command: "worker://opc_save_scan_policy",
        inputSchema: {
          type: "object",
          properties: {
            policy: { type: "object", description: "扫描策略（字段同 ScanPolicy，可缺省字段）" },
          },
          required: ["policy"],
        },
      },
      {
        name: "opc_discover_and_evaluate_leads",
        description: "扫描并评估需求线索（并发扫描 → 评估 → 去重入库）",
        command: "worker://opc_discover_and_evaluate_leads",
        inputSchema: {
          type: "object",
          properties: { query: { type: "string", description: "扫描检索词（不能为空）" } },
          required: ["query"],
        },
      },
      {
        name: "opc_create_lead",
        description: "手动补录一条需求线索（窗口内去重返回既有行）",
        command: "worker://opc_create_lead",
        inputSchema: {
          type: "object",
          properties: {
            input: { type: "object", description: "线索字段（title / description 必填）" },
          },
          required: ["input"],
        },
      },
      {
        name: "opc_update_lead_status",
        description: "更新需求线索生命周期状态（new → evaluated → contacted → won/lost）",
        command: "worker://opc_update_lead_status",
        inputSchema: {
          type: "object",
          properties: {
            leadId: { type: "string", description: "线索 ID" },
            status: { type: "string", description: "目标状态（迁移合法性由宿主判定）" },
          },
          required: ["leadId", "status"],
        },
      },
      {
        name: "opc_convert_lead_to_workflow",
        description: "把需求线索转化为实现工作流模板（已转化则报错）",
        command: "worker://opc_convert_lead_to_workflow",
        inputSchema: {
          type: "object",
          properties: { leadId: { type: "string", description: "线索 ID" } },
          required: ["leadId"],
        },
      },
      {
        name: "opc_run_lead_workflow",
        description: "启动线索实现工作流的执行（返回执行 ID）",
        command: "worker://opc_run_lead_workflow",
        inputSchema: {
          type: "object",
          properties: { leadId: { type: "string", description: "线索 ID" } },
          required: ["leadId"],
        },
      },
    ]),
    integrity: { algorithm: "sha256", hash: sha256 },
  };
}
