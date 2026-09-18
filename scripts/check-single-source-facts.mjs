// 「同一事实的多份载体」门禁 —— 两类载体，两套判据。
//
// 背景（PLAN-weknora-borrowings §12.7#4 / §12.10）：仓库里存在「一个事实写在多处」
// 的形态，而机器只看住了其中一部分：
//
//   ① **行号引用**（`文件:行` 写在 rustdoc / 行内注释里）
//      `check-decl-evidence.mjs` 只扫 `harness/src/knowledge_graph.rs` 三张声明表的
//      `evidence:` 字段；`page_type.rs` 的 7 条 rustdoc 引用**不在**它的扫描面内，
//      于是 09-14 起静默腐烂（实测 4 处偏 +10/+3，**两次都不是被检查发现的**）。
//   ② **reranker 模型文件名**
//      Rust 侧已是真真源（`harness::rag_config::RERANKER_MODEL_FILENAME`），且
//      `search::model_downloader` 有跨 crate 绑定测试钉住「下载清单 == 类型默认值」；
//      但前端 2 处字面量（`settingsStore.ts` / `KnowledgeBaseDocuments.tsx`）
//      **无锁** —— 全仓 grep 常量名在前端零命中 ⇒ 当前值虽已修对，可再次漂移
//      （当初就是前端那份缺了 `.Q4_K_M.gguf` 后缀，指向一个永不存在的文件）。
//
// 设计依据（判据 **#147** 门禁里的软判据不得硬拦）：
//   两类判据必须分开，且**都真的存在**（不能只有一类，否则「分流」是空话）：
//     · 客观错（修法唯一，**硬拦**，退出 1）：文件不存在 / 行号越界 / 行号为 0 /
//       指向空行 / `RERANKER_MODEL_FILENAME` 缺失 / `crossEncoderModel` 的字面量与
//       真源不等 / 扫描面一条都没扫到（= 脚本自己坏了）。
//     · 软判据（会随无关编辑抖动，**只报告**，退出 3）：引用出处那一行「不再含有
//       同句引号内的值」（= 疑似腐烂，需人判）。`--ci` 把它降级为「打印不失败」。
//   为什么软的不硬拦：它的假阳性来自「同一件事换了措辞」，会随无关编辑抖动；
//   硬拦 ⇒ 红灯只能靠改判据来灭 ⇒ 必被绕过（这条已沉淀为判据 #147）。
//
//   ③（2026-09-15 追加）**引用指向裸控制流语句**（`continue;` / `break;` / `return;`）
//      ⇒ **客观错**。理由见下「为什么这条可以硬拦」。
//
// 为什么这条可以硬拦（判据 #147 的硬判据门槛 = 「修法唯一 + 不随措辞抖动」）：
//   2026-09-15 给 `commands/knowledge.rs` 的 `graph_import` 加一个参数（**文件整体 +4 行**），
//   把 4 处指向该文件的引用全部推移：`knowledge_graph.rs` 的 `:489/:498/:533/:539`。
//   其中只有 `:577`（`DATA_DRIVEN_COLUMN.evidence`）被 harness 单测
//   `test_csv_import_relations_are_declared` 硬拦并修好；另外 3 条 `evidence:` 只走
//   `check-decl-evidence.mjs` 的**软**通道（`--ci` 降级为打印不失败）⇒ **静默腐烂**。
//   全仓实测（1916 个 .rs / 445 条引用 / 171 条可定位）：**指向裸控制流语句 7 处，7/7 全是真腐烂**
//   —— 一条 `continue;` 不可能是任何「声明 / 关系 / 实体类型」的证据，修法唯一（改行号）。
//   同一次实测里「指向收尾括号（`}` `},` `);`）」共 5 处，其中 3 处是**区间终点**
//   （`:1913-1916` / `:19-23` 的 `},` / `}`）、2 处是「整段」的松散写法 ⇒ **需人判**
//   ⇒ 保持软判据。**同一形态按信噪比分两档**，而不是一刀切 —— 这才是 #147 的原意。
//
// 用法：
//   node scripts/check-single-source-facts.mjs
//   node scripts/check-single-source-facts.mjs --selftest
//   node scripts/check-single-source-facts.mjs --ci                 # CI：只拦客观错
//   node scripts/check-single-source-facts.mjs --update-baseline    # 仅在**减少**违规后下调
//
// 退出码：0 全好 ｜ 1 客观错（真失败）｜ 2 脚本自身失效 ｜ 3 只有「疑似腐烂」需人判
//
// 覆盖范围（自陈，判据 #16）：
//   扫 `src-tauri/src/**/*.rs`、`src-tauri/crates/*/src/**/*.rs` 与
//   **`src-tauri/crates/*/tests/**/*.rs`**（2026-09-17 扩容，用户裁决；排除 `target/`）。
//   `tests/` 面加进来的理由：那里同样写着「此声明的出处是 `文件:行`」，不扫就是**未检查**；
//   而「未检查」在输出里与「检查且通过」长得一样（判据 #7：`0 命中 ≠ 没问题`）。
//   扩容成本已量化：该面 48 个 `.rs`、只有 2 条引用且**全红**（指向已删迁移文件），
//   修掉后 **0 条 / 0 新红** ⇒ 零成本。
//   **不扫**：Markdown 文档（PLAN / AGENTS.md 里的 `文件:行` 属散文，抖动大，
//   硬拦会逼人删登记项）、`scripts/*.mjs`（自带合成样本）、前端 `.ts/.tsx` 里的行号引用。
//   前端只覆盖 ②（reranker 文件名），因为那是「同一事实多载体」里唯一有 Rust 真源的。
//   **跑得动 ≠ 覆盖到了**：每次运行都会打印四分法计数（LOCATED / NONLOC / EXTERNAL /
//   BROKEN），并把 NONLOC 逐条点名 —— 那批引用**根本没被检查**，不是「检查通过」。
//
//   ⚠ 2026-09-17 实测（**1926 个 .rs** ⇒ 535 条引用，去重 433；含扩容后的 `tests/` 面）：
//     LOCATED 390 ｜ NONLOC 0 ｜ EXTERNAL 43 ｜ BROKEN/越界/空行/控制流 **0** ｜ 软判据 **0**。
//     EXTERNAL 已从「全部免检」升级为**真验证**（版本对 `Cargo.lock`、路径对 cargo registry）：
//     版本匹配 43 ｜ **版本陈旧 0** ｜ 路径缺失 0 ｜ 未验证 0。
//     （历史读数：`1878 个 .rs / 564 条 / 458 去重`、`LOCATED 315 ｜ NONLOC 117 ｜ EXTERNAL 26`。
//      那两个数字已随 `.worktrees` 进 `SKIP_DIRS` 与一批引用补路径而作废。
//      同 J 组：注释里的数值先腐烂。）
//     当日把 117 条 NONLOC 逐条分诊过，三摞的**当前状态**：
//       · **98 条是裸文件名**（`lib.rs` / `mod.rs` / `state.rs`…）—— 引用串里**没有任何
//         路径可对账**，机器无从判定它指哪一个 ⇒ 只能人判。**已逐条补路径 ⇒ 现 NONLOC 0**。
//       · **22 条带路径但仓内仍多命中**，其中约 14 条其实是第三方 crate 源码
//         （`sea-orm-2.0.2/src/schema/entity.rs`、`rhai-1.26.0/src/engine.rs`）。
//         ⚠ 这是本脚本曾有过的**判据缺口**：`EXTERNAL` 判定只挂在 `loc.kind === "B"`
//         分支上，而这类引用因 basename 多命中落到 `N` 分支 ⇒ **永不进入 EXTERNAL 判定**、
//         被误报成「未检查」。**已修**：该判定被提到 `locate()` **之前** —— 它是与「能否
//         在本仓定位」**正交**的一件事（由出处自己写的 `name-x.y.z/` 决定），挂在任何一个
//         `locate()` 结果分支里，都等于把生效条件绑在一个无关变量上。
//       · ✅ `output/` **已进 `SKIP_DIRS`**（2026-09-17 用户裁决）：`output/backup-*/` 与
//         `output/tmp-*-src/` 装的是**整棵树的副本**，会把 basename 撑成「同名 N 份」，
//         让本该能定位的引用落进 NONLOC（= 从未检查）。实测加进去后 LOCATED 296→375、
//         NONLOC 137→35（**132 条从「从未检查」变成「真检查」**），代价是多出 43 条
//         「引用了已删除迁移文件」的 provenance 注释 —— 那批按「裸版本号」范式收敛，
//         **不靠白名单**（白名单会把它一起豁免掉，反思见 SKILL §36 第 2 条）。
//
// ⚠ 能力边界（**实测踩到，勿高估本脚本**）：本脚本判的是**结构可解析性**（文件存在 /
//   行号在范围内 / 该行非空）+ **一条形态代理**（该行是不是裸控制流语句，见上 ③）。
//   它**仍然判不了「那一行是否还在讲同一件事」** —— 行号漂移后若落在一个**正常代码行**
//   上，本脚本照样报绿。真实实例（2026-09-15）：给 `commands/knowledge.rs` 加一个函数
//   参数（文件 +4 行）之后 `knowledge_graph.rs` 的 4 处引用整体推移（`:489/:498/:533/:539`）。
//   其中落成 `continue;` 的那几处**加了 ③ 之后能拦**；落成 `}` 的、以及漂到别的正常代码
//   行的，本脚本**依然看不见** —— 那种只能靠人写死的语义判据去拦
//   （`check-decl-evidence.mjs` 的「内容存疑」+ harness 单测
//   `test_csv_import_relations_are_declared`，当初抓到它的是后者）。
//   成因（为什么当时软判据一条都没响）：软判据只比对**引号包裹的字面量**（见 `quotedTokens`），
//   而 evidence 串里的引用含 `/` 被刻意排除（路径形态不参与比对）⇒ 该形态下软判据无从触发。
//   分工：**结构 + 控制流形态代理**归本脚本；**语义**归 `check-decl-evidence.mjs` + harness
//   单测。三者不可互相替代 —— 删任何一个都会把对应盲区放回来。
//
//   ⚠ EXTERNAL 的**真验证也是分层的**（2026-09-17 加，理由见 `verifyExternal`）：
//     版本陈旧 ⇒ **硬拦**（只读仓库内的 `Cargo.lock`，**离线也成立**，不依赖 registry 缓存）；
//     路径缺失 ⇒ **只报告**（那个 crate 未必下载过，硬拦会在离线 / 首次 clone 机器上假红）；
//     registry 根不可用 ⇒ **整条跳过并在输出里自陈**，不伪装成「检查过且通过」。
//     起因：原先该桶「全部免检」，而 registry **长期保留历史版本** ⇒ 指向 `sea-orm-2.0.1`
//     的路径照样解析成功，但它证明的已**不是** lock 里那份代码（判据 #491）。
//
// 棘轮（ratchet，判据 #16）：客观错里**存量修不动的**走基线白名单，只拦新增。
//   基线**只减不增**；修复后用 `--update-baseline` 下调（基线虚高 = 棘轮失效）。
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.resolve(HERE, "..");
const SRC = path.join(ROOT, "src-tauri");
const BASELINE_PATH = path.join(HERE, "single-source-facts-allowlist.json");

const CI_MODE = process.argv.includes("--ci");
const UPDATE_BASELINE = process.argv.includes("--update-baseline");
// 取证模式：只输出 NONLOC 的**引用处行原文 + 候选清单**，供「逐条补路径」施工。
// 存在的理由：补路径这件事必须基于**与本门禁同源**的引用集合。另写一个抽取器去凑清单，
// 凑出来的集合与被检查的集合不是同一个（区间写法 `:24-26` 会漏、行号会错位），
// 于是「补完了」与「门禁不再报」两件事之间没有因果关系 —— 判据 #16 的自陈覆盖范围。
const DUMP_NONLOC = process.argv.includes("--dump-nonloc");
// 取证模式：**活文档**（`.md`）里的「文件:行」引用体检。
//
// 为什么需要它：本门禁的扫描面 `collectRustFiles()` **只收 `.rs`** ⇒
// `docs/**/*.md`、根级 `*.md` 里的行号引用是**盲区**（2026-09-17 登记，报告 §7 第 11 项）。
// 「未检查」在输出里与「检查且通过」长得一样（判据 #7）⇒ 至少要让缺口**可被看见**。
//
// ⚠ 它与 `--dump-nonloc` 的性质不同，两者**不要合并**：
//   · `--dump-nonloc` 服务的是「把 NONLOC 补成 LOCATED」⇒ 目标是**让主判据能跑**；
//   · 本开关服务的是「先量化存量成本再决定要不要立判据」⇒ 目标是**给人判入口**。
// ⇒ 因此它**不产出任何 failure / suspect**，也**绝不接进主流程退出码**（见 `classifyDocRef`
//   的注释：`.md` 里大量引用是**故意保留的历史行号**，机器分不出「陈旧」与「考古」）。
const DUMP_DOC_REFS = process.argv.includes("--dump-doc-refs");

const rel = (p) => path.relative(ROOT, p).split(path.sep).join("/");

// ── ① 行号引用 ────────────────────────────────────────────────────────
//
// 只认「带扩展名的路径 + `:行号`」。**扩展名白名单是刻意的**：不设它的话，
// `13:43`（时刻）、`HashMap<String,u32>` 之类会把扫描面淹掉 —— 而淹掉之后
// 没人看输出，于是门禁等于不存在（判据 #7：审计脚本自身会撒谎，先保证信噪比）。
const EXTS = "rs|ts|tsx|js|mjs|sh|json|toml|md|rhai|py|sql|yml|yaml";
// 行号允许 `:N`、`:N/M`（同一文件多行，如 `wiki_compiler.rs:671/782`）、`:N-M`（区间）
const LINE_LIST = String.raw`\d+(?:[/-]\d+)*`;
const FILE_ANCHOR = new RegExp(
  String.raw`([A-Za-z0-9_][A-Za-z0-9_./-]*\.(?:${EXTS}))\s*:\s*(${LINE_LIST})`,
  "g",
);
// 裸 `:行号`（承接同一段里前一个文件名）。负向先行断言排除 `13:43` 这类时刻：
// 时刻的 `:` 前面是数字，而引用写法的 `:` 前面是标点 / 空白 / 文件名。
const BARE_ANCHOR = new RegExp(String.raw`(?<![A-Za-z0-9]):\s*(${LINE_LIST})`, "g");

/** 扫描面：src-tauri 下所有 .rs，排除 target/ 与 node_modules/ */
function collectRustFiles() {
  const out = [];
  const skip = new Set(["target", "node_modules", ".git"]);
  const walk = (dir) => {
    let entries;
    try {
      entries = fs.readdirSync(dir, { withFileTypes: true });
    } catch {
      return;
    }
    for (const e of entries) {
      if (skip.has(e.name)) continue;
      const p = path.join(dir, e.name);
      if (e.isDirectory()) walk(p);
      else if (e.isFile() && e.name.endsWith(".rs")) out.push(p);
    }
  };
  walk(path.join(SRC, "src"));
  for (const c of fs.readdirSync(path.join(SRC, "crates"), { withFileTypes: true })) {
    if (!c.isDirectory()) continue;
    walk(path.join(SRC, "crates", c.name, "src"));
    // ── 2026-09-17 扩容（用户裁决）：纳入 `crates/*/tests/` ────────────────
    // 理由：`src/` 之外同样有「此声明的出处是 `文件:行`」的注释，不扫它们就是**未检查**；
    // 而「未检查」在输出里与「检查且通过」长得一样（判据 #7：`0 命中 ≠ 没问题`）。
    // 扩容前已量化成本：该面 48 个 `.rs`、只有 **2 条**引用且**全红**
    // （`knowledge_graph_search.rs:21` 指向已删迁移文件）—— 那条已按「裸版本号」范式修掉
    // ⇒ 扩容后 0 条引用、0 条新红（零成本）。
    // 目标文件侧的候选索引不受影响：`getBasenameIndex()` 本就 walk 整个 ROOT。
    walk(path.join(SRC, "crates", c.name, "tests"));
    // ── 2026-09-17 第八轮扩容（用户裁决：按建议值处置「剩余未动项」）────────────
    // 纳入 `crates/*/examples` + `crates/*/benches`。成本**先量后扩**，且按判据 #487
    // 用**本门禁自己的 `extractRefs`** 量（面一改，下面那次主流程运行就是那个量）：
    //   · `benches/` 3 个 `.rs` ⇒ **0 条**引用（纳入是为封住整类，成本为 0）；
    //   · `examples/` 7 个 `.rs` ⇒ **4 条**引用，逐条核过 **4/4 可定位且语义正确**。
    // 收尾读数见本文件运行输出与报告 §4.2 收口行（扩容前后差值是唯一判据）。
    walk(path.join(SRC, "crates", c.name, "examples"));
    walk(path.join(SRC, "crates", c.name, "benches"));
  }
  // 顶层集成测试面。当前不存在，但 walk 对缺失目录是**安全 no-op** ——
  // 写在这里是为了「路径约定变化时被覆盖」，而不是静默漏掉一整类引用来源。
  walk(path.join(SRC, "tests"));
  // 顶层 `examples/` + `benches/`（同样安全 no-op：`src-tauri/examples` 实测不存在）。
  walk(path.join(SRC, "examples"));
  walk(path.join(SRC, "benches"));
  return out.sort();
}

