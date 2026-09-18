#!/usr/bin/env node

/**
 * AxAgent 本地 CI 模拟脚本
 * 按"最便宜最先失败"顺序执行所有 CI 检查步骤
 *
 * 用法:
 *   node scripts/ci-check.mjs           # 完整检查
 *   node scripts/ci-check.mjs --quick   # 快速检查 (dprint + rustfmt + tsc)
 *   node scripts/ci-check.mjs --frontend-only
 *   node scripts/ci-check.mjs --rust-only
 *   node scripts/ci-check.mjs --skip-rust  # 跳过 Rust 检查（无 Rust 环境时）
 */

import { execSync } from "node:child_process";
import { existsSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const root = resolve(__dirname, "..");
const srcTauri = resolve(root, "src-tauri");

// 参数解析
const args = process.argv.slice(2);
const quick = args.includes("--quick");
const frontendOnly = args.includes("--frontend-only");
const rustOnly = args.includes("--rust-only");
const skipRust = args.includes("--skip-rust");

const hasRust = existsSync(resolve(srcTauri, "Cargo.toml"));
const canRunRust = hasRust && !skipRust && !frontendOnly;
const canRunFrontend = !rustOnly;

// Python 探测：CI 是 `python3`，Windows 本地常只有 `python` ⇒ 逐个试，都不行则**显式跳过**
// （打「跳过 ≠ 通过」，不静默略过 —— 静默略过会让本地绿冒充 CI 绿）。
function detectPython() {
  for (const bin of ["python3", "python"]) {
    try {
      execSync(`${bin} --version`, { stdio: "pipe" });
      return bin;
    } catch {
      // 换下一个候选
    }
  }
  return null;
}
const pythonBin = detectPython();
const hasPython = pythonBin !== null;

// 颜色输出
const c = {
  reset: "\x1b[0m",
  bold: "\x1b[1m",
  red: "\x1b[31m",
  green: "\x1b[32m",
  yellow: "\x1b[33m",
  cyan: "\x1b[36m",
};
const ok = `${c.green}✓${c.reset}`;
const fail = `${c.red}✗${c.reset}`;

let failures = 0;

function step(label, cmd, opts = {}) {
  const displayLabel = label.padEnd(52);
  process.stdout.write(`  ${displayLabel}`);
  try {
    execSync(cmd, { stdio: "pipe", cwd: opts.cwd || root, ...opts });
    console.log(`${ok}`);
    return true;
  } catch (e) {
    console.log(`${fail}`);
    const stderr = e.stderr?.toString().trim() || "";
    const stdout = e.stdout?.toString().trim() || "";
    const output = [stderr, stdout].filter(Boolean).join("\n");
    // 只打印最后 20 行，避免刷屏
    const lines = output.split("\n");
    const tail = lines.slice(-20).join("\n");
    console.log(`\n${c.red}${tail}${c.reset}\n`);
    failures++;
    if (!opts.continueOnError) {
      console.log(`${c.bold}${c.red}→ 检查失败，中断执行。请修复上述错误后重新运行。${c.reset}`);
      process.exit(1);
    }
    return false;
  }
}

// ── 入口 ────────────────────────────────────────────────
console.log(`\n${c.bold}AxAgent CI 本地检查${c.reset}`);
console.log(
  `${c.cyan}模式: ${
    quick ? "快速 (dprint + rustfmt + tsc)" : frontendOnly ? "仅前端" : rustOnly ? "仅 Rust" : "完整"
  }${c.reset}`,
);
console.log(`${c.cyan}平台: ${process.platform} | Node: ${process.version}${c.reset}`);
if (!hasRust) { console.log(`${c.yellow}Rust 环境未检测到，自动跳过 Rust 检查${c.reset}`); }
console.log();

const startedAt = Date.now();

// ── 前端检查 ────────────────────────────────────────────
if (canRunFrontend) {
  console.log(`${c.bold}[前端检查]${c.reset}`);

  // 纯 Node 的静态门禁放在最前：秒级、不依赖 npm ci / 构建产物 ⇒ 最快失败。
  // 为什么先跑各自的 `--selftest`：扫描器自身会撒谎（漏文件 / 剥错注释 / 检测器写偏），
  // 正负对照不过 ⇒ 后面的「通过」没有任何意义。
  // ⚠ 这六条必须与 `.github/workflows/ci.yml` 的对应步骤**逐字同源**，
  //   否则会出现「本地绿、CI 红」或反过来的假安全感。
  step("领域语义（量纲登记）自检", "node scripts/check-domain-semantics.mjs --selftest");
  step("领域语义（量纲登记）门禁", "node scripts/check-domain-semantics.mjs --strict");
  step("分层护栏（依赖方向）自检", "node scripts/check-layer-discipline.mjs --selftest");
  step("分层护栏（依赖方向 / 越层）", "node scripts/check-layer-discipline.mjs");
  step("领域本体（权威源↔副本）自检", "node scripts/check-ontology-consistency.mjs --selftest");
  step("领域本体（权威源↔副本）一致性", "node scripts/check-ontology-consistency.mjs");

  // ── 能力域单一真相源门禁（权威枚举 ↔ 前端 / prompt / i18n 副本）──
  // 为什么需要：`CapabilityDomain`（`harness/src/capability.rs`）是唯一权威，
  //   但下游有 6 类**手抄副本**（TS 联合类型 / 前端域元数据 / 协议顺序 /
  //   L1 分类器 prompt 的域清单 / OPC 行业→域映射 / 11 语言的 `capabilityDomain` key）。
  //   此前**一份都没被守**：后端加一个域 ⇒ 前端下拉里没有它、LLM 永远学不会它、
  //   界面显示裸 id，全都不报错（静默退化）。已收敛的先例是
  //   `harness/tool.rs` 的 `pub use CapabilityDomain as ToolDomain` 与
  //   `capability_clusters.rs` 的枚举字段（见 `PLAN-domain-single-source.md`）。
  // ⚠ 与 `.github/workflows/ci.yml` 的对应条目**命令行逐字同源**。
  step("能力域单一真相源自检", "node scripts/check-domain-single-source.mjs --selftest");
  step("能力域（权威枚举↔前端/prompt/i18n 副本）", "node scripts/check-domain-single-source.mjs");

  // 声明表 evidence 的「引用腐烂」校验 —— 与上三条同理：纯 Node、秒级、不依赖构建产物。
  // 为什么需要：`harness::knowledge_graph` 三张声明表（RELATION / ENTITY_TYPE / DATA_DRIVEN_COLUMN）
  //   每条 evidence 都带「文件:行」，这类引用**会随行号漂移腐烂**（改了源码，出处悄悄指向
  //   另一行），而编译器 / clippy / 单测**全都不会报**。实测抓到 4 条指错
  //   （`uses` 曾指向一句 `assert_eq!(…, "mentions")`，`DATA_DRIVEN_COLUMN` 指向一句 `continue;`）。
  // 为什么先跑 --selftest：扫描器自身会撒谎（v1 把「DB 分布 / CSV 行数」这类本就没有
  //   文件:行的证据全判成失败 ⇒ 恒定假红；且只取段内首个 match ⇒ 括注里那条腐烂被静默漏扫）。
  // 为什么正式步用 --ci 而不是裸跑：脚本把「结构腐烂」（文件不存在 / 行号越界 / 指到空行
  //   — 客观错，修法唯一）与「内容与 id 不同源」（软判据，会被「同一件事换措辞」误伤）
  //   分开；CI 只拦前者，后者打印到 stderr 供人复盘。裸跑会因软判据抖动而红，
  //   而它的修法是改判据而不是修缺陷 ⇒ 红久了必被绕过。
  step("声明表 evidence 引用自检（正负对照）", "node scripts/check-decl-evidence.mjs --selftest");
  step("声明表 evidence 引用腐烂（只硬拦结构腐烂）", "node scripts/check-decl-evidence.mjs --ci");

  // 端口公理**写路径接线守卫** —— 与 `.github/workflows/ci.yml` 的
  // 「Check port-axiom write-path wiring」**逐字同源**。
  // 为什么需要：C1 门禁（`enforce_port_axioms`）只在被调用到时才生效，而写路径有 15 处
  //   （DAO 3 + 经 DAO 收口的命令层 3 + 直接落库的种子 9）。**新增一条写路径而忘了接门禁，
  //   不会有任何编译错误** —— `prod-startup-mvp` 的 `s-gonogo` 死链就是这么穿过整个门禁升级的。
  // 为什么先跑 --selftest：守卫自身连撒三次谎（被注释里的关键词骗成「已接」、只枚举
  //   两种模式漏掉裸 `ActiveModel {`、剥注释不剥字符串吞掉整段代码），每次都靠对照才发现。
  step("端口公理写路径接线自检（正负对照）", "node scripts/check-port-axiom-wiring.mjs --selftest");
  step("端口公理写路径接线（防新增写路径漏接）", "node scripts/check-port-axiom-wiring.mjs");

  // input_mapping 源路径门禁 —— 与上几步同理：纯 Node、秒级、不依赖 npm ci / 构建产物 ⇒ 归在最前。
  // 拦的是「源路径写错包裹层」（少一层 `content.` / 段数不够 / 字段名不对）：这类错误在运行期
  //   只是**静默解析为 None**，编译 / clippy / 单测**全都不报**，界面只表现为「某字段恒空」。
  // 为什么先跑 --selftest：本门禁的规则前提**曾被引擎改动静默作废**（v1 有两条夹具断言
  //   「ToolNode 用严格解析器」，而该前提在引擎统一 resolver 时已被删除 ⇒ 规则看似在跑、
  //   实际已失真），故必须先证明它「对已知缺陷形态仍能红」，再相信它的绿。
  // ⚠ 与 `.github/workflows/ci.yml` 的对应条目**命令行逐字同源**（那边一条 step + 两行 run，
  //   本脚本拆成两条 step 以便逐条计时/显错；容器形态不同，两行命令逐字相同）。
  step("input_mapping 源路径自检（正负对照）", "node scripts/check-input-mapping.mjs --selftest");
  // 正式步不带参数：默认扫全仓。退出码 0 = OK / OK-with-baseline / selftest 全过；1 = FAIL。
  step("input_mapping 源路径（包裹层形态/消费端可达/DAG/CodeNode 返回键）", "node scripts/check-input-mapping.mjs");

  // ── 「同一事实的多份载体」门禁（行号引用 + reranker 文件名）──
  // 为什么需要：一个事实写在多处时，机器此前只看住了其中一部分 ——
  //   · 行号引用：`check-decl-evidence.mjs` 只扫 `harness/src/knowledge_graph.rs` 的
  //     `evidence:` 字段；`page_type.rs` 的 7 条 rustdoc 引用**不在扫描面内**，
  //     于是 09-14 起静默腐烂（实测 4 处偏 +10/+3，两次都不是被检查发现的）。
  //   · reranker 文件名：Rust 侧有真源常量 + 跨 crate 绑定测试，前端 2 处字面量却无锁
  //     （全仓 grep 常量名在前端的命中数 = 0）—— 当初就是前端那份缺了 `.Q4_K_M.gguf` 后缀。
  // 为什么 --ci 用于正式步：脚本按判据 #147 把「客观错」（文件不存在 / 行号越界 / 指到空行 /
  //   载体值不等）与「软判据」（出处那一行已不含同句引号值）分开；裸跑在只有软命中时退 3，
  //   而软判据的修法是改判据、不是修缺陷 ⇒ 拿它拦 CI 必被绕过。
  // 覆盖率自陈（判据 #16）：脚本会把 LOCATED / NONLOC（裸 `mod.rs` 之类无法定位）/ EXTERNAL
  //   （第三方 crate 源码）/ BROKEN 四类分开打印，并把未覆盖的引用**点名列出** ——
  //   不列的话「0 腐烂」会被误读成「全都查过了」。
  // 与 `.github/workflows/ci.yml` 的「Check single-source facts」**命令行逐字同源**。
  step("同一事实多份载体自检（正负对照）", "node scripts/check-single-source-facts.mjs --selftest");
  step("同一事实多份载体（行号引用腐烂 / reranker 文件名漂移）", "node scripts/check-single-source-facts.mjs --ci");

  // ── 补两条 CI 早已有、本地镜像却缺的门禁（2026-09-14：漂移修复）──
  // 为什么必须补：它们此前**只挂在 CI**，本地跑 `ci-check` 一路绿 ⇒ 本地绿冒充 CI 绿。
  // 典型后果是「推送后才发现」——本地反馈环里根本看不见这类缺陷。
  // 顺序刻意不同于 `.github/workflows/ci.yml`（那边在 i18n 之后）：本脚本的设计意图是
  //   「纯 Node 的静态门禁放最前，秒级、不依赖 npm ci / 构建产物 ⇒ 最快失败」。
  //   两边**命令行逐字同源**即可，相对顺序不要求一致。
  //   id 校验扫 1091 个源文件约 1 秒，本机实测 38 条 WARN（**警告级不阻提交**，退出 0）。
  step("ID 数据边界（undefined/null 字符串腐化）", "node scripts/check-id-validation.mjs");
  step("后端错误码 i18n 对齐", "node scripts/check-errorcode-alignment.mjs");

  // ── i18n 三道互补门禁（2026-09-14 补：此前本地镜像**一条都没有**）──
  // 为什么必须三道都在：它们查的是**互不重叠**的东西，缺一条就有一种逃逸路径 ——
  //   ① 硬编码文案：代码里直接写死中文字面量（值层查不到，因为压根没进 locale）
  //   ② key 对齐  ：JSON 语法 / zh-CN 空值 / 代码 t() 引用的 key 是否存在
  //   ③ 值未翻译  ：非 CJK locale 的值是否**仍是中文**（key 齐全但值是中文 ⇒ 只有这条能拦）
  // 调用方式刻意走 **node 直调**（`i18n-scan.mjs` / `check-i18n-untranslated.mjs`），
  // 不用 `.sh` 包装 —— Windows 本地下 `bash` 未必在 PATH，会造成「本地跑不起来」。
  // ③ 的自检**内建在每次运行里**（含「豁免既生效又不过宽」对照），无需单独的 --selftest 步骤。
  step("i18n 硬编码文案", "node scripts/i18n-scan.mjs --strict");
  step("i18n 未翻译值（非 CJK locale 不得含汉字）", "node scripts/check-i18n-untranslated.mjs");
  if (hasPython) {
    // ② 是 py 脚本：CI 用 `python3`，Windows 本地用 `python`，故先探测再决定跑/跳过。
    step("i18n key 对齐 + zh-CN 空值", `${pythonBin} scripts/check_i18n.py --strict`);
  } else {
    // ⚠ 明确打出「跳过」而不静默略过 —— 静默略过会让本地绿冒充 CI 绿。
    console.log(
      `  ${"i18n key 对齐 + zh-CN 空值".padEnd(52)}${c.yellow}跳过（未找到 python，**跳过 ≠ 通过**）${c.reset}`,
    );
  }

  step("dprint 格式化检查", "npx dprint check --incremental=false");

  // ⚠️ TS7 临时禁用：typescript-eslint 全版本 peer 上限 typescript <6.1.0，
  // 在 TS 7 下 typescript-estree 读已变更内部 API 会崩（TypeError ... 'Cjs'）。
  // 待 typescript-eslint 发 TS7 支持版后，恢复下方 ESLint 检查。
  // if (!quick) {
  //   step("ESLint 检查", "npx eslint src --max-warnings=0");
  // }

  step("TypeScript 类型检查", "npx tsc --noEmit");

  if (!quick && !frontendOnly) {
    step("Vitest 单元测试", "npx vitest run");
  }
}

// ── Rust 检查 ────────────────────────────────────────────
if (canRunRust && !quick) {
  console.log(`\n${c.bold}[Rust 检查]${c.reset}`);

  // 补一条 CI 早已有、本地镜像却缺的 Rust 侧门禁（2026-09-14：漂移修复）。
  // CI 把它放在 `rust-check` job 的 `cargo check` **之后**（`ci.yml:230`）；
  // 本地放最前 —— 它是纯 Node 扫源码、秒级、不依赖编译产物，先跑才能最快失败。
  // 拦的是「裸 map_err(|x| x.to_string())」：错误信息退化成自由文本，
  // 前端拿不到错误码（与 check-errorcode-alignment 是同一契约的两端）。
  step("后端裸 map_err 检查（错误码契约）", "node scripts/check-rust-raw-map-err.mjs");

  step(
    "cargo fmt 格式化检查",
    "cargo fmt --check --all",
    { cwd: srcTauri, env: { ...process.env } },
  );

  step(
    "cargo clippy (deny warnings)",
    "cargo clippy --all-targets --all-features -- -D warnings",
    { cwd: srcTauri, timeout: 10 * 60 * 1000, env: { ...process.env } },
  );

  step(
    "cargo test 单元测试 (quant + analysis-engine + harness + tools)",
    "cargo test -p axagent-quant -p axagent-analysis-engine -p axagent-harness -p axagent-tools --lib 2>&1 || cargo test -p axagent-quant -p axagent-analysis-engine --lib",
    { cwd: srcTauri, timeout: 10 * 60 * 1000, env: { ...process.env } },
  );

  // 中文全文检索的「两侧同构」校验：索引侧由 PG 生成列算（ax_cjk_ngram），
  // 查询侧由 Rust 算（text_ngram::cjk_ngram）。两侧对同一输入产出不同 token 串时
  // **静默失配** —— 索引里有、查询取不到，且 cargo check 与 Rust 单测全绿。
  // 需要可连的 PG 与 pg 驱动（后者刻意不是项目依赖）：缺任一时脚本诚实 SKIP
  // （退出 0 + 打印原因，明确写着「跳过不代表通过」），故本步不会在无 PG 环境误报。
  step(
    "中文 ngram 两侧一致性 (Rust ↔ PG)",
    "node scripts/check-ngram-consistency.mjs",
    { env: { ...process.env } },
  );
}

// 快速模式中的 Rust 格式化
if (canRunRust && quick) {
  console.log(`\n${c.bold}[Rust 快速检查]${c.reset}`);
  step(
    "cargo fmt 格式化检查",
    "cargo fmt --check --all",
    { cwd: srcTauri, env: { ...process.env } },
  );
}

// ── 结果 ──────────────────────────────────────────────────
const elapsed = ((Date.now() - startedAt) / 1000).toFixed(1);
console.log(
  `\n${c.bold}${failures === 0 ? c.green : c.red}${
    failures === 0 ? "全部检查通过!" : `${failures} 项检查失败`
  }${c.reset} (耗时 ${elapsed}s)\n`,
);

process.exit(failures > 0 ? 1 : 0);
