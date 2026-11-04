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
const manifest = {
  name: "stock",
  version: workerPackage.version,
  description: "股票域功能包（当前贡献 1 个工具 get_stock_quote 与 1 条命令 stock_get_quote）",
  // worker 由宿主 spawn ⇒ 必须声明 `subprocess_execution`，否则 enable 时
  // `check_subprocess_permission` 直接拒绝（沙箱是从 manifest.permissions 推出来的，
  // 不是 `PluginWorkerConfig::new` 的 permissive 默认值）。
  permissions: ["subprocess_execution"],
  mcp_servers: [],
  skills: [],
  agents: [],
  defaultEnabled: false,
  worker: { program: path.posix.join("bin", binaryName) },
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
  ],
  integrity: { algorithm: "sha256", hash: sha256 },
};
const manifestPath = path.join(outRoot, "plugin.json");
fs.writeFileSync(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`);

console.log(`[plugin-bundle] 产物目录：${outRoot}`);
console.log(`[plugin-bundle] worker   ：${stagedBinary}`);
console.log(`[plugin-bundle] 清单     ：${manifestPath}`);
console.log(`[plugin-bundle] sha256   ：${sha256}`);

if (check) {
  await checkStagedBundle(manifest, outRoot);
}

/** 起一次成品 worker：describe 必须回合法声明，shutdown 必须回执并以 0 退出。 */
async function checkStagedBundle(declared, root) {
  const program = path.join(root, declared.worker.program);
  if (!fs.existsSync(program)) fail(`--check：清单声明的 worker 不存在：${program}`);
  const onDisk = createHash("sha256").update(fs.readFileSync(program)).digest("hex");
  if (onDisk !== declared.integrity.hash) {
    fail(`--check：integrity 不匹配（清单 ${declared.integrity.hash} / 盘上 ${onDisk}）`);
  }

  const child = spawn(program, [], { stdio: ["pipe", "pipe", "inherit"], windowsHide: true });
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

  const exitCode = await new Promise((resolve) => child.on("exit", resolve));
  if (exitCode !== 0) fail(`--check：worker 退出码 ${exitCode}（应为 0）`);

  console.log("[plugin-bundle] --check ✅ describe 握手 + shutdown 回执 + 退出码 0 + sha256 一致");
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