/**
 * 从一段文本里抽出全部引用。纯函数 ⇒ 可被 --selftest 复用（判据 #147②）。
 * @returns {{file:string, line:number, text:string}[]} `text` = 该引用所在的整行
 */
export function extractRefs(text) {
  const out = [];
  for (const [srcLine, raw] of text.split(/\r?\n/).entries()) {
    // 两个正则都跑在**整行**上，再按 index 归并 —— 必须整行，不能切片：
    // 切片会让负向先行断言丢掉左边界上下文，`:4:5`（cargo 诊断的 `行:列`）里
    // 第二个 `:` 会因为在切片里处于是「字符串开头」而被误当成裸引用，
    // 于是 `--> src/main.rs:4:5` 被读成 `src/main.rs:5`（实测踩到，判据 #7）。
    const toks = [];
    for (const a of raw.matchAll(FILE_ANCHOR)) {
      toks.push({ i: a.index, e: a.index + a[0].length, file: a[1], lines: a[2] });
    }
    for (const b of raw.matchAll(BARE_ANCHOR)) {
      toks.push({ i: b.index, e: b.index + b[0].length, file: null, lines: b[1] });
    }
    toks.sort((x, y) => x.i - y.i);

    let lastFile = null;
    let covered = -1;
    for (const t of toks) {
      if (t.i < covered) continue; // 落在前一个带文件名匹配的内部
      covered = t.e;
      if (t.file) lastFile = t.file;
      else if (!lastFile) continue; // 裸引用但本行还不知道文件名 ⇒ 无从判定
      const f = t.file ?? lastFile;
      // `:A/B` = 多行清单（每行都要非空）；`:A-B` = 区间（**只判起点非空**，
      // 终点允许落在空行 —— 实测 `:64-70` 的 70 就是一个空行，硬判它恒定假红）
      for (const part of t.lines.split("/")) {
        const mm = part.match(/^(\d+)(?:-(\d+))?$/);
        if (!mm) continue;
        // `col` / `end` = 该引用 token 在**本行**里的列区间。软判据用它把「引号值」归给
        // 文本上最近的那条引用（同一行常有多条引用，见 `quotedTokensOwnedBy`）。
        out.push({
          file: f,
          line: Number(mm[1]),
          text: raw,
          srcLine,
          col: t.i,
          end: t.e,
          blankOk: false,
        });
        if (mm[2]) {
          out.push({
            file: f,
            line: Number(mm[2]),
            text: raw,
            srcLine,
            col: t.i,
            end: t.e,
            blankOk: true,
          });
        }
      }
    }
  }
  return out;
}

/**
 * 注释块上下文：一个引用所在的「连续 `//` 注释块」。
 *
 * 用途：判定该引用是否在讨论**第三方 crate 源码**（`rhai-1.26.0/src/…`、
 * `sea-orm-2.0.1/src/…`）。这类引用**本来就不该在本仓解析**，判成 BROKEN 是纯假红。
 * 判据取「块级」而非「行级」：实测 `seed_content_media.rs:867-876` 是一个 doc 块，
 * 第 867 行写了 `rhai-1.26.0/src/engine.rs`，而第 869 / 876 行只写
 * `packages/pkg_std.rs:21-31` / `array_basic.rs:59`（路径被省略）—— 行级判据必然漏。
 */
export function commentBlocks(lines) {
  const ids = new Array(lines.length).fill(-1);
  const texts = [];
  let cur = -1;
  for (let i = 0; i < lines.length; i++) {
    if (lines[i].trimStart().startsWith("//")) {
      if (cur === -1) {
        cur = texts.length;
        texts.push([]);
      }
      ids[i] = cur;
      texts[cur].push(lines[i]);
    } else {
      cur = -1;
    }
  }
  return { ids, texts: texts.map((t) => t.join("\n")) };
}

/** 第三方 crate 源码的版本目录形态：`rhai-1.26.0/`、`sea-orm-2.0.1/` */
export const EXTERNAL_MARK = /[A-Za-z0-9_.-]+-\d+\.\d+\.\d+[/"`]/;

/**
 * 第三方 crate 源码的**完整路径**形态（版本目录 + 其后路径）：
 * `sea-orm-2.0.2/src/schema/entity.rs`、`rhai-1.26.0/src/engine.rs`。
 *
 * 为什么必须取完整路径、而不能只看「版本目录」：见 [`externalMarkFor`]。
 */
export const EXTERNAL_PATH = /[A-Za-z0-9_.-]+-\d+\.\d+\.\d+\/[A-Za-z0-9_./-]+/g;

/**
 * 该文本里是否存在**与本引用指向同一目标**的第三方路径标记。
 *
 * ⚠ 2026-09-17 修（**我自己引入的假绿**，实测代价见下）：上一版把判定写成
 *   「块里只要出现过 `name-x.y.z/` 就算本块全部引用都是外部」⇒ 判据太粗：
 *   一个注释块里往往**同时**讨论「第三方源码」与「本仓文件」，块级标记无法区分，
 *   于是本仓那几条**借别人的标记免检**。实测 `entities/src/*.rs` 的 5 条
 *   「出处是 `migrations/v210_opc_ext.rs:25`（该迁移已删）」全被同一块里
 *   `sea-orm-2.0.2/src/schema/entity.rs:156` 的标记吞掉，`EXTERNAL` 桶由 42 涨到 54、
 *   而 `BROKEN` 少了 5 ⇒ **读数变好看，问题没少**。
 *
 * 正确判据是**同一目标**：块里的第三方路径 `p` 必须以「`/` + 引用串」结尾
 *   —— `schema/entity.rs` ↔ `sea-orm-2.0.2/src/schema/entity.rs` ✅ 同目标
 *   —— `migrations/v210_opc_ext.rs` ↔ 上面那条 ❌ 不同目标 ⇒ 不得免检
 * 即「块级标记只能证明它自己那条引用是外部的」（同族教训：判据 #485 —— 别让一条判据
 * 的生效范围被无关条件放大的同时也被无关条件缩小）。
 *
 * @param {string} text 注释块文本（或单行文本）
 * @param {string} refFile 引用串里的文件路径（如 `schema/entity.rs`）
 */
export function externalMarkFor(text, refFile) {
  for (const m of text.matchAll(EXTERNAL_PATH)) {
    const p = m[0];
    if (p === refFile || p.endsWith("/" + refFile)) return true;
  }
  return false;
}

/**
 * 与 [`externalMarkFor`] **同一轮匹配**，但返回**匹配到的路径串**本身
 * （`sea-orm-2.0.2/src/schema/entity.rs`）而不是布尔。真验证需要那个串。
 */
export function externalPathFor(text, refFile) {
  for (const m of text.matchAll(EXTERNAL_PATH)) {
    const p = m[0];
    if (p === refFile || p.endsWith("/" + refFile)) return p;
  }
  return null;
}

// ── 活文档 BROKEN 的**细分**（2026-09-18 新增，用户裁决「豁免位的门禁改造」）────
//
// 为什么必须细分 —— 实测（把 A 栏 115 条 BROKEN 逐条摊开）：
//   · **66 条**（`.py` 63 / `.yaml` 1 / `.md` 1 / `.yml` 1）全部落在三份对比审计文档
//     （`PLAN-{evoflow,semantica,weknora}-borrowings.md`），引用的是**外部仓库源码**
//     （EvoFlow / Semantica / WeKnora 的 Python 栈）—— 本就不该在本仓解析；
//   · **49 条**（`.rs`）全部落在 schema / migration 文档，指向**本仓已删文件**
//     （`v207_chat_run.rs` 已删迁移、`src/divergence-log.rs` 整链删除、
//      `stock_analysis_setup.rs` 已拆成同名目录）—— 引用是**考古**，无法验证也不该验证。
// 三者混在同一数字里，「115 条 BROKEN」就不能指向任何行动（判据 #7 同族：读数不给
// 行动面 = 等于不存在）。细分后每一条都有一个明确的下一步。
//
// ⚠ 判据的**生效边界靠结构保证、不靠自觉**（判据 #485 的教训：块级标记曾把本仓引用
//   一起免检 —— `EXTERNAL` 42→54 而 `BROKEN` −5，读数变好看、问题没少）：
//   `classifyBroken` **只在 `locate()` 返回 B 时被调用**。只要本仓存在同名文件，
//   `locate` 就返回 L/N，**根本走不到这里** ⇒「豁免位吞掉本仓引用」结构上不可能发生。
//   另加一道显式护栏：扩展名 ∈ 本仓主力栈（`OWN_STACK`）时**一律不许**按外部豁免。
export const BORROWINGS_DOC = /^docs\/plans\/PLAN-([a-z0-9_]+)-borrowings\.md$/;
/** 文档自陈「这是外部项目的参考清单」的证据行 —— 三份文档各自的实测措辞。 */
export const EXTERNAL_DECL = /参考边界|非代码移植|不搬运代码|重新实现|许可[：:]/;
/** 本仓主力技术栈：命中者不许按「外部仓库」豁免（护栏，见上）。 */
export const OWN_STACK = /\.(rs|ts|tsx|js|mjs|cjs|json|toml|rhai|sql|sh|css|html)$/i;
/** 已删迁移文件的命名形态（`v207_chat_run.rs`）。 */
export const DELETED_MIG = /^v\d{3}_[a-z0-9_]+\.rs$/;
/** 引用句**自陈「该文件已不在」**的措辞 —— 台账句，引用本身就在记录它没了。 */
export const GONE_SAID =
  /已删|已移除|已退休|已拆成|已搬迁|已废弃|原文件|曾(?:存在|在)|已(?:于[^，。；）)]{1,24})?(?:删除|移除|退休|拆成|搬迁|废弃)/;
/** 文档头部取多少行找「参考边界」声明（实测三份都在前 10 行）。 */
export const DECL_SCAN_LINES = 60;

/**
 * 「**段内截断**」检测 —— 纯函数，便于 `--selftest` 复用（2026-09-18，用户裁决「文档简称」规范后落地）。
 *
 * 问题形态：文档把 `opc_domain_pack_actions.rs` 写成 `actions.rs`（砍掉中间段）。
 *   · `locate()` 的后缀消歧要求**路径段对齐**（`norm(p).endsWith("/" + want)`）⇒
 *     `"…/opc_domain_pack_actions.rs".endsWith("/actions.rs")` 为 **false** ⇒ 永不命中；
 *   · 但它**确实是**真实 basename 的**字符级后缀**（`"opc_domain_pack_actions.rs".endsWith("actions.rs")` 为 true）
 *     ⇒ 于是落 BROKEN，与「真的引用了不存在的文件」混进同一个「真待查」桶。
 * 实测代价（2026-09-18）：`BROKEN（真待查）` 12 条里 **11 条**是这种写法（同一份文档）⇒ 该桶指向不了真实风险。
 *
 * ⚠ 反向护栏（必须有，否则又是一个「猜」）：只在候选 basename **严格更长**且 `endsWith` 时才算 ——
 *   等长命中（= 同一个名字）不算截断，那是普通的重名 / 缺路径问题，归 NONLOC 或正常定位。
 * ⚠ 本判据**只改分类标签**：仍记 BROKEN、仍进 `miss` 计数、退出码不动（细分只给行动面）。
 *
 * @param {string} refFile 引用串原样
 * @param {Map<string, string[]>} index basename 索引（`getBasenameIndex()` 的产物）
 * @returns {string[]} 疑似被截断的真实文件（仓库相对路径，已排序）
 */
export function abbrevHitsFor(refFile, index) {
  const b = refFile.slice(refFile.lastIndexOf("/") + 1);
  if (!b || !index) return [];
  const out = [];
  for (const [name, arr] of index) {
    if (name.length > b.length && name.endsWith(b)) for (const p of arr) out.push(rel(p));
  }
  return out.sort();
}

/**
 * 把一个 **BROKEN**（本仓无此文件）引用细分。**纯函数**，便于 `--selftest` 复用。
 *
 * 返回值语义（**都仍是 BROKEN**，细分只是给行动面，不是洗白）：
 *   · `ALLOWLISTED`  —— 命中显式名册 `scripts/external-repo-refs.json`
 *   · `DELETED-SRC`  —— 本仓已删 / 已搬迁源文件（考古引用）
 *   · `EXT-REPO`     —— 外部仓库源码（对比审计文档的外部项目）
 *   · `ABBREV`       —— **段内截断**（真实 basename 的字符后缀，但非路径段对齐 ⇒ 机械不可解析）
 *   · `BROKEN`       —— 其余（**真待查**）
 *
 * @param {{docRel:string, refFile:string, refText:string, docHead:string, allow:Set<string>,
 *          abbrevHits?:string[]}} ctx
 */
export function classifyBroken(ctx) {
  const { docRel, refFile, refText, docHead, allow, abbrevHits = [] } = ctx;
  if (allow && allow.size > 0 && allow.has(`${docRel}::${refFile}`)) return "ALLOWLISTED";
  const base = refFile.slice(refFile.lastIndexOf("/") + 1);
  // ① 本仓已删/搬迁：文件名形态（迁移版本号）或引用句自陈 ⇒ 先于外部判定，
  //    因为「borrowings 文档里也可能引用本仓已删文件」（实测 weknora 文档有 5 条）。
  if (DELETED_MIG.test(base) || GONE_SAID.test(refText)) return "DELETED-SRC";
  // ② 外部仓库源码：文档是对比审计文档 + 头部自陈参考边界 + 扩展名不在本仓主力栈。
  if (BORROWINGS_DOC.test(docRel) && EXTERNAL_DECL.test(docHead) && !OWN_STACK.test(refFile)) {
    return "EXT-REPO";
  }
  // ③ 段内截断（`actions.rs` ← `opc_domain_pack_actions.rs`）：见 `abbrevHitsFor`。
  //    放在 EXT-REPO **之后**：文档级的「这是外部仓库源码」是更强的断言，先认它。
  if (abbrevHits.length > 0) return "ABBREV";
  return "BROKEN";
}

/**
 * 把名册条目切成两个**互不相通**的键集 —— 纯函数，便于 `--selftest` 复用。
 *
 * 为什么必须分两集（2026-09-18 第三轮，**实测事故**，不是预防性设计）：
 *   原先只有文件级键 `${doc}::${refFile}`，**BROKEN 与 NONLOC 共用它**。于是
 *   `PLAN-weknora-borrowings.md::wiki.rs` 这一条（理由写的是「`:1003` 那处是门禁演进史举例」）
 *   把同文档的 `:620`（历史 clippy 台账）与 `:651`（已裁决残留）**一并豁免**了 ——
 *   `ALLOWLISTED` 从应然的 1 条变成 3 条，而证据只解释了其中 1 条。
 *   这正是豁免位的唯一失败模式：**只会变松**（与 #485 的块级标记同族）。
 *
 * 结构性修法（结构保证，不靠自觉）：
 *   · 带 `lines` 的条目 ⇒ 进 `lineKeys`（键含**被引行号**）、**只对 NONLOC 生效**；
 *   · 不带 `lines` 的条目 ⇒ 进 `fileKeys`、**只对 BROKEN 生效**。
 *   调用点各接一条判据 ⇒ **没有任何一条路径能让文件级键豁免 NONLOC**。
 *   【为何 BROKEN 保持文件级】被引文件**根本不存在** ⇒ 「为何不需验证」这件事只关于文件、
 *   与行号无关；一次豁免整族是可接受的（整份文件都无从验证）。
 *   【为何 NONLOC 必须行级】文件**存在** ⇒ 同文档里对它的另一次引用**是崭新的待查引用**，
 *   拿「某一行是举例」去豁免整个文件，就是拿一条证据去解释它没解释过的东西。
 *
 * 为什么 NONLOC 用「**被引行号**」而不是「文档行号」：
 *   `.md` 侧去重键就是 `${doc}::${file}:${line}`（见 audit 内 `seenDoc`）⇒ **同键同粒度**，
 *   名册的点数恒等于该条目能解释的引用条数；且文档行号会随文档增删漂移（那是会腐烂的锚），
 *   被引行号写在引用串里、不随文档移动。
 *
 * ⚠ 失效方向（刻意的）：键对不上 ⇒ **不豁免** ⇒ 落到 `OTHER` 被点名出来。
 *   行号写错只会让名册**失去效力**（可见），永远不会静默放宽。
 * ⚠ 反向护栏：写了 `lines` 但**一个合法行号都没有**（拼成字符串 / 0 / 负数）⇒
 *   既不进 `lineKeys`、**也不退回 `fileKeys`**（退回 = 手滑一个字就变成整族豁免），
 *   而是进 `bad` 由调用方报出来。
 *
 * @param {{doc?:string, refFile?:string, lines?:unknown}[]} entries
 * @returns {{fileKeys:Set<string>, lineKeys:Set<string>, bad:string[]}}
 */
export function splitAllowlist(entries) {
  const fileKeys = new Set();
  const lineKeys = new Set();
  const bad = [];
  for (const e of Array.isArray(entries) ? entries : []) {
    if (!e?.doc || !e?.refFile) continue;
    const k = `${e.doc}::${e.refFile}`;
    if (!Object.prototype.hasOwnProperty.call(e, "lines")) {
      fileKeys.add(k);
      continue;
    }
    const ls = (Array.isArray(e.lines) ? e.lines : []).filter((n) => Number.isInteger(n) && n > 0);
    if (ls.length === 0) {
      bad.push(`${k}（lines 无合法行号 ⇒ 零效力；**不放宽**为文件级）`);
      continue;
    }
    for (const n of ls) lineKeys.add(`${k}::${n}`);
  }
  return { fileKeys, lineKeys, bad };
}

/** 显式豁免名册（人工兜底）。读不到时**返回空集并把原因带出来**，不静默假装有。 */
const EXT_REPO_ALLOW_PATH = path.join(HERE, "external-repo-refs.json");
let extRepoAllowCache = null;
function getExtRepoAllowlist() {
  if (extRepoAllowCache) return extRepoAllowCache;
  const out = { fileKeys: new Set(), lineKeys: new Set(), bad: [], err: null, n: 0 };
  try {
    const j = JSON.parse(fs.readFileSync(EXT_REPO_ALLOW_PATH, "utf8"));
    const sp = splitAllowlist(j.allow);
    out.fileKeys = sp.fileKeys;
    out.lineKeys = sp.lineKeys;
    out.bad = sp.bad;
    out.n = Array.isArray(j.allow) ? j.allow.length : 0;
  } catch (e) {
    out.err = e.code === "ENOENT" ? "名册文件不存在（视为空豁免）" : `名册读取失败：${e.message}`;
  }
  extRepoAllowCache = out;
  return out;
}

/**
 * 名册的**行级**键是否命中 —— 供 NONLOC 用（`splitAllowlist` 的注释解释了为何必须是行级）。
 * @param {{lineKeys:Set<string>}} allowInfo
 * @param {string} docRel
 * @param {string} refFile 引用串原样
 * @param {number} line **被引行号**（不是文档行号）
 */
function allowHitNonloc(allowInfo, docRel, refFile, line) {
  return allowInfo.lineKeys.has(`${docRel}::${refFile}::${line}`);
}

// ── EXTERNAL 引用的**真验证**（2026-09-17 扩容，用户裁决）──────────────────────
//
// 原先 `EXTERNAL` 桶是**完全免检**的：只要注释块里出现 `name-x.y.z/` 且目标相容，就算它
// 「本就不该在本仓解析」—— 既不看它指向的文件在不在，也不看版本对不对。两个静默缺口：
//   ① **版本陈旧**：registry **长期保留历史版本** ⇒ 指向 `sea-orm-2.0.1` 的路径照样解析
//      成功，而 `Cargo.lock` 里是 `2.0.2` ⇒ 引用证明的已**不是我们在用的那份代码**（判据 #491）。
//   ② **文件不存在**：路径拼错 / crate 被换掉 ⇒ 免检桶把假红也一起免掉了。
//
// 判据强度刻意**分层**，因为这两件事的可靠性差一个量级：
//   · `verMismatch`（版本 ∉ `Cargo.lock`）—— 可**硬拦**：只读仓库内的 lock 文件，
//     **离线也成立**，不依赖 registry 是否缓存。
//   · `pathMissing`（registry 里找不到该路径）—— **只报告**（advisory）：那个 crate 未必
//     下载过，硬拦会在离线 / 首次 clone 的机器上假红（同「env-gated 测试腐烂」的教训）。
//   · registry 根不存在 ⇒ 路径检查**整条跳过**，并在输出里自陈，**不伪装成**「检查过且通过」。
let lockIndex = null; // Map<name, Set<version>>；null=未读；false=读不到
function getLockIndex() {
  if (lockIndex !== null) return lockIndex;
  try {
    const txt = fs.readFileSync(path.join(SRC, "Cargo.lock"), "utf8");
    const m = new Map();
    for (const p of txt.matchAll(
      /\[\[package\]\]\s*\nname\s*=\s*"([^"]+)"\s*\nversion\s*=\s*"([^"]+)"/g,
    )) {
      if (!m.has(p[1])) m.set(p[1], new Set());
      m.get(p[1]).add(p[2]);
    }
    lockIndex = m;
  } catch {
    lockIndex = false;
  }
  return lockIndex;
}

/**
 * 拆 `sea-orm-2.0.2` → `{name:"sea-orm", ver:"2.0.2"}`：**从右往左找第一个以数字开头的段**。
 * 不能按「最后一个 `-`」拆 —— crate 名自身就带连字符（`sea-orm` / `axagent-dao`），
 * 版本段也可能是 `2.0.2-rc1` 这种形态（那时 `rc1` 不以数字开头，仍从 `2.0.2-rc1` 整体取下）。
 */
export function splitCrateVer(seg) {
  const parts = seg.split("-");
  for (let i = parts.length - 1; i >= 1; i--) {
    if (/^\d/.test(parts[i])) {
      return { name: parts.slice(0, i).join("-"), ver: parts.slice(i).join("-") };
    }
  }
  return null;
}

let registryRoots = null; // null=未探测；[]=不可用
function getRegistryRoots() {
  if (registryRoots !== null) return registryRoots;
  const home = process.env.USERPROFILE || process.env.HOME || "";
  const base = path.join(home, ".cargo", "registry", "src");
  try {
    registryRoots = fs
      .readdirSync(base, { withFileTypes: true })
      .filter((e) => e.isDirectory())
      .map((e) => path.join(base, e.name));
  } catch {
    registryRoots = [];
  }
  return registryRoots;
}

/** EXTERNAL 真验证。`ownMark` = 引用串（或同块）自带的 `crate-ver/路径`。 */
export function verifyExternal(ownMark) {
  if (!ownMark) return { kind: "UNPARSED", note: "无 crate-ver 标注" };
  const seg = ownMark.split("/")[0];
  const parts = splitCrateVer(seg);
  if (!parts) return { kind: "UNPARSED", note: "无法从 " + seg + " 拆出 crate 名/版本" };
  const { name, ver } = parts;
  const lock = getLockIndex();
  if (lock === false) return { kind: "NO_LOCK", crate: name, ver, note: "Cargo.lock 读不到" };
  const versions = lock.get(name);
  if (!versions) return { kind: "NOT_DEP", crate: name, ver, note: "该 crate 不在 Cargo.lock" };
  if (!versions.has(ver)) {
    return { kind: "STALE", crate: name, ver, note: "lock 里是 " + [...versions].join(" / ") };
  }
  const roots = getRegistryRoots();
  if (roots.length === 0) return { kind: "OK_NO_REGISTRY", crate: name, ver };
  const rest = ownMark.slice(seg.length + 1);
  const hit = roots.some((r) => {
    try {
      return fs.existsSync(path.join(r, seg, rest));
    } catch {
      return false;
    }
  });
  return hit
    ? { kind: "OK", crate: name, ver }
    : { kind: "NOFILE", crate: name, ver, note: "registry 里无此路径" };
}

/**
 * 引用所指的那一行是**裸控制流语句** —— 它**不可能**是任何声明的证据。
 *
 * 硬判据（客观错）。实测信噪比 **7/7**（见文件头 ③）：一条 `continue;` 既不讲某个
 * 关系类型、也不讲某个实体类型，行号必然已漂移，且**修法唯一**：把行号改成真正
 * 讲这件事的那一行。这正是判据 #147 允许硬拦的形态。
 *
 * ⚠ 刻意**不含** `}` / `},` / `);`：那一类里混着区间终点（`:1913-1916` 的 `1916`
 * 本就是收尾行）与「整段」松散写法 ⇒ 归软判据 `CLOSING_LINE`，不硬拦。
 */
export const CONTROL_FLOW_LINE = /^(?:continue|break|return|continue\s+'[A-Za-z_]\w*|break\s+'[A-Za-z_]\w*);$/;

/**
 * 引用所指的那一行是**收尾括号行**（`}` `},` `};` `)` `),` `});` `} else {`…）。
 *
 * 软判据（只报告，退出 3）。实测 5 处里 3 处是区间终点、2 处是松散写法 ⇒ 需人判：
 * 「引用的是这一段」与「行号漂移到收尾行」在下游同形，机器分不出来。
 */
export const CLOSING_LINE = /^(?:[)\]};,]*[)\]};][)\]};,]*|\}\s*else\s*\{?)$/;


/** 把一个仓库内引用解析成真实文件（引用写法可能是 `src/…` 或 `crates/…` 或 `src-tauri/…`） */
function resolveRef(relPath) {
  for (const base of [ROOT, SRC, path.join(SRC, "crates")]) {
    const p = path.join(base, relPath);
    if (fs.existsSync(p) && fs.statSync(p).isFile()) return p;
  }
  return null;
}

// ── 仓库文件索引（按 basename → 全部同名文件）─────────────────────────
//
// ⚠ 第一版**没有**这层，直接把「显式路径解析不到」判成「文件不存在」⇒ 实测一次报
//   274 处「客观错」，其中绝大多数是**裸文件名**（`wiki.rs:1660` / `mod.rs:196` /
//   `tool_executor.rs:177`）—— 引用处只写了文件名，没写路径。那 274 条里
//   **没有一条**是真腐烂，全是解析器的盲区被当成了缺陷（判据 #7：审计脚本自身会撒谎）。
//   修法就是兄弟脚本 `check-decl-evidence.mjs` 早已写明的三分法：
//     LOCATED（可唯一定位）/ NONLOC（无法定位，如裸 `mod.rs`）/ BROKEN（真不存在）。
//   **只有 BROKEN 才是失败**，且 NONLOC 必须**计数并打印**（自陈覆盖范围，判据 #16）。
//
// ⚠ 本集合是**覆盖面**的直接决定因素，改动前必须量（2026-09-15 实测）：
//   环境里 `.worktrees/` 有 **4 个完整检出副本**（`v297`/`v299`/`upstream`/`pre992947df`）。
//   它们**不在** `collectRustFiles()` 的扫描面内（那只走 `src-tauri/src` 与
//   `src-tauri/crates/*/src`；2026-09-17 起另含 `crates/*/tests/` + 顶层 `tests/` ——
//   三者都不走 `.worktrees/`，故本条结论不变），但 `getBasenameIndex()` 走的是**整个 ROOT**
//   ⇒ 几乎每个 basename 都变成「同名 5 份」⇒ **裸文件名引用一律判 NONLOC = 未检查**。
//   实测（同一棵树，只改这一行）：
//     加之前：LOCATED 137 ｜ NONLOC 209 ｜ BROKEN 6
//     加之后：LOCATED 269 ｜ NONLOC  77 ｜ BROKEN 6
//   ⇒ **132 条引用从「从未检查」变成「真检查」**，且 BROKEN 不变（零误红）。
//   教训：**NONLOC 的 209 条（47%）不是「引用写得不好」，是本脚本自己造的**。
const SKIP_DIRS = new Set([
  "target",
  "node_modules",
  ".git",
  "dist",
  "knowledge-sources",
  ".output",
  ".vite",
  ".monaco",
  ".worktrees",
  // ⚠ 2026-09-17 加入（用户裁决）：`output/` 是**产物目录**，里面装着整棵树的副本
  // （`output/backup-*/dao-src-migrations/`、`output/tmp-*-src/`）⇒ 把 basename 撑成
  // 「同名 N 份」，让本该能定位的引用落进 NONLOC（=**从未检查**）。
  // 它同时会**掩盖**两类真问题，故必须与下面那条 `N` 分支的 EXTERNAL 判定一起改：
  //   · 第三方 crate 源码引用（`sea-orm-2.0.2/src/schema/entity.rs`）—— 副本在 output/ 里
  //     时才落 NONLOC，去掉副本后变 BROKEN（假红）；
  //   · 指向**已删除迁移文件**的 provenance 注释（`v100_consolidated.rs`）—— 副本只在
  //     output/backup 里，去掉后变 BROKEN。这两类都不是「引用写错了」，见下方分类处置。
  "output",
]);
let basenameIndex = null;
function getBasenameIndex() {
  if (basenameIndex) return basenameIndex;
  basenameIndex = new Map();
  const walk = (dir) => {
    let entries;
    try {
      entries = fs.readdirSync(dir, { withFileTypes: true });
    } catch {
      return;
    }
    for (const e of entries) {
      if (SKIP_DIRS.has(e.name)) continue;
      const p = path.join(dir, e.name);
      if (e.isDirectory()) walk(p);
      else if (e.isFile()) {
        if (!basenameIndex.has(e.name)) basenameIndex.set(e.name, []);
        basenameIndex.get(e.name).push(p);
      }
    }
  };
  walk(ROOT);
  return basenameIndex;
}

const norm = (p) => p.split(path.sep).join("/");

// ── 活文档（`.md`）引用体检：扫描面 + 粗筛判据（供 --dump-doc-refs）──────────
//
// ⚠ 这两个函数**只服务取证**，主流程一次都不调用它们 ⇒ 加进来不会改变任何现有读数
//   （`LOCATED` / `NONLOC` / `EXTERNAL` / `failures` 全不受影响）。

/**
 * 活文档扫描面。**分三栏**，因为三栏的**处置方向不同**，合并会误导：
 *
 *   · `live`  —— `docs/plans/**` + **根级** `*.md`（不递归）：这是「当前指针」型文档，
 *                行号**应该**指向现状 ⇒ 发现陈旧就该订正。
 *   · `memory` —— `.workbuddy/memory/**`：**混装**。判据层里 `` `x.rs:460` `` 是
 *                「当前指针」，而流水 `YYYY-MM-DD.md` 里的 `x.rs:123` 是「当时事实」。
 *                机器分不出 ⇒ **单列报告，由人判**。
 *   · `hist`  —— `docs/` 下**除 `plans/` 之外**（`audits/` 是带日期戳的调查快照、
 *                `marketing/`）+ `output/**`（历史报告 / 备份副本）。
 *                行号记的是**当时**的事实 ⇒ **订正它们是破坏史料**，只计数不列明细。
 *
 * ⚠ **2026-09-17 二次修正（我自己的口径错，实测代价见下）**：首版把整个 `docs/**` 都算
 *   `live` 栏，于是 `docs/audits/AUDIT-*-2026-09-14.md` 那 35 份**历史调查快照**全被当成
 *   「应保持准确」⇒ A 栏可疑从真实量级虚涨到 **186 条**（其中绝大多数是那些快照在记录
 *   **当时**的行号）。判据是**目录语义**、不是「文件长什么样」：`plans/` 会被持续更新，
 *   `audits/` 一旦落笔就是史料（判据 #490 的反面：把史料改成当前值是**破坏**）。
 *
 * 不复用 `SKIP_DIRS` / `getBasenameIndex()`：后者的排除口径是为「按 basename 找**目标文件**」
 * 设计的（要排除 `output/` 里的整棵树副本，否则同名 N 份 ⇒ 本该可定位的引用落 NONLOC）。
 * 本函数找的是「**引用来源**」，两件事的扫描面**本就不同**，混用会一边漏一边多。
 */
export function collectDocFiles() {
  const out = { live: [], memory: [], hist: [] };
  const walk = (dir, sink) => {
    let entries;
    try {
      entries = fs.readdirSync(dir, { withFileTypes: true });
    } catch {
      return; // 目录不存在 = 安全 no-op（与 collectRustFiles 同口径）
    }
    for (const e of entries) {
      if (e.name === "node_modules" || e.name === ".git") continue;
      const p = path.join(dir, e.name);
      if (e.isDirectory()) walk(p, sink);
      else if (e.isFile() && e.name.endsWith(".md")) sink.push(p);
    }
  };
  // ① 活计划：会被持续更新 ⇒ 行号应指向现状
  walk(path.join(ROOT, "docs", "plans"), out.live);
  // ② 根级 `*.md`（**不递归**）：`AGENTS.md` / `CHECKS.md` / `README*.md` —— 同属「应保持准确」。
  for (const e of fs.readdirSync(ROOT, { withFileTypes: true })) {
    if (e.isFile() && e.name.endsWith(".md")) out.live.push(path.join(ROOT, e.name));
  }
  // ③ 记忆：混装 ⇒ 单列
  walk(path.join(ROOT, ".workbuddy", "memory"), out.memory);
  // ④ 历史：`docs/` 下除 `plans/` 之外（按**目录语义**分流，不按文件名猜日期）
  const docsAll = [];
  walk(path.join(ROOT, "docs"), docsAll);
  const plansPrefix = path.join(ROOT, "docs", "plans") + path.sep;
  for (const p of docsAll) if (!p.startsWith(plansPrefix)) out.hist.push(p);
  walk(path.join(ROOT, "output"), out.hist);
  for (const k of Object.keys(out)) out[k].sort();
  return out;
}

/**
 * 活文档里一条引用的**粗筛**结论。
 *
 * ⚠ 这是**粗筛（初筛）**，不是判据 —— 之所以刻意不复用主流程的 `failures` 通道：
 *   `.md` 里的行号引用大量是**故意保留的历史值**，机器分不出「陈旧」与「考古」：
 *     · 历史型：`PLAN-declarative-schema-sync.md` 自写「**修前**：`extras.rs:356-362`」
 *       —— 它**必须**保持旧行号，改成当前值反而把论证毁掉；
 *     · 当前指针型：`` 见 `extras.rs:903-904` `` —— 它**应该**指向现状。
 *   两者在行文里同形，且**同一份文档里可以并存** ⇒ 硬拦 = 恒定假红（判据 #147）。
 *
 * 判据口径与 `.rs` 侧**逐条对齐**（同一套正则、同一条 `blankOk` 豁免），否则同一份漂移
 * 在两个扫描面上会给出不同结论 —— 那是判据口径分歧，比不检查更糟：
 *   ① 空行：`blankOk`（区间终点）**豁免**；`tmp-plan-refs6.mjs` 首版漏了这条豁免，
 *      把 `extras.rs:374` ×3 全报成可疑 —— 实测那三条都是 `:374-…` 的区间起点写法连带，
 *      **高估了可疑量**（判据 #147：先查判据口径，别查被测对象）。
 *   ② `CONTROL_FLOW_LINE` / `CLOSING_LINE` 直接复用 `export` 的那两个常量（`--selftest` 也在验它们）。
 *
 * @param {string|undefined} cited 目标行原文；`undefined` = 行号越界
 * @param {{blankOk:boolean}} ref `extractRefs` 产出的那一条
 * @returns {string|null} 可疑类型；`null` = 粗筛通过
 */
export function classifyDocRef(cited, ref) {
  if (cited === undefined) return "越界";
  const t = cited.trim();
  if (t === "") return ref.blankOk ? null : "空行";
  if (ref.blankOk) return null; // 区间终点：端点只是范围边界，不参与结构判据（与 .rs 侧同）
  if (CONTROL_FLOW_LINE.test(t)) return "控制流行";
  if (CLOSING_LINE.test(t)) return "收尾行";
  return null;
}

/**
 * 把一个 **NONLOC**（同名多份、路径后缀也不能消歧）引用细分。**纯函数**，便于 `--selftest` 复用。
 *
 * 返回值语义（**都仍是 NONLOC / 未检查**，细分只给行动面，不是洗白）：
 *   · `ALLOWLISTED` —— 命中显式名册 `scripts/external-repo-refs.json`（自指反例段 / 门禁演进史举例）
 *     ⚠ 调用方**必须**传**行级**键（`allowHitNonloc`），不许把文件级键递进来 ——
 *       文件级键「某一行是举例」够不着「同一文件另一次引用」（2026-09-18 第三轮实测事故：3 条被 1 条证据豁免）。
 *   · `OVERFLOW`    —— **行号越界**：同名候选**没有任何一份**长到能容纳引用行号
 *   · `OTHER`       —— 其余（确实需要人工补路径）
 *
 * ⚠ 与 `classifyBroken` 是**两件事**，别合并：
 *   ① 输入不同 —— 这里要的是「候选清单 + 各自行数」，不是「文件存不存在」。
 *   ② 效力不同 —— `classifyBroken` 只改**分类标签**（BROKEN 仍是 BROKEN，计数不变）；
 *      这里**连分类都不改**，只决定 dump 里**点到谁的名字**（不点名的计数会被误读成
 *      「都查过了」——判据 #16）。
 *   ③ 唯一有判据强度的是 `OVERFLOW`：它**与选哪个候选无关** —— 无论这条引用想指哪一份，
 *      行号都不存在 ⇒ 它必然是错的，而不是「读不到」。**反向护栏（必须有）**：只要有
 *      **≥1** 份候选够长，就不许落这个桶，否则「没写路径」会被当成「引用写错」（判据 #147：
 *      先查判据口径、别查被测对象）。实测 `mod.rs:196` 有 21/71 份够长 ⇒ 必须落 OTHER。
 *
 * @param {{rel:string, lines:number}[]} cands 同名候选及其行数
 * @param {number} line 引用行号
 * @param {boolean} allowHit 是否命中显式名册（由调用方查表，函数本身保持纯）
 * @returns {"ALLOWLISTED"|"OVERFLOW"|"OTHER"}
 */
export function classifyNonloc(cands, line, allowHit) {
  if (allowHit) return "ALLOWLISTED";
  if (!Array.isArray(cands) || cands.length === 0) return "OTHER"; // 0 份 ⇒ 本该是 BROKEN
  return cands.every((c) => Number(c.lines) < line) ? "OVERFLOW" : "OTHER";
}

/**
 * 定位一个引用指向的文件。三分法（`L` / `N` / `B`）—— 纯函数，便于 --selftest 复用。
 * @returns {{kind:"L", abs:string} | {kind:"N", why:string} | {kind:"B"}}
 */
export function locate(refFile, index) {
  // ① 显式相对路径（含 `/`）：按仓库内的多种基准目录试
  if (refFile.includes("/")) {
    const hit = resolveRef(refFile);
    if (hit) return { kind: "L", abs: hit };
  }
  const base = refFile.slice(refFile.lastIndexOf("/") + 1);
  const cands = index.get(base) ?? [];
  if (cands.length === 0) return { kind: "B" };
  if (cands.length === 1) return { kind: "L", abs: cands[0] };
  // ② 同名多份 ⇒ 用「引用串的路径后缀」消歧（`init/state.rs` 只对应一个真实文件）
  const want = refFile.replace(/^\.\//, "");
  const suffix = cands.filter((p) => norm(p).endsWith("/" + want));
  if (suffix.length === 1) return { kind: "L", abs: suffix[0] };
  return { kind: "N", why: `同名 ${cands.length} 份、路径后缀也不能消歧（如 \`mod.rs\`/\`lib.rs\`）` };
}

/**
 * 软判据：引用所在行里**引号包裹**的字面量，是否还能在出处那一行附近找到。
 *
 * 刻意**只看引号**、不看反引号：表格里的反引号包的是引用本身或变体名
 * （`` `Daily` `` / `` `src/commands/wiki.rs:1660` ``），拿它去比对出处恒定假红。
 * 实测腐烂形态恰恰是**引号里的值**搬了家：
 *   `| `Doc` | `src/init/opc_knowledge.rs:254`；DB 实测 `notes.page_type='doc'` |`
 *   → 出处那一行必须还含 `doc`。
 */
/** 扫出本行所有「引号包裹的值」及列区间（**单一正则源**，供下面两个口径共用）。 */
function scanQuoted(lineText) {
  const toks = [];
  for (const m of lineText.matchAll(/["']([A-Za-z0-9_.:-]{2,})["']/g)) {
    const t = m[1];
    if (t.includes("/")) continue; // 路径形态（引用本身）不参与比对
    // ⚠ 2026-09-17 补：形如 `entity.rs:72` / `seed_content_media.rs:1902` 的**带行号引用**
    // 同样是「引用本身」，不是「该在出处出现的值」。原口径只排除含 `/` 的路径形态，
    // 于是 `assert!(d.evidence.contains("entity.rs:72"))` 这类**断言行**被当成了
    // 「引用处声明了一个值 `entity.rs:72`，出处必须出现它」⇒ 恒定假红。
    // 判据区分「引用」与「值」的边界是：**引用指向别处**（带 `:行号`），
    // **值是被断言存在的字面量**。前者永远不该拿去处处的副本里找。
    if (/\.\w+:\d/.test(t)) continue;
    toks.push({ t, i: m.index, e: m.index + m[0].length });
  }
  return toks;
}

/**
 * 软判据：引用所在行里**引号包裹**的字面量，是否还能在出处那一行附近找到。
 *
 * 刻意**只看引号**、不看反引号：表格里的反引号包的是引用本身或变体名
 * （`` `Daily` `` / `` `src/commands/wiki.rs:1660` ``），拿它去比对出处恒定假红。
 * 实测腐烂形态恰恰是**引号里的值**搬了家：
 *   `| `Doc` | `src/init/opc_knowledge.rs:254`；DB 实测 `notes.page_type='doc'` |`
 *   → 出处那一行必须还含 `doc`。
 */
export function quotedTokens(lineText) {
  return [...new Set(scanQuoted(lineText).map((x) => x.t))];
}

/**
 * 软判据的**归属**口径：只取「归给本条引用」的那些值。
 *
 * ⚠ 2026-09-17 修（**同一根因的第三条**）：上一版在主循环里直接拿 `quotedTokens(r.text)`
 *   ——即「**整行**的全部引号值」—— 去比**每一条**引用 ⇒ 一行有 N 条引用时，每个值被
 *   派给 N 条 ⇒ 假红量级 N×(N−1)。实测 3 条，全部人工核验为**假红**：
 *     · `extras.rs:457`：`Prefix("vec_")` 的值 `vec_` 被派给同行的 `:235`（实属 `:200`）
 *     · `opc_workflow_kpi_hook.rs:150`：`language: "rhai"` 的值 `rhai` 被派给
 *       同行的 `1821-1840` 与 `1840`（实属 `:595` —— 该行第二、三条引用）
 *   与判据 #485 / [`externalMarkFor`] 同族：**判据的生效范围被无关条件放大**。
 *
 * 正确口径 = 值归给**文本距离最近**的那条引用（到引用 token 列区间的距离）。
 *   严格度**不变**（值仍必须出现在出处 ±2 行）；只是不再把值误配给邻座引用。
 *   等距时归属不唯一 ⇒ **都给**（宁可多报，不放过真漂）。
 *
 * @param {string} lineText 引用所在整行
 * @param {{col:number,end:number}[]} peers 同一行的**全部**引用（含 self）
 * @param {{col:number,end:number}} self 本条引用
 */
export function quotedTokensOwnedBy(lineText, peers, self) {
  const own = [];
  for (const tk of scanQuoted(lineText)) {
    const dist = (p) => {
      if (tk.i >= p.col && tk.e <= p.end) return 0; // 值落在引用 token 内部
      return Math.min(Math.abs(tk.i - p.end), Math.abs(tk.e - p.col));
    };
    let min = Infinity;
    for (const p of peers) min = Math.min(min, dist(p));
    if (dist(self) === min) own.push(tk.t);
  }
  return own;
}

// ── ② reranker 模型文件名 ─────────────────────────────────────────────
const RAG_CONFIG = path.join(SRC, "crates/harness/src/rag_config.rs");
const CONST_NAME = "RERANKER_MODEL_FILENAME";

/** 真源：从 `rag_config.rs` 读常量值。读不到 ⇒ 脚本自身失效（退出 2） */
export function readRerankerConst(text) {
  const re = new RegExp(String.raw`pub const ${CONST_NAME}\s*:\s*&str\s*=\s*"([^"]+)"`);
  const m = text.match(re);
  return m ? m[1] : null;
}

/** 前端扫描面：`src/**` 下的 .ts/.tsx（排除测试与 mock） */
function collectFrontendFiles() {
  const out = [];
  const skip = new Set(["node_modules", "dist", ".git"]);
  const walk = (dir) => {
    let entries;
    try {
      entries = fs.readdirSync(dir, { withFileTypes: true });
    } catch {
      return;
    }
    for (const e of entries) {
      if (skip.has(e.name)) continue;
      const p = path.join(dir, e.name);
      if (e.isDirectory()) walk(p);
      else if (e.isFile() && /\.tsx?$/.test(e.name)) out.push(p);
    }
  };
  walk(path.join(ROOT, "src"));
  return out.sort();
}

/**
 * 前端里「reranker 文件名」这个事实的载体：落在 `crossEncoderModel` 字段上的
 * `.gguf` 字面量。锚点用**字段名**而非字符串值 —— 用值当锚点的话，
 * 值一旦漂移就再也找不到载体，「漂移」会被静默当成「没有载体」。
 */
export function findCrossEncoderLiterals(fileText) {
  const out = [];
  for (const raw of fileText.split(/\r?\n/)) {
    if (!/crossEncoderModel/.test(raw)) continue;
    for (const m of raw.matchAll(/"([^"]+\.gguf)"/g)) out.push({ value: m[1], text: raw.trim() });
  }
  return out;
}

// ── --selftest：正负对照，证明两类判据**真的会红**（判据 #147②）──
if (process.argv.includes("--selftest")) {
  let bad = 0;
  const chk = (name, got, want) => {
    const ok = JSON.stringify(got) === JSON.stringify(want);
    if (!ok) bad++;
    console.log(`${ok ? "✔" : "✖"} ${name}${ok ? "" : `  期望 ${JSON.stringify(want)}，实得 ${JSON.stringify(got)}`}`);
  };

  // 抽出器：三种实测形态
  chk(
    "抽取 `文件:行`",
    extractRefs("见 `src/commands/wiki.rs:1660` 与 `crates/agent/src/x.rs:671/782`").map((r) => `${r.file}:${r.line}`),
    ["src/commands/wiki.rs:1660", "crates/agent/src/x.rs:671", "crates/agent/src/x.rs:782"],
  );
  chk(
    "裸 :行号 承接前一个文件名",
    extractRefs("（白名单允许）、`:782`（分支）".replace("、`:782`", "`crates/agent/src/x.rs:671`、`:782`")).map((r) => `${r.file}:${r.line}`),
    ["crates/agent/src/x.rs:671", "crates/agent/src/x.rs:782"],
  );
  // 负样本：时刻 / 泛型不得被当成引用 —— 否则扫描面被淹掉（信噪比判据）
  chk("时刻 `13:43` 不算引用", extractRefs("// 2026-09-15 13:43 排队").length, 0);
  chk("无扩展名的 `:12` 不算引用", extractRefs("let a: u32 = 12;").length, 0);
  // 负样本：cargo 诊断的 `行:列`（实测 `--> src/main.rs:4:5` 曾被读成 `src/main.rs:5`）
  chk(
    "`--> src/main.rs:4:5` 只算 1 条且 = 4",
    extractRefs("error: --> src/main.rs:4:5").map((r) => r.line),
    [4],
  );
  // 区间端点允许空行、清单每行都要非空
  chk(
    "区间 `:64-70` 端点 blankOk",
    extractRefs("见 `a/b.rs:64-70`").map((r) => `${r.line}:${r.blankOk}`),
    ["64:false", "70:true"],
  );

  // ③ 形态代理的分档（2026-09-15 加入）：**裸控制流语句 ⇒ 硬**，**收尾括号 ⇒ 软**。
  // 这一组样本钉住的是「同一形态按信噪比分两档」：把 `}` 也塞进硬判据 ⇒ 区间终点
  // （`:1913-1916`）会恒定假红；把 `continue;` 降成软 ⇒ 那 7 处真腐烂又会静默溜过。
  chk("裸控制流语句 ⇒ 硬判据形态", CONTROL_FLOW_LINE.test("continue;"), true);
  chk("标签版裸控制流也算", CONTROL_FLOW_LINE.test("break 'outer;"), true);
  chk("正常代码行不落入硬判据", CONTROL_FLOW_LINE.test("let rtype = fields[2].to_string();"), false);
  chk(
    "收尾括号 ⇒ 软判据形态",
    ["}", "},", "};", ");", "} else {"].map((s) => CLOSING_LINE.test(s)),
    [true, true, true, true, true],
  );
  chk("收尾括号**不**落入硬判据", CONTROL_FLOW_LINE.test("}"), false);
  // 负样本：空行 / 普通赋值不得被收尾括号判据吃掉（空行另有专门的客观错判据）
  chk("空行不算收尾括号", CLOSING_LINE.test(""), false);
  chk("冒号后缀的正常行不算收尾括号", CLOSING_LINE.test("let mut v: Vec<u8> = vec![];"), false);

  // ── 活文档粗筛（--dump-doc-refs）────────────────────────────────────
  // ⚠ 这一组必须与 `.rs` 侧**逐条对齐**：同一份漂移在两个扫描面上若给出不同结论，
  // 那是**判据口径分歧**，比「不检查」更糟（判据 #147：先查判据口径，别查被测对象）。
  // 负样本（应报可疑）与正样本（**不许**报可疑）都要有 —— 只钉一侧等于没钉。
  const dr = (line, blankOk) => classifyDocRef(line, { blankOk });
  chk("活文档：行号越界 ⇒ 可疑", dr(undefined, false), "越界");
  chk("活文档：空行 ⇒ 可疑", dr("   ", false), "空行");
  chk("活文档：控制流行 ⇒ 可疑", dr("continue;", false), "控制流行");
  chk("活文档：收尾括号行 ⇒ 可疑（软）", dr("  });", false), "收尾行");
  // 正样本：`docs/plans/*.md` 里引用落点的实测形态，一律不许判可疑
  chk("活文档：正常代码行不算可疑", dr("  let reason = l2_virtual_table_reason(name);", false), null);
  chk("活文档：doc 文本行不算可疑", dr("/// 见 `plan.rs:1090-1108` 的 Some(ai) 分支", false), null);
  chk("活文档：markdown 表格行不算可疑", dr("| 8 | FTS5 虚表不进 `TableDecl` | `extras.rs:903-904` |", false), null);
  // 区间终点豁免：必须在**结构判据之前**生效，否则 `:A-B` 的 B 恒定刷噪声
  // （实测踩到：首版粗筛漏了这条豁免，把 `extras.rs:374` ×3 报成可疑 = 高估）
  chk("活文档：区间终点的空行 ⇒ 豁免", dr("", true), null);
  chk("活文档：区间终点即使落在 `}` 也豁免", dr("}", true), null);

  // ── NONLOC 细分（--dump-doc-refs 的 NONLOC 桶）────────────────────────
  // 正负样本都要有（判据 #147②）：只钉「越界会红」等于没钉 —— 漏掉的是
  // 「**没写路径** 被当成 **引用写错**」，那是把判据口径问题算到被测对象头上。
  const nn = (cands, line, hit) => classifyNonloc(cands, line, hit);
  // 正样本：候选全部短于引用行号 ⇒ 越界（无论指向哪份都不存在该行）
  chk(
    "NONLOC：候选全部够不到 ⇒ 行号越界",
    nn([{ rel: "a/x.rs", lines: 633 }, { rel: "b/x.rs", lines: 200 }], 1660, false),
    "OVERFLOW",
  );
  // 失效安全：候选行数读不到（字段缺失 / NaN）⇒ 退化 OTHER，**不许**判越界。
  // 取向与全门禁一致：宁可漏报（不点名）也不误报（把「读不到」说成「引用写错」）。
  chk(
    "NONLOC：候选行数读不到 ⇒ 退化 OTHER，不许判越界",
    nn([{ rel: "a/x.rs" }, { rel: "b/x.rs", lines: 10 }], 1660, false),
    "OTHER",
  );
  // 负样本（**反向护栏**）：只要 ≥1 份够长就不许进越界桶 ——
  // 实测形态：`wiki.rs:1660` 有 1/2 份够长、`mod.rs:196` 有 21/71 份够长，都不许进。
  chk(
    "NONLOC：有 1 份够长 ⇒ **不**算越界（wiki.rs 实测形态）",
    nn([{ rel: "a/wiki.rs", lines: 3027 }, { rel: "b/wiki.rs", lines: 633 }], 1660, false),
    "OTHER",
  );
  chk(
    "NONLOC：21/71 份够长 ⇒ **不**算越界（mod.rs 实测形态）",
    nn([...Array(21)].map((_, i) => ({ rel: `x${i}/mod.rs`, lines: 1000 + i })).concat([...Array(50)].map((_, i) => ({ rel: `y${i}/mod.rs`, lines: 10 }))), 196, false),
    "OTHER",
  );
  chk("NONLOC：候选恰等于行号 ⇒ 够长（边界，不能算越界）", nn([{ rel: "a/x.rs", lines: 196 }], 196, false), "OTHER");
  chk("NONLOC：候选比行号少 1 行 ⇒ 越界（边界另一侧）", nn([{ rel: "a/x.rs", lines: 195 }], 196, false), "OVERFLOW");
  // 名册豁免优先于越界判定：命中名册的条目**不该**被算成「引用写错」
  chk("NONLOC：命中名册 ⇒ 豁免优先", nn([{ rel: "a/x.rs", lines: 10 }], 1660, true), "ALLOWLISTED");
  // 0 份候选不是 NONLOC（那是 BROKEN）⇒ 不落越界桶，避免两套分类口径打架
  chk("NONLOC：0 份候选 ⇒ OTHER（BROKEN 才走 BROKEN 细分）", nn([], 9999, false), "OTHER");

  // 名册粒度（2026-09-18 第三轮）：文件级 / 行级**两集互不相通**。这组断言的存在理由是
  // 「豁免位的唯一失败模式是**只会变松**」—— 事故现场：一条 `doc::wiki.rs`（理由只解释 `:1660`）
  // 把同文档另外两条引用（`:620` / `:651`）一并豁免了。⇒ 判据必须**同时**证明
  // 「行级能生效」与「文件级够不着 NONLOC」，只证前者会被"顺手放宽一行代码"绕开。
  {
    const D = "docs/plans/PLAN-x.md";
    const F = "wiki.rs";
    const fileOnly = splitAllowlist([{ doc: D, refFile: F, reason: "r", evidence: "e" }]);
    chk("名册：无 lines ⇒ 进文件级集", fileOnly.fileKeys.has(`${D}::${F}`), true);
    // ★ 反向断言（核心）：文件级条目在行级集里**必须零足迹**
    chk(
      "名册：无 lines ⇒ 行级集零足迹（否则「只会变松」复发）",
      [...fileOnly.lineKeys].some((k) => k.startsWith(`${D}::${F}`)),
      false,
    );
    const lineOnly = splitAllowlist([{ doc: D, refFile: F, lines: [373, 1660], reason: "r", evidence: "e" }]);
    chk("名册：有 lines ⇒ 每行一个行级键", lineOnly.lineKeys.size, 2);
    chk("名册：行级键用**被引行号**", lineOnly.lineKeys.has(`${D}::${F}::1660`), true);
    chk("名册：有 lines ⇒ 不进文件级集（不放大成整族）", lineOnly.fileKeys.has(`${D}::${F}`), false);
    // 反向护栏：lines 非法 ⇒ 零效力，且**不退化**为文件级（防手滑把 `[1660]` 写成 `"1660"`）
    const bogus = splitAllowlist([{ doc: D, refFile: F, lines: ["1660", 0, -3], reason: "r", evidence: "e" }]);
    chk("名册：lines 全非法 ⇒ 零效力（bad）", bogus.bad.length, 1);
    chk("名册：零效力条目**不**退回文件级", bogus.fileKeys.size + bogus.lineKeys.size, 0);
    const mixed = splitAllowlist([{ doc: D, refFile: F, lines: [1132, "x"], reason: "r", evidence: "e" }]);
    chk(
      "名册：lines 混合 ⇒ 合法那个仍生效、不报 bad",
      mixed.lineKeys.has(`${D}::${F}::1132`) && mixed.bad.length === 0,
      true,
    );
    chk("名册：缺 doc / refFile / null 的条目被忽略", splitAllowlist([{ doc: D }, { refFile: F }, null]).fileKeys.size, 0);
  }

  // 扫描面分栏：三栏**处置方向相反**（live 该订正 / hist 不许动）⇒ 不许合并，
  // 合并会诱导人去「订正」本该保留的历史行号。用「某栏混进了别的来源」当红条件。
  {
    const sc = collectDocFiles();
    // ⚠ 判据必须锚定**仓库相对路径的前缀**，不能按路径片段匹配：`output/backup-*/docs/plans/X.md`
    // 是**副本**，属史料；按片段匹配会把它当「活计划混进历史栏」⇒ 假红（实测踩到）。
    const r = (p) => rel(p);
    const inOut = (p) => r(p).startsWith("output/");
    const inMem = (p) => r(p).startsWith(".workbuddy/memory/");
    const inPlans = (p) => r(p).startsWith("docs/plans/");
    const inDocs = (p) => r(p).startsWith("docs/");
    chk("活栏混进 output/ 或 .workbuddy/", sc.live.some((p) => inOut(p) || inMem(p)), false);
    // ⚠ 这条是钉**我自己的口径错**：首版把整个 `docs/**` 当「活文档」，于是
    //   `docs/audits/AUDIT-*-<日期>.md`（35 份**调查快照**）全被要求「指向现状」⇒
    //   可疑数从真实量级虚涨到 186。判据是**目录语义**：`plans/` 会更新，`audits/` 是史料。
    chk("活栏混进 docs/ 下**非 plans** 的目录（如 audits/）", sc.live.some((p) => inDocs(p) && !inPlans(p)), false);
    chk("活栏非空（空面 ⇒ 取证静默输出 0，那是假绿）", sc.live.length > 0, true);
    chk("记忆栏非空且只来自 .workbuddy/memory/", sc.memory.length > 0 && sc.memory.every(inMem), true);
    chk("历史栏含 `docs/plans/` 下的文件（= 把史料当待办的口径错）", sc.hist.some(inPlans), false);
    chk("历史栏非空", sc.hist.length > 0, true);
    chk("三栏均为 .md", [...sc.live, ...sc.memory, ...sc.hist].every((p) => p.endsWith(".md")), true);
    // 三栏**互不重叠**：同一份文档被两种相反的处置方向同时要求 ⇒ 下面「分桶恒等」也没意义了
    chk(
      "三栏互不重叠",
      new Set([...sc.live, ...sc.memory, ...sc.hist]).size,
      sc.live.length + sc.memory.length + sc.hist.length,
    );
  }

  // 第三方源码豁免（`.md` 侧）：漏掉它 ⇒ 成片假红（`sea-orm-2.0.2/src/schema/entity.rs:156`
  // 会被 locate() 按 basename 认成**本仓**的 entity.rs ⇒ 报「越界」）。判据与 `.rs` 侧同源。
  chk("引用串自带版本目录 ⇒ 外部", EXTERNAL_MARK.test("sea-orm-2.0.2/src/schema/entity.rs"), true);
  chk("无版本目录 ⇒ 不算外部", EXTERNAL_MARK.test("src/schema/entity.rs"), false);
  chk(
    "同行有**同一目标**的完整第三方路径 ⇒ 外部",
    externalMarkFor("见 `sea-orm-2.0.2/src/schema/entity.rs:156` 的建表", "sea-orm-2.0.2/src/schema/entity.rs"),
    true,
  );
  chk(
    "同行第三方路径**不同目标** ⇒ 不免检（判据 #485）",
    externalMarkFor("见 `sea-orm-2.0.2/src/schema/entity.rs:156`", "migrations/v210_opc_ext.rs"),
    false,
  );
  // 块级上下文：第三方 crate 源码判定靠它（行级判据会漏掉被省略路径的那几行）
  const cb = commentBlocks(["//! a", "//! rhai-1.26.0/x.rs:1", "//! `pkg_std.rs:2`"]);
  chk("块级上下文合并连续注释行", cb.ids[1] === cb.ids[2], true);
  chk("块级标记命中第三方源码目录", EXTERNAL_MARK.test(cb.texts[cb.ids[2]]), true);
  chk("普通注释块不命中", EXTERNAL_MARK.test("//! 见 src/a.rs:1"), false);

  // 软判据 token：反引号不参与（否则表格里的变体名恒定假红）
  chk("引号 token", quotedTokens("| `Doc` | `a/b.rs:1`；实测 `notes.page_type='doc'` |"), ["doc"]);
  chk("反引号 token 被剔除", quotedTokens("| `Daily` | `src/commands/wiki.rs:1660` |"), []);

  // 软判据的**归属**（2026-09-17 修的第三条同族缺陷）：一行多引用时，引号值只能归给
  // 文本距离最近的那条引用；否则 N 条引用 ⇒ 每个值被派 N 次 ⇒ N×(N−1) 量级假红。
  // 正负样本一起钉：既要「归对了」，也要「邻座拿不到别人的值」。
  {
    const L =
      '/// （`seed_content_media.rs:1821-1840`），该辅助函数把 **`language: "rhai"`（`:595`）与';
    const R = extractRefs(L);
    chk("归属样本：一行抽出 3 条引用（`:595` + 区间两端）", R.length, 3);
    const own = (pred) => quotedTokensOwnedBy(L, R, R.find(pred));
    chk("`rhai` 归给最近的裸引用 `:595`", own((r) => r.line === 595), ["rhai"]);
    chk("区间起点拿不到 `rhai`（这正是原先的假红）", own((r) => r.line === 1821), []);
    chk("区间终点同样拿不到", own((r) => r.line === 1840), []);
    // 单引用行不受影响（回归：别把归属逻辑做成「一律不报」）
    const L2 = '/// 判词见 `seed_content_media.rs:955` 的「包成只含这两个键的对象」。';
    const R2 = extractRefs(L2);
    chk("单引用行且无引号值 ⇒ 空（负样本：反引号不算值）", quotedTokensOwnedBy(L2, R2, R2[0]).length, 0);
    const L3 = '/// `a/b.rs:12` 的 `"markdown"` 键';
    const R3 = extractRefs(L3);
    chk("单引用行 + 值 ⇒ 归属成立（正样本）", quotedTokensOwnedBy(L3, R3, R3[0]), ["markdown"]);
  }

  // 真源读取：负样本
  chk("读常量", readRerankerConst(`pub const ${CONST_NAME}: &str = "m.gguf";`), "m.gguf");
  chk("常量缺失 ⇒ null（脚本应退 2）", readRerankerConst("// 无"), null);

  // 载体锚点：正/负样本
  chk(
    "锚点命中 crossEncoderModel",
    findCrossEncoderLiterals(`  crossEncoderModel: "a.gguf",`).map((x) => x.value),
    ["a.gguf"],
  );
  chk(
    "无 crossEncoderModel 的行不算载体",
    findCrossEncoderLiterals(`  embeddingModel: "bge-m3-Q5_K_M.gguf",`).length,
    0,
  );

  // 三分法 LOCATED / NONLOC / BROKEN —— 第一版把 NONLOC 当失败，一次报 274 处假错。
  // 这几条负样本就是钉住「不许再退回两分法」的（判据 #7）。
  const idx = new Map([
    ["wiki.rs", ["/r/src-tauri/src/commands/wiki.rs"]],
    ["mod.rs", ["/r/a/mod.rs", "/r/b/mod.rs"]],
    ["state.rs", ["/r/src-tauri/src/init/state.rs", "/r/other/state.rs"]],
  ]);
  chk("唯一同名 ⇒ LOCATED", locate("wiki.rs", idx).kind, "L");
  chk("同名多份且后缀不能消歧 ⇒ NONLOC", locate("mod.rs", idx).kind, "N");
  chk("同名多份但后缀可消歧 ⇒ LOCATED", locate("init/state.rs", idx).kind, "L");
  chk("仓库里确实没有 ⇒ BROKEN", locate("nope.rs", idx).kind, "B");

  // ── BROKEN 细分（2026-09-18 新增）─────────────────────────────────────────
  // 这一组钉两件事：① 三类各自**真的能命中**；② 三条护栏**真的会拦**。
  // 缺了 ② 就会重演 #485（豁免位吞掉本仓引用 ⇒ 读数变好看、问题没少），
  // 而「豁免位只会变松」正是这类改造唯一的失败模式。
  {
    const borDoc = "docs/plans/PLAN-evoflow-borrowings.md";
    const declHead = "# PLAN — EvoFlow 借鉴清单\n- **参考边界**：EvoFlow 为 Python/LangGraph 栈，不搬运代码。";
    const cb = (o) =>
      classifyBroken({ docRel: borDoc, refFile: "", refText: "", docHead: declHead, allow: new Set(), ...o });

    chk("外部仓库源码（borrowings + 参考边界 + 非本仓栈）⇒ EXT-REPO", cb({ refFile: "collab/task_source.py" }), "EXT-REPO");
    chk("同上，`.yaml` 也算外部（实测 config.example.yaml）", cb({ refFile: "config.example.yaml" }), "EXT-REPO");

    // 护栏① 本仓主力栈：borrowings 文档里同样有已删 .rs（实测 weknora 文档 5 条），
    //        按扩展名一刀切会把它们洗成「外部」⇒ 必须拦在 EXT-REPO 之外。
    chk("护栏①：`.rs` 不按外部豁免", cb({ refFile: "src/commands/opc_industry_actions.rs" }), "BROKEN");
    chk("护栏①：`.ts` 不按外部豁免", cb({ refFile: "src/stores/x.ts" }), "BROKEN");
    // 护栏② 只对 borrowings 文档生效（另 14 份 .md 是本仓计划，引用的必须是本仓文件）
    chk(
      "护栏②：非 borrowings 文档不豁免",
      classifyBroken({ docRel: "docs/plans/PLAN-x.md", refFile: "a/b.py", refText: "", docHead: declHead, allow: new Set() }),
      "BROKEN",
    );
    // 护栏③ 声明必须在**头部**：正文里出现「参考边界」四个字不构成声明
    chk(
      "护栏③：头部无声明行 ⇒ 不豁免",
      classifyBroken({ docRel: borDoc, refFile: "a/b.py", refText: "", docHead: "# 随便一份文档", allow: new Set() }),
      "BROKEN",
    );

    // 已删源文件的两条独立证据路径，各自单独钉（否则一条坏了另一条盖住）
    chk("已删迁移（版本号命名）⇒ DELETED-SRC", cb({ refFile: "v207_chat_run.rs" }), "DELETED-SRC");
    chk(
      "引用句自陈「已拆成」⇒ DELETED-SRC",
      cb({ refFile: "stock_analysis_setup.rs", refText: "该文件已拆成同名目录" }),
      "DELETED-SRC",
    );
    // 日期插在中间的中文形态（实测漏过一次：「该文件**已于 2026-09-16 退休**」
    // 里 `已退休` 并不连续 ⇒ 只列「已退休」会漏掉真正写成日期的那批，判据 #535 同族）
    chk(
      "引用句自陈「已于 <日期> 退休」⇒ DELETED-SRC",
      cb({ refFile: "dao/tests/pg_migrations.rs", refText: "该文件已于 2026-09-16 退休且未备份" }),
      "DELETED-SRC",
    );
    // 负样本：只说「不存在」（是**被引用的事实**，不是「文件没了」的自陈）⇒ 不许误判
    chk(
      "负样本：只写「不存在」不算已删自陈",
      cb({ refFile: "nope.rs", refText: "库里实际没有这 3 张表" }),
      "BROKEN",
    );
    // 优先级：显式名册 > 已删证据 > 外部判定
    chk(
      "显式名册优先于外部判定",
      classifyBroken({ docRel: borDoc, refFile: "a/b.py", refText: "", docHead: declHead, allow: new Set([`${borDoc}::a/b.py`]) }),
      "ALLOWLISTED",
    );
    // 负样本：换一份非 borrowings 文档 + 本仓栈 ⇒ 真待查（不许被任何分支吃掉）
    chk(
      "真待查：非外部、非已删 ⇒ BROKEN",
      classifyBroken({
        docRel: "docs/plans/PLAN-declarative-schema-sync.md",
        refFile: "nope.rs",
        refText: "见 `nope.rs:1`",
        docHead: "",
        allow: new Set(),
      }),
      "BROKEN",
    );
    // 空名册（缺失）不得让判定改变
    chk("名册缺失（空集）时行为不变", cb({ refFile: "collab/task_source.py" }), "EXT-REPO");

    // ── 段内截断（ABBREV，2026-09-18「文档简称」规范落地）─────────────────────────
    // 用**真实形态**做样本：仓内确无 `actions.rs`，而 `opc_domain_pack_actions.rs` 存在
    // （实测见 output/verify-graph-2026-09-15/actions-probe-0918k.txt）。
    // 本判据的全部价值在于「把被误标成『真待查』的写法摘出来」⇒ 两个方向都必须钉死。
    const ixOf = (names) => new Map(names.map((n) => [n, [`${ROOT}/x/${n}`]]));
    chk(
      "截断：`actions.rs` 是 `opc_domain_pack_actions.rs` 的字符后缀 ⇒ 命中",
      abbrevHitsFor("actions.rs", ixOf(["opc_domain_pack_actions.rs"])).length,
      1,
    );
    chk(
      "截断：带路径的引用串按 **basename** 判",
      abbrevHitsFor("a/b/domain_pack_bridge.rs", ixOf(["opc_domain_pack_bridge.rs"])).length,
      1,
    );
    // ★ 反向断言：等长同名**不是**截断 —— 去掉 `name.length > b.length` 这条会把它变成 1，
    //   于是「真·重名 / 缺路径」问题会被洗成「缩写」，那是拿判据掩盖行动面。
    chk("截断：等长同名 ⇒ **不算**截断（反向护栏）", abbrevHitsFor("nope.rs", ixOf(["nope.rs"])).length, 0);
    chk("截断：无关名 ⇒ 0", abbrevHitsFor("nope.rs", ixOf(["other.rs"])).length, 0);
    chk("截断：index 缺失 ⇒ 0（不抛错）", abbrevHitsFor("actions.rs", null).length, 0);
    chk(
      "分类：有截断候选 ⇒ ABBREV",
      cb({ refFile: "actions.rs", abbrevHits: ["src/commands/opc_domain_pack_actions.rs"] }),
      "ABBREV",
    );
    chk("分类：无截断候选 ⇒ 仍是 BROKEN（不误报）", cb({ refFile: "nope.rs", abbrevHits: [] }), "BROKEN");
    // 优先级：DELETED-SRC / EXT-REPO 都**优先于** ABBREV（更强的断言先认）
    chk(
      "优先级：自陈已删 > 截断",
      cb({ refFile: "actions.rs", refText: "该文件已拆成同名目录", abbrevHits: ["x/opc_domain_pack_actions.rs"] }),
      "DELETED-SRC",
    );
    chk(
      "优先级：外部仓库 > 截断",
      cb({ refFile: "collab/task_source.py", abbrevHits: ["x/collab/task_source.py"] }),
      "EXT-REPO",
    );
  }

  console.log(bad === 0 ? "\n✔ selftest 全过：两类判据都能红" : `\n✖ selftest 失败 ${bad} 项`);
  process.exit(bad === 0 ? 0 : 2);
}

// ── 主流程 ───────────────────────────────────────────────────────────
const failures = []; // 客观错（BROKEN / 越界 / 空行 / 载体值不等）
const suspects = []; // 软判据：疑似腐烂
const notes = []; // 提示 / 覆盖范围自陈
const nonloc = new Map(); // NONLOC：无法定位的引用（计数用，不算失败）
const nonlocSamples = []; // 未覆盖的引用点名（不列的话「0 腐烂」会被误读成「全查过了」）
const nonlocDetail = []; // --dump-nonloc 用：引用处行原文 + 抽取器给出的 srcLine（**不自己再找 needle**）
// EXTERNAL 明细。⚠ 没有它，「EXTERNAL N 条」就是一个**无法审计的数**：既看不到是哪几条、
// 也看不到它们凭什么被判免检 —— 而 M1 把该判定提到 `locate()` 之前后，它的**生效面变宽了**
// （原先只覆盖 `N`/`B`，现在覆盖全部），于是「有多少条是借块级标记搭便车免检的」必须可见。
const externalDetail = [];
// 真验证结果（2026-09-17 扩容；判据强度分层的理由见 `verifyExternal`）：
const externalStale = []; // 版本 ∉ `Cargo.lock` ⇒ **硬拦**
const externalNoFile = []; // registry 里无此路径 ⇒ 只报告
const externalUnverified = []; // registry 不可用 / 拆不出 crate-ver / 非本项目依赖 ⇒ **未验证**

// ① 行号引用
const rustFiles = collectRustFiles();
if (rustFiles.length === 0) {
  console.error("✖ 扫描面为空 —— 未找到任何 .rs（路径约定变了？），不是「没有引用」");
  process.exit(2);
}
const index = getBasenameIndex();
const lineCache = new Map();
const linesOf = (abs) => {
  if (!lineCache.has(abs)) lineCache.set(abs, fs.readFileSync(abs, "utf8").split(/\r?\n/));
  return lineCache.get(abs);
};

let refCount = 0;
let externalCount = 0; // 第三方 crate 源码引用（本就不该在本仓解析）
const seen = new Set(); // 同一引用常被同一文件重复引用多次 ⇒ 去重，否则输出被淹
for (const abs of rustFiles) {
  const text = fs.readFileSync(abs, "utf8");
  const blocks = commentBlocks(linesOf(abs));
  const refs = extractRefs(text);
  // 软判据的归属需要「同一行的全部引用」（判据见 `quotedTokensOwnedBy`）⇒ 先分组。
  const peersOfLine = new Map();
  for (const r of refs) {
    if (!peersOfLine.has(r.srcLine)) peersOfLine.set(r.srcLine, []);
    peersOfLine.get(r.srcLine).push(r);
  }
  for (const r of refs) {
    refCount++;
    const key = `${rel(abs)}::${r.file}:${r.line}`;
    if (seen.has(key)) continue;
    seen.add(key);

    // ── ① 第三方 crate 源码引用：与「能否在本仓定位」**正交**，故必须先问 ──────
    //
    // ⚠ 2026-09-17 重构（判据缺口，实测代价见下）：该判定原先**分别挂在 `N` / `B` 两个
    //   分支内部**，`L` 分支没有 ⇒ 只要引用串**能唯一定位**到某个同名文件，就永不进入
    //   该判定。根因不是「漏了一个分支」（那样补第三个分支就行），而是**判据挂错了层**：
    //   「这条引用指的不是本仓」这件事由**出处自己说的**（注释块里的 `name-x.y.z/`）
    //   决定，与「本仓恰不恰有同名文件」毫无关系 ⇒ 放进任何一个 `locate()` 结果分支，
    //   都等于把它的生效条件绑在一个无关变量上。
    //
    //   实测代价：`sea-orm-2.0.2/src/schema/entity.rs:158` 的引用串带 `/` ⇒
    //   `resolveRef` 失败 ⇒ 退到 basename 索引 ⇒ 命中**唯一**那份 `entity.rs`
    //   （`crates/trajectory/src/memory_providers/entity.rs`，仅 125 行）⇒ 判 `L` ⇒
    //   拿 158 去比 125 行 ⇒ 报「引用越界」。**同类假红 14 条**，且它们全都指向
    //   sea-orm 源码。⇒ 修复方式是把判定提到 `locate()` **之前**，从此任何新增分支都
    //   不可能绕过它（同族教训：判据 #485「方言限定声明在另一方言的期望集里整条不存在」，
    //   两者都是**判据生效范围被无关条件收窄**）。
    const blockId = blocks.ids[r.srcLine];
    if (blockId >= 0 && externalMarkFor(blocks.texts[blockId], r.file)) {
      externalCount++;
      // 分栏：`OWN` = 引用串**自己**带版本目录（`sea-orm-2.0.2/src/…`），基本无争议；
      // `BLOCK` = 引用串不带，靠**同块内指向同一目标的**版本目录免检。两栏都过了
      // `externalMarkFor` 的目标相容校验，故 `BLOCK` 不再是「搭便车」，只是「同一段里
      // 省略了公共路径前缀」的正常写法。
      const ownMark = externalMarkFor(r.text, r.file);
      // 真验证（2026-09-17 扩容）：优先用引用串自己带的标注，`BLOCK` 那条退回同块的公共前缀。
      const path = externalPathFor(r.text, r.file) ?? externalPathFor(blocks.texts[blockId], r.file);
      const verdict = verifyExternal(path);
      const entry = { src: rel(abs), srcLine: r.srcLine, file: r.file, line: r.line, text: r.text, ownMark, path, verdict };
      externalDetail.push(entry);
      if (verdict.kind === "STALE") externalStale.push(entry);
      else if (verdict.kind === "NOFILE") externalNoFile.push(entry);
      else if (verdict.kind !== "OK" && verdict.kind !== "OK_NO_REGISTRY") externalUnverified.push(entry);
      continue;
    }

    const loc = locate(r.file, index);
    if (loc.kind === "N") {
      nonloc.set(loc.why, (nonloc.get(loc.why) ?? 0) + 1);
      nonlocSamples.push(`${rel(abs)} → ${r.file}:${r.line}`);
      nonlocDetail.push({
        src: rel(abs),
        srcLine: r.srcLine,
        text: r.text,
        file: r.file,
        line: r.line,
        why: loc.why,
      });
      continue;
    }
    if (loc.kind === "B") {
      failures.push({ key, what: `${rel(abs)} 引用了不存在的文件 ${r.file}:${r.line}` });
      continue;
    }
    const target = loc.abs;
    const tl = linesOf(target);
    if (r.line <= 0 || r.line > tl.length) {
      failures.push({
        key,
        what: `${rel(abs)} 引用越界：${r.file}:${r.line}（${rel(target)} 共 ${tl.length} 行）`,
      });
      continue;
    }
    const cited = tl[r.line - 1].trim();
    if (cited === "" && !r.blankOk) {
      failures.push({ key, what: `${rel(abs)} 引用指向空行：${r.file}:${r.line}` });
      continue;
    }
    // ③ 硬判据：指向**裸控制流语句** ⇒ 行号必然已漂移（修法唯一：改成真正讲这件事的那一行）。
    //    区间终点（`blankOk`）豁免 —— 端点只是范围边界，不是被引用的那件事。
    if (!r.blankOk && CONTROL_FLOW_LINE.test(cited)) {
      failures.push({
        key,
        what:
          `${rel(abs)} 引用指向裸控制流语句：${r.file}:${r.line} 实为 \`${cited}\` ⇒ 行号已漂移` +
          `（修法唯一：改成真正讲这件事的那一行）`,
      });
      continue;
    }
    // 软判据：指向**收尾括号行** ⇒ 可能是「整段 / 区间」的松散写法，需人判。
    // 为什么不硬拦：区间终点（`:1913-1916` 的 `},`）与「行号漂移」在下游同形，实测 5 处里
    // 3 处是端点 ⇒ 硬拦会恒定假红（判据 #147）。**端点一并豁免**（`blankOk`），
    // 否则 `:A-B` 那个 B 会恒定刷 3 条纯噪声。
    if (!r.blankOk && CLOSING_LINE.test(cited)) {
      suspects.push({
        key,
        what: `${rel(abs)} → ${r.file}:${r.line} 指向收尾括号行 ⇒ 需人判（可能是「整段/区间」写法）`,
        at: `     出处现为：${cited.slice(0, 110)}`,
      });
    }
    // 软判据：同句引号里的值还能否在出处附近找到（±2 行）
    //
    // ⚠ 归属必须用 `quotedTokensOwnedBy`（只取归给本条引用的值），**不能**用
    //   `quotedTokens(r.text)`（整行的值）—— 后者在一行多引用时会产生 N×(N−1) 量级假红，
    //   实测 3 条全是假红（见该函数文档）。
    const toks = quotedTokensOwnedBy(r.text, peersOfLine.get(r.srcLine) ?? [r], r);
    if (toks.length === 0) continue;
    const window = tl.slice(Math.max(0, r.line - 3), r.line + 2).join("\n");
    if (!toks.some((t) => window.includes(t))) {
      suspects.push({
        key,
        what: `${rel(abs)} → ${r.file}:${r.line} 附近已找不到同句引号值 ${toks.map((t) => `"${t}"`).join("/")}`,
        at: `     出处现为：${tl[r.line - 1].trim().slice(0, 110)}`,
      });
    }
  }
}
if (refCount === 0) {
  console.error("✖ 一条行号引用都没抽到 —— 抽取器已失效（不是「没有引用」）");
  process.exit(2);
}

// ② reranker 模型文件名
if (!fs.existsSync(RAG_CONFIG)) {
  console.error(`✖ 真源文件不存在：${rel(RAG_CONFIG)}`);
  process.exit(2);
}
const truth = readRerankerConst(fs.readFileSync(RAG_CONFIG, "utf8"));
if (!truth) {
  console.error(`✖ 未能在 ${rel(RAG_CONFIG)} 读到 ${CONST_NAME} —— 真源没了（不是「无需检查」）`);
  process.exit(2);
}

let carrierCount = 0;
let allGguf = 0;
for (const abs of collectFrontendFiles()) {
  const text = fs.readFileSync(abs, "utf8");
  allGguf += [...text.matchAll(/"[^"]+\.gguf"/g)].length;
  for (const hit of findCrossEncoderLiterals(text)) {
    carrierCount++;
    if (hit.value !== truth) {
      failures.push({
        key: `reranker@${rel(abs)}`,
        what: `${rel(abs)} 的 crossEncoderModel 字面量 "${hit.value}" ≠ 真源 ${CONST_NAME} = "${truth}"`,
      });
    }
  }
}
if (carrierCount === 0) {
  console.error(
    `✖ 前端一条 crossEncoderModel 字面量都没找到 —— 载体没了，说明锚点失效（不是「无需检查」）`,
  );
  process.exit(2);
}
// 未被锚点覆盖的 .gguf 字面量 ⇒ 只提示（可能是合法它模型，如 embedding 的 bge-m3）
if (allGguf > carrierCount) {
  notes.push(
    `前端另有 ${allGguf - carrierCount} 处 .gguf 字面量不在 crossEncoderModel 上（可能是其它模型，未纳入硬拦；新增 reranker 载体请用 crossEncoderModel 字段）`,
  );
}

// ── 棘轮：存量客观错走白名单，只拦新增 ────────────────────────────────
const loadBaseline = () => {
  if (!fs.existsSync(BASELINE_PATH)) return { known: [] };
  try {
    return JSON.parse(fs.readFileSync(BASELINE_PATH, "utf8"));
  } catch (e) {
    console.error(`✖ 基线解析失败（${rel(BASELINE_PATH)}）：${e.message}`);
    process.exit(2);
  }
};
const baseline = loadBaseline();
const known = new Set(baseline.known ?? []);
// ⚠ 必须看「文件是否存在」，不能看 `baseline.known` 真值性：首次运行时它是 `[]`
// 而 **`[]` 在 JS 里是真值** ⇒ 会把「首次 bootstrap」误判成「已有基线 ⇒ 拒绝新增」，
// 于是基线永远写不进去、脚本永远退 1（实测踩到）。
const HAD_BASELINE = fs.existsSync(BASELINE_PATH);

if (UPDATE_BASELINE) {
  // 只允许「当前失败集 ⊆ 基线」时重写；出现新增失败时拒绝下调（棘轮只减不增的守门）
  const added = failures.filter((f) => !known.has(f.key));
  if (added.length > 0 && HAD_BASELINE) {
    console.error(`✖ 有 ${added.length} 处**新增**客观错，拒绝更新基线（棘轮只减不增）：`);
    for (const f of added) console.error(`   · ${f.what}`);
    process.exit(1);
  }
  const out = {
    note: "行号引用「客观错」的棘轮基线：**只减不增**。修复后跑 --update-baseline 下调，否则基线虚高 = 棘轮失效。",
    generatedBy: "scripts/check-single-source-facts.mjs",
    count: failures.length,
    known: failures.map((f) => f.key).sort(),
  };
  fs.writeFileSync(BASELINE_PATH, JSON.stringify(out, null, 2) + "\n", "utf8");
  console.log(`✔ 基线已写入 ${rel(BASELINE_PATH)}（${failures.length} 条）`);
  process.exit(0);
}

const isNew = (f) => !known.has(f.key);
const newFailures = failures.filter(isNew);
const oldFailures = failures.filter((f) => !isNew(f));

// ── 报告 ─────────────────────────────────────────────────────────────
// 取证模式（--dump-doc-refs）：**活文档**（`.md`）的行号引用体检。
//
// 同样必须在**任何 exit 之前**短路，但理由与 `--dump-nonloc` 不同：
//   · `--dump-nonloc` 怕的是「存在一条 BROKEN ⇒ 永远 dump 不出来」（它要**修**那些引用）；
//   · 本模式与主线上的一切——BROKEN / 版本陈旧 / 软判据——**毫无因果关系**。它要回答的是
//     「`.md` 这个盲区里有多少存量」，而**只要主线上有任何一条红**（比如版本陈旧那条
//     `process.exit(1)`），放在后面就永远看不到盲区的量 ⇒ 那正是「先量化再决定」卡死的位置。
//
// ⚠ 本模式**不产出 failure / suspect、不改退出码**（恒 `process.exit(0)`）：理由是
//   `classifyDocRef` 里写的「机器分不出陈旧与考古」。把这个盲区的粗筛结果接进硬判据，
//   等于用一条**会恒定假红**的判据替换「没有判据」—— 那不是修复，是噪声（判据 #7）。
if (DUMP_DOC_REFS) {
  const allowInfo = getExtRepoAllowlist();
  const scan = collectDocFiles();
  console.log(
    `# 活文档行号引用体检（--dump-doc-refs）\n` +
      `#   · 引用集合 = 本门禁的 extractRefs 产出（与 .rs 侧**同一份抽取器**，判据 #487）\n` +
      `#   · 目标定位 = locate() + 同一份 basename 索引（同含 SKIP_DIRS 口径）\n` +
      `#   · 粗筛判据 = classifyDocRef（**与 .rs 侧同一套正则 + 同一条 blankOk 豁免**）\n` +
      `#   · BROKEN 细分 = classifyBroken（EXT-REPO 外部仓库源码 / DELETED-SRC 本仓已删 / ` +
      `ALLOWLISTED 显式名册 / ABBREV **段内截断**（真实 basename 的字符后缀、非路径段对齐 ⇒ 机械不可解析）/ ` +
      `BROKEN 真待查）—— **细分只给行动面，不改任何真伪判定**\n` +
      `#   · 显式豁免名册 = ${
        allowInfo.err
          ? `⚠ ${allowInfo.err}`
          : `external-repo-refs.json（${allowInfo.n} 条 ⇒ 文件级 ${allowInfo.fileKeys.size} 键「**仅 BROKEN**」/ ` +
            `行级 ${allowInfo.lineKeys.size} 键「**仅 NONLOC**」；两集互不相通，见 splitAllowlist）` +
            (allowInfo.bad.length ? `｜⚠ 零效力条目 ${allowInfo.bad.length}` : ``)
      }\n` +
      `# ⚠ 只取证：不产出 failure / suspect、退出码恒 0（理由见 classifyDocRef）\n`,
  );
  const seenDoc = new Set();
  // 名册条目的**命中留痕**：三栏（live / memory / hist）**共用同一个集合** —— 若按栏各记一份，
  // 某条目「零命中」就会被误读成名册腐烂，而真相可能只是该条目所在的 doc 不在那一栏里（判据 #147）。
  const allowSeen = new Set();
  const audit = (files) => {
    const st = {
      refs: 0,
      uniq: 0,
      ext: 0,
      loc: 0,
      miss: new Map(),
      sus: [],
      perFile: [],
      brokenClass: new Map(), // BROKEN 细分：EXT-REPO / DELETED-SRC / ALLOWLISTED / BROKEN
      brokenSamples: new Map(), // 每类**全量点名** —— 否则细分也只是一堆数字（判据 #16）
      // NONLOC 细分（2026-09-18 第二轮）：ALLOWLISTED / OVERFLOW / OTHER。
      // ⚠ 与 brokenClass 并列但**性质不同**：它不改任何计数（NONLOC 本就不产生可疑），
      //   只决定「点名谁」。见 classifyNonloc 的注释。
      nonlocClass: new Map(),
      nonlocSamples: new Map(),
    };
    for (const abs of files) {
      const text = fs.readFileSync(abs, "utf8");
      const all = extractRefs(text);
      const docRel = rel(abs);
      // 「参考边界」声明只在**文档头部**找：正文里也可能出现「参考边界」四个字
      // （比如本节自己在讨论这条判据），扫全文会把整份文档变成外部豁免区。
      const docHead = text.split(/\r?\n/).slice(0, DECL_SCAN_LINES).join("\n");
      let loc = 0;
      let myUniq = 0;
      const localSus = [];
      for (const r of all) {
        st.refs++;
        const key = `${rel(abs)}::${r.file}:${r.line}`;
        if (seenDoc.has(key)) continue; // 同一引用常在同一文件里重复 N 次 ⇒ 去重（与 .rs 侧同口径）
        seenDoc.add(key);
        st.uniq++;
        myUniq++;
        // ── ① 第三方 crate 源码：与「能否在本仓定位」**正交** ⇒ 必须**先问**（判据 #491）──
        // 漏掉这一步的代价（实测）：`sea-orm-2.0.2/src/schema/entity.rs:156` 会被 `locate()`
        // 按 basename 认成**本仓**的 `entity.rs` ⇒ 行号对不上 ⇒ 报「越界」。那是纯假红，
        // 而且**成片**出现（`PLAN-declarative-schema-sync.md` 有整节在讨论 sea-orm / rhai 源码）。
        // 判据两层，与 `.rs` 侧同源：① 引用串自带版本目录；② 同行有**同一目标**的完整路径
        // （判据 #485：别让一条判据的生效面被无关条件放大或缩小）。
        if (EXTERNAL_MARK.test(r.file) || externalMarkFor(r.text, r.file)) {
          st.ext++;
          continue;
        }
        const l = locate(r.file, index);
        if (l.kind !== "L") {
          // 「无法定位 ⇒ 未检查」在 `.md` 侧同理：不点名的话「0 可疑」会被误读成「全查过了」。
          const why = l.kind === "B" ? "BROKEN（仓库内无此文件；**未检查**）" : l.why;
          st.miss.set(why, (st.miss.get(why) ?? 0) + 1);
          // ── BROKEN 细分（2026-09-18）──────────────────────────────────────
          // ⚠ 只在 `l.kind === "B"` 上调用：NONLOC（同名多份）说明**本仓有这个文件**，
          //   给它挂「外部仓库」标签就是 #485 的重演 ⇒ 结构上不给调用点。
          if (l.kind === "B") {
            // 段内截断检测（`actions.rs` ← `opc_domain_pack_actions.rs`）：见 abbrevHitsFor。
            // **先算、再分类、再点名** —— 命中清单要跟着点名一起打出来（下）：
            // 只有「把它缩写的那个真实文件」写在同一条上，读者才能判断这是真缩写还是巧合后缀。
            const abbrevHits = abbrevHitsFor(r.file, index);
            const cls = classifyBroken({
              docRel,
              refFile: r.file,
              refText: r.text,
              docHead,
              // 文件级键 —— BROKEN 只认「整份文件无从验证」这一类理由（见 splitAllowlist 注释）
              allow: allowInfo.fileKeys,
              abbrevHits,
            });
            if (cls === "ALLOWLISTED") allowSeen.add(`${docRel}::${r.file}`);
            st.brokenClass.set(cls, (st.brokenClass.get(cls) ?? 0) + 1);
            if (!st.brokenSamples.has(cls)) st.brokenSamples.set(cls, []);
            const arr = st.brokenSamples.get(cls);
            // **全量点名**：细分的意义就是给出行动清单，只列前 8 条等于又造一个黑箱
            //（判据 #16：不点名的计数会被误读成「都查过了」）。
            // ABBREV 额外带上命中的真实文件 —— 否则「段内截断」只是我的一句断言（自证 > 自陈）。
            arr.push(
              `${docRel}:${r.srcLine + 1} → ${r.file}:${r.line}` +
                (cls === "ABBREV" ? `（真实文件：${abbrevHits.join(" / ")}）` : ``),
            );
          }
          // ── NONLOC 细分（2026-09-18 第二轮，用户裁决）─────────────────────
          // 存在的理由：NONLOC 是**唯一「既不报可疑、又从未检查」**的桶 ——
          //   它在上面的 miss 里只有一个「同名 N 份」的计数，**不点名**任何一条。
          //   于是「该不该去补路径」这件事上，它和「检查过且通过」在下游同形；
          //   而它的实际代价是**判据盲区**：行号根本没被读过。
          // 细分只给行动面，**不改任何计数**：miss / sus / 退出码全不动。
          if (l.kind === "N") {
            const cands = (index.get(r.file.slice(r.file.lastIndexOf("/") + 1)) ?? []).map((c) => ({
              rel: rel(c),
              lines: linesOf(c).length,
            }));
            // ⚠ **行级**键（`allowHitNonloc`）：文件级键在结构上够不着 NONLOC（见 splitAllowlist）。
            const cls = classifyNonloc(cands, r.line, allowHitNonloc(allowInfo, docRel, r.file, r.line));
            st.nonlocClass.set(cls, (st.nonlocClass.get(cls) ?? 0) + 1);
            if (!st.nonlocSamples.has(cls)) st.nonlocSamples.set(cls, []);
            const at = `${docRel}:${r.srcLine + 1} → ${r.file}:${r.line}`;
            st.nonlocSamples.get(cls).push(at);
            if (cls === "ALLOWLISTED") allowSeen.add(`${docRel}::${r.file}::${r.line}`);
          }
          continue;
        }
        st.loc++;
        loc++;
        const tl = linesOf(l.abs);
        const cited = r.line >= 1 && r.line <= tl.length ? tl[r.line - 1] : undefined;
        const why = classifyDocRef(cited, r);
        if (why) {
          localSus.push({
            at: `${rel(abs)}:${r.srcLine + 1}`,
            key: `${r.file}:${r.line}`,
            why,
            cited: (cited ?? "(越界)").trim(),
            text: r.text.trim(),
          });
        }
      }
      st.perFile.push({ abs, all: all.length, uniq: myUniq, loc, sus: localSus.length });
      st.sus.push(...localSus);
    }
    return st;
  };

  const missN = (st) => [...st.miss.values()].reduce((a, b) => a + b, 0);
  const render = (label, st, note) => {
    console.log(`${"=".repeat(96)}`);
    console.log(`【${label}】${st.perFile.length} 份 .md ⇒ ${st.refs} 条引用（去重后 ${st.uniq}）；` +
      `第三方源码 ${st.ext}（免检）｜可定位 ${st.loc}｜**无法定位（未检查）** ${missN(st)}｜粗筛可疑 ${st.sus.length}`);
    // **分桶恒等**：每条唯一引用必须进且仅进一个桶（外部 / 可定位 / 未检查）。
    // 不成立 ⇒ 上面那个「可疑数」是假的 —— 判据 #7：统计量恒等先怀疑测量工具，这次是工具自检。
    console.log(
      `  分桶恒等 ${st.ext + st.loc + missN(st) === st.uniq ? "true" : `FALSE（${st.ext}+${st.loc}+${missN(st)} ≠ ${st.uniq}）`}`,
    );
    console.log(`  ${note}`);
    for (const [why, n] of [...st.miss.entries()].sort((a, b) => b[1] - a[1])) {
      console.log(`   · 未检查 ${n} 条：${why}`);
    }
    // ── BROKEN 细分（2026-09-18）：给「115 条」一个行动面 ────────────────
    // 自检：细分之和必须 == BROKEN 计数。不等 ⇒ 分类器漏/重了条目，
    // 此时**整个细分都不可信**（判据 #7：统计量恒等先怀疑测量工具）。
    if (st.brokenClass && st.brokenClass.size > 0) {
      const tot = [...st.brokenClass.values()].reduce((a, b) => a + b, 0);
      const brokenN = st.miss.get("BROKEN（仓库内无此文件；**未检查**）") ?? 0;
      console.log(
        `  ── BROKEN 细分（合计 ${tot} vs BROKEN 计数 ${brokenN}：${tot === brokenN ? "✔ 闭合" : "✖ 不闭合，细分不可信"}）:`,
      );
      for (const [k, n] of [...st.brokenClass.entries()].sort((a, b) => b[1] - a[1])) {
        console.log(`   ${String(n).padStart(4)}  ${k}`);
        const s = st.brokenSamples.get(k) ?? [];
        for (const x of s) console.log(`           ${x}`);
        if (n > s.length) console.log(`           …另 ${n - s.length} 条`);
      }
    }
    // ── NONLOC 细分（2026-09-18 第二轮）：给「同名 N 份」那个纯计数一个行动面 ──────
    // 自检：三个桶之和必须 == NONLOC 总数（= 未检查总数 − BROKEN 计数）。
    // 不等 ⇒ 分类器漏/重了条目，此时**整个细分都不可信**（判据 #7）。
    if (st.nonlocClass && st.nonlocClass.size > 0) {
      const NONLOC_LABEL = {
        ALLOWLISTED: "名册豁免（自指反例段 / 门禁演进史举例 —— 不需验证）",
        OVERFLOW: "**行号越界**（同名候选**无一份**够长 ⇒ 与指向哪份无关，必然错）",
        OTHER: "其余（本仓确有同名文件，行号对某些候选成立 ⇒ 需人工补路径）",
      };
      const tot = [...st.nonlocClass.values()].reduce((a, b) => a + b, 0);
      const brokenN = st.miss.get("BROKEN（仓库内无此文件；**未检查**）") ?? 0;
      const nonlocN = missN(st) - brokenN;
      console.log(
        `  ── NONLOC 细分（合计 ${tot} vs NONLOC 计数 ${nonlocN}：${tot === nonlocN ? "✔ 闭合" : "✖ 不闭合，细分不可信"}）` +
          `—— **不改任何计数**，只给行动面:`,
      );
      for (const k of ["OVERFLOW", "ALLOWLISTED", "OTHER"]) {
        const n = st.nonlocClass.get(k) ?? 0;
        console.log(`   ${String(n).padStart(4)}  ${NONLOC_LABEL[k]}`);
        // `OVERFLOW` 量小且每条都要行动 ⇒ **全量点名**；`OTHER` 量可能很大 ⇒ 只点名不列（见下）
        if (k === "OTHER") {
          if (n) console.log(`           （${n} 条 —— 量大，不在此逐条列；用 --dump-nonloc 取候选明细）`);
          continue;
        }
        for (const x of st.nonlocSamples.get(k) ?? []) console.log(`           ${x}`);
      }
    }
    const withRefs = st.perFile.filter((f) => f.all > 0);
    console.log(`  ── 逐文件（仅列有引用的 ${withRefs.length} 份；` +
      `列的是 原始条数 / 去重后可定位 / 可疑）:`);
    for (const f of withRefs) {
      console.log(
        `   原始 ${String(f.all).padStart(3)}（去重 ${String(f.uniq).padStart(3)}）/ 可定位 ${String(f.loc).padStart(3)} / 可疑 ${String(f.sus).padStart(2)}  ${rel(f.abs)}`,
      );
    }
  };

  const live = audit(scan.live);
  render(
    "A 栏：**活计划 + 规范**（`docs/plans/**` + 根级 *.md）—— 应保持准确 ⇒ 陈旧就该订正",
    live,
    "⚠ 仍是**粗筛**，不是待办清单：① 同一份文档里「当前指针」与「修前 / 当时」两种写法**并存**；" +
      "② **已知假红形态** —— 文档里「讨论引用写法本身 / 门禁演进史」的段落（如 `PLAN-weknora-borrowings.md` §12.x）" +
      "会把被当作**反例**引用的行号也抽出来（如 `--> src/main.rs:4:5` 被读成 `src/main.rs:5`），那几条不是待办。",
  );
  if (live.sus.length === 0) {
    console.log("\n✔ A 栏粗筛 0 可疑（注意「0 可疑」≠「全查过了」—— 未检查条数见上）");
  } else {
    console.log(`\n# A 栏可疑明细 ${live.sus.length} 条（**需人判**，不要批量订正）:`);
    for (const s of live.sus) {
      console.log(`   ~ ${s.at} → ${s.key}  [${s.why}]`);
      console.log(`       出处现为：${s.cited.slice(0, 120)}`);
      console.log(`       引用句：  ${s.text.slice(0, 160)}`);
    }
  }

  // B / C 栏**只报计数**：B 栏混装、C 栏是史料 —— 两栏的行号都**不该**按「应指向现状」来要求。
  // 列明细会诱导人去改（那是**破坏史料**，判据 #490 的反面），所以刻意不列。
  for (const [label, files, note] of [
    [
      "B 栏：记忆（.workbuddy/memory/**）",
      scan.memory,
      "⚠ 混装：判据层里的「当前指针」型与流水里的「当时事实」型并存 ⇒ 单列报告、**由人判**。",
    ],
    [
      "C 栏：历史记录（docs/ 除 plans 外 + output/**）",
      scan.hist,
      "`docs/audits/AUDIT-*-<日期>.md` 是**调查快照**、`output/**` 是历史报告 / 备份副本 ⇒ " +
        "行号记的是**当时**事实，**不订正**，仅计数。",
    ],
  ]) {
    const st = audit(files);
    console.log(`${"=".repeat(96)}`);
    console.log(`【${label}】${st.perFile.length} 份 .md ⇒ ${st.refs} 条引用（去重 ${st.uniq}）；` +
      `第三方源码 ${st.ext}｜可定位 ${st.loc}｜未检查 ${missN(st)}｜粗筛可疑 ${st.sus.length}`);
    console.log(`  ${note}`);
  }
  // ── 名册自检（2026-09-18 第三轮，**纯增量**：不改任何计数、不改退出码）───────────────
  // 检的是名册**自身**的腐烂：① 条目**零命中**（声明的键无人命中 ⇒ 键写错 / 条目已过期）；
  //   ② `lines` 非法的条目（零效力，见 `splitAllowlist` 的反向护栏）。
  // ⚠ 零命中**未必**是腐烂，必须先排除两种口径：· 该 doc 不在本次扫描面（三栏已共用 allowSeen）；
  //   · 该引用被 EXTERNAL 判定**先行截获**（引用串自带版本目录）⇒ `classifyBroken` 结构上不被调用。
  //   ⇒ 本行只报告、不判定（本模式恒 exit 0，理由见 classifyDocRef）。
  {
    const decl = new Map();
    for (const k of allowInfo.fileKeys) decl.set(k, "（文件级 ⇒ 仅 BROKEN）");
    for (const k of allowInfo.lineKeys) decl.set(k, "（行级 ⇒ 仅 NONLOC）");
    const unhit = [...decl.keys()].filter((k) => !allowSeen.has(k));
    console.log(
      `\n# 名册自检：声明 ${decl.size} 键（文件级 ${allowInfo.fileKeys.size} / 行级 ${allowInfo.lineKeys.size}）` +
        `｜命中 ${decl.size - unhit.length}｜**零命中 ${unhit.length}**` +
        (unhit.length ? " ⇒ 需逐条判「键写错 / 已过期 / 被 EXTERNAL 截获」：" : " ✔"),
    );
    for (const k of unhit) console.log(`   ⚠ 零命中 ${k} ${decl.get(k)}`);
    if (allowInfo.bad.length) {
      console.log(`   ⚠ 零效力条目 ${allowInfo.bad.length} 条（lines 非法 ⇒ **不放宽**为文件级）：`);
      for (const b of allowInfo.bad) console.log(`      ${b}`);
    }
  }
  // ── 备份副本自陈（2026-09-18 第三轮）───────────────────────────────────────────
  // 起因：C 栏「粗筛可疑」出现过 **+2 未归因**。实测归因 = `output/backup-2026-09-18-round17-linefix/`
  //   里那 2 份 `docs__plans__PLAN-*.md`（**改前副本**）仍含 `storage.rs:488` ⇒ 指向 `}`（收尾行），
  //   各贡献 1 条可疑（改后副本 round17b 不含该引用 ⇒ 贡献 0）。⇒ 备份副本与 A 栏原件**同源**，
  //   被计进 C 栏时读数是**备份策略的函数**、不是「史料规模」的函数 ⇒ C 栏读数随时间不可比。
  // ⚠ 本行**只自陈、不排除**：把 `output/backup-*/` 从扫描面剔掉 = 缩小判据面 = **变松**
  //   （与 #485 同族），那种「读数变好看」的改法正是本脚本反复吃过亏的形态。
  //   约定：备份副本**不沿用 `.md` 后缀**（用 `.md.orig`）⇒ 自然脱出扫描面，且原件必在 A / B 栏被扫。
  {
    const mdUnder = (dir) => {
      let es = [];
      try {
        es = fs.readdirSync(dir, { withFileTypes: true });
      } catch {
        return [];
      }
      const acc = [];
      for (const e of es) {
        const p = path.join(dir, e.name);
        if (e.isDirectory()) acc.push(...mdUnder(p));
        else if (e.isFile() && e.name.endsWith(".md")) acc.push(p);
      }
      return acc;
    };
    const backupMd = [];
    try {
      const outDir = path.join(ROOT, "output");
      for (const e of fs.readdirSync(outDir, { withFileTypes: true })) {
        if (e.isDirectory() && /^backup-/.test(e.name)) backupMd.push(...mdUnder(path.join(outDir, e.name)));
      }
    } catch {
      /* output/ 不存在 = 无备份，安全 no-op */
    }
    console.log(
      `# 备份副本自陈：\`output/backup-*/\` 下 .md 共 ${backupMd.length} 份 ⇒ **会被计入 C 栏**` +
        (backupMd.length
          ? `（与原件同源 ⇒ 重复/版本计数）。约定：副本改用 \`.md.orig\` ⇒ 自然脱出扫描面；` +
            `**不在此排除**（排除 = 缩小判据面 = 变松）。`
          : ` ✔`),
    );
  }
  console.log(
    `\n# 定案口径：**只有 A 栏**是「应保持准确」的（逐条人判：当前指针 ⇒ 订正；修前·当时 ⇒ **保留**）；\n` +
      `#   B / C 栏**不动** —— 把史料改成当前值是破坏，不是修复。\n` +
      `# 另见判据 #490：任何**增删行**的改动，收尾必须自己扫一遍「指向被改文件的引用」；\n` +
      `#   .md 侧同理，而本门禁的**主判据不覆盖它** —— 这正是本条开关存在的理由。`,
  );
  process.exit(0);
}

// 取证模式（--dump-nonloc）：在**任何 exit 之前**短路，因为它要服务的目标是
// 「把 NONLOC 逐条补成 LOCATED」，与「有没有客观错」无关 —— 若放在后面，
// 只要存在一条 BROKEN 就永远 dump 不出来。
if (DUMP_NONLOC) {
  // 候选过滤判据（**确定性**，不是启发式）：引用处那一行的原文里若出现了带 `/` 的路径片段
  // （如 `src/commands/wiki.rs`、`crates/disk-cache`），而某候选的仓库相对路径包含它，
  // 那这条引用指向该候选就是**原文自己说的**。无片段命中时退回「全列但限量」。
  const pathFrags = (t) =>
    [...new Set([...t.matchAll(/[A-Za-z0-9_][A-Za-z0-9_./-]*\/[A-Za-z0-9_./-]+/g)].map((m) => m[0]))];
  console.log(
    `# NONLOC 取证（${nonlocDetail.length} 条）—— 引用集合由本门禁的 extractRefs 产出，` +
      `与检查面**同源**；候选清单由 locate() 同一份 basename 索引产出（同含 SKIP_DIRS 口径）。\n`,
  );
  let starredTotal = 0;
  for (const d of nonlocDetail) {
    console.log("=".repeat(96));
    console.log(`【引用处】${d.src}:${d.srcLine + 1}  →  ${d.file}:${d.line}`);
    console.log(`  ${d.text.trim().slice(0, 200)}`);
    const base = d.file.slice(d.file.lastIndexOf("/") + 1);
    const cands = index.get(base) ?? [];
    const frags = pathFrags(d.text);
    const starred = cands.filter((c) => {
      const r = rel(c);
      return frags.some((f) => r.includes(f) || f.includes(r));
    });
    if (frags.length) console.log(`  原文路径片段: ${frags.map((f) => `\`${f}\``).join(" ")}`);
    const show = starred.length ? starred : cands.slice(0, 5);
    console.log(
      `  候选 basename=${base} 共 ${cands.length} 份` +
        (starred.length ? `，路径片段命中 ${starred.length} 份 ★：` : `，无片段命中，列前 ${show.length} 份：`),
    );
    for (const c of show) {
      const cl = linesOf(c);
      const cited = (cl[d.line - 1] ?? "(越界)").trim();
      console.log(`    ${starred.length ? "★" : "·"} ${rel(c)}  [${cl.length} 行]  :${d.line} = ${cited.slice(0, 120)}`);
    }
    if (starred.length === 1) starredTotal++;
    else if (!starred.length) starredTotal += 0;
  }
  // EXTERNAL 明细：分 `OWN` / `BLOCK` 两栏（理由见 `externalDetail` 声明处）
  const own = externalDetail.filter((d) => d.ownMark);
  const blk = externalDetail.filter((d) => !d.ownMark);
  console.log(
    `\n# EXTERNAL 明细 ${externalDetail.length} 条（**真验证**：版本对 Cargo.lock、路径对 cargo registry）` +
      `—— OWN ${own.length}（引用串自带版本目录）` +
      `｜BLOCK ${blk.length}（靠同注释块别处的版本目录定位，**搭便车风险集中在这栏**）：`,
  );
  const vtxt = (d) =>
    `\n           verify=${d.verdict.kind}` +
    (d.verdict.crate ? ` ${d.verdict.crate}@${d.verdict.ver}` : ``) +
    (d.verdict.note ? `（${d.verdict.note}）` : ``) +
    `  mark=${d.path ?? "(none)"}`;
  for (const d of blk) {
    console.log(`   [BLOCK] ${d.src}:${d.srcLine + 1} → ${d.file}:${d.line}`);
    console.log(`           ${d.text.trim().slice(0, 150)}${vtxt(d)}`);
  }
  for (const d of own) {
    console.log(`   [OWN]   ${d.src}:${d.srcLine + 1} → ${d.file}:${d.line}${vtxt(d)}`);
  }
  console.log(`\n# 其中「路径片段唯一命中」的 ${starredTotal} 条可直接定案；其余需人工开候选核验。`);
  process.exit(0);
}

// 三分法必须**分开打印**：只报「失败」会让人以为扫描面覆盖了全部引用，
// 属于「门禁自陈覆盖范围」（判据 #16）的反面。
console.log(
  `扫描面：${rustFiles.length} 个 .rs ⇒ ${refCount} 条「文件:行」引用（去重后 ${seen.size} 条）；` +
    `reranker 真源 ${CONST_NAME} = "${truth}"，前端载体 ${carrierCount} 处`,
);
// ⚠ **覆盖范围自陈**：不覆盖什么，必须落到**输出**里（写在注释里不算 —— 判据 #7）。
// 「未检查」与「检查且通过」在输出里长得一样，是本门禁最容易被误读的地方：
// 上面那行 `0 条失败` 很容易被读成「全仓引用都没问题」，而事实上 `.md`（`docs/plans/*.md`
// 里成篇的 `文件:行` 论证）**一条都没被查过**。起因：2026-09-17 登记，报告 §7 第 11 项。
// 为何只自陈、不把 `.md` 接进判据：`.md` 里「当前指针」与「修前 / 当时」两种写法**并存**，
// 硬拦会恒定假红（详见 `classifyDocRef`）。⇒ 给一个**取证开关**，量由人判。
notes.push(
  `**未覆盖面自陈**：本门禁只扫 \`.rs\` ⇒ \`.md\`（活文档 \`docs/**\` + 根级、记忆、历史报告）\n` +
    `     里的「文件:行」引用**一条都没查过** —— 上面「BROKEN/越界/空行 0」**不覆盖它们**。\n` +
    `     体检请跑 \`node scripts/check-single-source-facts.mjs --dump-doc-refs\`` +
    `（只取证、退出码恒 0）。`,
);
const locatedN =
  seen.size - failures.length - [...nonloc.values()].reduce((a, b) => a + b, 0) - externalCount;
console.log(
  `分类：LOCATED ${locatedN}（硬判据 + 软判据都跑）｜` +
    `NONLOC ${[...nonloc.values()].reduce((a, b) => a + b, 0)}（**无法定位 ⇒ 未检查**）｜` +
    `EXTERNAL ${externalCount}（第三方 crate 源码，本就不该在本仓解析）｜` +
    `BROKEN/越界/空行/控制流 ${failures.length}`,
);
// EXTERNAL **真验证**自陈（2026-09-17 扩容）。⚠ 必须打印**分母**：只报「异常 0 条」会把
// 「未验证」混进「已验证」—— `NOT_DEP`（不是本项目依赖）/ `NO_LOCK` / registry 不可用那几类
// 其实**一条也没验**（判据 #7：`0 命中 ≠ 没问题`）。
if (externalCount > 0) {
  // ⚠ 必须把 `OK_NO_REGISTRY` **单独成栏**：它与 `OK` 都算「版本匹配」，但**路径压根没验**。
  // 实测踩到：最初把两者合起来只报一个「版本匹配 N」，于是把 `USERPROFILE` 指到不存在的
  // 目录（模拟离线）跑出来的输出与在线**一字不差** —— 「跳过」被伪装成了「检查过且通过」
  // （判据 #7 的同族：自陈必须落到**输出**里，写在注释里不算）。
  const noReg = externalDetail.filter((d) => d.verdict.kind === "OK_NO_REGISTRY").length;
  const okN =
    externalDetail.length -
    externalStale.length -
    externalNoFile.length -
    externalUnverified.length -
    noReg;
  const why = [...new Set(externalUnverified.map((d) => d.verdict.kind))].join("/");
  console.log(
    `   · EXTERNAL 真验证：版本匹配 + 路径已验证 ${okN}｜仅版本匹配 ${noReg}` +
      `（registry 不可用 ⇒ 路径**未验**）｜**版本陈旧 ${externalStale.length}**｜` +
      `路径缺失（只报）${externalNoFile.length}｜未验证 ${externalUnverified.length}` +
      (why ? `（${why}）` : ``),
  );
}
for (const [why, n] of [...nonloc.entries()].sort((a, b) => b[1] - a[1])) {
  console.log(`   · NONLOC ${n} 条：${why}`);
}
// 未覆盖的引用**点名列出**：不列的话「0 腐烂」会被误读成「全都查过了」
if (nonlocSamples.length > 0) {
  const uniq = [...new Set(nonlocSamples)].sort();
  // ⚠ 2026-09-17：原为 `slice(0, 12)` + 「另 N 条」。截断的代价不是「输出短一点」，
  //   而是**这份清单不可施工** —— 用户要「逐条补路径」时，19 条看不见的只能靠重跑
  //   别的工具去凑，而凑出来的清单与被检查的集合不是同一个（判据 #16 的反面：
  //   自陈覆盖范围必须与实际扫描面同源）。31 条的量级直接全列。
  notes.push(
    `以下 ${uniq.length} 条引用**无法定位因而未检查**（裸同名文件，如 ` +
      `\`mod.rs\`/\`lib.rs\`/\`state.rs\`；补上路径即可纳入检查）：\n     ` +
      uniq.join("\n     "),
  );
}

if (oldFailures.length > 0) {
  console.log(`\n⏸ 基线内已知客观错 ${oldFailures.length} 处（不拦，但**只减不增**）`);
  for (const f of oldFailures) console.log(`   · ${f.what}`);
}
if (suspects.length > 0) {
  console.error(`\n~ 疑似腐烂 ${suspects.length} 处（软判据：出处附近已无同句引号值 ⇒ 需人判）`);
  for (const s of suspects) {
    console.error(`   ~ ${s.what}`);
    if (s.at) console.error(s.at);
  }
}
for (const n of notes) console.log(`ℹ ${n}`);

if (externalStale.length > 0) {
  console.error(`\n✖ EXTERNAL 版本陈旧 ${externalStale.length} 条（修法唯一，**硬拦**）：`);
  for (const d of externalStale) {
    console.error(`   ✖ ${d.src}:${d.srcLine + 1} → ${d.path}`);
    console.error(`        ${d.verdict.crate} ${d.verdict.ver}：${d.verdict.note}`);
  }
  console.error(
    `\n  · 引用指向的是 cargo registry 里的**历史版本**，而 registry 长期保留它们 ⇒ 路径照样\n` +
      `    解析成功，但它证明的已**不是** Cargo.lock 里那份代码（判据 #491）。\n` +
      `  · 修法：把引用串里的版本号改成 lock 里的版本；**改前先证「换版本后结论不变」**\n` +
      `    （实测手法：逐字节比两版该文件，行数与关键行一致，才敢只改标签）。`,
  );
  process.exit(1);
}

if (newFailures.length > 0) {
  console.error(`\n✖ 新增客观错 ${newFailures.length} 处（修法唯一，**硬拦**）：`);
  for (const f of newFailures) console.error(`   ✖ ${f.what}`);
  console.error(
    `\n  · 行号越界 / 指到空行 / 指向裸控制流语句 ⇒ 去改**引用处的行号**（真源是代码，不是引用）。\n` +
      `  · 载体值不等 ⇒ 改前端字面量去对齐 ${CONST_NAME}（Rust 侧是真源）。`,
  );
  process.exit(1);
}

if (suspects.length > 0) {
  if (CI_MODE) {
    // 只打印不失败：GitHub Actions 只看退出码，stderr 不会让步骤转红。
    console.error("\n  （--ci：本项为软判据，按设计容忍；要人工复核请本地跑不带 --ci 的版本）");
    process.exit(0);
  }
  process.exit(3);
}
console.log(`\n✔ 无新增客观错；软判据 0 命中`);
