#!/usr/bin/env node
// scripts/check-doc-refs.mjs — 文档「文件:行号」引用的段对齐门禁（2026-11-04）
//
// 补的是什么盲区：`AGENTS.md`「文档中的源码引用写法（强制）」立了规则 —— 文档里的
//   `文件:行号` 必须**段对齐**（完整路径 / 唯一后缀 / 唯一 basename），禁止从文件名
//   **中间**截断或换同义词。但这条规则**此前没有任何自动检查**，只靠人撞：
//   `docs/plans/PLAN-shell-plus-plugins.md` §99.4 手工抓到过 `worker.rs:3798`
//   实际已差 492 行 —— 行号引用**会随源码编辑持续腐烂**，而编译 / clippy / 单测 / 现有
//   任何门禁都不会红（它们不读文档）。
//
// 为什么必须是独立门禁（而不是并进 `check-single-source-facts.mjs`）：
//   那个脚本自己声明「**不扫** Markdown 文档」—— 它的 `--dump-doc-refs` 是**取证模式**，
//   刻意不产出 failure / 不进退出码（`.md` 里存在大量「故意保留的历史行号」，见其头部）。
//   本门禁是**判据**：按桶分类后只硬拦客观错，历史存量走棘轮基线冻结。
//   引用抽取 / 定位原语与它**共用** `scripts/lib/doc-refs.mjs`（同一份实现，判据不分叉）。
//
// 扫描面：`docs/**/*.md`（**全部**，不限 `docs/INDEX.md` 登记过的）——
//   未登记的文档同样会被阅读，同样会腐烂；不扫它们等于「未检查」（判据 #7：
//   未检查与检查通过长得一样）。`--selftest` 走临时目录，不依赖仓内真实文档。
//
// 分桶（每条引用 token 一个判定，同一文档内重复引用去重）：
//   · OK        —— 唯一可定位（完整路径 / 唯一后缀 / 唯一 basename），且行号在文件行数内
//   · NONLOC    —— 同名多份、路径后缀也不能消歧（如裸 `mod.rs`）⇒ **未检查**，不是通过
//   · STALE     —— 文件存在但行号越界（含区间终点越界）⇒ **红**
//   · BROKEN    —— 仓内不存在该文件，且找不到被截断/换词的候选 ⇒ **红**
//   · ABBREV    —— 不存在，但**自证**到它缩写的真实文件（段内截断 / 同义词换词）⇒ **只报告**
//                  每条必须打印候选；找不到候选的**不许进本桶**（否则就成了猜）
//   · EXTERNAL  —— 跨仓引用（路径带 `name-x.y.z/` 版本目录，或本机绝对路径）⇒ 只报告
//   · EPHEMERAL —— 落在**不入版本控制 / 环境相关**的目录（`.workbuddy/`、`output/` …）⇒ 只报告
//                  理由：这些目录在 CI 检出里**不存在**，硬拦会让「本地绿、CI 红」，
//                  而那不是文档缺陷，是环境差异（同 `check-single-source-facts` 的 `.rs→.md` 豁免）
//   · MALFORMED —— 抽取出来的串根本不是路径（散文 / 表格残片）⇒ 丢弃，只计数
//                  实证：`bash.rs:266/:293` 这类写法旁边若还有裸 `:293`，归并时会拼出含
//                  反引号 / 中文的怪物串。判成 BROKEN 只会给基线灌垃圾（桶就指不回真实风险了）。
//   · UNVERIFIED —— （2026-11-06 追加，任务 #203）**读盘失败 = 未检查，不是通过**：
//                  本仓跑在 SMB / VMware 共享目录上，`fs.readFileSync` / `readdirSync` 会**偶发**
//                  抛 EBADF / EIO / EPERM / `UNKNOWN(errno -4094)`（libuv 对 SMB 专有 NTSTATUS
//                  码映射不出来，统一降级成 -4094 —— 实测形态，见下「I/O 兜底」注释块）。
//                  此前这两种错**直接把整道门打崩**：退 1、一行判定都没有 ⇒ 排查方向被引偏
//                  （本会话实证：把「工具读不到盘」当成了「文档引用坏了」去查）。
//                  ⚠ 反方向更危险：`catch { /* 跳过 */ }` 会让这些引用**静默进「健康」桶**
//                     —— 「门禁退 0」≠「检查跑了」（AGENTS.md「按目标口径验收」的假绿形态）。
//                  故：确定性「盘上确无此文件」仍走既有 BROKEN（**只有 ENOENT**，语义逐字不变）；
//                  瞬时错重试若干次仍失败 ⇒ 落本桶（引用级），文档 / 目录级读不到 ⇒ 同记入本桶的
//                  来源清单。本桶**默认只报告、不影响退出码**，加严要显式给 `--require-no-unverified`。
//   · SYMDRIFT —— （2026-11-05 追加，任务 #182 ②）**独立的第二层计数，不改动上面任何一个桶**：
//                  上面各桶只回答「**指不指得到**」（文件存在 + 行号在范围内），
//                  不回答「**那一行是不是所述符号**」。实测反例：
//                    `docs/plans/PLAN-shell-plus-plugins.md:1936` 引 `src/types/index.ts:2123`
//                    并称那是 `PluginPagesInfoDto` —— 该声明实际在 `:2135`，`:2123` 落在
//                    `PluginPageAssetLocationDto` 体内 ⇒ 行号仍在文件行数内 ⇒ 既有判据报
//                    `PASS`（长期假绿）。判据与保守口径见下方「行内符号一致性」注释块。
//
// 退出码：0 = 无新增 BROKEN / STALE；1 = 有新增（或 selftest 失败 / 第二层棘轮判红）；
//         2 = 脚本自身失效（扫描面 0 篇文档 / **一篇文档都没读成功** / 基线不可读 / 参数非法）。
//   ⚠ ABBREV 首轮**只报告**（存量未知且量级可能很大）；判据未放宽 —— 它只是还没被提升为红。
//   ⚠ SYMDRIFT（第二层）**靶面按「文档类别」收窄**（任务 #202 ①，判据⑥ + `SNAPSHOT_DOC_RULES`）：
//     只有**活文档**参与第二层收口；**时点快照**（`docs/audits/AUDIT-*-<YYYY-MM-DD>.md`）
//     **一律不订正、不进棘轮、永不判红** —— 快照里的旧读数是「当时的世界」，腐烂是正常且应保留的。
//     快照面仍**计数并打印**（判据 #7：免检与检查通过长得一样）。
//     ⚠ 第一层各桶**不随本条收窄**：对快照与活文档按原口径同跑（2026-11-04 建门时的既有裁定，
//       本批一字未动 —— 桶判定、键、`hardViolations` 都没改）。
//   ⚠ SYMDRIFT 的**退码**（任务 #202 ②）分两档，默认都**只报告**：
//     · 基线里没有 `symEntries`（= 第二层从未冻结）⇒ 恒不判红，但**自陈「棘轮未生效」**，
//       并给出冻结命令 —— 「没武装」必须是看得见的状态，不能伪装成「查过了、没问题」。
//     · 已冻结 ⇒ 活文档面中**基线外新增的键**判红（这才叫收口）；基线内条目只报告。
//     · 全量升硬开关仍是 `--require-symbol-match`（**不看基线**，整个活文档面判红），
//       CI 与 `ci-check.mjs` 的命令行**都不带**该开关。
//   ⚠ 自动回写（任务 #202 ③）**默认不存在**：`--suggest-fix` 只出「建议清单」（dry-run，零写盘）；
//     真写盘要 `--fix` **且** `--i-attest-quiet-window` **且** `writeModeGate` 六道闸全过（见下方「自动回写」：
//     写盘自检当场跑且非零断言、标注语料精确率 100%、棘轮已武装、无未检查面、逐条写前验证、
//     改后行四条不变式）。任一道不过 ⇒ 退 2 自陈并一行盘都不写。
//   ⚠ UNVERIFIED（读盘失败）**默认也只报告**，理由与本仓既有两道巡检门同型：
//     把「盘的抖动」变成红灯会把排查方向引向不存在的文档缺陷（本会话已实证一次），
//     而巡检门的价值在于**读数可信 + 覆盖面自陈**。但两件事必须硬：
//       ① 读不到的引用**不许**静默算检查过 ⇒ UNVERIFIED 的数量 + 样例一律打印（不进 OK 桶）；
//       ② `docs.length > 0` 却**一篇文档都没读成功** ⇒ 本门一行没跑 ⇒ 退 2（与「扫描面为 0」
//          同档：那不是盘的抖动，是门本身失效）。要连「部分读不到」也判红，显式给
//          `--require-no-unverified`（CI 与 `ci-check.mjs` 的命令行**都不带**，加严由对账批裁定）。
//
// ── 行内符号一致性（SYMDRIFT）的判据 ────────────────────────────────
// 六条，**逐条都是往「宁可漏报」的方向收敛**（散文里绝大多数反引号不是符号，
// 硬要比对就是结构性假阳性）：
//   ① 只检既有桶为 **OK** 的引用（STALE / BROKEN / NONLOC / … 已有自己的语义，不叠第二层红）；
//   ② 符号必须**可机械比对**：与引用**同一行**、列距离 ≤ `MAX_SYM_GAP` 的反引号簇里，
//      剥掉前导属性 `#[...]`、把 `::` 归一为 `.` 后**恰好只剩一个**标识符链
//      （`foo` / `Foo::bar` / `pub fn foo` / `list_x()`）；
//      含数字 / 含 `/` 或 `\` / 以文件扩展名结尾 / 剩下 ≥2 个标识符 / 含非 ASCII
//      ⇒ 判「句中无可比对符号」⇒ **不检也不判红**（最重要的一条护栏）；
//      ⚠ **点号链 `notify.channel` 一类默认也排除**（判据修正，2026-11-05 实测）：本仓的
//      能力登记表把这类**接缝名**（= `scripts/capability-register.json` 的键）放在表格首列、
//      `文件:行` 放在次列，它不是「该行有此符号」的断言；当成符号比对的实测后果是把
//      `docs/plans/PLAN-shell-plus-plugins.md` 等整表拖进红（占该档读数的主要部分）。
//      故默认走「接缝名 ⇒ 未检」，只计数自陈（`--sym-include-dotted` 可放回，供对账）；
//      `::` 路径形态（`Foo::bar`）**不受此影响**，仍按符号比对。
//   ③ 该符号必须**在目标文件里存在**（零存在 ⇒ 跳过：那是重命名或口语，属另一道题，
//      不是「行号漂了但没出界」）；
//   ④ 命中窗口 = `[start, end + WINDOW_BELOW]`，默认 **2 行**，理由：rustfmt / prettier 的
//      折行只会把名字推到紧邻的下一行（`impl Trait for Type` 断行、长泛型 `where`），
//      再往下进函数体了；**上方不留 0 行** —— `start-1` 通常是属性行或 doc 注释，
//      算它命中会让「符号只写在注释里」的引用白赚。窗口可用 `--sym-window=N` 现场放大复核；
//   ⑤ 多个候选符号里**任一命中即视为一致**（保守），全不命中才成一条 SYMDRIFT。
//   ⑥ **靶面只收活文档**（任务 #202 ①，2026-11-06）：`docLifecycle(docRel)` 判成 `snapshot` 的
//      文档**不参与第二层**（不判漂移、不进棘轮、不进建议、永不判红）。
//      规范依据：`AGENTS.md`「文档放置」表把 `docs/audits/` 定为「审计 / 体检 / 对比 /
//      根因定位报告（**时点快照**）」，而 `docs/plans/` 等是「计划 / 提案 / 修复清单」（活文档）。
//      快照记录的是**当时的世界**，其行号腐烂是正常且应保留的证据 —— 拿第二层去「订正」快照
//      等于销毁取证现场（第一层不在此列：它查的是「这串还能不能指到文件」，与订正无关）。
//      ⚠ 代价（必须写明）：快照面从此**没有**第二层保护，其符号漂移只以计数形式可见。
//      ⚠ 判据集中在 `SNAPSHOT_DOC_RULES` 一处，**默认判活文档**（fail-toward-checking）：
//        未来新增快照目录 ⇒ 在该表**追加一条** `{dirPrefix, baseRe, basis}`，`basis` 必须写
//        规范出处（`AGENTS.md` 的哪一句）；未登记的目录一律按活文档收口，不许靠命名猜。
//
// 棘轮基线 `scripts/doc-refs-baseline.json`（**只减不增**）：
//   第一层键 = `entries`：`<文档相对路径>::<引用串>:<起始行>[-<结束行>]`（**不含文档行号** ——
//   文档增删会让文档行号整体漂移，那正是本门禁要抓的腐烂形态，拿它当键会自己先烂）。
//   第二层键 = `symEntries`（任务 #202 ② 新增）：`<文档相对路径>::<被引文件> → <被引符号>`，
//   **刻意不含行号** —— 第二层的病灶就是行号，把它钉进键会让「源码里符号挪了一行」直接变成
//   「基线外新增」，红得与文档无关（同族先例：`component-neutrality-baseline.json` 的键设计
//   与 `AGENTS.md`「白名单不含行号」的理由）。符号取 `symDrift.symbols[0]` = 报告里点名
//   「实际在第几行」的那个符号 ⇒ 每条键都能指回一行真实读数。
//   · 常规运行：基线内的存量不拦，基线外新增 → 红；基线内已消失 → 打印「可收紧」，不红；
//   · `--init-baseline`：**仅当基线文件不存在**时建立（首轮冻结存量；存在则拒绝，不许洗白）。
//     第二层同理：`symEntries` 缺失时由本模式**首冻**（此时 `entries` 逐字保留，不重建）；
//     两层都已存在 ⇒ 拒绝。
//   · `--update-baseline`：收紧（先跑常规运行确认**两层都**无新增，否则拒绝执行）。
//   · ⚠ 两个写基线的开关都**在未检查面非空时无条件拒绝**（退 2，见 `refuseBaselineWrite`）：
//     读不到的引用会从本轮读数里消失，而「消失」正是基线认定「已修复」的口径 —— 不放这道闸，
//     一次 SMB 抖动就能静默删掉一批仍然有效的存量条目（那是盘的抖动代劳的洗白）。
//   · ⚠ 第二层**首冻必须等安静窗口**：并行期跑 `--init-baseline` 会把别的 lane 未提交的
//     中间态钉进棘轮（本仓踩过）。自检用合成靶面证明三个分支，不碰真实基线。
//
// 自动回写（任务 #202 ③，**默认零写盘**）：
//   · `--suggest-fix`：只打印「建议清单」（dry-run）。判据 `suggestSymbolFix` 刻意保守 ——
//     只有「单行引用 + 句中恰一个可比对符号 + 该符号在目标文件里**恰一处声明形态命中**」才给建议；
//     区间引用 / 多符号 / 声明命中 0 处或 ≥2 处 / 符号只出现在注释 ⇒ **判歧义 ⇒ 不出建议**，
//     并计数自陈（「歧义 M 条」就是精确率的分母，不许把它算成已解决）。
//   · `--fix`：真写盘。**必须**同时给 `--i-attest-quiet-window`，且 `writeModeGate` 六道闸全过：
//     ① 写盘路径的自检（`runSymFixSelftest`）**当场跑、执行断言数 > 0、全绿** —— 无自检覆盖即拒绝；
//     ② 精确率闸（`fixPrecisionGate`）在自检标注语料上实测 = 要求值（当前 **100%**），
//        且歧义负控必须**一条建议都不出**（错了要能被发现：`verifyProposedFix` 对故意写错的
//        行号必须返回 false，这条本身也在自检里）；
//     ③ 第二层棘轮**已武装**（基线里有 `symEntries`）—— 没有 before/after 对照面时，
//        写错一条永远不会被发现 ⇒ 未首冻就写盘等于盲改；
//     ④ 本轮无未检查面（与写基线同型的闸）；
//     ⑤ 每条建议写前逐条验证「新行号上确实有该符号」（`verifyProposedFix`）；
//     ⑥ 改后的行必须过 `verifyPatchedLine` 四条不变式（只动那一段数字 + 抽取器复读得到新行号）。
//     任一道不过 ⇒ 退 2 自陈，**一行盘都不写**（且拒绝时**不产生部分写**：任一建议被拒 ⇒ 整批放弃）。
//     写盘只碰活文档（判据⑥ 排除快照面）。
//
// 用法：
//   node scripts/check-doc-refs.mjs
//   node scripts/check-doc-refs.mjs --selftest
//   node scripts/check-doc-refs.mjs --dump-doc-refs        # 逐条打印 + 判定 + 命中的真实文件
//   node scripts/check-doc-refs.mjs --init-baseline        # 首轮冻结存量（仅当基线不存在）
//   node scripts/check-doc-refs.mjs --update-baseline      # 存量修复后收紧基线
//   node scripts/check-doc-refs.mjs --require-symbol-match # 把 SYMDRIFT 提升为红（**默认不开**）
//   node scripts/check-doc-refs.mjs --sym-include-dotted   # 把点号接缝名也算符号（读数虚高，仅对账）
//   node scripts/check-doc-refs.mjs --sym-window=5         # 放大折行窗口复核（只影响 SYMDRIFT 读数）
//   node scripts/check-doc-refs.mjs --require-no-unverified # 把 UNVERIFIED（读盘失败）提升为红（**默认不开**）
//   node scripts/check-doc-refs.mjs --suggest-fix          # 只出建议清单（dry-run，零写盘）
//   node scripts/check-doc-refs.mjs --fix --i-attest-quiet-window  # 写盘（六道闸，见上方「自动回写」）

import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import {
  abbrevCandidates,
  buildBasenameIndex,
  extractRefs,
  indexFromFileList,
  locateRef,
  synonymCandidates,
  walkRepoFiles,
} from "./lib/doc-refs.mjs";
import { discoverWorkspaceRoots, repoRel as repoRelOf } from "./lib/workspace-roots.mjs";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.resolve(HERE, "..");
const BASELINE_PATH = path.join(HERE, "doc-refs-baseline.json");
const DOCS_DIR = path.join(ROOT, "docs");

const args = process.argv.slice(2);
const SELFTEST = args.includes("--selftest");
const DUMP = args.includes("--dump-doc-refs");
const INIT_BASELINE = args.includes("--init-baseline");
const UPDATE_BASELINE = args.includes("--update-baseline");
// SYMDRIFT 的**唯一**升硬开关。默认关 ⇒ 既有退出码与桶计数逐字不变（CI 与 ci-check.mjs
// 的命令行都不带它，所以本地绿 / CI 绿口径不受本批影响）。
const REQUIRE_SYMBOL_MATCH = args.includes("--require-symbol-match");
// UNVERIFIED（读盘失败）的**唯一**升硬开关。默认关 ⇒ 既有退出码与桶计数逐字不变
// （CI 与 `ci-check.mjs` 的命令行都不带它）。理由见文件头「退出码」段第 ⚠ UNVERIFIED 条：
// 本门跑在 SMB / VMware 共享目录上，盘的瞬时抖动不该变成红灯；但**读不到必须被看见**，
// 所以「打印」一侧是硬的（数量 + 样例），只有「退码」一侧留给开关。
const REQUIRE_NO_UNVERIFIED = args.includes("--require-no-unverified");
// 把 `notify.channel` 这类**点号接缝名**也当符号比对（默认 **否**，理由见 `comparableSymbolSpan`）。
// 只是读数口径开关：开着它 SYMDRIFT 会虚高（本仓实测主要是能力登记表那一族），不进 CI 命令行。
const INCLUDE_DOTTED = args.includes("--sym-include-dotted");
// ── 自动回写（任务 #202 ③）：dry-run 与写盘**必须是两个开关**，且写盘要额外的人为确认 ──
// `--suggest-fix` 只打印建议清单（零写盘，任何窗口都可跑）；`--fix` 才写盘，且必须**同时**给
// `--i-attest-quiet-window`（「我知道现在没有别的 lane 在改 docs/」）。六道闸见 `writeModeGate`。
const SUGGEST_FIX = args.includes("--suggest-fix");
const FIX = args.includes("--fix");
const QUIET_WINDOW_ATTESTED = args.includes("--i-attest-quiet-window");
// 折行窗口（下方行数）。`--sym-window=N` 只放大 SYMDRIFT 的**读数**，不改其它任何判据；
// 非法值（格式不对 / >50）⇒ 退 2 自陈，不静默按默认跑（判据 #7：未检查与通过长得一样）。
// 解析放在下方常量块之后（`WINDOW_BELOW_DEFAULT` 的 TDZ），真正取值在 main 里。
const WINDOW_FLAG = args.find((a) => a.startsWith("--sym-window"));

// ── 文档面扩展名白名单 ───────────────────────────────────────────────
// 比源码面（`DEFAULT_REF_EXTS`）宽：文档引用的目标常是 `.json` / `.css` / `.html` /
// `.lock` / `.proto`。抽取器本体仍只有一份（`lib/doc-refs.mjs`），这里只换白名单。
const DOC_REF_EXTS = [
  "rs", "ts", "tsx", "js", "jsx", "mjs", "cjs", "sh", "ps1", "bat", "cmd",
  "json", "toml", "lock", "md", "rhai", "py", "sql", "yml", "yaml",
  "css", "html", "proto", "txt", "csv",
].join("|");

// ── 目标文件索引的排除口径（覆盖面判据，改动前必须量）──────────────
// 检出副本（`.worktrees` / `.qoder/worktrees`）与产物副本（`output/**`、`.tmp`）里装着
// 整棵树的副本 ⇒ 同一 basename 撑成「同名 N 份」⇒ 本该可定位的引用退化成 NONLOC（= 未检查）。
// `docs` **不排除**：文档之间的互引（`AUDIT-x.md:12`）同样要被检查。
const SKIP_DIRS = new Set([
  "target", "node_modules", ".git", "dist", "coverage", "knowledge-sources",
  ".output", ".vite", ".monaco", ".worktrees", ".qoder", "output", ".tmp",
]);

// ── 环境相关目录（CI 检出里不存在 ⇒ 不判红）────────────────────────
// 判据是**路径首段**（或路径中某段）命中；这些目录都不在版本控制里
// （`.workbuddy/memory` 是本地记忆机制，`output/` 是产物/备份，`.tmp/` 是运行日志）。
// ⚠ 这只是「不判红」，仍然**计数并打印** —— 别把「免检」读成「检查通过」。
const EPHEMERAL_PREFIXES = [
  ".workbuddy/", "workbuddy/", ".qoder/", "qoder/", ".tmp/", "tmp/", "output/", "target/",
  "node_modules/", "dist/", "coverage/", ".vite/", ".monaco/",
  "knowledge-sources/", ".worktrees/",
];

/** 第三方 crate / 跨仓源码的版本目录形态：`sea-orm-2.0.2/src/…`。 */
const EXTERNAL_PATH_RE = /(?:^|\/)[A-Za-z0-9_.-]+-\d+\.\d+\.\d+\//;
/** 本机绝对路径（`D:\…`、`\\host\…`、`/home/…`）—— 换机器即失效，属环境相关。 */
const ABSOLUTE_PATH_RE = /^(?:[A-Za-z]:[\\/]|\\\\|\/)/;

// ── I/O 兜底（任务 #203）：SMB / VMware 共享目录上的瞬时读错 ──────────────────
// 缺陷形态（改动前实测，两条都是**整道门崩溃**：退 1、一行判定都没有）：
//   · `auditDocs` 读文档正文（原第 625 行）**完全没有 try/catch** —— 注入 EPERM 即崩栈；
//   · `readLines` 只兜 `ENOENT`，其它 code 一律 `throw` —— 注入 `errno -4094` 即崩栈。
// 分类口径按 **Node 实际抛出的 `e.code`** 写（本机 node v24.19.0 逐条对过
//   `util.getSystemErrorMap()` 与 `os.constants.errno`）：
//   · **瞬时（重试有意义）**：`EBADF`(9，句柄被回收) · `EIO`(5) · `EPERM`(1，杀软/索引器/hgfs 抢锁)
//     · `EACCES`(13，Windows 上与 EPERM 同样常由文件被占用引起) · `EAGAIN`(11) · `EBUSY`(16)
//     · `EMFILE`(24) / `ENFILE`(23)（fd 耗尽，退避后通常即可）· `ENOTCONN` · `ESHUTDOWN`
//     · `UNKNOWN` = **errno -4094**，libuv 对 SMB 专有 NTSTATUS 码**映射不出来**时统一降级成它
//       （`getSystemErrorMap().get(-4094)` = `["UNKNOWN","unknown error"]`）。
//       ⚠ 这一条是 SMB 上**最常见**的形态，按 code 白名单分类时漏了它等于没兜（leader 报的
//       「errno -4094」在 `e.code` 一侧就是字符串 `"UNKNOWN"`，两者都要认）。
//   · **确定性（重试无意义）**：`ENOENT`(2) · `ENOTDIR`(20) · `EISDIR`(21) · `EINVAL`(22)
//     · `ELOOP`(40) · `ENAMETOOLONG`(38)。
//     ⚠ 其中**只有 `ENOENT`** 保留既有语义（索引内有、盘上已删 ⇒ 与「仓内无此文件」同判 BROKEN，
//     `readLines` 注释与既有 selftest 钉死）。其余确定性错误说明的是「**我们的读取方式**出了问题」
//     而不是「引用坏了」⇒ 判 BROKEN 会把工具故障记成文档缺陷（假红），判 OK 是假绿，
//     故与瞬时错一样落 UNVERIFIED。
//   · 判据**两头都收**：认不准的错误一律 UNVERIFIED（既不算通过也不算红），
//     因为 UNVERIFIED 的代价只是「这一条没查」，而误判 BROKEN/OK 的代价是「读数不可信」。

/** 瞬时 I/O 错误码（`e.code`）——重试可能成功。 */
const TRANSIENT_IO_CODES = new Set([
  "EBADF", "EIO", "EPERM", "EACCES", "EAGAIN", "EWOULDBLOCK", "EBUSY",
  "EMFILE", "ENFILE", "ENOTCONN", "ESHUTDOWN", "UNKNOWN",
]);
/** 确定性错误码 —— 重试恒失败，不付费。 */
const STATIC_IO_CODES = new Set(["ENOENT", "ENOTDIR", "EISDIR", "EINVAL", "ELOOP", "ENAMETOOLONG"]);
/** errno 为 -4094（libuv UNKNOWN）时的显式识别：某些封装只填 `errno` 不填 `code`。 */
const UV_UNKNOWN_ERRNO = -4094;

/**
 * 重试次数与退避（**理由写在这里，改动必须连同理由一起改**）：
 *   · `IO_RETRY_ATTEMPTS = 3`（1 次正常 + 2 次重试）：SMB/hgfs 的瞬时错来自
 *     「readdir 快照与 open 之间的竞态」与「客户端句柄缓存失效」，第二次 open 基本必然成功
 *     （本仓 `readLines` 的 ENOENT 兜底注释记录过同型现象：文件确实在，只是那一瞬读不到）；
 *     再多只是把「挂载真死了」这件事拖成分钟级卡死 —— **卡死与崩溃同样是「没有判定输出」**。
 *   · `30ms → 90ms`（×3 递增，最坏单次 120ms）：30ms 远大于 hgfs 的往返抖动（个位数 ms），
 *     又远小于人可感知的卡顿；递增而非固定，是给「服务端忙」留退避空间。
 *   · `IO_RETRY_BUDGET_MS = 5000` 全局预算：扫描面实测 286 篇文档 + 约 1900 个目标文件，
 *     挂载整体失联时「每条都重试满」= 上百秒，比崩溃更糟（CI 会被判超时）。预算耗尽后
 *     **立即停止重试**并置 `exhausted` ⇒ 后续读错直接落 UNVERIFIED，并在打印里自陈预算耗尽。
 */
const IO_RETRY_ATTEMPTS = 3;
const IO_RETRY_BASE_MS = 30;
const IO_RETRY_FACTOR = 3;
const IO_RETRY_BUDGET_MS = 5000;

/** 读盘失败的哨兵值 —— 与 `null`（既有「盘上确无此文件」语义）**必须**区分开。 */
export const UNREADABLE = Symbol("unreadable");

/**
 * 同步睡眠。本脚本全程同步（`readFileSync` 系），不能用 `setTimeout`；
 * `Atomics.wait` 在 Node **主线程可用**（只有浏览器 JS 禁），故不引入 busy-wait。
 * SharedArrayBuffer 极端不可用时退回忙等 —— 宁可烧 CPU 也不许把退避变成 0（那等于没退避）。
 */
export function syncSleepMs(ms) {
  if (!(ms > 0)) return;
  try {
    Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, Math.ceil(ms));
  } catch {
    const end = Date.now() + ms;
    while (Date.now() < end) {
      /* 忙等兜底 */
    }
  }
}

/**
 * 纯函数：该 fs 错误属瞬时（可重试）/ 确定性（不重试）/ 「文件确实没有」（既有 BROKEN 语义）。
 * ⚠ 兜底方向刻意是 **transient**：认不准的错误（含 `errno === -4094` 但没带 `code` 的封装、
 *   含被 `throw "字符串"` 的极端形态）代价只是**多试两次然后落 UNVERIFIED**；
 *   若兜底成 static/missing，一个我们没认出来的错就会被判成「仓内查无此文件」（假红）
 *   或「盘上确无此文件」，两者都比多花 120ms 严重。
 *   `UV_UNKNOWN_ERRNO` 常量保留是为了让「-4094 归哪档」在源码里可查（见上方实测注）。
 */
export function classifyIoError(e) {
  const code = e && typeof e.code === "string" ? e.code : "";
  if (code === "ENOENT") return "missing";
  if (STATIC_IO_CODES.has(code)) return "static";
  if (TRANSIENT_IO_CODES.has(code)) return "transient";
  return "transient"; // 认不准 ⇒ 按瞬时处理（理由见上）
}

/** 错误自陈串（进 UNVERIFIED 桶的 `why`，逐条要能指回是哪一次读、报什么错）。 */
function ioErrDesc(e) {
  const code = e && typeof e.code === "string" ? e.code : "?";
  const errno = e && typeof e.errno === "number" ? e.errno : "?";
  return `${code}(errno ${errno})`;
}

/**
 * 读盘失败的登记表（**覆盖面自陈的数据源**）。
 * @param {number} budgetMs 全局退避预算（selftest 里取 0 以验证「耗尽后不再重试」）
 */
export function makeIoNotes(budgetMs = IO_RETRY_BUDGET_MS) {
  const notes = {
    retries: 0, // 实际执行的重试「次数」（不含首次尝试）——正控用它证明重试真的在跑
    retryMs: 0, // 累计退避睡眠毫秒
    budgetMs,
    exhausted: false, // 预算是否已耗尽（耗尽 ⇒ 只试一次就落未检查）
    failures: [], // 每条 = { kind, at, code, errno, err, attempts, sleptMs }
    byTarget: new Map(), // kind+at ⇒ 条目（同一路径被上千条引用命中只记一次）
    record(kind, at, e, attempts, sleptMs) {
      const key = `${kind}\u0000${at}`;
      if (this.byTarget.has(key)) return this.byTarget.get(key);
      const item = { kind, at, code: e?.code ?? "?", errno: e?.errno ?? "?", err: ioErrDesc(e), attempts, sleptMs };
      this.byTarget.set(key, item);
      this.failures.push(item);
      return item;
    },
    find(kind, at) {
      return this.byTarget.get(`${kind}\u0000${at}`);
    },
    count(kind) {
      return this.failures.filter((f) => f.kind === kind).length;
    },
  };
  return notes;
}

/**
 * 带重试的读取包装。**不抛**（这正是本任务要修的缺陷）。
 * @param {() => T} read 真正执行读取的闭包
 * @param {object} notes makeIoNotes() 的产物
 * @param {{kind?:string, at?:string, sleep?:(ms:number)=>void, label?:string}} opts
 *   `kind`：`target`（引用的目标文件）｜`doc`（文档正文）｜`dir`（文档目录遍历）
 * @returns {{ok:boolean, value?:T, missing?:boolean, error?:Error, attempts:number, waitedMs:number}}
 *   `missing=true` **只**在 ENOENT 时出现 ⇒ 调用方按既有「盘上确无此文件」判据处理（BROKEN）；
 *   其余 `ok:false` 一律是「未检查」，不许复用既有桶。
 */
export function withIoRetry(read, notes, opts = {}) {
  const kind = opts.kind ?? "target";
  const at = opts.at ?? (opts.label ?? "");
  const sleep = opts.sleep ?? syncSleepMs;
  let attempt = 0;
  let waited = 0;
  for (;;) {
    attempt++;
    try {
      return { ok: true, value: read(), attempts: attempt, waitedMs: waited };
    } catch (e) {
      const cls = classifyIoError(e);
      // 确定性错误（含 ENOENT）：**不重试**。这条同时是「重试不许把『文件不存在』判据拖慢」
      // 的实现保证 —— 仓内 BROKEN 存量实测 271 条，逐条重试会白付 ~33 秒。
      if (cls !== "transient") {
        // ENOENT 既有语义（BROKEN / 目录 no-op）不进未检查面；其余确定性错误**要记**，
        // 否则调用方落 UNVERIFIED 时 `why` 指不回「哪一次读、报什么错」。
        if (cls === "static") notes.record(kind, at, e, attempt, waited);
        return { ok: false, missing: cls === "missing", error: e, attempts: attempt, waitedMs: waited };
      }
      const canRetryAgain = attempt < IO_RETRY_ATTEMPTS && notes.retryMs < notes.budgetMs;
      if (!canRetryAgain) {
        if (notes.retryMs >= notes.budgetMs) notes.exhausted = true;
        notes.record(kind, at, e, attempt, waited);
        return { ok: false, missing: false, error: e, attempts: attempt, waitedMs: waited };
      }
      const wait = IO_RETRY_BASE_MS * IO_RETRY_FACTOR ** (attempt - 1);
      notes.retries++;
      notes.retryMs += wait;
      waited += wait;
      sleep(wait);
    }
  }
}

/**
 * `readdirSync` 的兜底封装。
 *   · 成功 ⇒ 返回条目；
 *   · **目录确实不存在（ENOENT）** ⇒ 抛出，让调用方走既有「安全 no-op 返回」⇒ 上层再用
 *     「扫描面 0 篇 ⇒ 退 2」判据自陈（既有 selftest 负控：`listDocFiles(tmp/nope)` 必须仍是 0，
 *     且**不进**未检查面 —— 那里没有任何东西被跳过，指不回真实风险）；
 *   · 其它读不到 ⇒ 已由 `withIoRetry` 记入未检查面，这里**返回空数组**（不抛：抛会被上层
 *     的 catch 吞成「目录不存在」，正是本任务要消灭的静默形态）。
 */
function readDirSyncRetry(dir, notes) {
  const r = withIoRetry(() => fs.readdirSync(dir, { withFileTypes: true }), notes, { kind: "dir", at: dir });
  if (r.ok) return r.value;
  if (r.missing) {
    const e = new Error(`ENOENT: no such directory ${dir}`);
    e.code = "ENOENT";
    throw e;
  }
  return [];
}

/**
 * 读一个「引用的目标文件」正文。ENOENT ⇒ `null`（既有 BROKEN 语义）；
 * 其它读不到 ⇒ `UNREADABLE`（未检查）。**永不抛**。
 */
export function readFileSafe(abs, notes, kind = "target") {
  const r = withIoRetry(() => fs.readFileSync(abs, "utf8"), notes, { kind, at: abs });
  if (r.ok) return r.value;
  return r.missing ? null : UNREADABLE;
}

/**
 * 读**一篇文档**的正文（`kind:"doc"`）。与目标文件侧的判据**刻意不同**：
 *   · 目标文件 ENOENT ⇒ `null` ⇒ 既有 BROKEN（「仓内查无此文件」）—— 语义逐字不变；
 *   · 文档 ENOENT ⇒ **仍记入未检查面**。理由：文档清单是几秒前 `readdirSync` 出来的，
 *     此刻读不到只可能是竞态 / 挂载抖动，而「这篇文档里的引用」一条都没被检查过 ——
 *     把它静默跳过 = 扫描面凭空变小而读数看上去完全正常，正是本任务点名的假绿形态。
 * @returns {string|symbol} 正文，或 `UNREADABLE`
 */
export function readDocText(abs, notes) {
  const r = withIoRetry(() => fs.readFileSync(abs, "utf8"), notes, { kind: "doc", at: abs });
  if (r.ok) return r.value;
  if (!notes.find("doc", abs)) notes.record("doc", abs, r.error, r.attempts, r.waitedMs);
  return UNREADABLE;
}

/**
 * 纯函数：读盘失败的**覆盖面**摘要（三个来源分档，互不混同）。
 * @param {object} notes
 * @param {number} unverifiedRefs 落入 UNVERIFIED 桶的引用条数（由 counts 提供）
 */
export function ioCoverageSummary(notes, unverifiedRefs) {
  return {
    refs: unverifiedRefs,
    targets: notes.count("target"),
    docs: notes.count("doc"),
    dirs: notes.count("dir"),
    retries: notes.retries,
    retryMs: notes.retryMs,
    exhausted: notes.exhausted,
    any: unverifiedRefs + notes.failures.length > 0,
  };
}

/** 纯函数：UNVERIFIED 是否进退出码。**默认关** ⇒ 恒 false ⇒ 退码与改动前逐字相同。 */
export function unverifiedHardFail(cov, requireNoUnverified) {
  return requireNoUnverified && cov.any;
}

/**
 * 纯函数：本轮存在未检查面时是否**必须拒绝**写棘轮基线（`--init-baseline` / `--update-baseline`）。
 * 判据：基线的键面是 BROKEN/STALE 引用。文档读不到 ⇒ 它里面的引用**从本轮读数里消失** ⇒
 *   `diffAgainstBaseline` 会把它们算成「本轮已消失（可收紧）」，`writeBaseline` 于是**静默删掉
 *   一批本来仍然有效的存量条目** —— 那是比假绿更糟的假收紧（违反「不许用基线洗白」同条禁令，
 *   只是这次的「洗白」是盘的抖动代劳的）。故这一条**不是开关，是无条件硬拦**。
 */
export function refuseBaselineWrite(cov) {
  return cov.any;
}

/**
 * 纯函数：本门是否**一行都没跑**（与「扫描面为 0」同档的自身失效 ⇒ 退 2）。
 * ⚠ 这一条与「部分读不到 ⇒ 默认只报告」不矛盾：部分读不到是盘的抖动（不该红），
 *   而「收到 N 篇文档、0 篇读成功」= 门本身没有产出任何判定 ⇒ 报「通过」就是假绿。
 * @param {number} docsSeen listDocFiles 收到的文档数
 * @param {number} docsRead 正文读取成功的文档数
 */
export function scanProducedNothing(docsSeen, docsRead) {
  return docsSeen === 0 || (docsSeen > 0 && docsRead === 0);
}

/**
 * 遍历 `docs/` 收全部 `.md`（绝对路径，排序保证输出稳定）。
 * @param {string} root
 * @param {object} [notes] 读盘失败登记表；**不传**时自产一份（selftest 单参调用保持既有行为）
 *   ⚠ 目录级 `readdirSync` 失败此前是 `catch { return; }` —— 静默丢文档 = **静默缩小扫描面**，
 *   正是「门禁退 0 ≠ 检查跑了」的形态。现在：ENOENT（目录本就不存在）仍静默（既有负控），
 *   瞬时错重试后仍失败 ⇒ 记 `kind:"dir"` 进未检查面，由 summary 打印。
 */
export function listDocFiles(root, notes) {
  const ioNotes = notes ?? makeIoNotes();
  const out = [];
  const walk = (dir) => {
    let entries;
    try {
      entries = readDirSyncRetry(dir, ioNotes);
    } catch {
      return; // 目录不存在 = 安全 no-op（调用方另有「扫描面为 0 即红」的判据）
    }
    for (const e of entries) {
      if (e.name === "node_modules" || e.name === ".git") continue;
      const p = path.join(dir, e.name);
      if (e.isDirectory()) walk(p);
      else if (e.isFile() && e.name.endsWith(".md")) out.push(p);
    }
  };
  walk(path.join(root, "docs"));
  return out.sort();
}

/**
 * 仓库文件清单（绝对路径）。**优先 `git ls-files`**：
 *   · 本机是 VMware 共享目录，递归 `readdir` 整个仓要数分钟（实测首版卡死）；
 *     git 索引读取是秒级，且 CI（Linux 本地盘）同样可用；
 *   · 口径恰好是「已入版本控制 + 未忽略的新文件」⇒ 本地与 CI 检出**同一集合**，
 *     不会因为某台机器上多了一份检出副本 / 产物目录而改变判定（那是环境差异，不是缺陷）；
 *   · 未跟踪但**未被忽略**的新文件（正在写的文档 / 源码）也在内（`--others --exclude-standard`）。
 * 拿不到 git（无 `.git` / 未安装）时退回文件系统遍历，并在输出里自陈口径。
 * @returns {{files:string[], source:"git"|"walk"}}
 */
export function listRepoFiles(root, skipDirs) {
  const r = spawnSync("git", ["-C", root, "ls-files", "-z", "--cached", "--others", "--exclude-standard"], {
    maxBuffer: 256 * 1024 * 1024,
    encoding: "buffer",
  });
  if (!r.error && r.status === 0) {
    const files = r.stdout
      .toString("utf8")
      .split("\0")
      .filter(Boolean)
      .map((rel) => path.join(root, rel.split("/").join(path.sep)));
    if (files.length > 0) return { files, source: "git" };
  }
  return { files: walkRepoFiles(root, skipDirs), source: "walk" };
}

/**
 * 基于**文件清单集合**的路径解析器（与 `resolveRepoFile` 同序候选，但零 I/O）。
 *
 * 为什么不用 `resolveRepoFile`：它逐条 `stat` 候选路径，SMB 上实测 ~190ms/条
 * （1.2 万条引用 ⇒ 38 分钟，首版就是这么卡住的）。这里把候选路径前缀预先展开，
 * 只在 `Set` 里查存在性 —— 微秒级。候选顺序与 lib 逐字对齐：
 *   ① 仓库根 + rel ｜ ② 各 workspace 根 + rel ｜ ③ 各 workspace 根 + `crates/` + rel
 */
export function makeFileSetResolver(root, files) {
  const relSet = new Set(files.map((abs) => repoRelOf(root, abs)));
  const roots = discoverWorkspaceRoots(root).map((r) => repoRelOf(root, r));
  const prefixes = ["", ...roots.map((r) => `${r}/`), ...roots.map((r) => `${r}/crates/`)];
  return (refFile) => {
    const rel = refFile.split("\\").join("/").replace(/^\.\//, "");
    for (const pre of prefixes) {
      const cand = `${pre}${rel}`;
      if (relSet.has(cand)) return path.join(root, cand.split("/").join(path.sep));
    }
    return null;
  };
}

/** 纯函数：该引用串是否落在环境相关目录（见 `EPHEMERAL_PREFIXES`）。 */
export function isEphemeralRef(refFile) {
  const p = refFile.replace(/^\.\//, "");
  return EPHEMERAL_PREFIXES.some((pre) => p.startsWith(pre) || p.includes("/" + pre));
}

/** 纯函数：该引用串是否跨仓 / 本机绝对路径（见上方两条正则）。 */
export function isExternalRef(refFile) {
  return EXTERNAL_PATH_RE.test(refFile) || ABSOLUTE_PATH_RE.test(refFile);
}

/**
 * 纯函数：该「引用串」是否根本不是路径（散文 / 表格残片）。
 *
 * 实证（首版）：`bash.rs:266/:293` 这类**多行清单**写法配上一行里后续的裸 `:293`，
 * 归并后曾拼出 `266/:293`` 用` 这种既含反引号又含空白/中文的字符串。它不是引用，
 * 是抽取器的边界噪声 —— 判成 BROKEN 只会给基线灌垃圾（`BROKEN` 桶就指不回真实风险了）。
 * 判据是**形态**、不是白名单：出现路径字符集之外的字符 / 以 `/` 收尾 / 空段 ⇒ 不是路径。
 */
export function isMalformedRef(refFile) {
  if (!refFile || refFile.length > 300) return true;
  // 允许前导点 / 省略号 / 斜杠 / 连字符：文档里常见「承前省略」写法
  // （`portfolio-mgr-h-ultra-short.rhai:303`、`-short.rhai:215`、`.../sqlite/table.rs:108`）
  // —— 它们**仍是引用**，交给 ABBREV 的候选自证去认领，不能在这里丢掉。
  if (!/^[.\-/]*[A-Za-z0-9_][A-Za-z0-9_./-]*$/.test(refFile)) return true;
  if (refFile.endsWith("/") || refFile.includes("//")) return true;
  return false;
}

/**
 * 行数（按 `\n` 切；末尾换行**不多算一行** —— 否则 `:N+1` 会被当成合法行）。
 * 结果缓存由调用方持有（同一目标文件常被多条引用命中）。
 * ⚠ 2026-11-05 起 `lineCache` 缓存的是**行数组**（`readLines`）而非行数 —— 符号一致性判据
 * 需要逐行内容。语义逐字不变：读不到（ENOENT）⇒ `null` ⇒ 调用方判 BROKEN。
 * ⚠ 2026-11-06（任务 #203）新增第三种返回：`UNREADABLE` ⇒ **盘在那一瞬间没读到**，
 * 调用方判 UNVERIFIED（未检查），**既不是** BROKEN（那是「仓内查无此文件」的客观判据）
 * **也不是** OK。ENOENT 的既有语义一条没改。
 */
function lineCount(abs, cache, notes) {
  const lines = readLines(abs, cache, notes);
  if (lines === UNREADABLE) return UNREADABLE;
  return lines === null ? null : lines.length;
}

/**
 * 读目标文件正文并按行切（LRU，上限 `LINES_CACHE_MAX` 个文件 —— 不设上限等于把整仓读进内存）。
 * 负缓存（读不到的文件记 `null`）与 `lineCount` 原有一致，不重复抛。
 * ⚠ 改动前的 `if (e.code !== "ENOENT") throw e;` **就是本任务修的崩溃点之一**：
 *   SMB 上的 EPERM / EBADF / `UNKNOWN(-4094)` 会把整道门打崩（退 1、零判定输出）。
 *   现在读盘一律走 `withIoRetry`（永不抛），瞬时错重试 `IO_RETRY_ATTEMPTS` 次，
 *   仍失败 ⇒ `UNREADABLE`。
 * ⚠ `UNREADABLE` **进缓存**：同一目标文件常被几十条引用命中，逐条重试等于把盘的抖动
 *   放大成几十倍；一次判定的代价只付一遍。代价是「这一瞬间读不到」在本次运行内不回滚 ——
 *   可接受：巡检门每次都重跑，且未检查面会被打印出来（不是静默通过）。
 */
export function readLines(abs, cache, notes) {
  if (cache.has(abs)) {
    const hit = cache.get(abs);
    cache.delete(abs); // 重新插入 ⇒ 移到队尾（LRU）
    cache.set(abs, hit);
    return hit;
  }
  let lines = null;
  const text = readFileSafe(abs, notes ?? makeIoNotes(), "target");
  if (text === UNREADABLE) {
    cache.set(abs, UNREADABLE);
    if (cache.size > LINES_CACHE_MAX) cache.delete(cache.keys().next().value);
    return UNREADABLE;
  }
  if (text !== null) {
    lines = text.split(/\r?\n/);
    if (lines.length > 0 && lines[lines.length - 1] === "") lines.pop();
  } // text === null ⇒ ENOENT：索引内有、盘上已删 ⇒ 与「仓内无此文件」同判 BROKEN（语义逐字不变）
  cache.set(abs, lines);
  if (cache.size > LINES_CACHE_MAX) cache.delete(cache.keys().next().value);
  return lines;
}

// ── 活文档 / 时点快照 的分类判据（任务 #202 ①，第二层判据⑥）────────────
// ⚠ 只有**第二层**用得到它。第一层（`classifyRef` / `hardViolations` / `entries` 基线）
//   对快照与活文档按原口径同跑，本批一字未改 —— 那是 2026-11-04 建门时的既有裁定。
//
// ⚠ **禁区 12 查重结果（为什么不复用）**：`check-single-source-facts.mjs` 的
//   `collectDocFiles()`（该文件 :663）**已有**一套同语义的三栏分档（`live` / `memory` / `hist`），
//   但① 它**不可导入** —— 那个脚本是顶层可执行过程，被 import 会整段跑扫描并 `process.exit`
//      （同一件事在 `scripts/lib/doc-refs.mjs` 头部已写死，抽取/定位原语正是因此才下沉的）；
//   ② 两处的**处置方向不同**：那边是取证分栏（`live` 只含 `docs/plans/**` + 根级 `*.md`，
//      `docs/features/`、`docs/reference/`、`docs/INDEX.md`、`docs/marketing/` 全进 `hist`
//      ⇒ 取证上「不许动」），本处的排除面**只有一条**（`docs/audits/AUDIT-*-<日期>`）⇒
//      本门的第二层收口面比那边**更宽**（还多收 `marketing/` `features/` `reference/` `architecture/`
//      `proposals/` 五个目录 + `docs/INDEX.md` + `docs/gate-coverage-ledger.md`；其中 marketing
//      在 AGENTS.md 表里逐字写作「非规范类**活文档**」），因为第二层是
//      「判红 + 棘轮」而不是「列可疑」。两边若强行合并成一个谓词，本门会跟着**放宽**，
//      而放宽判据是禁区级禁止项。
//   ⇒ 待办（不在本 lane 归属内，见汇报的未做清单）：把「目录语义 ⇒ 史料 / 活文档」
//     下沉到 `scripts/lib/doc-refs.mjs`，两道门各取所需的**映射**留在本侧，
//     共享的是**判据**而非处置。

/**
 * 快照目录登记表 —— **封闭白名单，判据只在这一处**（不散落字符串比较）。
 * 每条必须给 `basis`（规范出处），且必须**同时**命中目录前缀与文件名形态才算快照：
 *   · 只登记目录 ⇒ `docs/audits/README.md` 这类「目录说明」会被误当快照免检；
 *   · 只认日期形态 ⇒ 任何 `PLAN-x-2026-09-01.md` 都会被误当快照（活文档从此失守）。
 * 命中判据是「`dirPrefix` 是路径前缀 **且** `baseRe` 匹配 basename」，两者缺一不可。
 *
 * **新增快照目录怎么登记**（照本表追加一行，别在别处写 `if (doc.includes("audits"))`）：
 *   1. `dirPrefix`：以 `/` 结尾的仓库相对目录前缀（posix 分隔，与 `repoRel` 同口径）；
 *   2. `baseRe`：该目录里**快照文件**的 basename 形态（必须含日期戳；无日期戳的文件按活文档收口）；
 *   3. `basis`：规范出处一句话（`AGENTS.md`「文档放置」表的哪一行 / 哪一句）。
 *   追加后必须补一对正负控到 `runLifecycleSelftest()`（登记的目录内**带日期**判快照、
 *   **不带日期**判活文档），否则该条登记等于没登记。
 */
export const SNAPSHOT_DOC_RULES = [
  {
    id: "docs/audits/AUDIT-<主题>-<YYYY-MM-DD>",
    dirPrefix: "docs/audits/",
    baseRe: /^AUDIT-.+-20\d\d-\d\d-\d\d(?:-\d{3,4})?\.md$/i,
    basis:
      'AGENTS.md「文档放置」表：docs/audits/ = 「审计 / 体检 / 对比 / 根因定位报告（**时点快照**）」，' +
      "命名 `AUDIT-<主题>-<YYYY-MM-DD>.md`；实测本仓另有 `-HHMM` 时刻后缀（audit-l1-blockers 一族），" +
      "同为快照，故正则把可选时刻段一并认下。",
  },
];

/**
 * 纯函数（判据⑥）：这篇文档是**活文档**还是**时点快照**。
 * 兜底方向刻意是 `live`：认不准 ⇒ 照样收口检查（多查不是漏查）。
 * @param {string} docRel 仓库相对路径（`\` 与 `/` 都接受）
 * @returns {"live"|"snapshot"}
 */
export function docLifecycle(docRel) {
  const p = String(docRel ?? "").split("\\").join("/").replace(/^\.\//, "");
  const base = p.slice(p.lastIndexOf("/") + 1);
  for (const rule of SNAPSHOT_DOC_RULES) {
    if (p.startsWith(rule.dirPrefix) && rule.baseRe.test(base)) return "snapshot";
  }
  return "live";
}

// ── 行内符号一致性（SYMDRIFT）───────────────────────────────────────
// 判据全文见文件头「行内符号一致性」注释块。这里只放**纯函数**，便于 selftest 逐条命中。

/** 折行窗口下方行数（④）。取 2：rustfmt / prettier 只把名字推到紧邻下一行，再往下是函数体。 */
const WINDOW_BELOW_DEFAULT = 2;
/** ②：符号簇与引用 token 的最大列距离。表格行「引 | 符号」的间隔实测 5 列，24 列覆盖中文连接词。 */
const MAX_SYM_GAP = 24;
/** 缓存的目标文件数上限（LRU）。 */
const LINES_CACHE_MAX = 400;
/** 单个符号最多记多少个出现行（超出即截断，只影响「实际在第几行」的展示，不影响命中判定）。 */
const MAX_OCCURRENCES = 400;

/** 以文件扩展名结尾的簇（`index.ts` / `lib.rs`）是**文件名**不是符号。 */
const EXT_TAIL_RE = new RegExp(String.raw`^(?:.+\.)?(?:${DOC_REF_EXTS})$`, "i");

/**
 * 修饰词 / 关键字表：反引号簇里**只剩一个**非本表标识符时才算「给出了可比对符号」。
 * 用途见 `comparableSymbolSpan`（②）。表里没有 `new` / `default` 这类**可能真是符号名**的词。
 * ⚠ 后半段是 **prelude / 内置类型名**（`None` / `Option` / `Vec` / `Result` …）：
 *   它们在仓内几乎每个 `.rs` 里都「存在于别处」，判据③ 的零存在护栏拦不住，
 *   而散文里写 `` `None` `` 通常是在说**值**而不是说「该行定义了这个符号」。
 *   实测（2026-11-05，1046 条读数里）这一族贡献 28 条（`None` 16 / `Result` 4 / `Option` 3 /
 *   `Err` 2 / `Some` 1 / `Vec` 1 / `Arc` 1），故一并排除。
 */
const CODE_KEYWORDS = new Set(
  `pub crate async unsafe extern let var val const static fn func function class interface type enum
   struct trait impl mod macro record union abstract this self Self return if else for while loop match
   mut ref use where as dyn in of get set true false null undefined nil void string number boolean
   export import declare from throw try catch finally package private protected override virtual
   await yield typeof instanceof public internal
   none some ok err vec option result boxed box arc rc cell mutex rwlock hashmap hashset btreemap
   set list map dict tuple unit char float int uint isize usize i8 i16 i32 i64 u8 u16 u32 u64 f32 f64`
    .split(/\s+/)
    .filter(Boolean),
);

/** 反引号簇 `` `...` ``（不跨行）。返回内容与其**列区间**（不含两侧反引号）。 */
export function backtickSpans(line) {
  const out = [];
  for (const m of line.matchAll(/`([^`\r\n]+)`/g)) {
    out.push({ text: m[1], col: m.index + 1, endCol: m.index + 1 + m[1].length });
  }
  return out;
}

/**
 * 纯函数（判据②）：一个反引号簇是否是「可机械比对的符号」，并给出它的**形态**。
 *
 * @returns {{form:string, syms:string[]}}
 *   `form`：`ident`（裸标识符）｜ `decl`（`pub fn foo` 这类带关键字的声明形态）｜
 *   `path`（`Foo::bar` 路径）｜ `dotted`（`notify.channel` 点号链）｜ `none`（不是符号）
 *   `syms`：比对用的标识符（**末段在前**，路径首段作兜底）。
 *
 * ⚠ `dotted` 形态在本仓**不是「该行有这个符号」的断言**（实测 2026-11-05）：
 *   `docs/plans/PLAN-shell-plus-plugins.md` 的能力登记表把 `notify.channel` / `workflow.reflector` /
 *   `platform.adapter` / `session.store` 这类**接缝名**放在表格首列，`文件:行` 在次列 ——
 *   这些点号串是 `scripts/capability-register.json` 里的**登记表键**（逐条可查），
 *   不是目标文件里的符号。把它们当符号比对 ⇒ 整表成批假阳性。
 *   故默认（判据②）**排除 `dotted`**，只把 `::` 路径形态当作路径符号；
 *   需要把它们也计入时用 `--sym-include-dotted`（读数会虚高，仅供对账）。
 */
export function comparableSymbolSpan(spanText) {
  const none = { form: "none", syms: [] };
  const raw = spanText.trim();
  if (raw.length < 2 || raw.length > 80) return none;
  if (!/^[\x20-\x7e]+$/.test(raw)) return none; // 含中文 / 全角 ⇒ 散文
  if (/[\\/]/.test(raw)) return none; // 含路径分隔 ⇒ 是文件
  if (/\d/.test(raw)) return none; // 含数字 ⇒ 行号 / 版本 / 序号，不作符号比对（保守）
  const stripped = raw.replace(/^#\[[^\]]*\]\s*/, "");
  const s = stripped.replace(/::/g, "."); // 路径归一为点，便于整条链当一个符号
  const chains = s.match(/[A-Za-z_][A-Za-z0-9_]*(?:\.[A-Za-z_][A-Za-z0-9_]*)*/g) ?? [];
  const kept = chains.filter((c) => {
    const segs = c.split(".");
    const last = segs[segs.length - 1];
    if (CODE_KEYWORDS.has(last.toLowerCase())) return false;
    return !EXT_TAIL_RE.test(c); // `mod.rs` 这种「点号链」其实是文件名
  });
  if (kept.length !== 1) return none; // 0 = 散文；≥2 = 归属不明 ⇒ 不猜
  const chain = kept[0].split(".");
  const primary = chain[chain.length - 1];
  if (primary.length < 3) return none; // `id` / `x` 这类短名撞车率太高
  let form;
  if (chain.length > 1) form = raw.includes("::") ? "path" : "dotted";
  else form = /\s/.test(stripped.trim()) ? "decl" : "ident";
  const first = chain[0];
  const syms = first !== primary ? [primary, first] : [primary];
  return { form, syms };
}

/** 兼容包装：只要符号列表（selftest 与调用方按数组用）。 */
export function comparableSymbols(spanText) {
  return comparableSymbolSpan(spanText).syms;
}

/**
 * 纯函数（判据②的归属）：给**同一行**里的每条引用 token 归因「它自己那簇反引号里给出的符号」。
 * 规则：跳过与任何引用列区间重叠的簇（那就是引用本体 / 同行另一条引用）；其余簇归给**列距离最近**
 * 且 ≤ `MAX_SYM_GAP` 的那条引用；超出距离 ⇒ 归属不明 ⇒ 谁都不给（宁可漏报）。
 * 就地写 `r.symCandidates`（数组，每条 = `{ text, form, syms }`：原簇文本 / 形态 / 比对标识符）。
 */
export function attributeSymbolsToLine(refsOnLine) {
  if (refsOnLine.length === 0) return;
  const line = refsOnLine[0].docText ?? "";
  for (const r of refsOnLine) r.symCandidates = [];
  for (const sp of backtickSpans(line)) {
    let owner = null;
    let best = Infinity;
    for (const r of refsOnLine) {
      if (r.col === undefined) continue;
      if (sp.col < r.colEnd && r.col < sp.endCol) continue; // 与引用本体重叠 ⇒ 不是符号簇
      const gap = sp.endCol <= r.col ? r.col - sp.endCol : sp.col - r.colEnd;
      if (gap <= MAX_SYM_GAP && gap < best) {
        best = gap;
        owner = r;
      }
    }
    if (!owner) continue;
    const { form, syms } = comparableSymbolSpan(sp.text);
    if (syms.length > 0) owner.symCandidates.push({ text: sp.text, form, syms });
  }
}

/** 词边界匹配（`_`/字母/数字之外才算边界）；同一符号只编译一次。 */
const wordReCache = new Map();
function wordRegex(tok) {
  if (!wordReCache.has(tok)) {
    wordReCache.set(tok, new RegExp(`(?<![A-Za-z0-9_])${tok.replace(/[$^.*+?()|{}[\]\\]/g, "\\$&")}(?![A-Za-z0-9_])`));
  }
  return wordReCache.get(tok);
}

/** 符号在目标文件里的出现行（1-based，上限 `MAX_OCCURRENCES`）；按 (文件, 符号) 缓存。 */
function occurrenceLines(lines, abs, tok, occCache) {
  const k = `${abs}\u0000${tok}`;
  if (occCache.has(k)) return occCache.get(k);
  const re = wordRegex(tok);
  const out = [];
  for (let i = 0; i < lines.length && out.length < MAX_OCCURRENCES; i++) {
    if (re.test(lines[i])) out.push(i + 1);
  }
  occCache.set(k, out);
  return out;
}

/**
 * 第二层判据（SYMDRIFT）。就地给命中的引用写 `r.symDrift = { symbols, window, actual, forms }`，
 * 并按判据⑥ 写 `r.symFace`（`live` / `snapshot`）。
 * @param {{refs:object[], windowBelow?:number, lineCache?:Map<string, string[]|null|symbol>,
 *          includeDotted?:boolean, notes?:object}} opts
 * @returns {{drift:object[], driftSnapshot:object[], face:Record<string,{withSymbol:number,checked:number,absent:number,drift:number}>,
 *            withSymbol:number, checked:number, absent:number, dottedSkipped:number,
 *            byForm:Record<string,number>, byFormSnapshot:Record<string,number>,
 *            windowBelow:number, unreadableTargets:number}}
 *   · `drift`          —— **活文档面**的漂移（= 棘轮 / 升硬靶面；判据⑥ 把快照排除在外）
 *   · `driftSnapshot`  —— **快照面**的漂移：**只报告**，不进 `drift`、不进基线、永不判红、永不写盘
 *   · `face`           —— 两个面各自的抽样计数（「免检了多少」必须能算出来，见判据 #7）
 *   · `withSymbol`     —— 句中给出可比对符号的引用条数（判据②，**不含**被排除的点号接缝名；**两个面合计**）
 *   · `checked`        —— 其中符号确实存在于目标文件、真正进入行窗比对的条数（判据③）
 *   · `absent`         —— 符号在目标文件里零出现 ⇒ **跳过**（不是「通过」，是「未检」）
 *   · `dottedSkipped`  —— 只给出 `notify.channel` 这类点号接缝名的引用条数 ⇒ 判据② 排除（未检）
 *   · `byForm`         —— **活文档面**漂移按形态计数（`ident` / `decl` / `path` / `dotted`），供对账升硬时取舍
 *   · `byFormSnapshot` —— 快照面同口径计数（只为让两面读数能各自复算，不参与任何判定）
 *   · `unreadableTargets` —— （任务 #203）第一层已判 OK、但第二层重读正文时读不到的目标文件
 *     **所覆盖的引用条数** ⇒ 第二层未检。必须计数自陈：`lineCache` 是 LRU（上限 400 个文件，
 *     实测扫描面 ~1900 个目标文件），条目会被逐出后重读，SMB 上重读**确实可能**失败。
 */
export function auditSymbolDrift({
  refs,
  windowBelow = WINDOW_BELOW_DEFAULT,
  lineCache = new Map(),
  includeDotted = false,
  notes = undefined,
}) {
  const occCache = new Map();
  attributeAllLines(refs);
  const byTarget = new Map();
  const dottedOnly = [];
  for (const r of refs) {
    if (r.bucket !== "OK" || !r.abs) continue; // classifyRef 只给可定位的引用写 abs
    if (!r.symCandidates || r.symCandidates.length === 0) continue;
    const usable = r.symCandidates.filter((c) => includeDotted || c.form !== "dotted");
    if (usable.length === 0) {
      r.symDottedOnly = true; // 只给了 `notify.channel` 这类接缝名 ⇒ 未检，不是通过
      dottedOnly.push(r);
      continue;
    }
    if (!byTarget.has(r.abs)) byTarget.set(r.abs, []);
    byTarget.get(r.abs).push({ r, usable });
  }
  const drift = [];
  const driftSnapshot = [];
  const byForm = {};
  const byFormSnapshot = {};
  let withSymbol = 0;
  let checked = 0;
  let absent = 0;
  let unreadable = 0;
  const ioNotes = notes ?? makeIoNotes();
  // 分面抽样计数（判据⑥ 的自陈面）：`drift` 只收活文档，但两面的 withSymbol / checked /
  // absent 都必须各自有数 —— 否则「快照面到底免检了多少」无法复算（判据 #7）。
  const fc = {
    live: { withSymbol: 0, checked: 0, absent: 0, drift: 0 },
    snapshot: { withSymbol: 0, checked: 0, absent: 0, drift: 0 },
  };
  for (const [abs, list] of byTarget) {
    const lines = readLines(abs, lineCache, ioNotes);
    // 读盘失败的目标文件：`null` = 盘上确无（既有桶已判 BROKEN），`UNREADABLE` = 没读到
    // （既有桶已判 UNVERIFIED）⇒ 两种都**不叠第二层**，也不在此处再记一次。
    if (lines === null) continue;
    if (lines === UNREADABLE) {
      unreadable += list.length; // 第二层对这些引用**没跑** ⇒ 计数自陈，不许静默算比过
      continue;
    }
    for (const { r, usable } of list) {
      withSymbol++;
      // 判据⑥：按文档类别分面。**两面都照样抽样计数**（快照面若一条不计，就等于
      // 「悄悄少查了 282 条而读数看不出来」），区别只在 drift 归属与后续判定。
      const face = docLifecycle(r.doc);
      r.symFace = face;
      fc[face].withSymbol++;
      const toks = [...new Set(usable.flatMap((c) => c.syms))];
      const occ = new Map(toks.map((t) => [t, occurrenceLines(lines, abs, t, occCache)]));
      const present = toks.filter((t) => occ.get(t).length > 0);
      if (present.length === 0) {
        absent++;
        fc[face].absent++;
        r.symAbsent = true;
        continue; // 判据③：文件里根本没这个符号 ⇒ 不是行号漂移，另属「重命名/口语」那道题
      }
      checked++;
      fc[face].checked++;
      const lo = Math.max(1, r.start);
      const hi = Math.min(lines.length, Math.max(r.end, r.start) + windowBelow);
      // 判据⑤：任一候选命中即视为一致（保守）
      if (present.some((t) => occ.get(t).some((l) => l >= lo && l <= hi))) continue;
      const forms = [...new Set(usable.map((c) => c.form))];
      // 按该条**第一个候选簇**的形态计数（一条只计一次；一行常同时给 ident + path 两簇，
      // 逐形态各计会让 ΣbyForm > drift.length，读数就对不上账了）。
      const actual = occ
        .get(present[0])
        .slice()
        .sort((a, b) => Math.abs(a - r.start) - Math.abs(b - r.start))
        .slice(0, 3);
      // 第二层需要的比对材料**一并挂在 drift 条目上**（`occ` / `toks` / `lines`）：
      // 回写建议 `suggestSymbolFix` 因此是**纯计算**，不再读盘、不再猜。
      r.symDrift = {
        symbols: present,
        window: [lo, hi],
        actual,
        forms,
        spans: usable.map((c) => c.text),
        occ,
        toks,
        targetLines: lines,
      };
      if (face === "snapshot") {
        byFormSnapshot[forms[0]] = (byFormSnapshot[forms[0]] ?? 0) + 1;
        fc.snapshot.drift++;
        driftSnapshot.push(r);
      } else {
        byForm[forms[0]] = (byForm[forms[0]] ?? 0) + 1;
        fc.live.drift++;
        drift.push(r);
      }
    }
  }
  return {
    drift,
    driftSnapshot,
    face: fc,
    byFormSnapshot,
    withSymbol,
    checked,
    absent,
    dottedSkipped: dottedOnly.length,
    byForm,
    windowBelow,
    unreadableTargets: unreadable,
  };
}

/** 按（文档, 文档行）分组后逐行做符号归属（`attributeSymbolsToLine` 需要同行上下文）。 */
function attributeAllLines(refs) {
  const byLine = new Map();
  for (const r of refs) {
    if (r.srcLine === undefined) continue;
    const k = `${r.doc}\u0000${r.srcLine}`;
    if (!byLine.has(k)) byLine.set(k, []);
    byLine.get(k).push(r);
  }
  for (const list of byLine.values()) attributeSymbolsToLine(list);
}

/**
 * 纯函数：`--require-symbol-match` 档的判红集合（#182 语义：**不看基线**，整个靶面判红）。
 * 默认关 ⇒ 恒空数组 ⇒ 退码与该档落地前逐字相同。
 * ⚠ 常规档的判红**不在这里**，在 `symRatchetDiff`（棘轮：只看基线外新增）——
 *   两档各有消费方，别把它们读成同一个开关。`drift` 传入的必须是**活文档面**（判据⑥）。
 */
export function symbolHardViolations(drift, requireSymbolMatch) {
  return requireSymbolMatch ? drift : [];
}

// ── 第二层棘轮（任务 #202 ②）───────────────────────────────────────

/**
 * 纯函数：第二层的基线键 —— `<文档>::<被引文件> → <被引符号>`，**刻意不含行号**。
 * 理由（与第一层键相反）：第一层的病灶是「文档里写死的引用串」，引用串本身含行号 ⇒ 键必须含；
 * 第二层的病灶**就是行号**，把行号钉进键会让「源码里符号挪了一行」直接变成「基线外新增」，
 * 红得不属于文档缺陷（同族先例：`component-neutrality-baseline.json` 键 = file + target，
 * 与 `AGENTS.md`「白名单不含行号 —— 行号随编辑漂移」）。
 * 符号取 `symbols[0]` = 报告里点名「实际在第几行」的那个 ⇒ 每条键都能指回一行真实读数。
 * ⚠ 代价：同一「文档 + 被引文件 + 符号」的**多处**漂移共用一个条目（键去重），
 *   修掉其中一部分不会让条目消失 —— 条目消失的唯一形态是这个组合不再漂移。
 */
export function symDriftKey(r) {
  const sym = r && r.symDrift && r.symDrift.symbols.length > 0 ? r.symDrift.symbols[0] : "?";
  return `${r.doc}::${r.refFile} → ${sym}`;
}

/**
 * 纯函数：第二层与基线比对。**三种状态必须分得开**：
 *   · `symKeys === null` —— 基线里从来没有 `symEntries` ⇒ **棘轮未武装**：不判红，
 *     但必须由调用方自陈「未生效」并给出首冻命令（「没武装」与「查过没问题」不许长得一样）；
 *   · 已武装 ⇒ 键面去重后：基线外新增 ⇒ 红；基线内已消失 ⇒ 可收紧；
 *   · `requireSymbolMatch` ⇒ 整个活文档面判红，**不看基线**（#182 的全量对账档，语义不变）。
 */
export function symRatchetDiff(drift, symKeys, requireSymbolMatch = false) {
  const uniq = [];
  const seen = new Set();
  for (const r of drift) {
    const key = symDriftKey(r);
    if (seen.has(key)) continue;
    seen.add(key);
    uniq.push({ key, ref: r });
  }
  const armed = symKeys !== null && symKeys !== undefined;
  const cur = new Set(uniq.map((u) => u.key));
  return {
    armed,
    current: uniq,
    keyCount: uniq.length,
    baseSize: armed ? symKeys.size : 0,
    newViolations: armed ? uniq.filter((u) => !symKeys.has(u.key)) : [],
    fixedCount: armed ? [...symKeys].filter((k) => !cur.has(k)).length : 0,
  };
}

/**
 * 纯函数：基线 JSON ⇒ 两层键。`symKeys === null` 表示**第二层从未冻结**（区别于空集
 * 「冻结过、当时一条没有」）—— 这个区别是「武装 / 未武装」的唯一数据来源，不许混同。
 * @returns {{ok:boolean, refKeys:Set<string>, symKeys:Set<string>|null, why?:string}}
 */
export function parseBaselineShape(raw) {
  if (raw === null || raw === undefined) return { ok: true, refKeys: new Set(), symKeys: null };
  const refKeys = new Set(Array.isArray(raw.entries) ? raw.entries : []);
  if (raw.symEntries === undefined) return { ok: true, refKeys, symKeys: null };
  if (!Array.isArray(raw.symEntries)) {
    return { ok: false, refKeys, symKeys: null, why: "symEntries 存在但不是数组（基线被手改坏了 ⇒ 拒绝猜）" };
  }
  return { ok: true, refKeys, symKeys: new Set(raw.symEntries) };
}

// ── 自动回写的建议 / 验证 / 三道闸（任务 #202 ③；**默认零写盘**）──────────
/**
 * 声明形态词表：`isDeclarationOf` 用它判断「这一行是不是**定义**了这个符号」。
 * 分两档，缺一不可 —— 这一处就是回写精确率的地基：
 *   · `DECL_LEAD` —— 可跳过的**前导词**（可见性 / 修饰符 / 异步…），本身不定义任何东西；
 *   · `DECL_KEYWORDS` —— **定义词**：符号必须是「最后一个定义词」后面的第一个标识符。
 * ⚠ `impl` 刻意**不在**定义词里：`impl Foo {` 是「给 Foo 加实现」而不是 Foo 的定义处。
 *   算进去有两种坏结局：`struct Foo` + `impl Foo` ⇒ 「两处声明 ⇒ 歧义放弃」（覆盖率损失，可接受），
 *   或（若取首个命中）把文档订正到 impl 行（**假读数**，不可接受）。
 * ⚠ `new` / `for` / `use` / `from` / `in` / `match` / `return` 同样不在：
 *   它们的后随标识符是调用点 / 循环变量 / 导入项，把它们当定义就是拿假阳性写盘。
 */
const DECL_LEAD = new Set(
  `pub crate async unsafe extern readonly mutable shared public protected private internal
   abstract final open override virtual declare export default inline friend volatile`.split(/\s+/),
);
const DECL_KEYWORDS = new Set(
  `fn func function struct enum trait type class interface record union mod module macro namespace
   package def proc let var val property get set define const static`.split(/\s+/),
);
/** 注释行首形态：符号只写在注释里 **不算**声明（与判据④「上方不留 0 行」同一个理由）。 */
const COMMENT_HEAD_RE = /^\s*(?:\/\/|\/\*|\*+\s|\*\/|--(?!-)|#|%|"""|'''|;;)/;

/**
 * 纯函数：`lineText` 这一行是否是 `symTok` 的**定义行**。
 * 判据（机械、可复算）：① 非注释行；② 剥缩进与前导属性 `#[...]` / `@dec`；
 * ③ 从左扫标识符：前导只允许 `DECL_LEAD`；④ 符号必须紧跟在**至少一个** `DECL_KEYWORDS` 之后；
 * ⑤ 符号后紧跟 `.` / `::` ⇒ 是接收者 / 路径前缀，不是定义（`self.tick` 由此落空）。
 * 于是这些形态全部**不算**定义：`foo(3);`（调用点）、`use crate::foo;`、`crate::foo(x);`、
 * `impl Foo {`、`tick: u64,`（字段，无定义词）、`// foo`（注释）。
 * 这些算：`pub fn foo(`、`pub struct EngineState {`、`export interface X {`、`const X: u64 = 1`。
 */
export function isDeclarationOf(symTok, lineText) {
  const s = String(lineText ?? "");
  if (!s.trim() || !symTok) return false;
  if (COMMENT_HEAD_RE.test(s)) return false;
  const stripped = s.replace(/^\s*(?:#\[[^\]]*\]\s*|@[A-Za-z_][\w.]*\s*)?/, "");
  let sawKeyword = false;
  for (const m of stripped.matchAll(/[A-Za-z_][A-Za-z0-9_]*/g)) {
    const p = m[0];
    if (p === symTok) {
      if (!sawKeyword) return false;
      return !/^\s*(?:\.|::)/.test(stripped.slice(m.index + p.length));
    }
    const low = p.toLowerCase();
    if (DECL_KEYWORDS.has(low)) {
      sawKeyword = true;
      continue;
    }
    if (DECL_LEAD.has(low)) continue;
    return false; // 前导里冒出别的标识符 ⇒ 符号不在「定义词 + 名字」的位置
  }
  return false;
}

/**
 * 纯函数：给一条 SYMDRIFT 算**唯一无歧义**的订正（改到第几行）。
 * 「宁可不出建议」的闸共 8 个 `code`，任一命中 ⇒ `{ok:false, code, why}`
 * （`code` 用于计数分档，`why` 逐条打印 ⇒ 精确率的分母必须可见）：
 *   `snapshot`（判据⑥）｜`range`（区间 / 多行清单）｜`multi-symbol`（句中 ≥2 个可比对符号）｜
 *   `no-material`（缺 occ / targetLines）｜`no-decl`（出现全是调用点 / 注释）｜
 *   `multi-decl`（同名 ≥2 处定义 ⇒ 歧义）｜`same`（建议 == 现值）｜`verify`（写前验证不通过）。
 * 精确率就建立在这八条上：它们把「猜」全部挡在门外，代价是覆盖率低（自陈，不掩饰）。
 */
export function suggestSymbolFix(r) {
  const no = (code, why) => ({ ok: false, code, why });
  const d = r && r.symDrift;
  if (!d) return no("not-drift", "非 SYMDRIFT 条目");
  if (docLifecycle(r.doc) === "snapshot") return no("snapshot", "时点快照（判据⑥ 不订正）");
  if (r.start !== r.end) return no("range", "区间 / 多行清单引用 ⇒ 端点语义不明");
  if (!Array.isArray(d.symbols) || d.symbols.length !== 1) {
    return no("multi-symbol", `句中有 ${d.symbols ? d.symbols.length : 0} 个可比对符号 ⇒ 归属不明`);
  }
  const sym = d.symbols[0];
  const occ = d.occ && d.occ.get(sym);
  const lines = d.targetLines;
  if (!Array.isArray(occ) || !Array.isArray(lines)) return no("no-material", "缺少比对材料（occ / targetLines）");
  const decl = occ.filter((l) => l >= 1 && l <= lines.length && isDeclarationOf(sym, lines[l - 1]));
  if (decl.length === 0) return no("no-decl", `符号 \`${sym}\` 的 ${occ.length} 处出现没有一处是声明形态 ⇒ 不知该指到哪行`);
  if (decl.length > 1) return no("multi-decl", `符号 \`${sym}\` 有 ${decl.length} 处声明（${decl.slice(0, 4).join("/")}）⇒ 歧义`);
  const to = decl[0];
  if (to === r.start) return no("same", "建议与现值相同 ⇒ 无需写盘");
  if (!verifyProposedFix(sym, lines, to)) return no("verify", "写前验证不通过（内部不一致）");
  return { ok: true, code: "decl", to, sym, via: "decl" };
}

/**
 * 纯函数：**独立**复核「第 to 行确实是 sym 的声明行」。
 * 与 `suggestSymbolFix` 用的是同一个谓词，但它是**写盘前逐条必过**的一道闸 ⇒
 * 自检里必须用「故意写错的行号」（调用点 / 注释 / 越界）证明它会返回 false，
 * 否则这道闸就是装饰（任务要求「错了能不能被发现」）。
 */
export function verifyProposedFix(sym, lines, to) {
  if (!sym || !Array.isArray(lines)) return false;
  if (!Number.isInteger(to) || to < 1 || to > lines.length) return false;
  return isDeclarationOf(sym, lines[to - 1]);
}

/** 精确率要求：标注语料上 100%（回写是**改文档正文**，错一条就是新的假读数）。 */
export const FIX_PRECISION_REQUIRED = 1;

/**
 * 纯函数：精确率闸。`stats` = 自检标注语料的实测计数
 * `{emitted, correct, emittedOnAmbiguous}`。三条硬判据，缺一即闸不过：
 *   · `emitted === 0` ⇒ **不过**（无分母的「100%」是空洞通过，同 #199「扫 0 文件仍退 0」）；
 *   · `emittedOnAmbiguous > 0` ⇒ **不过**（歧义负控上还敢出建议 ⇒ 判据不保守）；
 *   · `correct / emitted < required` ⇒ **不过**。
 */
export function fixPrecisionGate(stats, required = FIX_PRECISION_REQUIRED) {
  const s = stats || { emitted: 0, correct: 0, emittedOnAmbiguous: 0 };
  if (!s.emitted) return { ok: false, precision: 0, why: "自检语料一条建议都没产出 ⇒ 精确率没有分母（空洞通过）" };
  if (s.emittedOnAmbiguous > 0) {
    return { ok: false, precision: s.correct / s.emitted, why: `歧义负控上仍产出 ${s.emittedOnAmbiguous} 条建议 ⇒ 判据不保守` };
  }
  const p = s.correct / s.emitted;
  if (p < required) {
    return { ok: false, precision: p, why: `实测精确率 ${(p * 100).toFixed(1)}%（${s.correct}/${s.emitted}）< 要求 ${(required * 100).toFixed(0)}%` };
  }
  return { ok: true, precision: p, why: `精确率 ${(p * 100).toFixed(1)}%（${s.correct}/${s.emitted}），歧义负控零建议` };
}

/**
 * 纯函数：写盘模式（`--fix`）的闸。**「开关在无自检覆盖时拒绝执行」就落在这一处**：
 * 自检不是「平时碰巧跑过一次」，而是 `--fix` 当场跑一遍并把条数交进来。
 * @param {{fix:boolean, attested:boolean, selftestRan:boolean, selftestAsserts:number,
 *          selftestPassed:boolean, precisionOk:boolean, precisionWhy:string,
 *          unverifiedAny:boolean, armed:boolean}} f
 * @returns {{ok:boolean, code:number, why:string}}
 */
export function writeModeGate(f) {
  const no = (code, why) => ({ ok: false, code, why });
  if (!f.fix) return no(2, "未请求写盘模式");
  if (!f.attested) {
    return no(2, "写盘必须**同时**给 --i-attest-quiet-window：并行期改 docs/ 正文会把别的 lane 的中间态钉进文档（本仓踩过）");
  }
  if (!f.selftestRan) return no(2, "回写判据无自检覆盖（自检没跑）⇒ 拒绝写盘");
  if (!(f.selftestAsserts > 0)) return no(2, `回写自检跑了但执行断言数 = ${f.selftestAsserts} ⇒ 等于没覆盖 ⇒ 拒绝写盘`);
  if (!f.selftestPassed) return no(2, "回写自检有失败项 ⇒ 拒绝写盘");
  if (!f.precisionOk) return no(2, `精确率闸未过：${f.precisionWhy}`);
  // 「先验精确率」的另一半：真实仓**没有对照基线**（第二层从未冻结）⇒ 改错了无从发现。
  // 自检标注语料只证明判据本身对，不证明这一批真实建议对；有 `symEntries` 才有 before/after 账。
  if (!f.armed) return no(2, "第二层棘轮未武装（基线无 symEntries）⇒ 拒绝写盘：改错一条没有对照面可发现，先跑 --init-baseline 首冻");
  if (f.unverifiedAny) return no(2, "本轮存在未检查面（读盘失败）⇒ 拒绝写盘（与 refuseBaselineWrite 同型：盘的抖动不该代劳改文档）");
  return { ok: true, code: 0, why: "写盘六道闸全过" };
}

/**
 * 纯函数：把文档某一行里**这一个**引用 token 的起始行号改成 `toLine`，返回新行文本。
 * 只认「列区间内确实是一个 `文件:行号` token」的形态，其余一律放弃（不猜、不模糊替换）：
 *   · 裸 `:行号`（承接前一文件名）⇒ 列区间不含文件名 ⇒ 放弃（替换目标不唯一）；
 *   · token 内的数字与读数不符 ⇒ 放弃（说明列区间/文本已被并发改动）；
 *   · 行号后紧跟 `-` 或 `/`（区间 / 多行清单）⇒ 放弃。
 * 保证**只动那一个数字段**：数字段以外的所有列逐字不变（长度差异只可能来自数字本身位数）。
 */
export function patchRefStartNumber(lineText, ref, toLine) {
  const s = String(lineText ?? "");
  const seg = s.slice(ref.col, ref.colEnd);
  if (!seg) return { ok: false, why: "列区间读不到（文档行已变短？）" };
  if (seg.trimStart().startsWith(":")) return { ok: false, why: "裸 `:行号`（文件名在别处）⇒ 放弃" };
  const m = seg.match(/^([A-Za-z0-9_./-]+[ \t]*:[ \t]*)(\d+)/);
  if (!m) return { ok: false, why: `token 形态不符：${JSON.stringify(seg.slice(0, 40))}` };
  if (m[2] !== String(ref.start)) return { ok: false, why: `token 内是 ${m[2]}、读数是 ${ref.start} ⇒ 已被改动，放弃` };
  const tail = seg.slice(m[0].length);
  if (/^[-/]/.test(tail.trimStart())) return { ok: false, why: "区间 / 多行清单尾 ⇒ 放弃" };
  const patched = `${m[1]}${toLine}${tail}`;
  return { ok: true, text: s.slice(0, ref.col) + patched + s.slice(ref.colEnd) };
}

/**
 * 纯函数：**写盘前**逐条复核改后的行文本（任务要求「错了要能被发现」的那一半）。
 * 四条机械不变式，任一不符 ⇒ 拒绝（不写、不部分写）：
 *   ① 真的改动了；② 引用列**之前**的文本逐字不变；③ 引用列**之后**的文本逐字不变；
 *   ④ 用**同一个抽取器**在改后文本上能读出 `toLine`，且该行引用**条数不变**。
 * ⚠ ④ 用抽取器复验而不是自己拼正则：判据必须与读数同源，否则「写进去的」和
 *   「下一轮读出来的」可能是两套语法（本仓「产物与判据不同源」的反复形态）。
 */
export function verifyPatchedLine(orig, patched, ref, toLine) {
  if (orig === patched) return { ok: false, why: "未产生任何改动" };
  if (!patched.startsWith(orig.slice(0, ref.col))) return { ok: false, why: "引用列之前的文本被改动" };
  if (!patched.endsWith(orig.slice(ref.colEnd))) return { ok: false, why: "引用列之后的文本被改动" };
  const before = extractRefs(`${orig.slice(ref.col)}\n`, DOC_REF_EXTS).length;
  const after = extractRefs(`${patched.slice(ref.col)}\n`, DOC_REF_EXTS);
  if (after.length !== before) return { ok: false, why: `该行引用条数由 ${before} 变成 ${after.length}` };
  if (!after.some((t) => t.line === toLine)) return { ok: false, why: `抽取器在改后文本里读不到 ${toLine}` };
  return { ok: true };
}

/** 纯函数：`--sym-window=N` 的解析（非法 ⇒ null ⇒ main 退 2 自陈）。 */
export function parseSymWindowFlag(flag) {
  if (flag === undefined) return WINDOW_BELOW_DEFAULT;
  const m = flag.match(/^--sym-window=(\d+)$/);
  if (!m || Number(m[1]) > 50) return null;
  return Number(m[1]);
}

/**
 * 单条引用判定（文件系统只经 `lineCount` / `index` 访问）。
 * @returns {{bucket:string, target?:string, abs?:string, candidates?:string[], kind?:string, lines?:number, why?:string}}
 */
export function classifyRef(ref, ctx) {
  const { refFile, start, end } = ref;
  if (isMalformedRef(refFile)) return { bucket: "MALFORMED" };
  if (isEphemeralRef(refFile)) return { bucket: "EPHEMERAL" };
  if (isExternalRef(refFile)) return { bucket: "EXTERNAL" };

  const loc = ctx.locate(refFile);
  if (loc.kind === "N") return { bucket: "NONLOC", why: loc.why };
  if (loc.kind === "B") {
    // 自证：先找「段内截断」（字符级后缀），再找「换词」（词干集合包含）
    const trunc = abbrevCandidates(refFile, ctx.index, ctx.root);
    if (trunc.length > 0) return { bucket: "ABBREV", kind: "截断", candidates: trunc };
    const syn = synonymCandidates(refFile, ctx.index, ctx.root);
    if (syn.length > 0) return { bucket: "ABBREV", kind: "同义词", candidates: syn };
    // 找不到候选 ⇒ 明确「无候选」，绝不猜（猜出来的桶指向不了真实风险，见 AGENTS.md）
    return { bucket: "BROKEN", candidates: [] };
  }
  const n = lineCount(loc.abs, ctx.lineCache, ctx.notes);
  if (n === null) return { bucket: "BROKEN", candidates: [] };
  // 读盘失败（瞬时错重试仍失败 / 非 ENOENT 的确定性错）⇒ **未检查**。
  // ⚠ 三条硬约束：① 不许判 OK（那是「检查且通过」）；② 不许判 BROKEN（那会说谎：
  //    「仓内查无此文件」，而文件其实在，只是我们没读到 —— 会把人引向改一处根本没错的引用）；
  //    ③ 不许与既有 BROKEN/NONLOC 桶混同，故独立成桶并自带 `why`（错误码 + 尝试次数）。
  if (n === UNREADABLE) {
    const f = ctx.notes && ctx.notes.find("target", loc.abs);
    return {
      bucket: "UNVERIFIED",
      target: ctx.repoRel(loc.abs),
      abs: loc.abs,
      why: f ? `读不出正文：${f.err}（尝试 ${f.attempts} 次${f.sleptMs ? `，退避 ${f.sleptMs}ms` : ""}）` : "读不出正文（原因未登记）",
    };
  }
  const overflow = start > n || (end && end > n);
  return {
    bucket: overflow ? "STALE" : "OK",
    target: ctx.repoRel(loc.abs),
    // `abs` 只给第二层（SYMDRIFT）判据定位目标文件用；**不参与**桶判定、键、输出行 ⇒ 既有读数不变
    abs: loc.abs,
    lines: n,
  };
}

/**
 * 引用串的**左边界补全**。
 *
 * 为什么需要：共用的抽取器（`lib/doc-refs.mjs` 的 `extractRefs`）要求文件名的首字符是
 * `[A-Za-z0-9_]`，于是文档里常见的**点开头路径**会被截短：
 *   `.github/workflows/ci.yml:12` → 抽出 `github/workflows/ci.yml:12`（少了前导点）
 *   `.workbuddy/memory/X.md:3`    → 抽出 `workbuddy/memory/X.md:3`
 * 截短后的串在本仓**不存在** ⇒ 会被判 BROKEN（假红），而它指向的文件明明在。
 * 修法放在**本门禁侧**而不是改共用抽取器：改抽取器会改变 `check-single-source-facts.mjs`
 * 的 `.rs` 面读数（那边有棘轮基线，凭空多出命中会直接红），风险远大于收益。
 *
 * 判据：从匹配起点向左吃掉连续的路径字符（含 `.`），只影响**前导**，不改后缀。
 */
function expandRefFile(ref) {
  const line = ref.text ?? "";
  // ⚠ 只对**带文件名的锚**生效：裸 `:293`（承接同一行的前一个文件名）在原文里
  //   并没有 `ref.file` 这几个字符，盲目向左吃会把前一段行号（`266/`）和文件名拼成
  //   `266/:293`` 用` 这类怪物串（首版实测，全是假 BROKEN）。判据：该列区间内的原文
  //   必须**逐字等于** refFile，否则原样返回。
  if (line.slice(ref.col, ref.col + ref.file.length) !== ref.file) return ref.file;
  let i = ref.col;
  while (i > 0 && /[A-Za-z0-9_./-]/.test(line[i - 1])) i--;
  if (i === ref.col) return ref.file;
  return line.slice(i, ref.col + ref.file.length);
}

/**
 * 扫描 + 判定（不写盘、不 exit）。selftest 与正式跑共用这一条路径。
 *
 * @param {{root:string, docs:string[], index:Map<string,string[]>, repoRel:(abs:string)=>string,
 *          resolve?:(refFile:string)=>(string|null), notes?:object}} opts
 *   `resolve` 缺省走 `locateRef` 的逐条 stat 解析（selftest 用）；批量跑必须传内存解析器
 *   （见 `makeFileSetResolver`，理由：SMB 上逐条 stat ~190ms）。
 *   `notes` 缺省自产一份（读盘失败登记表）；正式跑必须传同一份给 main，否则未检查面没人打印。
 * @returns {{refs:object[], counts:Record<string,number>, docs:number, docsRead:number,
 *            lineCache:Map<string,string[]|null|symbol>, notes:object}}
 *   ⚠ `counts` 的**桶口径不含 SYMDRIFT** —— 第二层判据（`auditSymbolDrift`）只往引用对象上挂
 *   `symDrift` / `symAbsent` / `symFace`（判据⑥ 的活文档/快照归属）字段，**不改 `bucket`**，
 *   所以既有各桶计数与退出码逐字不变（快照与活文档在第一层完全同口径）。
 *   ⚠ `docsRead` = 正文真的读到手的文档篇数。**只统计成功**，不靠 `docs.length` 反推 ——
 *   改动前读不到一篇文档会让 `refs` 变少而 `docs` 不变，summary 看上去毫无异常（假绿形态）。
 */
export function auditDocs({ root, docs, index, repoRel, resolve, notes }) {
  const lineCache = new Map();
  const ioNotes = notes ?? makeIoNotes();
  let docsRead = 0;
  // 同一条引用常被多份文档重复引用（登记表 / 汇总页）⇒ 解析结果按引用串缓存，
  // 否则同一目标会被 stat（或查集合）上千次。
  const locCache = new Map();
  const locate = (refFile) => {
    if (!locCache.has(refFile)) {
      locCache.set(refFile, locateRef(refFile, index, root, resolve));
    }
    return locCache.get(refFile);
  };
  const ctx = { root, index, repoRel, lineCache, locate, notes: ioNotes };
  const seen = new Set();
  const refs = [];
  for (const abs of docs) {
    const docRel = repoRel(abs);
    // ⚠ 改动前这里是**裸 `fs.readFileSync`**（无任何 try/catch）⇒ SMB 瞬时错直接打崩整道门
    //   （实测：注入 EPERM ⇒ 退 1、零判定输出）。兜底**不许**写成「catch 后 continue 什么都不记」：
    //   那等于静默缩小扫描面，比崩溃更糟（崩溃至少是响的）。故读不到的文档记 `kind:"doc"`，
    //   由 summary 打印「N 篇文档没读到 ⇒ 其内引用面整体未检查」，并参与「一篇都没读成功 ⇒ 退 2」的判据。
    const text = readDocText(abs, ioNotes);
    if (text === UNREADABLE) continue; // 记入未检查面；该文档的引用一条不产生（不猜、不记 OK）
    docsRead++;
    // `extractRefs` 把 `:A-B` 拆成两条（起点 + 终点），`:A/B` 拆成两条多行清单。
    // 同一个 token（同一列区间）归并成**一条**引用：区间终点只是边界，不是另一条引用。
    const toks = new Map();
    const lineTexts = new Map();
    for (const r of extractRefs(text, DOC_REF_EXTS)) {
      const k = `${r.srcLine}:${r.col}`;
      if (!toks.has(k)) {
        // `col` / `colEnd` = 引用 token 在本行里的列区间（第二层判据要靠它做符号归属）；
        // `docText` 只截前 2000 字符，够覆盖一行散文，也避免把整篇文档留在内存里。
        toks.set(k, {
          file: expandRefFile(r),
          srcLine: r.srcLine,
          start: r.line,
          end: r.line,
          col: r.col,
          colEnd: r.end,
          text: (r.text ?? "").slice(0, 2000),
        });
      }
      const t = toks.get(k);
      t.start = Math.min(t.start, r.line);
      t.end = Math.max(t.end, r.line);
      lineTexts.set(r.srcLine, t.text);
    }
    for (const t of [...toks.values()].sort((a, b) => a.srcLine - b.srcLine || a.start - b.start)) {
      const refFile = t.file;
      const key = `${docRel}::${refFile}:${t.start}${t.end !== t.start ? `-${t.end}` : ""}`;
      if (seen.has(key)) continue; // 同一文档内重复引用 ⇒ 只判一次（输出不被淹）。
      // ⚠ 被丢掉的重复 token 也就不参与第二层的「同行符号归属」上下文 —— 它判定与幸存那条完全相同，
      //   归属只会落到同文件的同一行号上，不影响结论。
      seen.add(key);
      const verdict = classifyRef({ refFile, start: t.start, end: t.end }, ctx);
      refs.push({
        key,
        doc: docRel,
        docLine: t.srcLine + 1,
        srcLine: t.srcLine,
        refFile,
        start: t.start,
        end: t.end,
        col: t.col,
        colEnd: t.colEnd,
        docText: lineTexts.get(t.srcLine) ?? "",
        ...verdict,
      });
    }
  }
  const counts = {};
  for (const r of refs) counts[r.bucket] = (counts[r.bucket] ?? 0) + 1;
  // `lineCache` 一并返回：第二层（SYMDRIFT）复用同一份正文缓存，避免 SMB 上把目标文件读两遍。
  // `notes` 一并返回：未检查面（读盘失败）是**覆盖面数据**，必须由调用方打印，不许留在函数里蒸发。
  return { refs, counts, docs: docs.length, docsRead, lineCache, notes: ioNotes };
}

/** 纯函数：与基线比对。返回 {newViolations, fixedCount, baseSize}。 */
export function diffAgainstBaseline(baseKeys, violations) {
  const cur = new Set(violations.map((v) => v.key));
  return {
    newViolations: violations.filter((v) => !baseKeys.has(v.key)),
    fixedCount: [...baseKeys].filter((k) => !cur.has(k)).length,
    baseSize: baseKeys.size,
  };
}

/** 只有 BROKEN / STALE 判红（ABBREV / NONLOC / EXTERNAL / EPHEMERAL 只报告）。 */
export function hardViolations(refs) {
  return refs.filter((r) => r.bucket === "BROKEN" || r.bucket === "STALE");
}

// ── selftest 断言计数（任务 #203 自证要求 1）─────────────────────────
// 为什么必须自己数并打印：`--selftest` 此前只打印「通过与否」，**不打印跑了多少条**。
// 于是「补了 M 条断言」这件事无法被第三方核对 —— 而本仓踩过的正是「断言写了但扫描面
// 没起来 ⇒ 一条没执行 ⇒ 照样绿」。这里数的是**执行**次数（含循环展开），不是源码行数，
// 两者实测不同（`for (const [at, why] of [...])` 那族一条调用点跑 6 次）。
let ASSERT_RUN = 0;
let ASSERT_FAIL = 0;

// ── selftest：正负对照（临时目录，不依赖仓内真实文件）────────────────
function runSelftest() {
  let fail = 0;
  const check = (cond, msg, extra) => {
    ASSERT_RUN++;
    if (!cond) {
      console.error(`SELFTEST FAIL：${msg}${extra === undefined ? "" : `（实际 ${JSON.stringify(extra)}）`}`);
      fail++;
      ASSERT_FAIL++;
    }
  };
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "doc-refs-selftest-"));
  try {
    const write = (rel, content) => {
      const abs = path.join(tmp, rel);
      fs.mkdirSync(path.dirname(abs), { recursive: true });
      fs.writeFileSync(abs, content, "utf8");
    };
    // 目标文件：三档合法引用各自有对应真实文件；另有 trunc/synonym/越界/同名多份的靶子
    write("src/a.rs", "fn a() {}\n");
    write("src/commands/b.rs", "fn b() {}\n");
    write("src/c.rs", "l1\nl2\nl3\n");
    write("src/init/state.rs", "// s\n");
    write("src/other/state.rs", "// s\n");
    write("src/x/m.rs", "// m\n");
    write("src/y/m.rs", "// m\n");
    write("src/opc_admin_actions.rs", "// 候选（段内截断）\n");
    write("src/opc_capability_pack_bridge.rs", "// 候选（换词）\n");
    write("src/short.rs", "l1\nl2\nl3\n");
    // 点开头路径：抽取器会截掉前导点 ⇒ 本门禁必须靠左边界补全把它救回来（不得判 BROKEN）
    write(".github/workflows/ci.yml", "jobs: {}\n");

    // 正控三档 + 负控五类（段内截断 / 同义词 / 无候选 / 越界 / 同名多份不可消歧）
    write(
      "docs/guide.md",
      [
        "见 `src/a.rs:1`（完整路径）",
        "见 `commands/b.rs:1`（唯一后缀）",
        "见 `c.rs:3`（唯一 basename）",
        "见 `init/state.rs:1`（唯一后缀消歧）",
        "见 `admin_actions.rs:4`（段内截断）",
        "见 `domain_pack_bridge.rs:5`（同义词换词）",
        "见 `ghost_file.rs:6`（无候选）",
        "见 `src/short.rs:99`（行号越界）",
        "见 `m.rs:1`（同名多份且后缀不可消歧）",
        "见 `sea-orm-2.0.2/src/schema/entity.rs:10`（跨仓）",
        "见 `.workbuddy/memory/MEMORY.md:3`（环境相关）",
        "见 `src/short.rs:1-99`（区间终点越界）",
        "见 `.github/workflows/ci.yml:1`（点开头路径：抽取器会截掉前导点，必须补全）",
        "",
      ].join("\n"),
    );

    check(listDocFiles(tmp).length === 1, "扫描面：临时目录应收到 1 篇文档", listDocFiles(tmp).length);
    // 负控（面试）：docs 不存在时扫描面必须为 0 ⇒ 调用方据此判「脚本失效」，不许静默绿
    check(listDocFiles(path.join(tmp, "nope")).length === 0, "扫描面为 0 时必须能被识别（不得静默通过）");

    const index = buildBasenameIndex(tmp, SKIP_DIRS);
    const repoRel = (abs) => path.relative(tmp, abs).split(path.sep).join("/");
    const { refs, counts } = auditDocs({ root: tmp, docs: listDocFiles(tmp), index, repoRel });

    // 期望：OK 5 ｜ ABBREV 2 ｜ BROKEN 1 ｜ STALE 2（`:99` 与 `:1-99`）｜ NONLOC 1 ｜ EXTERNAL 1 ｜ EPHEMERAL 1
    check(
      counts.OK === 5,
      "正控：完整路径 / 唯一后缀 / 唯一 basename / 后缀消歧 / 点开头路径 应判 OK=5",
      counts.OK,
    );
    check(counts.ABBREV === 2, "段内截断 + 同义词换词 应判 ABBREV=2", counts.ABBREV);
    check(counts.BROKEN === 1, "无候选的引用应判 BROKEN=1", counts.BROKEN);
    check(counts.STALE === 2, "行号越界（单行 + 区间终点）应判 STALE=2", counts.STALE);
    check(counts.NONLOC === 1, "同名多份且后缀不可消歧应判 NONLOC=1", counts.NONLOC);
    check(counts.EXTERNAL === 1, "版本目录形态的跨仓引用应判 EXTERNAL=1", counts.EXTERNAL);
    check(counts.EPHEMERAL === 1, "环境相关目录的引用应判 EPHEMERAL=1", counts.EPHEMERAL);

    // ABBREV 的「自证」必须点到真实文件 —— 找不到候选就不许落这个桶
    const abbrev = refs.filter((r) => r.bucket === "ABBREV");
    const trunc = abbrev.find((r) => r.refFile === "admin_actions.rs");
    check(!!trunc, "段内截断样本必须落在 ABBREV 桶");
    check(
      !!trunc && trunc.candidates.includes("src/opc_admin_actions.rs"),
      "段内截断必须自证到 src/opc_admin_actions.rs",
      trunc?.candidates,
    );
    check(trunc?.kind === "截断", "段内截断应标 kind=截断", trunc?.kind);
    const syn = abbrev.find((r) => r.refFile === "domain_pack_bridge.rs");
    check(
      !!syn && syn.candidates.includes("src/opc_capability_pack_bridge.rs"),
      "同义词换词必须自证到 src/opc_capability_pack_bridge.rs",
      syn?.candidates,
    );
    check(syn?.kind === "同义词", "同义词换词应标 kind=同义词", syn?.kind);
    const ghost = refs.find((r) => r.refFile === "ghost_file.rs");
    check(ghost?.bucket === "BROKEN" && ghost.candidates.length === 0, "无候选必须明确报「无候选」");

    // 回归钉：文件索引由 git 索引构建，工作区里「已删除但未提交」的文件仍在其中。
    // 这时 locate 会返回一个盘上不存在的绝对路径 —— 必须判 BROKEN，不许 readFileSync 打崩整道门。
    const vanishedCtx = {
      locate: () => ({ kind: "L", abs: path.join(tmp, "src", "vanished.rs") }),
      index,
      root: tmp,
      repoRel,
      lineCache: new Map(),
    };
    check(
      classifyRef({ refFile: "vanished.rs", start: 1 }, vanishedCtx).bucket === "BROKEN",
      "索引内有、盘上已删（搬迁未提交）必须判 BROKEN 而非抛 ENOENT",
    );

    // 红面 = BROKEN + STALE（ABBREV 只报告，不许进红）
    const hard = hardViolations(refs);
    check(hard.length === 3, "判红面应恰为 BROKEN 1 + STALE 2 = 3", hard.length);
    check(!hard.some((r) => r.bucket === "ABBREV"), "ABBREV 不得进红面（首轮只报告）");

    // 形态护栏：不是路径的串必须被丢弃（否则给基线灌垃圾，BROKEN 桶就指不回真实风险）
    check(isMalformedRef("266/:293` 用") === true, "散文残片（含反引号 / 中文）必须判 MALFORMED");
    check(isMalformedRef("src/a.rs") === false, "正常路径不得误判 MALFORMED");
    check(isMalformedRef("a//b.rs") === true, "空路径段必须判 MALFORMED");

    // 棘轮正负控：基线内存量不红；新增一条 ⇒ 红
    const base = new Set(hard.map((r) => r.key));
    check(diffAgainstBaseline(base, hard).newViolations.length === 0, "基线内存量不得判红");
    const plusOne = [...hard, { key: "docs/guide.md::new_file.rs:1" }];
    check(diffAgainstBaseline(base, plusOne).newViolations.length === 1, "基线外新增必须判红");
    check(diffAgainstBaseline(new Set(), hard).newViolations.length === 3, "空基线（首轮）必须全部判红");
    check(diffAgainstBaseline(new Set(["gone"]), hard).fixedCount === 1, "基线内已消失应计入可收紧");
  } finally {
    fs.rmSync(tmp, { recursive: true, force: true });
  }

  if (fail > 0) return false;
  console.log(
    "SELFTEST OK：三档合法（完整 / 唯一后缀 / 唯一 basename）+ 后缀消歧 ⇒ OK；" +
      "段内截断 / 同义词换词 ⇒ ABBREV 且自证候选；无候选 ⇒ BROKEN；" +
      "单行与区间终点越界 ⇒ STALE；同名多份不可消歧 ⇒ NONLOC；跨仓 / 环境相关免检；棘轮正负控通过",
  );
  return true;
}

// ── selftest（第二层）：行内符号一致性 SYMDRIFT ────────────────────
// 独立的临时目录，**不碰上面那套桶判据的靶面** ⇒ 既有用例的每个期望值都原样不动。
function runSymbolSelftest() {
  let fail = 0;
  const check = (cond, msg, extra) => {
    ASSERT_RUN++;
    if (!cond) {
      console.error(`SELFTEST FAIL：${msg}${extra === undefined ? "" : `（实际 ${JSON.stringify(extra)}）`}`);
      fail++;
      ASSERT_FAIL++;
    }
  };
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "doc-refs-symtest-"));
  try {
    const write = (rel, content) => {
      const abs = path.join(tmp, rel);
      fs.mkdirSync(path.dirname(abs), { recursive: true });
      fs.writeFileSync(abs, content, "utf8");
    };
    // rust 靶（行号逐字可数）
    write(
      "src/engine.rs",
      [
        "// 头部注释", // 1
        "pub struct EngineState {", // 2
        "    pub tick: u64,", // 3
        "}", // 4
        "", // 5
        "impl EngineState {", // 6
        "    pub fn snapshot_all(&self) -> u64 {", // 7
        "        self.tick", // 8
        "    }", // 9
        "}", // 10
        "", // 11
        "pub fn resolve_gap_width(a: u64, b: u64) -> u64 {", // 12
        "    a.max(b)", // 13
        "}", // 14
        "", // 15
        "impl NamedThing", // 16
        "    for EngineState", // 17  ← rustfmt 折行：名字在引用行的**下一行**
        "{", // 18
        "    fn wide(x: u64) -> u64 { x }", // 19
        "}", // 20
        "",
      ].join("\n"),
    );
    // ts 靶（复刻 leader 实测的 `src/types/index.ts` 表格行形态）
    write(
      "src/pages.ts",
      [
        "/**", // 1
        " * 返回形状。", // 2
        " */", // 3
        "export interface PluginPagesInfoDto {", // 4
        "  pages: PluginPageDto[];", // 5
        "  tick: number;", // 6
        "}", // 7
        "",
      ].join("\n"),
    );
    write(
      "docs/syms.md",
      [
        "见 `src/engine.rs:12` 的 `resolve_gap_width`（正控：符号确在所指行）", // 1
        "见 `src/engine.rs:7` 的 `resolve_gap_width`（负控：漂到同文件另一处 ⇒ 须点名 12）", // 2 ⇒ 漂移
        "见 `src/engine.rs:16` 的 `EngineState`（折行窗口内命中 17 行 ⇒ 不报）", // 3
        "见 `src/engine.rs:13` 的 `EngineState`（出窗 ⇒ 报，实际最近 17）", // 4 ⇒ 漂移
        "见 `src/engine.rs:8` 的 `EngineState::tick`（rust 路径形态：末段 tick 命中）", // 5
        "见 `src/engine.rs:2` 的 `rename_all = camelCase`（一簇里剩 2 个标识符 ⇒ 归属不明，不检）", // 6
        "见 `src/engine.rs:3` 的 `TotallyAbsentIface`（符号在目标文件零出现 ⇒ 跳过，不是漂移）", // 7
        "见 `src/engine.rs:5`（句中没有可机械比对符号 ⇒ 不检也不许退红）", // 8
        "| `src/pages.ts:4` | `PluginPagesInfoDto`（ts 正控） |", // 9
        "| `src/pages.ts:7` | `PluginPagesInfoDto`，`pages` 复用 `PluginPageDto` |（ts 负控）", // 10 ⇒ 漂移
        "见 `src/pages.ts:6` 的 `engine.tick`（点号接缝名形态 ⇒ 判据② 默认排除，不检）", // 11
        "见 `src/engine.rs:2-4` 的 `EngineState`（区间引用：窗口覆盖全区间）", // 12
        "见 `src/engine.rs:19-20` 的 `EngineState`（区间内确实没有 ⇒ 报，实际最近 17）", // 13 ⇒ 漂移
        "见 `src/engine.rs:9` 与一段中文填充文字，跨度超过二十四个字符才出现的符号 `resolve_gap_width` 不该被归属", // 14
        "| `src/pages.ts:2` | 接缝 `pages.tick`（同为点号形态；`--sym-include-dotted` 时才报） |", // 15
        "",
      ].join("\n"),
    );

    const index = buildBasenameIndex(tmp, SKIP_DIRS);
    const repoRel = (abs) => path.relative(tmp, abs).split(path.sep).join("/");
    const symDocs = [path.join(tmp, "docs", "syms.md")];
    const { refs, counts, lineCache } = auditDocs({ root: tmp, docs: symDocs, index, repoRel });
    const sym = auditSymbolDrift({ refs, windowBelow: 2, lineCache });

    // 前置：全部引用必须仍是 OK（第二层**不许**改桶），既有判红面仍是 0 ⇒ 默认退码不变
    check(counts.OK === 15, "靶面：15 条引用都应仍判 OK（SYMDRIFT 不进桶）", counts);
    check(hardViolations(refs).length === 0, "第二层判据不得改动桶：既有判红面必须仍为 0");
    check(symRefsAllOk(refs), "SYMDRIFT 条目自身 bucket 仍为 OK（独立计数，不与 BROKEN/STALE 混桶）");

    // 抽样面计数：可比对符号 / 进入行窗比对 / 零出现跳过 / 点号接缝名排除
    check(sym.withSymbol === 10, "判据②：10 条引用给出可比对符号（多标识符 / 无符号 / 超距离 / 点号接缝名 共 5 条不检）", sym.withSymbol);
    check(sym.absent === 1, "判据③：符号零出现的那条应记为「跳过（未检）」而非漂移", sym.absent);
    check(sym.checked === 9, "真正进入行窗比对的应恰为 9 条", sym.checked);
    check(sym.dottedSkipped === 2, "点号接缝名形态（`engine.tick` / `pages.tick`）必须被排除并**计数自陈**", sym.dottedSkipped);

    // 负控：行号漂到同文件另一处 ⇒ 报进新桶，且点名实际行
    check(sym.drift.length === 4, "漂移应恰为 4 条（rs 两条 + ts 表格行一条 + 区间一条）", sym.drift.length);
    check(sym.byForm.ident === 4 && Object.keys(sym.byForm).length === 1, "漂移按形态计数：4 条全是裸标识符", sym.byForm);
    const d2 = sym.drift.find((r) => r.start === 7);
    check(!!d2 && d2.symDrift.actual[0] === 12, "负控：漂移到同文件另一处必须点名实际行 12", d2?.symDrift?.actual);
    const d4 = sym.drift.find((r) => r.start === 13);
    check(!!d4 && d4.symDrift.actual[0] === 17, "负控：出窗样本必须点名最近的真实出现行 17", d4?.symDrift?.actual);
    const d10 = sym.drift.find((r) => r.docLine === 10);
    check(!!d10 && d10.symDrift.symbols.includes("PluginPagesInfoDto"), "ts 表格行负控必须被抓到（leader 实测形态）");
    check(!!d10 && d10.symDrift.actual[0] === 4, "ts 负控：实际声明行应点名 4", d10?.symDrift?.actual);

    // 正控：三类「不许报」
    for (const [at, why] of [
      [1, "符号确在所指行"],
      [3, "折行窗口内命中（下一行）"],
      [5, "rust 路径形态（::）取末段命中"],
      [9, "ts 正控"],
      [12, "区间引用覆盖全区间"],
      [14, "符号离引用超 24 列 ⇒ 不归属"],
    ]) {
      check(!sym.drift.some((r) => r.docLine === at), `正控（第 ${at} 行，${why}）不得报漂移`);
    }
    // 最重要的一条：句中无可比对符号 ⇒ 不检、也不许退红
    check(!sym.drift.some((r) => r.docLine === 8), "句中无可比对符号 ⇒ 不检（这条若误报，整篇散文都会红）");
    check(!sym.drift.some((r) => r.docLine === 6), "一簇里剩 ≥2 个标识符 ⇒ 归属不明，不检");
    check(!sym.drift.some((r) => r.docLine === 7), "符号在目标文件零出现 ⇒ 判「跳过」不判漂移");
    check(!sym.drift.some((r) => r.docLine === 11), "点号接缝名形态默认排除 ⇒ 不检（本仓实测主要是它，误检会整表假阳性）");

    // 读数口径开关：`--sym-include-dotted` 只把点号形态放回来（第 15 行由此转红，第 11 行仍命中）
    const wide = auditSymbolDrift({ refs: [...refs], windowBelow: 2, lineCache, includeDotted: true });
    check(wide.dottedSkipped === 0, "打开 --sym-include-dotted 后不应再有「接缝名排除」");
    check(wide.withSymbol === 12, "打开后给出符号的引用应是 12 条（多出第 11、15 两行）", wide.withSymbol);
    check(
      wide.drift.length === 5 && wide.drift.some((r) => r.docLine === 15),
      "打开后只应多报第 15 行那条（第 11 行的 tick 确在所指行 ⇒ 仍不报）",
      wide.drift.map((r) => r.docLine),
    );
    check(wide.byForm.dotted === 1, "新增那条必须记在 dotted 形态下（自陈它是接缝名而非符号）", wide.byForm);

    // 窗口收紧只多报不少报（折行窗口取值 2 的反证：0 行时第 3 条正控会变红）
    check(
      auditSymbolDrift({ refs: [...refs], windowBelow: 0, lineCache }).drift.length === 5,
      "窗口取 0 时只应多报那 1 条折行样本（⇒ 默认取 2 是在往漏报方向收敛）",
    );

    // 升硬开关：关 ⇒ 违规恒空（退码与改前逐字相同）；开 ⇒ 全部成为违规
    check(symbolHardViolations(sym.drift, false).length === 0, "--require-symbol-match 关闭时 SYMDRIFT 不进退出码");
    check(symbolHardViolations(sym.drift, true).length === 4, "--require-symbol-match 打开时 SYMDRIFT 全部判红（⇒ 退非 0）");
    check(parseSymWindowFlag(undefined) === 2, "缺省窗口 = 2");
    check(parseSymWindowFlag("--sym-window=5") === 5, "--sym-window=5 应解析成 5");
    check(parseSymWindowFlag("--sym-window=x") === null, "非法窗口值必须返回 null（main 据此退 2）");
    check(parseSymWindowFlag("--sym-window=99") === null, "超出 0..50 的窗口值必须返回 null");

    // 符号形态单测（判据②的护栏逐条）
    check(comparableSymbols("PluginPagesInfoDto").join() === "PluginPagesInfoDto", "单个 PascalCase 标识符 = 可比对符号");
    check(comparableSymbols("pub fn resolve_gap_width").join() === "resolve_gap_width", "声明形态取名字");
    check(comparableSymbols("EngineState::tick")[0] === "tick", "rust 路径取末段");
    check(comparableSymbols("engine.tick")[0] === "tick", "点号链取末段（形态由 comparableSymbolSpan 判）");
    check(comparableSymbols("list_plugin_pages()")[0] === "list_plugin_pages", "带括号调用形态");
    check(comparableSymbols("src/types/index.ts:2123").length === 0, "引用本体（含路径与数字）不是符号");
    check(comparableSymbols("mod.rs").length === 0, "文件名（以扩展名结尾）不是符号");
    check(comparableSymbols("rename_all = camelCase").length === 0, "剩 2 个标识符 ⇒ 归属不明");
    check(comparableSymbols("无 新增").length === 0, "含非 ASCII ⇒ 散文");
    check(comparableSymbols("v2").length === 0, "短于 3 字符的末段不比对（撞车率太高）");
    check(comparableSymbols("impl EngineState").join() === "EngineState", "impl 形态剥掉关键字");
    check(comparableSymbolSpan("notify.channel").form === "dotted", "点号链 = 接缝名形态（默认排除的依据）");
    check(comparableSymbolSpan("SessionManager::new").form === "path", ":: 路径形态仍按符号比对");
    check(comparableSymbolSpan("pub fn foo").form === "decl", "带关键字的声明形态");
    check(comparableSymbolSpan("PluginPagesInfoDto").form === "ident", "裸标识符形态");
    check(comparableSymbolSpan("见 foo 说明").form === "none", "散文形态 ⇒ none");
    check(comparableSymbolSpan("Foo::Bar::baz").syms.join() === "baz,Foo", "多级路径：末段在前 + 首段兜底");
  } finally {
    fs.rmSync(tmp, { recursive: true, force: true });
  }

  if (fail > 0) return false;
  console.log(
    "SELFTEST OK（第二层 SYMDRIFT）：符号在行 ⇒ 不报；漂到同文件另一处 ⇒ 报并点名实际行；" +
      "折行窗口内命中 ⇒ 不报（窗口取 0 时该条转红，故 2 是漏报方向收敛）；" +
      "句中无符号 / 多标识符 / 超距离 / 符号零出现 / 点号接缝名 ⇒ 不检也不退红；rs 与 ts 两形态各覆盖；" +
      "--sym-include-dotted 只放回接缝名读数；开关关闭 ⇒ 违规恒空（既有退码不变），打开 ⇒ 全部判红；" +
      "桶计数仍为 OK（不与 BROKEN/STALE 混桶）",
  );
  return true;
}

/** 纯函数：SYMDRIFT 条目是否**没有**污染既有桶（第二层的语义护栏）。 */
export function symRefsAllOk(refs) {
  return refs.every((r) => !r.symDrift || r.bucket === "OK");
}

// ── selftest（第三层）：I/O 兜底（任务 #203）────────────────────────
// 独立临时目录 + **临时 monkeypatch `fs`**（SMB 的瞬时错无法在现场按指令复现，注入是唯一能
// 逐类覆盖 `e.code` 的办法；`finally` 里逐字还原，不留全局副作用）。
// 正反控配对（缺任一条都不算覆盖）：
//   正控 —— 注入的错**真的**把未检查桶填上了（引用级 / 文档级 / 目录级 / 第二层 四路各自验证）；
//   负控 —— 正常路径与既有「文件不存在」判据**不进**该桶，且重试没有拖慢它。
function runIoSelftest() {
  let fail = 0;
  const check = (cond, msg, extra) => {
    ASSERT_RUN++;
    if (!cond) {
      console.error(`SELFTEST FAIL：${msg}${extra === undefined ? "" : `（实际 ${JSON.stringify(extra)}）`}`);
      fail++;
      ASSERT_FAIL++;
    }
  };
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "doc-refsiotest-"));
  const origReadFileSync = fs.readFileSync;
  const origReaddirSync = fs.readdirSync;
  try {
    const write = (rel, content) => {
      const abs = path.join(tmp, rel);
      fs.mkdirSync(path.dirname(abs), { recursive: true });
      fs.writeFileSync(abs, content, "utf8");
    };
    write("src/ok.rs", "fn ok() {}\n");
    write("src/recovered.rs", "fn rec() {}\n");
    write("src/dead-eperm.rs", "fn d() {}\n");
    write("src/dead-unknown.rs", "fn d2() {}\n");
    write("src/dead-eisdir.rs", "fn d3() {}\n");
    write("src/flip.rs", "pub struct FlipProbe;\n");
    write("src/short.rs", "l1\nl2\nl3\n");
    write(
      "docs/io-alive.md",
      [
        "见 `src/ok.rs:1`（正常路径 ⇒ 仍判 OK，不落未检查桶）",
        "见 `src/recovered.rs:1`（首次 EBADF、重试即成功 ⇒ 判 OK 且付 1 次退避）",
        "见 `src/dead-eperm.rs:1`（恒 EPERM ⇒ 重试仍失败 ⇒ 未检查，**不是** OK 也**不是** BROKEN）",
        "见 `src/dead-unknown.rs:2`（SMB 实际形态 `UNKNOWN` / errno -4094 ⇒ 未检查）",
        "见 `src/dead-eisdir.rs:3`（确定性非 ENOENT ⇒ 未检查，且**不重试**）",
        "见 `src/no_such_anywhere.rs:6`（既有判据：仓内查无此文件 ⇒ BROKEN，不许被本批改判）",
        "见 `src/short.rs:99`（既有判据：行号越界 ⇒ STALE）",
        "",
      ].join("\n"),
    );
    write("docs/io-back.md", "见 `src/ok.rs:1`（文档正文首次读失败、重试成功 ⇒ 该引用必须被检查到）\n");
    // ⚠ 这篇「列在目录里但每次读都 ENOENT」—— readdir 与 open 之间的竞态（SMB 上真实形态）。
    //   文档侧的 ENOENT **必须**进未检查面：目标文件侧的 ENOENT 有既有 BROKEN 语义可归，
    //   而「文档读不到」没有任何桶可归 ⇒ 不登记就是静默缩水（变异测试实测过这一条会漏）。
    write("docs/io-vanish.md", "见 `src/short.rs:2`（文档 ENOENT ⇒ 这条不许产出，但篇目必须被登记）\n");
    // 这篇文档**恒读不到**：它里面的「越界引用」一条都不许产生 —— 产生即说明我们在猜。
    write("docs/io-dead.md", "见 `src/short.rs:99`（读不到 ⇒ 这条不许进 STALE，也不许进 OK）\n");
    // 这篇在一个**整目录读不到**的子目录里 ⇒ 同上，一条不许产生，但目录级未检查要被登记。
    write("docs/sub/io-sub-deep.md", "见 `src/short.rs:2`（目录 readdir 恒 EPERM ⇒ 不许静默消失）\n");
    write("docs/io-flip.md", "见 `src/flip.rs:1` 的 `FlipProbe`（第一层读成功、第二层重读失败 ⇒ 第二层未检自陈）\n");

    // ── 注入器：按 basename 匹配，`plan(name, n)` 返回 Error 表示「第 n 次读就失败」────
    const E = (code, errno) => {
      const e = new Error(`${code}: selftest 注入（任务 #203）`);
      e.code = code;
      e.errno = errno;
      return e;
    };
    const plan = new Map([
      ["dead-eperm.rs", () => E("EPERM", 1)],
      ["dead-unknown.rs", () => E("UNKNOWN", -4094)],
      ["dead-eisdir.rs", () => E("EISDIR", 21)],
      ["recovered.rs", (n) => (n === 1 ? E("EBADF", 9) : null)],
      ["flip.rs", (n) => (n >= 2 ? E("EIO", 5) : null)],
      ["io-dead.md", () => E("EPERM", 1)],
      ["io-vanish.md", () => E("ENOENT", 2)],
      ["io-back.md", (n) => (n === 1 ? E("EBADF", 9) : null)],
    ]);
    const dirPlan = new Map([["sub", () => E("EPERM", 1)]]);
    const hits = new Map();

    const index = buildBasenameIndex(tmp, SKIP_DIRS); // 先按真实文件系统建索引（未被污染）
    fs.readFileSync = function (p, ...rest) {
      const name = path.basename(String(p));
      const f = plan.get(name);
      if (f) {
        const n = (hits.get(name) ?? 0) + 1;
        hits.set(name, n);
        const err = f(n);
        if (err) throw err;
      }
      return origReadFileSync.call(fs, p, ...rest);
    };
    fs.readdirSync = function (p, ...rest) {
      const name = path.basename(String(p));
      const f = dirPlan.get(name);
      if (f) {
        const n = (hits.get("dir:" + name) ?? 0) + 1;
        hits.set("dir:" + name, n);
        const err = f(n);
        if (err) throw err;
      }
      return origReaddirSync.call(fs, p, ...rest);
    };

    const notes = makeIoNotes();
    const docs = listDocFiles(tmp, notes);
    // 正控（目录级）：`docs/sub` 恒 EPERM ⇒ 篇目里**没有** io-sub-deep.md，但被登记为未检查面
    check(!docs.some((d) => d.endsWith(path.join("sub", "io-sub-deep.md"))), "目录读不到时该目录下的文档不得进入扫描面（不猜）");
    check(notes.count("dir") === 1, "目录级 readdir 失败必须记 1 条未检查（此前是 `catch { return; }` 静默丢文档）", notes.failures);
    check(docs.length === 5, "文档面应为 5 篇（io-alive / io-back / io-dead / io-vanish / io-flip；sub 整目录读不到 ⇒ 0 篇）", docs.length);

    const repoRel = (abs) => path.relative(tmp, abs).split(path.sep).join("/");
    const { refs, counts, docsRead, lineCache } = auditDocs({ root: tmp, docs, index, repoRel, notes });

    check(docsRead === 3, "读成功的文档篇数 = 3（io-dead.md 那篇没读到，必须由 docsRead 如实自陈）", docsRead);

    // ── 正控 ①：真·读不到 ⇒ 判 UNVERIFIED，**既不判健康也不判 BROKEN**（要求 ①）──
    const bucketOf = (name) => refs.filter((r) => r.refFile.endsWith(name)).map((r) => r.bucket);
    check(bucketOf("src/dead-eperm.rs").join() === "UNVERIFIED", "恒 EPERM 的目标文件必须落 UNVERIFIED（不是 OK / 不是 BROKEN）", bucketOf("src/dead-eperm.rs"));
    check(bucketOf("src/dead-unknown.rs").join() === "UNVERIFIED", "errno -4094 / code UNKNOWN（SMB 最常见形态）必须落 UNVERIFIED", bucketOf("src/dead-unknown.rs"));
    check(bucketOf("src/dead-eisdir.rs").join() === "UNVERIFIED", "确定性非 ENOENT 的读错同样落 UNVERIFIED（判 BROKEN 会把工具故障记成文档缺陷）", bucketOf("src/dead-eisdir.rs"));
    check(counts.UNVERIFIED === 3, "「未检查」桶计数必须恰为 3", counts.UNVERIFIED);
    check(counts.OK === 4, "正常路径 + 重试成功 的引用必须仍判 OK（共 4 条）", counts.OK);

    // ── 正控 ②：未检查桶的**来源**逐项可指回（哪一次读、报什么错、退避多久）──
    const d1 = refs.find((r) => r.refFile.endsWith("src/dead-eperm.rs"));
    const f1 = notes.find("target", d1?.abs ?? "");
    check(!!f1 && f1.attempts === IO_RETRY_ATTEMPTS, "恒失败的目标文件必须重试满 3 次才落未检查", f1);
    check(!!f1 && f1.sleptMs === IO_RETRY_BASE_MS + IO_RETRY_BASE_MS * IO_RETRY_FACTOR, "退避必须逐字是 30ms + 90ms（改动先改理由再改数）", f1?.sleptMs);
    check(/EPERM/.test(String(d1?.why)), "UNVERIFIED 的 why 必须带错误码（每条要指回是哪一次读）", d1?.why);
    const dU = refs.find((r) => r.refFile.endsWith("src/dead-unknown.rs"));
    check(notes.find("target", dU?.abs ?? "")?.errno === -4094, "UNKNOWN 必须按 errno -4094 登记（否则按 code 白名单漏网）", notes.find("target", dU?.abs ?? ""));

    // ── 正控 ③：重试**真的救回来了**（不是 catch 里跳过）──
    check(bucketOf("src/recovered.rs").join() === "OK", "第 2 次读成功的引用必须判 OK（证明重试在跑，而不是被跳过）", bucketOf("src/recovered.rs"));
    const recF = notes.failures.filter((x) => x.kind === "target" && x.at.endsWith("recovered.rs"));
    check(recF.length === 0, "重试成功的那条**不得**留在未检查面", recF);
    check(bucketOf("src/ok.rs").length === 2, "同一目标被两篇文档引用 ⇒ 两条都在（文档侧重试成功那篇的引用没被静默丢）", bucketOf("src/ok.rs"));

    // ── 正控 ④：文档级读不到 ⇒ 该文档的引用**一条不产生**，且被登记 ──
    // 文档侧 ENOENT（readdir 与 open 之间的竞态）也要登记 —— 变异测试实测：删掉 `readDocText`
    // 里那句显式 `notes.record` 后，**只有这两条**会红（EPERM 那条由 `withIoRetry` 兜住，测不出来）。
    const vanishAbs = docs.find((d) => d.endsWith("io-vanish.md")) ?? "";
    check(!!notes.find("doc", vanishAbs) && notes.find("doc", vanishAbs).code === "ENOENT", "文档读成 ENOENT 仍必须登记进未检查面", notes.find("doc", vanishAbs));
    check(!refs.some((r) => r.doc === "docs/io-vanish.md"), "ENOENT 的文档不许凭空产出引用（它那条本可判 OK）");
    check(!refs.some((r) => r.doc === "docs/io-dead.md"), "读不到的文档不得凭空产出引用（既不判红也不判健康）", refs.filter((r) => r.doc === "docs/io-dead.md"));
    check(notes.count("doc") === 2, "文档级读不到必须记 2 条（EPERM 的 io-dead + ENOENT 竞态的 io-vanish）", notes.failures);
    check(counts.STALE === 1, "越界引用只能来自**读成功**的那篇文档（io-dead.md 里同一条不许重复判 STALE）", counts.STALE);

    // ── 负控 ①（要求 ③）：既有「文件不存在」判据**没被改判、也没被拖慢** ──
    check(counts.BROKEN === 1, "既有 BROKEN 判据不变：仓内查无此文件仍是 BROKEN", counts.BROKEN);
    const ghost = refs.find((r) => r.refFile.endsWith("src/no_such_anywhere.rs"));
    check(!!ghost && ghost.bucket === "BROKEN" && ghost.candidates.length === 0, "无候选仍明确报「无候选」（不许退化成 UNVERIFIED）");
    const enotes = makeIoNotes();
    const ectx = { locate: () => ({ kind: "L", abs: path.join(tmp, "src", "gone.ts") }), index, root: tmp, repoRel, lineCache: new Map(), notes: enotes };
    check(classifyRef({ refFile: "gone.ts", start: 1 }, ectx).bucket === "BROKEN", "索引内有、盘上已删（ENOENT）⇒ 仍判 BROKEN（既有语义逐字不变）");
    check(enotes.retries === 0 && enotes.failures.length === 0, "ENOENT 路径必须**零重试**（重试不许把「文件不存在」判据拖慢）", { retries: enotes.retries, failures: enotes.failures.length });
    // 负控 ②：正常路径完全不落未检查面
    check(counts.UNVERIFIED === 3 && notes.failures.length === 6, "未检查面来源数恰为 6（3 目标 + 2 文档 + 1 目录），无多余条目", notes.failures.length);
    check(!refs.some((r) => r.bucket === "UNVERIFIED" && r.refFile.endsWith("src/ok.rs")), "可读的正常引用绝不落未检查桶（负控）");

    // ── 正控 ⑤：第一层读成功、第二层重读失败 ⇒ 第二层未检**计数自陈** ──
    const symShared = auditSymbolDrift({ refs: [...refs], windowBelow: 2, lineCache, notes });
    check(symShared.unreadableTargets === 0, "复用同一份正文缓存时第二层不该出现重读失败（负控）", symShared.unreadableTargets);
    const symFresh = auditSymbolDrift({ refs: [...refs], windowBelow: 2, lineCache: new Map(), notes });
    check(symFresh.unreadableTargets === 1, "缓存逐出后重读失败（LRU 上限 400，实测面 ~1900 个文件 ⇒ 真实存在）必须被计数", symFresh.unreadableTargets);
    // 第二层重读失败**也要**进未检查面登记（翻篇：3 个目标 → 4 个），否则那一条只留下
    // `unreadableTargets` 一个数字，报告里指不回是哪个文件。
    check(notes.failures.length === 7 && notes.count("target") === 4, "第二层重读失败必须同样登记（3+1 个目标 / 共 7 个来源）", { n: notes.failures.length, t: notes.count("target") });

    // ── 覆盖面摘要 + 升硬开关（退码口径的正负控）──
    const cov = ioCoverageSummary(notes, counts.UNVERIFIED ?? 0);
    check(cov.refs === 3 && cov.targets === 4 && cov.docs === 2 && cov.dirs === 1, "覆盖面摘要必须分档给出四个数（引用 3 / 目标 4（含第二层重读那 1 个）/ 文档 2 / 目录 1）", cov);
    check(cov.retries === 12, `累计重试必须恰为 12 次（recovered 1 + dead-eperm 2 + dead-unknown 2 + flip 2 + io-back 1 + io-dead 2 + 目录 2）`, cov.retries);
    check(cov.exhausted === false, "本面预算未耗尽（660ms < 5000ms）", cov.retryMs);
    check(unverifiedHardFail(cov, false) === false, "默认（不带开关）⇒ UNVERIFIED **不**进退出码 ⇒ 盘的抖动不变红灯");
    check(unverifiedHardFail(cov, true) === true, "--require-no-unverified ⇒ 未检查面非空即判红（显式加严开关可用）");
    check(unverifiedHardFail(ioCoverageSummary(makeIoNotes(), 0), true) === false, "未检查面为空时即便带开关也不许判红（负控）");
    // 基线写入门禁：未检查面非空 ⇒ **无条件**拒绝 --init/--update-baseline（不是开关）
    check(refuseBaselineWrite(cov) === true, "有未检查面时必须拒绝写棘轮基线（读不到的引用会被当成「已修复」而洗白基线）");
    check(refuseBaselineWrite(ioCoverageSummary(makeIoNotes(), 0)) === false, "干净一轮（零未检查）照旧允许写基线（负控，不许挡正常收紧）");
    check(hardViolations(refs).length === counts.BROKEN + counts.STALE, "判红面仍是且仅是 BROKEN + STALE（UNVERIFIED 不得混入既有红面）", hardViolations(refs).map((r) => r.bucket));

    // ── 「一篇都没读成功 ⇒ 门本身失效」的判据（退 2 档，与盘的抖动不同）──
    check(scanProducedNothing(0, 0) === true, "扫描面 0 篇 ⇒ 门失效（既有判据不变）");
    check(scanProducedNothing(5, 0) === true, "收到 5 篇但一篇都没读到 ⇒ 门失效，不许报「无违规」");
    check(scanProducedNothing(5, 4) === false, "部分读不到 ⇒ 不是门失效（那是未检查面，默认只报告）");

    // ── 退避预算：耗尽后**立即停止重试**（卡死与崩溃同样是「没有判定输出」）──
    const tight = makeIoNotes(0);
    const t1 = readFileSafe(path.join(tmp, "src", "dead-eperm.rs"), tight, "target");
    check(t1 === UNREADABLE, "预算为 0 时仍要落 UNREADABLE（未检查），不许退回抛异常打崩门禁", t1);
    check(tight.retries === 0 && tight.exhausted === true, "预算耗尽 ⇒ 一次都不重试并置 exhausted（既不卡死也不静默）", tight);
    check(tight.failures.length === 1, "预算耗尽仍要登记未检查面（否则「没重试」就等同于「没这回事」）", tight.failures.length);

    // ── 错误分类单测（按 Node 实际 `e.code` 归档，见上方注释块的实测依据）──
    check(classifyIoError(E("ENOENT", 2)) === "missing", "ENOENT ⇒ 既有的「盘上确无此文件」路径");
    check(classifyIoError(E("EISDIR", 21)) === "static", "EISDIR/ENOTDIR/EINVAL/ELOOP/ENAMETOOLONG ⇒ 确定性，不重试");
    check(classifyIoError(E("EBADF", 9)) === "transient", "EBADF ⇒ 瞬时（句柄被回收）");
    check(classifyIoError(E("EPERM", 1)) === "transient", "EPERM ⇒ 瞬时（杀软 / 索引器 / hgfs 抢锁）");
    check(classifyIoError(E("UNKNOWN", -4094)) === "transient", "UNKNOWN ⇒ 瞬时（SMB 映射不出来的 NTSTATUS）");
    check(classifyIoError({ errno: UV_UNKNOWN_ERRNO }) === "transient", "只带 errno -4094 没带 code 的 SMB 封装 ⇒ 按瞬时处理（兜底方向）");
    check(classifyIoError({ code: "ENOENT", errno: -4094 }) === "missing", "code 与 errno 打架时以 code 为准 ⇒ 不许把 ENOENT 降级成瞬时");
    // 非 Error 形态的抛出（`throw "字符串"`）也不许把门打崩 —— 这是「整道门崩溃」的另一种形态
    let thrown = null;
    try {
      withIoRetry(() => {
        throw "不是 Error 的抛出";
      }, makeIoNotes());
    } catch (e) {
      thrown = e;
    }
    check(thrown === null, "withIoRetry 对任意抛出值都不得外抛（否则又回到「退非 0 且零判定输出」）", thrown);
    check(classifyIoError(E("ESOMETHING_ELSE", 999)) === "transient", "认不准的错误按瞬时处理 ⇒ 代价只是多试一次，落未检查而非误判");
    check(syncSleepMs(0) === undefined && syncSleepMs(-5) === undefined, "syncSleepMs 对非正值 no-op（预算耗尽时不空转）");
  } finally {
    fs.readFileSync = origReadFileSync;
    fs.readdirSync = origReaddirSync;
    fs.rmSync(tmp, { recursive: true, force: true });
  }
  if (fail > 0) return false;
  console.log(
    "SELFTEST OK（第三层 I/O 兜底）：恒 EPERM / UNKNOWN(errno -4094) / 确定性非 ENOENT ⇒ UNVERIFIED（既不判 OK 也不判 BROKEN）；" +
      "重试成功 ⇒ 判 OK 且不留痕（证明重试真的在跑）；文档 / 目录读不到 ⇒ 该面一条不产出但被登记，篇数与 docsRead 对齐；" +
      "ENOENT 仍判 BROKEN 且零重试（既有判据没被改判也没被拖慢）；LRU 逐出后第二层重读失败 ⇒ 计数自陈；" +
      "退避预算耗尽后停止重试但仍登记；默认只报告、--require-no-unverified 才判红；一篇都没读成功 ⇒ 判门失效（退 2）",
  );
  return true;
}

// ── selftest（第四段，任务 #202 ①②）：活文档 / 快照分面 + 第二层棘轮 ──────
// 独立临时目录：不碰前三段的靶面 ⇒ 既有期望值原样不动。
function runLifecycleSelftest() {
  let fail = 0;
  const check = (cond, msg, extra) => {
    ASSERT_RUN++;
    if (!cond) {
      console.error(`SELFTEST FAIL：${msg}${extra === undefined ? "" : `（实际 ${JSON.stringify(extra)}）`}`);
      fail++;
      ASSERT_FAIL++;
    }
  };
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "doc-refs-lifetest-"));
  try {
    const write = (rel, content) => {
      const abs = path.join(tmp, rel);
      fs.mkdirSync(path.dirname(abs), { recursive: true });
      fs.writeFileSync(abs, content, "utf8");
    };
    write(
      "src/engine.rs",
      [
        "// 头部注释：EngineState 出现在注释里（不该被算作命中）", // 1
        "pub struct EngineState {", // 2
        "    pub tick: u64,", // 3
        "}", // 4
        "", // 5
        "impl EngineState {", // 6
        "    pub fn snapshot_all(&self) -> u64 {", // 7
        "        self.tick", // 8
        "    }", // 9
        "}", // 10
        "", // 11
        "pub fn resolve_gap_width(a: u64, b: u64) -> u64 {", // 12
        "    a.max(b)", // 13
        "}", // 14
        "", // 15
      ].join("\n"),
    );
    // 活文档三篇（其中一篇含**同键两条**漂移，验键去重）
    write(
      "docs/plans/PLAN-thing-2026-09-01.md",
      [
        "见 `src/engine.rs:9` 的 `EngineState`（活文档漂移 A）", // 1 ⇒ 漂移
        "见 `src/engine.rs:13` 的 `EngineState`（活文档漂移 B：同键，去重后只有一个条目）", // 2 ⇒ 漂移
        "见 `src/engine.rs:2` 的 `EngineState`（符号在行 ⇒ 不漂）", // 3 ⇒ 一致
        "",
      ].join("\n"),
    );
    write("docs/audits/README.md", "见 `src/engine.rs:13` 的 `resolve_gap_width`（登记目录里的**非快照形态** ⇒ 活文档）\n");
    write("docs/audits/NOTES.md", "见 `ghost_in_notes.rs:6`（第一层对活文档判 BROKEN）\n");
    // 快照两篇（日期形态 + 时刻后缀形态）
    write(
      "docs/audits/AUDIT-drift-2026-09-13.md",
      ["见 `src/engine.rs:9` 的 `EngineState`（快照面漂移 ⇒ 不订正）", "见 `ghost_in_snapshot.rs:6`（**第一层仍照旧判 BROKEN**）", ""].join("\n"),
    );
    write("docs/audits/AUDIT-l1-blockers-2026-11-05-0944.md", "见 `src/engine.rs:13` 的 `EngineState`（带时刻后缀也是快照）\n");
    // 未登记目录里带日期 ⇒ **仍是活文档**（负控：只认日期形态会把活文档洗成免检）
    write("docs/plans/PLAN-dated-2026-09-01.md", "见 `src/engine.rs:9` 的 `EngineState`（未登记目录 ⇒ 活文档）\n");

    const index = buildBasenameIndex(tmp, SKIP_DIRS);
    const repoRel = (abs) => path.relative(tmp, abs).split(path.sep).join("/");
    const docs = listDocFiles(tmp);
    check(docs.length === 6, "靶面：应收到 6 篇文档（4 篇活文档 + 2 篇快照）", docs.length);
    const { refs, counts } = auditDocs({ root: tmp, docs, index, repoRel });
    const sym = auditSymbolDrift({ refs, windowBelow: 2, lineCache: new Map() });

    // ── docLifecycle 单测（正控 + 负控逐条）──
    check(docLifecycle("docs/audits/AUDIT-drift-2026-09-13.md") === "snapshot", "正控：docs/audits + AUDIT-*-<ISO 日期> ⇒ 快照");
    check(docLifecycle("docs/audits/AUDIT-l1-blockers-2026-11-05-0944.md") === "snapshot", "正控：带 -HHMM 时刻后缀 ⇒ 快照（实测形态）");
    check(docLifecycle("docs\\audits\\AUDIT-win-2026-09-13.md") === "snapshot", "正控：反斜杠路径同样判快照（repoRel 之外的入口也要稳）");
    check(docLifecycle("docs/audits/README.md") === "live", "负控：登记目录里的**无日期形态** ⇒ 活文档（否则目录说明会跟着免检）");
    check(docLifecycle("docs/audits/NOTES.md") === "live", "负控：登记目录里非 AUDIT- 前缀 ⇒ 活文档");
    check(docLifecycle("docs/plans/PLAN-dated-2026-09-01.md") === "live", "负控：**未登记目录**里带日期也判活文档（分类靠登记而非命名猜）");
    check(docLifecycle("docs/INDEX.md") === "live", "正控：docs/INDEX.md ⇒ 活文档");
    check(docLifecycle("AGENTS.md") === "live", "正控：根目录白名单 .md ⇒ 活文档（兜底方向是「照查」）");
    check(SNAPSHOT_DOC_RULES.every((r) => r.dirPrefix.endsWith("/") && r.baseRe && r.basis.length > 10), "登记表每条都必须有规范依据（basis）");

    // ── 分面读数（活文档 4 条漂移 / 快照 2 条；键去重后 3 个条目）──
    check(sym.drift.length === 4, "活文档面漂移应恰为 4 条（PLAN 两条 + README 一条 + PLAN-dated 一条）", sym.drift.length);
    check(sym.driftSnapshot.length === 2, "快照面漂移应恰为 2 条（两篇 AUDIT-*-<日期>）", sym.driftSnapshot.length);
    check(sym.drift.every((r) => r.symFace === "live"), "活文档面每条都标 symFace=live");
    check(sym.driftSnapshot.every((r) => r.symFace === "snapshot"), "快照面每条都标 symFace=snapshot");
    check(
      !sym.drift.some((r) => r.doc.startsWith("docs/audits/AUDIT-")),
      "判据⑥：快照文档的漂移**不得**出现在活文档面（进棘轮 / 进红 / 进建议的那个数组）",
    );
    // 分面自陈计数：两面各自的抽样数必须都能看到（合计 = 全量 ⇒ 「少查了多少」可复算）
    check(sym.face.live.drift === 4 && sym.face.snapshot.drift === 2, "face 计数与两个数组一致", sym.face);
    check(
      sym.face.live.withSymbol + sym.face.snapshot.withSymbol === sym.withSymbol,
      "两面的 withSymbol 合计必须等于总读数（不许有第三条面蒸发掉）",
      [sym.face, sym.withSymbol],
    );

    // ── 第一层口径**未被本批改动**（硬对照：快照里的 BROKEN 照旧红）──
    check(counts.BROKEN === 2, "第一层：快照里的 ghost_in_snapshot.rs 与活文档里的 ghost_in_notes.rs 都仍判 BROKEN", counts.BROKEN);
    check(
      refs.some((r) => r.bucket === "BROKEN" && r.doc === "docs/audits/AUDIT-drift-2026-09-13.md"),
      "第一层不许按文档类别收窄（只有第二层收窄）：快照内的 BROKEN 必须仍在红面",
    );
    check(hardViolations(refs).some((r) => r.bucket === "BROKEN"), "判红面（第一层）仍包含快照里的 BROKEN");

    // ── 第二层棘轮：三个分支 ──
    const keys = new Set(sym.drift.map(symDriftKey));
    check(
      keys.size === 3,
      "同「文档 + 被引文件 + 符号」的两处漂移共用一个条目（键**不含行号**）⇒ 4 条漂移 = 3 个键",
      [...keys],
    );
    check(
      [...keys].every((k) => !/:\d+/.test(k.split(" → ")[1] ?? "")) && [...keys].every((k) => k.includes(" → ")),
      "键形态自证：含 ` → 符号` 且**不含行号**（含行号会让源码位移变成基线外新增）",
    );
    check(
      ![...keys].some((k) => k.startsWith("docs/audits/AUDIT-")),
      "负控：快照键绝不进第二层键面（快照不订正 = 也不冻结）",
      [...keys],
    );
    const rNull = symRatchetDiff(sym.drift, null);
    check(rNull.armed === false && rNull.newViolations.length === 0, "分支一：未冻结（symKeys=null）⇒ 不判红（但 main 必须自陈未武装）", rNull);
    const rIn = symRatchetDiff(sym.drift, keys);
    check(
      rIn.armed === true && rIn.newViolations.length === 0 && rIn.fixedCount === 0,
      "分支二：基线内存量 ⇒ 不判红、也不算「已消失可收紧」",
      rIn,
    );
    const fakeKey = "docs/plans/PLAN-new.md::src/engine.rs → EngineState";
    const rNew = symRatchetDiff([...sym.drift, { doc: "docs/plans/PLAN-new.md", refFile: "src/engine.rs", symDrift: { symbols: ["EngineState"] } }], keys);
    check(
      rNew.newViolations.length === 1 && rNew.newViolations[0].key === fakeKey,
      "分支三：新出现的、不在基线里的漂移 ⇒ 判红（这才是收口）",
      rNew.newViolations.map((v) => v.key),
    );
    // 「拒绝洗白」= 收紧时若存在基线外新增则拒绝：判据就是上面这个 newViolations 非空
    check(rNew.newViolations.length > 0, "收紧拒绝洗白的触发条件与判红同源（同一个数组，不是两套口径）");
    const rTighter = symRatchetDiff(sym.drift.slice(0, 2), keys);
    check(rTighter.newViolations.length === 0 && rTighter.fixedCount === 1, "收紧档：条目消失 ⇒ fixedCount 1（只减，合法）", rTighter);
    // --require-symbol-match 语义：不看基线，整个活文档面判红（含基线内条目）
    check(symbolHardViolations(sym.drift, true).length === 4 && symbolHardViolations(sym.drift, false).length === 0, "全量升硬档仍与基线无关");

    // ── 基线形态：三种状态分得开 ──
    check(parseBaselineShape(null).symKeys === null, "文件不存在 ⇒ symKeys=null（未冻结）");
    check(parseBaselineShape({ entries: ["x"] }).symKeys === null, "只有 entries ⇒ 第二层未冻结（区别于空集）");
    const armedShape = parseBaselineShape({ entries: ["x"], symEntries: [] });
    check(armedShape.symKeys instanceof Set && armedShape.symKeys.size === 0, "symEntries=[] ⇒ **已武装且零存量**（与未冻结必须不同）", armedShape);
    check(armedShape.refKeys.size === 1, "第一层键照常读入（两层互不牵连）");
    check(parseBaselineShape({ symEntries: "乱改的字符串" }).ok === false, "负控：symEntries 形态不符 ⇒ 拒绝（不许静默当空集 = 棘轮失守）");
  } finally {
    fs.rmSync(tmp, { recursive: true, force: true });
  }
  if (fail > 0) return false;
  console.log(
    "SELFTEST OK（第四段 活文档/快照 + 第二层棘轮）：docs/audits + AUDIT-*-<日期> ⇒ 快照、其余一律活文档（含未登记目录里带日期的）；" +
      "第二层只收活文档面，快照面仍计数自陈；第一层口径一字未动（快照里的 BROKEN 照旧红）；" +
      "棘轮键「文档::被引文件 → 符号」**不含行号**且同键去重；未冻结不判红、基线内不判红、基线外新增判红、条目消失算收紧；" +
      "基线形态三态分得开（缺 symEntries ≠ 空数组 ≠ 形态不符）",
  );
  return true;
}

// ── selftest（第五段，任务 #202 ③）：自动回写的建议 / 验证 / 精确率闸 / 写盘闸 ──
// ⚠ 本段**只在临时目录里写盘**，绝不碰真实 `docs/**`；`--fix` 的 main 路径也仅此一段自检
//   作为前置闸（跑不过 ⇒ 退 2 且一行不写）。
function runSymFixSelftest() {
  let fail = 0;
  const check = (cond, msg, extra) => {
    ASSERT_RUN++;
    if (!cond) {
      console.error(`SELFTEST FAIL：${msg}${extra === undefined ? "" : `（实际 ${JSON.stringify(extra)}）`}`);
      fail++;
      ASSERT_FAIL++;
    }
  };
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "doc-refs-fixtest-"));
  const stats = { emitted: 0, correct: 0, emittedOnAmbiguous: 0 };
  // 语料：标注**期望订正行**（`label` 数组），正控 = 该出建议且出对；
  //       负控 = 该判歧义、一条建议都不许出。
  const LABELS = {
    "docs/fixes.md:1": { kind: "ok", to: 2 },
    "docs/fixes.md:2": { kind: "ambiguous" },
    "docs/fixes.md:3": { kind: "ambiguous" },
    "docs/fixes.md:4": { kind: "ambiguous" },
    "docs/fixes.md:5": { kind: "ambiguous" },
    "docs/fixes.md:6": { kind: "ok", to: 2 },
    "docs/audits/AUDIT-fixer-2026-09-13.md:1": { kind: "snapshot" },
  };
  try {
    const write = (rel, content) => {
      const abs = path.join(tmp, rel);
      fs.mkdirSync(path.dirname(abs), { recursive: true });
      fs.writeFileSync(abs, content, "utf8");
    };
    write(
      "src/fixme.rs",
      [
        "// FixmeStruct 的说明（注释里出现 ⇒ 不算定义）", // 1
        "pub struct FixmeStruct {", // 2  ← 唯一定义行
        "    pub tick: u64,", // 3
        "}", // 4
        "impl FixmeStruct {", // 5     ← impl 块不算定义（词表里刻意没有 impl）
        "    pub fn fixme_run(&self) -> u64 { self.tick }", // 6
        "}", // 7
        "pub fn trailing_helper() {}", // 8 ← 保证 `:8` 仍是**文件内**行号（越界 ⇒ 第一层判 STALE ⇒ 第二层根本不跑）
        "", // （尾空行，readLines 会 pop）
      ].join("\n"),
    );
    write(
      "src/dup.rs",
      [
        "pub fn dup_thing(a: u64) -> u64 { a }", // 1 ← 两处同名定义 ⇒ 歧义
        "pub fn other_one() {}", // 2
        "pub fn other_two() {}", // 3
        "pub fn dup_thing(b: u64) -> u64 { b }", // 4
        "pub fn other_three() {}", // 5
        "pub fn other_four() {}", // 6  ← 引用指在这里
        "",
      ].join("\n"),
    );
    write(
      "src/only.rs",
      [
        "pub fn host_a() { only_helper(1); }", // 1 调用点
        "pub fn host_b() { only_helper(2); }", // 2
        "pub fn host_c() { only_helper(3); }", // 3
        "// only_helper 定义在别的文件", // 4   注释
        "pub fn host_d() {}", // 5
        "pub fn host_e() {}", // 6
        "pub fn host_f() {}", // 7
        "pub fn host_g() {}", // 8  ← 引用指在这里
        "",
      ].join("\n"),
    );
    write(
      "src/ranged.rs",
      [
        "pub struct RangedThing {", // 1
        "    pub tick: u64,", // 2
        "}", // 3
        "pub fn other_a() {}", // 4
        "pub fn other_b() {}", // 5
        "pub fn other_c() {}", // 6
        "pub fn other_d() {}", // 7
        "",
      ].join("\n"),
    );
    write(
      "docs/fixes.md",
      [
        "见 `src/fixme.rs:7` 的 `FixmeStruct`（正控：唯一定义行 ⇒ 建议 2）", // 1 ⇒ ok → 2
        "见 `src/dup.rs:6` 的 `dup_thing`（负控：两处同名定义 ⇒ multi-decl 放弃）", // 2 ⇒ multi-decl
        "见 `src/only.rs:8` 的 `only_helper`（负控：只有调用点与注释 ⇒ no-decl 放弃）", // 3 ⇒ no-decl
        "见 `src/ranged.rs:5-7` 的 `RangedThing`（负控：区间引用 ⇒ range 放弃）", // 4 ⇒ range
        "见 `src/fixme.rs:8` 的 `FixmeStruct`，其 `tick` 字段另说（负控：两个符号 ⇒ multi-symbol）", // 5 ⇒ multi-symbol
        "见 `src/fixme.rs:6` 的 `FixmeStruct`（正控二：另一处漂移，同样该指 2）", // 6 ⇒ ok → 2
        "",
      ].join("\n"),
    );
    write("docs/audits/AUDIT-fixer-2026-09-13.md", "见 `src/fixme.rs:7` 的 `FixmeStruct`（快照 ⇒ 永不进建议面）\n");

    const index = buildBasenameIndex(tmp, SKIP_DIRS);
    const repoRel = (abs) => path.relative(tmp, abs).split(path.sep).join("/");
    const { refs, lineCache } = auditDocs({ root: tmp, docs: listDocFiles(tmp), index, repoRel });
    const sym = auditSymbolDrift({ refs, windowBelow: 2, lineCache });
    check(sym.drift.length === 6, "回写靶面：活文档面应恰有 6 条漂移（快照那条不在内）", sym.drift.length);
    check(sym.driftSnapshot.length === 1, "回写靶面：快照面 1 条（不订正）", sym.driftSnapshot.length);

    const byLine = new Map(sym.drift.map((r) => [`${r.doc}:${r.docLine}`, r]));
    const codes = {};
    for (const [at, label] of Object.entries(LABELS)) {
      const r = byLine.get(at) ?? sym.driftSnapshot.find((x) => `${x.doc}:${x.docLine}` === at);
      check(!!r, `语料缺条目 ${at} ⇒ 标注与靶面不符（自检不能空跑）`);
      if (!r) continue;
      const sug = suggestSymbolFix(r);
      if (label.kind === "ok") {
        check(sug.ok === true, `正控 ${at} 必须出建议（实际 code=${sug.code}：${sug.why}）`);
        if (sug.ok) {
          stats.emitted++;
          if (sug.to === label.to) stats.correct++;
          check(sug.to === label.to, `正控 ${at} 的建议行必须恰为 ${label.to}`, sug.to);
        }
      } else if (label.kind === "snapshot") {
        check(sug.ok === false && sug.code === "snapshot", `负控 ${at}：快照必须判 snapshot 且**不出建议**`, [sug.ok, sug.code]);
      } else {
        check(sug.ok === false, `负控 ${at}：歧义面一条建议都不许出`, sug);
        if (sug.ok) stats.emittedOnAmbiguous++;
        else codes[sug.code] = (codes[sug.code] ?? 0) + 1;
      }
    }
    check(
      codes["multi-decl"] === 1 && codes["no-decl"] === 1 && codes.range === 1 && codes["multi-symbol"] === 1,
      "四类歧义各被抓到一次（放弃原因分档必须对得上语料，否则是「蒙对」不是「判对」）",
      codes,
    );
    // ── 直接调用的正负控：不依赖漂移也能验判据 ──
    const fakeSame = {
      doc: "docs/fixes.md",
      start: 2,
      end: 2,
      symDrift: { symbols: ["FixmeStruct"], occ: new Map([["FixmeStruct", [2]]]), targetLines: ["x", "pub struct FixmeStruct {"] },
    };
    check(suggestSymbolFix(fakeSame).code === "same", "正控：建议 == 现值 ⇒ 放弃（same），不许写一份与现状相同的盘");
    check(suggestSymbolFix({ doc: "docs/fixes.md", start: 1, end: 1 }).code === "not-drift", "负控：非漂移条目不进建议面");
    check(suggestSymbolFix(null).code === "not-drift", "负控：null 输入不崩、判 not-drift");
    check(
      suggestSymbolFix({ doc: "docs/fixes.md", start: 1, end: 1, symDrift: { symbols: ["A"], occ: new Map([["A", [9]]]), targetLines: ["x"] } })
        .code === "no-decl",
      "负控：occ 越界 ⇒ 越界行被**过滤掉** ⇒ 判 no-decl，绝不产出一条越界建议（`verify` 档是防御性兜底，同谓词下正常路径不可达）",
    );
    check(
      suggestSymbolFix({ doc: "docs/fixes.md", start: 1, end: 1, symDrift: { symbols: ["A"], occ: new Map([["A", [1]]]), targetLines: ["pub fn A() {}"] } })
        .code === "same",
      "正控：材料齐备但建议 == 现值 ⇒ 放弃（不许写一份与现状相同的盘）",
    );

    // ── isDeclarationOf：逐形态正负控（回写精确率的地基）──
    const L = {
      struct: "pub struct FixmeStruct {",
      impl: "impl FixmeStruct {",
      fn: "    pub fn fixme_run(&self) -> u64 { self.tick }",
      call: "        fixme_run(1);",
      use: "use crate::fixme_run;",
      pathCall: "    crate::fixme_run(1);",
      comment: "// fixme_run 的说明",
      field: "    pub tick: u64,",
      tsIface: "export interface PluginPagesInfoDto {",
      tsMethod: "  listPluginPages(): void {",
      constLine: "pub const GAP_WIDTH_MAX: u64 = 3;",
      wrappedImpl: "impl NamedThing",
      attrThenDef: "#[derive(Clone)] struct FixmeStruct;",
    };
    check(isDeclarationOf("FixmeStruct", L.struct), "正控：pub struct ⇒ 定义");
    check(isDeclarationOf("fixme_run", "    pub fn fixme_run(&self) -> u64 { self.tick }"), "正控：pub fn ⇒ 定义");
    check(isDeclarationOf("PluginPagesInfoDto", L.tsIface), "正控：export interface ⇒ 定义");
    check(isDeclarationOf("GAP_WIDTH_MAX", L.constLine), "正控：pub const ⇒ 定义");
    check(isDeclarationOf("dup_thing", "pub fn dup_thing(a: u64) -> u64 { a }"), "正控：定义词后紧跟名字 ⇒ 定义（即使同行有调用）");
    check(!isDeclarationOf("FixmeStruct", L.impl), "负控：impl 块**不是**定义处（算进去会订正到实现行）");
    check(!isDeclarationOf("NamedThing", L.wrappedImpl), "负控：`impl Foo` 续行（rustfmt 折行）不算定义 —— 命中由第二层窗口负责，回写不猜");
    check(!isDeclarationOf("fixme_run", L.call), "负控：裸调用点不是定义");
    check(!isDeclarationOf("fixme_run", L.use), "负控：use 导入项不是定义");
    check(!isDeclarationOf("fixme_run", L.pathCall), "负控：crate::path 调用不是定义");
    check(!isDeclarationOf("fixme_run", L.comment), "负控：注释里的名字不是定义");
    check(!isDeclarationOf("tick", L.field), "负控：无定义词的字段 ⇒ 不猜（宁缺毋滥，代价是覆盖率）");
    check(!isDeclarationOf("listPluginPages", L.tsMethod), "负控：TS 方法简写（行首即名字）不算定义 —— 与字段同档，代价同上");
    check(!isDeclarationOf("tick", "self.tick"), "负控：接收者位置不算");
    check(!isDeclarationOf("FixmeStruct", L.attrThenDef), "负控：属性 + 定义挤在同一串里 ⇒ 前导里冒出非前导词（derive）⇒ 放弃");
    check(!isDeclarationOf("FixmeStruct", ""), "负控：空行不算");
    check(!isDeclarationOf("", L.struct), "负控：空符号不算");

    // ── verifyProposedFix：故意写错的行号必须被抓出（「错了能不能被发现」）──
    const fixmeLines = fs.readFileSync(path.join(tmp, "src", "fixme.rs"), "utf8").split(/\r?\n/);
    check(verifyProposedFix("FixmeStruct", fixmeLines, 2) === true, "正控：定义行 2 通过");
    check(verifyProposedFix("FixmeStruct", fixmeLines, 1) === false, "负控：注释行 1 被拒");
    check(verifyProposedFix("FixmeStruct", fixmeLines, 5) === false, "负控：impl 行 5 被拒");
    check(verifyProposedFix("FixmeStruct", fixmeLines, 6) === false, "负控：方法行 6 被拒");
    check(verifyProposedFix("FixmeStruct", fixmeLines, 99) === false, "负控：越界行号被拒");
    check(verifyProposedFix("FixmeStruct", fixmeLines, 0) === false && verifyProposedFix("", fixmeLines, 2) === false, "负控：行号 0 / 空符号被拒");

    // ── patchRefStartNumber + verifyPatchedLine：写盘形态的正负控 ──
    // 列区间一律**由同一个抽取器现算**（手写列号会让负控测到的是「列号写错」而不是判据本身）。
    const refOf = (line, wantLine) => {
      const t = extractRefs(line, DOC_REF_EXTS).filter((x) => x.line === wantLine).pop();
      return { col: t.col, colEnd: t.end, start: t.line, refFile: t.file };
    };
    const docLine = "见 `src/fixme.rs:7` 的 `FixmeStruct`（正控：唯一定义行 ⇒ 建议 2）";
    const toks = extractRefs(docLine, DOC_REF_EXTS);
    check(toks.length === 1, "形态自证：靶行应恰好抽出 1 条引用", toks.length);
    const fakeRef = refOf(docLine, 7);
    const patched = patchRefStartNumber(docLine, fakeRef, 2);
    check(patched.ok === true && patched.text.includes("src/fixme.rs:2"), "正控：把 :7 换成 :2", patched);
    check(
      patched.ok && verifyPatchedLine(docLine, patched.text, fakeRef, 2).ok === true,
      "正控：写前验证通过（前缀/后缀未动 + 抽取器读得出 2）",
      patched.ok ? verifyPatchedLine(docLine, patched.text, fakeRef, 2) : patched,
    );
    check(verifyPatchedLine(docLine, docLine, fakeRef, 2).ok === false, "负控：改后 == 改前 ⇒ 拒绝");
    check(verifyPatchedLine(docLine, docLine.replace("见 `", "见 X`"), fakeRef, 2).ok === false, "负控：引用之前的文本被动 ⇒ 拒绝");
    check(verifyPatchedLine(docLine, patched.text.replace("）", "）多余"), fakeRef, 2).ok === false, "负控：引用之后的文本被动 ⇒ 拒绝");
    check(verifyPatchedLine(docLine, patched.text, fakeRef, 77).ok === false, "负控：验证目标行与文本不符 ⇒ 拒绝");
    check(patchRefStartNumber("见 `src/fixme.rs:7-9` 的说明", refOf("见 `src/fixme.rs:7-9` 的说明", 7), 2).ok === false, "负控：区间尾形态放弃");
    const bareLine = "见 `src/fixme.rs:7` 与 `:8` 两处";
    check(patchRefStartNumber(bareLine, refOf(bareLine, 8), 2).ok === false, "负控：裸 `:行号`（列区间不含文件名）放弃");
    check(patchRefStartNumber(docLine, { ...fakeRef, start: 8 }, 2).ok === false, "负控：token 数字与读数不符 ⇒ 放弃（说明盘已被人改过）");
    check(patchRefStartNumber("散文里没有引用", { col: 0, colEnd: 3, start: 1 }, 2).ok === false, "负控：列区间里不是引用形态 ⇒ 放弃");

    // ── 合成写盘（只写临时目录）：改完复跑第二层，漂移必须消失；写错必须被拒 ──
    const docAbs = path.join(tmp, "docs", "fixes.md");
    const rawBefore = fs.readFileSync(docAbs, "utf8");
    const linesArr = rawBefore.split(/\r?\n/);
    let wrote = 0;
    for (const r of sym.drift) {
      const sug = suggestSymbolFix(r);
      if (!sug.ok) continue;
      const idx = r.docLine - 1;
      const p = patchRefStartNumber(linesArr[idx], r, sug.to);
      if (!p.ok) continue;
      const v = verifyPatchedLine(linesArr[idx], p.text, r, sug.to);
      if (!v.ok) continue;
      linesArr[idx] = p.text;
      wrote++;
    }
    check(wrote === 2, "正控：两条正控都通过写前验证并落盘（其余四条被歧义闸挡住）", wrote);
    fs.writeFileSync(docAbs, linesArr.join("\n"), "utf8");
    const { refs: refs2 } = auditDocs({ root: tmp, docs: listDocFiles(tmp), index, repoRel });
    const sym2 = auditSymbolDrift({ refs: refs2, windowBelow: 2, lineCache: new Map() });
    check(sym2.drift.length === 4, "复跑：写盘后活文档面漂移由 6 降到 4（订正真的生效，不是只算了没改）", sym2.drift.length);
    check(
      refs2.filter((r) => r.bucket === "BROKEN" || r.bucket === "STALE").length === 0,
      "回写不许动第一层：改前改后第一层红面都必须仍是 0",
    );
    const wrong = patchRefStartNumber("见 `src/fixme.rs:7` 的 `FixmeStruct`", fakeRef, 1);
    check(
      wrong.ok && verifyProposedFix("FixmeStruct", fixmeLines, 1) === false,
      "负控：若把正控改成注释行 1，写前验证必须拒（证明闸有牙，不是装饰）",
    );

    // ── 精确率闸与写盘闸 ──
    check(stats.emitted === 2 && stats.correct === 2 && stats.emittedOnAmbiguous === 0, "标注语料实测：2 出 2 对、歧义面零建议", stats);
    const gate = fixPrecisionGate(stats);
    check(gate.ok === true, "正控：精确率闸放行", gate);
    check(fixPrecisionGate({ emitted: 0, correct: 0, emittedOnAmbiguous: 0 }).ok === false, "负控：无分母 ⇒ 拒绝（空洞通过）");
    check(fixPrecisionGate({ emitted: 2, correct: 1, emittedOnAmbiguous: 0 }).ok === false, "负控：错一条 ⇒ 拒绝（要求 100%）");
    check(fixPrecisionGate({ emitted: 2, correct: 2, emittedOnAmbiguous: 1 }).ok === false, "负控：歧义面上还敢出建议 ⇒ 拒绝");
    check(FIX_PRECISION_REQUIRED === 1, "精确率要求必须是 100%（改这个数就等于放宽判据，需要显式裁定）");
    const okCall = {
      fix: true,
      attested: true,
      selftestRan: true,
      selftestAsserts: 30,
      selftestPassed: true,
      precisionOk: true,
      precisionWhy: "x",
      unverifiedAny: false,
      armed: true,
    };
    check(writeModeGate(okCall).ok === true, "正控：六道闸全过才允许写盘");
    check(writeModeGate({ ...okCall, fix: false }).ok === false, "负控：没给 --fix ⇒ 不写");
    check(writeModeGate({ ...okCall, attested: false }).ok === false, "负控：只给 --fix 不给 --i-attest-quiet-window ⇒ 拒绝（并行期保护）");
    check(writeModeGate({ ...okCall, selftestRan: false }).ok === false, "负控：无自检覆盖 ⇒ 拒绝执行（任务硬要求）");
    check(writeModeGate({ ...okCall, selftestAsserts: 0 }).ok === false, "负控：自检跑了但零断言 ⇒ 等于没覆盖 ⇒ 拒绝");
    check(writeModeGate({ ...okCall, selftestPassed: false }).ok === false, "负控：自检有失败 ⇒ 拒绝");
    check(writeModeGate({ ...okCall, precisionOk: false }).ok === false, "负控：精确率闸未过 ⇒ 拒绝");
    check(writeModeGate({ ...okCall, armed: false }).ok === false, "负控：第二层未冻结 ⇒ 改错无从对照 ⇒ 拒绝写盘");
    check(writeModeGate({ ...okCall, unverifiedAny: true }).ok === false, "负控：本轮有未检查面 ⇒ 拒绝写盘");
    check(writeModeGate({ ...okCall, fix: false }).code === 2, "拒绝一律退 2（门自身没干活，不是文档缺陷）");
  } finally {
    fs.rmSync(tmp, { recursive: true, force: true });
  }
  const precision = fixPrecisionGate(stats);
  if (fail > 0) return { passed: false, precision };
  console.log(
    `SELFTEST OK（第五段 回写判据）：正控 2 条（唯一 pub struct 定义行 ⇒ 建议 2，实测精确率 ${(precision.precision * 100).toFixed(0)}%）；` +
      "负控 4 类歧义（multi-decl / no-decl / range / multi-symbol）+ 快照 ⇒ 一条建议都不出；" +
      "impl 块 / 调用点 / use / 注释 / 字段 / TS 方法简写 / 接收者 全部不算定义行；写前验证对「故意写错的行号」返回 false；" +
      "合成写盘只落临时目录，改后复跑活文档面 6→4 且第一层红面不动；" +
      "writeModeGate 八个负控（缺开关 / 缺安静窗口确认 / 无自检覆盖 / 零断言 / 自检失败 / 精确率不足 / 有未检查面 / 退码档）全部拒绝",
  );
  return { passed: true, precision };
}

// ── main ────────────────────────────────────────────────────────────
if (SELFTEST) {
  // ⚠ 五段**都要跑**再合并结果（原式 `runSelftest() && runSymbolSelftest()` 短路：第一段红就
  // 看不到后两段的失败）。断言条数按**执行**口径打印，并**逐段拆分** —— 自证要求
  // 「原 N + 补 M = 总 K」必须能被第三方逐字核对，只打印「OK」或只打印总数都核不了。
  const segA = ASSERT_RUN;
  const r1 = runSelftest();
  const segB = ASSERT_RUN;
  const r2 = runSymbolSelftest();
  const segC = ASSERT_RUN;
  const r3 = runIoSelftest();
  const segD = ASSERT_RUN;
  const r4 = runLifecycleSelftest();
  const segE = ASSERT_RUN;
  const r5 = runSymFixSelftest();
  console.log(
    `SELFTEST 断言合计 ${ASSERT_RUN} 条（执行口径，含循环展开）= 基础判据 ${segB - segA}` +
      ` + 第二层 SYMDRIFT ${segC - segB} + 第三层 I/O 兜底 ${segD - segC}` +
      ` + 第四层 活文档/快照+第二层棘轮 ${segE - segD} + 第五层 回写判据 ${ASSERT_RUN - segE}；失败 ${ASSERT_FAIL} 条`,
  );
  process.exit(r1 && r2 && r3 && r4 && r5.passed ? 0 : 1);
}

const SYM_WINDOW = parseSymWindowFlag(WINDOW_FLAG);
if (SYM_WINDOW === null) {
  console.error(`✖ 参数非法：${JSON.stringify(WINDOW_FLAG)} —— 只接受 --sym-window=<0..50>（第二层判据的折行窗口）`);
  process.exit(2);
}

const repoFiles = listRepoFiles(ROOT, SKIP_DIRS);
const index = indexFromFileList(repoFiles.files);
// 读盘失败登记表：**目录遍历 + 文档正文 + 目标正文 + 第二层重读**共用同一份 ⇒ 退避预算是
// 全局的（见 `IO_RETRY_BUDGET_MS` 的理由），未检查面也只需在一处打印。
const ioNotes = makeIoNotes();
const docs = listDocFiles(ROOT, ioNotes);
if (docs.length === 0) {
  console.error("✖ 扫描面为空 —— docs/**/*.md 一篇都没收到（路径约定变了？），不是「没有引用」");
  process.exit(2);
}
const repoRel = (abs) => path.relative(ROOT, abs).split(path.sep).join("/");
const resolveRef = makeFileSetResolver(ROOT, repoFiles.files);
const { refs, counts, docsRead, lineCache } = auditDocs({ root: ROOT, docs, index, repoRel, resolve: resolveRef, notes: ioNotes });
// 「收到 N 篇、0 篇读成功」= 本门一行判定都没产出 ⇒ 与「扫描面 0 篇」同档（退 2 自陈）。
// ⚠ 不是把盘的抖动判红：部分读不到仍走「未检查面 + 默认只报告」；这里拦的是**全盲**。
if (scanProducedNothing(docs.length, docsRead)) {
  console.error(
    `✖ 扫描面 ${docs.length} 篇文档，正文读取成功 ${docsRead} 篇 —— 一条引用都没被检查过，` +
      `这不是「没有违规」而是「本门没跑」（读盘失败样例：${ioNotes.failures.slice(0, 3).map((f) => `${f.kind} ${f.at} ${f.err}`).join(" ｜ ") || "无"}）`,
  );
  process.exit(2);
}
const hard = hardViolations(refs);
// 第二层（SYMDRIFT）：复用同一份正文缓存；**不改桶、不改第一层键**，只往引用对象上挂
// symDrift / symAbsent / symFace（判据⑥ 的面归属）/ occ + targetLines（回写建议的材料）。
const sym = auditSymbolDrift({ refs, windowBelow: SYM_WINDOW, lineCache, includeDotted: INCLUDE_DOTTED, notes: ioNotes });
// ⚠ `symbolHardViolations` 保留为**纯函数档**（#182 的「全量升硬」语义，selftest 钉着），
//   main 侧的判红改由 `symViolations` 统一给出（开关档 / 棘轮档 / 未武装三态，见下方退出码块）。

const BASELINE_NOTE =
  "棘轮基线：键 = <文档>::<引用串>:<起始行>[-<结束行>]（不含文档行号，防漂移）。" +
  "只许减少不许增加；新增 BROKEN/STALE 必须改文档引用，不许改基线洗白。";
const SYM_BASELINE_NOTE =
  "第二层（SYMDRIFT）棘轮基线：键 = <文档>::<被引文件> → <被引符号>（**不含行号** —— 本层的病灶就是行号，" +
  "含行号会让「源码里符号挪了一行」变成基线外新增）。只收**活文档**（判据⑥：docs/audits/AUDIT-*-<日期>.md " +
  "是时点快照，永不订正、永不进本表）。只许减少不许增加；新增 = 该活文档新出现「符号不在所指行」，改文档或改引用。";
/**
 * 基线读取（**一次读、两层解**）。原 `loadBaselineKeys` 只回 `entries`，
 * 现改回 `parseBaselineShape` 的产物：`symKeys === null` ⇒ 第二层从未冻结（≠ 空集）。
 * 基线 JSON 语法错 / `symEntries` 形态不符 ⇒ 退 2（不可读 = 门自身失效，与既有裁定同档）。
 */
function loadBaseline() {
  if (!fs.existsSync(BASELINE_PATH)) return { refKeys: new Set(), symKeys: null, exists: false };
  let raw;
  try {
    raw = JSON.parse(fs.readFileSync(BASELINE_PATH, "utf8"));
  } catch (e) {
    console.error(`✖ 基线不可读（${BASELINE_PATH}）：${e.message}`);
    process.exit(2);
  }
  const shape = parseBaselineShape(raw);
  if (!shape.ok) {
    console.error(`✖ 基线形态不符（${BASELINE_PATH}）：${shape.why} —— 拒绝按「空第二层」猜测，那会让棘轮静默失守`);
    process.exit(2);
  }
  return { refKeys: shape.refKeys, symKeys: shape.symKeys, exists: true };
}
/**
 * 写基线。**两层分开传**，且 `symKeys === undefined` 表示「本轮不碰第二层」（保持缺失，
 * 由调用方走「未冻结」自陈分支）；传数组 ⇒ 写入。`entries` 永不被第二层的读写牵连。
 */
function writeBaseline(keys, symKeyList) {
  const countsNow = {};
  for (const r of refs) if (r.bucket === "BROKEN" || r.bucket === "STALE") countsNow[r.bucket] = (countsNow[r.bucket] ?? 0) + 1;
  const obj = { note: BASELINE_NOTE, version: 2, counts: countsNow, entries: [...keys].sort() };
  if (symKeyList !== undefined) {
    obj.symNote = SYM_BASELINE_NOTE;
    obj.symCounts = {
      live: sym.drift.length,
      snapshot: sym.driftSnapshot.length,
      keys: new Set(sym.drift.map(symDriftKey)).size,
    };
    obj.symEntries = [...symKeyList].sort();
  }
  fs.writeFileSync(BASELINE_PATH, JSON.stringify(obj, null, 2) + "\n", "utf8");
}

// 未检查面（读盘失败）的覆盖面摘要 —— **无条件进 summary**，因为「这条门有没有真的跑完」
// 本身就是读数的一部分：只在出错时才打印的自陈，等于让正常路径与全盲路径长得一样。
const cov = ioCoverageSummary(ioNotes, counts.UNVERIFIED ?? 0);

// 基线（两层一次读）+ 第二层棘轮比对。
// ⚠ `base.symKeys === null` = 第二层**从未冻结** ⇒ 棘轮未武装 ⇒ 不判红，但必须自陈（见 summary）。
const base = loadBaseline();
const symRatchet = symRatchetDiff(sym.drift, base.symKeys, REQUIRE_SYMBOL_MATCH);

const summary =
  `扫描 ${docs.length} 篇文档 · 引用 ${refs.length} 条` +
  ` ｜ OK ${counts.OK ?? 0} ｜ NONLOC（未检查）${counts.NONLOC ?? 0} ｜ BROKEN ${counts.BROKEN ?? 0}` +
  ` ｜ ABBREV ${counts.ABBREV ?? 0} ｜ STALE ${counts.STALE ?? 0}` +
  ` ｜ EXTERNAL ${counts.EXTERNAL ?? 0} ｜ EPHEMERAL ${counts.EPHEMERAL ?? 0}` +
  ` ｜ MALFORMED ${counts.MALFORMED ?? 0}` +
  // 第二层是**并列独立计数**（上面各桶的读数与它无关）：给出可比对符号的引用里，
  // 符号不在所指行（±窗口）的条数。窗口 0 行时上面这些数字不变，只有这个数会变。
  // ⚠ 按判据⑥ **分两面**报：活文档面（棘轮 / 升硬靶面）与快照面（永不订正）——
  //   只报合计会让「收窄了多少靶面」不可复算（判据 #7：未检查与检查通过长得一样）。
  ` ｜ SYMDRIFT 活文档面 ${sym.drift.length}` +
  `（第二层·符号不在所指行，${REQUIRE_SYMBOL_MATCH ? "**--require-symbol-match 判红**" : symRatchet.armed ? (symRatchet.newViolations.length > 0 ? "**基线外新增 ${symRatchet.newViolations.length} 条判红**" : "棘轮已武装·无新增") : "**棘轮未武装**（基线无 symEntries ⇒ 只报告）"}；` +
  `可比对符号 ${sym.face.live.withSymbol} 条 / 进入比对 ${sym.face.live.checked} 条 / 符号零存在跳过 ${sym.face.live.absent} 条` +
  `；漂移按形态 ${Object.entries(sym.byForm).map(([k, v]) => `${k} ${v}`).join(" / ") || "无"}）` +
  ` / 快照面 ${sym.driftSnapshot.length}（docs/audits 时点快照·**不订正、不进棘轮、永不判红**；` +
  `可比对 ${sym.face.snapshot.withSymbol} / 比对 ${sym.face.snapshot.checked} / 零存在 ${sym.face.snapshot.absent}）` +
  ` ｜ 点号接缝名排除 ${sym.dottedSkipped} 条 / 窗口 +${sym.windowBelow} 行` +
  // 第三层（任务 #203）：读盘失败 = **未检查**，与上面任何桶都不混同，也不改它们的读数。
  // 四个来源分档给数：引用条数（判进 UNVERIFIED 桶）／目标文件个数／文档篇数／文档目录个数。
  // ⚠ 后面三项为 0 时也照样打印 —— 「没读到」这件事必须永远可见。
  ` ｜ UNVERIFIED（未检查·读盘失败）${cov.refs} 条引用 / ${cov.targets} 个目标 / ${cov.docs} 篇文档 / ${cov.dirs} 个目录` +
  `（重试 ${cov.retries} 次 · 退避 ${cov.retryMs}ms${cov.exhausted ? " · **预算耗尽后已停止重试**" : ""}` +
  ` · ${REQUIRE_NO_UNVERIFIED ? "**判红**" : "只报告"}）` +
  (sym.unreadableTargets > 0 ? `（其中第二层重读失败 ${sym.unreadableTargets} 条未比对）` : "");

if (DUMP) {
  for (const r of refs) {
    const at = `${r.doc}:${r.docLine}`;
    const ref = r.end !== r.start ? `${r.refFile}:${r.start}-${r.end}` : `${r.refFile}:${r.start}`;
    let detail = "";
    if (r.bucket === "OK") detail = `→ ${r.target}`;
    else if (r.bucket === "STALE") detail = `→ ${r.target}（共 ${r.lines} 行，行号越界）`;
    else if (r.bucket === "ABBREV") detail = `→ 疑似${r.kind}（候选，需人判）：${r.candidates.join(" / ")}`;
    else if (r.bucket === "BROKEN") detail = "→ 无候选（仓内不存在该文件）";
    else if (r.bucket === "NONLOC") detail = `→ ${r.why}`;
    else if (r.bucket === "EXTERNAL") detail = "→ 跨仓 / 绝对路径（免检，只报告）";
    else if (r.bucket === "EPHEMERAL") detail = "→ 环境相关目录（免检，只报告）";
    else if (r.bucket === "MALFORMED") detail = "→ 不是路径形态（散文 / 表格残片，已丢弃）";
    // UNVERIFIED 逐条点名（读盘失败的目标 + 错误码 + 尝试次数）——每条都要能指回是哪一次读。
    // ⚠ 输出行的**字段顺序 / 分隔与既有各桶逐字相同**（`文档:行  引用串  桶  详情`）；
    //   桶名 `UNVERIFIED` 比既有最长桶名（EPHEMERAL / MALFORMED，9 字符）宽 1，
    //   故这一列的 `padEnd(9)` 对它不生效 —— **刻意不改 padEnd**：改了会让既有 8 类桶的输出
    //   整体位移，而 `--dump-doc-refs` 的形态有别的文档在抄（见 AGENTS.md「段对齐」一节）。
    else if (r.bucket === "UNVERIFIED") detail = `→ ${r.target} ${r.why} ⇒ **未检查**（不是通过，也不是引用坏了）`;
    // 第二层附注（不改上面任何桶的读数）：符号一致 / 符号漂移 / 符号零存在（未检）/ 仅接缝名（未检）
    // ⚠ 漂移条目必须**自陈属于哪个面**（判据⑥）：否则「活文档面 N / 快照面 M」这个读数
    //   只能靠 summary 的合计去反推，逐条复算就没了依据（本仓「合计对不上明细」的反复形态）。
    if (r.symDrift) {
      detail +=
        ` ⇒ SYMDRIFT【${r.symFace === "snapshot" ? "快照面·不订正" : "活文档面·进棘轮"}】簇 \`${r.symDrift.spans.join("` / `")}\`` +
        `（形态 ${r.symDrift.forms.join("/")}）` +
        `的符号 ${r.symDrift.symbols.join("/")} 不在 ${r.symDrift.window.join("-")} 行` +
        `（实际 ${r.symDrift.actual.join("/")}`;
      detail += r.symDrift.symbols.length > 1 ? `；候选共 ${r.symDrift.symbols.length} 个）` : "）";
      if (r.symFace !== "snapshot") {
        const sug = suggestSymbolFix(r);
        detail += sug.ok ? ` ⇒ 回写建议 :${r.start}→:${sug.to}（${sug.sym} 的声明行）` : ` ⇒ 回写放弃（${sug.code}）`;
      }
    } else if (r.symAbsent) detail += ` ⇒ 符号零存在（未检，属重命名/口语那道题）`;
    else if (r.symDottedOnly) detail += ` ⇒ 仅点号接缝名（未检，见 comparableSymbolSpan 注释）`;
    else if (r.symCandidates && r.symCandidates.length > 0 && r.bucket === "OK") detail += ` ⇒ 符号一致`;
    console.log(`${at}  ${ref.padEnd(64)} ${r.bucket.padEnd(9)} ${detail}`);
  }
  console.log(`\n${summary}`);
  process.exit(0);
}

const { newViolations, fixedCount, baseSize } = diffAgainstBaseline(base.refKeys, hard);
// 第二层棘轮的违规：`--require-symbol-match` 时整个活文档面（不看基线），
// 否则「基线外新增的活文档面键」（未武装 ⇒ 恒空，但 summary 会自陈「未武装」）。
const symViolations = REQUIRE_SYMBOL_MATCH ? symRatchet.current : symRatchet.newViolations;

// 未检查面非空 ⇒ **无条件拒绝**动基线（判据见 `refuseBaselineWrite`）：读不到的引用会让
// 本轮读数变小，而基线收紧正是拿「本轮没有」当「已修复」—— 盘的抖动不该代劳洗白。
// ⚠ 这条闸对**两层同时**生效（任务 #202 ②：第二层沿用既有判据，不另开口径）。
if ((INIT_BASELINE || UPDATE_BASELINE) && refuseBaselineWrite(cov)) {
  console.error(
    `\n✖ ${INIT_BASELINE ? "--init-baseline" : "--update-baseline"} 拒绝执行：本轮有未检查面` +
      `（引用 ${cov.refs} 条 / 目标 ${cov.targets} 个 / 文档 ${cov.docs} 篇 / 目录 ${cov.dirs} 个读盘失败）。` +
      `读不到的引用**不会出现在本轮读数里**，基线会把它们误当「已修复」而收紧 —— 那是洗白，与盘的抖动同源的风险。` +
      `请在挂载 / 杀软 / 索引器稳定后重跑（常规巡检不受本条影响：不带这两个开关时照旧只报告）。`,
  );
  process.exit(2);
}

if (INIT_BASELINE) {
  const symKeysNow = [...new Set(sym.drift.map(symDriftKey))];
  if (base.exists && base.symKeys !== null) {
    console.error(
      `✖ --init-baseline 拒绝执行：${BASELINE_PATH} 已有 symEntries（${base.symKeys.size} 条）。` +
        `收紧存量请用 --update-baseline，不要重建第二层基线洗白。`,
    );
    process.exit(1);
  }
  if (base.exists) {
    // 只补第二层首冻：`entries` **逐字保留**（重建第一层 = 洗白，正是本模式要防的）。
    const preserved = base.refKeys;
    writeBaseline(preserved, symKeysNow);
    console.log(`\n${summary}`);
    console.log(
      `第二层基线已首冻：活文档面 ${sym.drift.length} 条漂移 ⇒ ${symKeysNow.length} 个键（快照面 ${sym.driftSnapshot.length} 条**不进本表**）。\n` +
        `第一层 entries 未动（${preserved.size} 条逐字保留）。之后两层都只减不增。`,
    );
    process.exit(0);
  }
  writeBaseline(new Set(hard.map((r) => r.key)), symKeysNow);
  console.log(`\n${summary}`);
  console.log(
    `基线已建立：第一层冻结 BROKEN/STALE 存量 ${hard.length} 条；` +
      `第二层冻结活文档面 ${symKeysNow.length} 个键（快照面 ${sym.driftSnapshot.length} 条不订正）。之后只减不增。`,
  );
  process.exit(0);
}

if (UPDATE_BASELINE) {
  // 「为凑绿往基线里塞新条目」的形态在这一层同样存在 ⇒ 两个面一起拦，缺一个就是洗白口子。
  const bad1 = newViolations.length > 0;
  const bad2 = symRatchet.newViolations.length > 0;
  if (bad1 || bad2) {
    if (bad1) {
      console.error(`\n✖ --update-baseline 拒绝执行：第一层有 ${newViolations.length} 条基线外新增，先修引用（不许用基线洗白）：`);
      for (const v of newViolations.slice(0, 20)) console.error(`  + ${v.key}（${v.doc}:${v.docLine}）`);
    }
    if (bad2) {
      console.error(
        `\n✖ --update-baseline 拒绝执行：第二层（SYMDRIFT 活文档面）有 ${symRatchet.newViolations.length} 条基线外新增` +
          `（收紧只允许条目消失，塞新条目 = 洗白）：`,
      );
      for (const v of symRatchet.newViolations.slice(0, 20)) console.error(`  + ${v.key}（${v.ref.doc}:${v.ref.docLine}）`);
    }
    process.exit(1);
  }
  if (base.symKeys === null) {
    console.error(
      `\n⚠ 第二层从未冻结（基线无 symEntries）⇒ 本次 --update-baseline **只收紧第一层**，` +
        `symEntries 保持缺失（首冻请用 --init-baseline，且必须等安静窗口：并行期首冻会把别人的未提交中间态钉进棘轮）。`,
    );
    writeBaseline(new Set(hard.map((r) => r.key)));
    console.log(`\n${summary}`);
    console.log(`第一层基线已收紧：${baseSize} → ${hard.length} 条（本轮修复 ${fixedCount} 条）。`);
    process.exit(0);
  }
  const symKeysNow = [...new Set(sym.drift.map(symDriftKey))];
  writeBaseline(new Set(hard.map((r) => r.key)), symKeysNow);
  console.log(`\n${summary}`);
  console.log(
    `基线已收紧：第一层 ${baseSize} → ${hard.length} 条（修复 ${fixedCount} 条）；` +
      `第二层 ${base.symKeys.size} → ${symKeysNow.length} 条（修复 ${symRatchet.fixedCount} 条）。`,
  );
  process.exit(0);
}

console.log(`\n${summary}`);
console.log(`目标索引来源：${repoFiles.source === "git" ? "git ls-files（已跟踪 + 未忽略的新文件）" : "文件系统遍历（未找到 git，口径可能与 CI 不同）"}`);
if (baseSize > 0) console.log(`第一层棘轮基线 ${baseSize} 条；本轮已消失 ${fixedCount} 条（可择机 --update-baseline 收紧）。`);
console.log(
  base.symKeys === null
    ? `第二层棘轮**未武装**（基线无 symEntries）⇒ 活文档面 ${sym.drift.length} 条漂移本轮只报告。` +
        `安静窗口首冻：node scripts/check-doc-refs.mjs --init-baseline（并行期禁止：会把别的 lane 的未提交中间态钉进棘轮）`
    : `第二层棘轮基线 ${base.symKeys.size} 条（活文档面键 ${symRatchet.keyCount} 个）；本轮已消失 ${symRatchet.fixedCount} 条（可择机收紧）。`,
);

// ABBREV 逐条自证（报告面）：每条都必须点到它缩写的真实文件，找不到候选的不许进本桶
const abbrev = refs.filter((r) => r.bucket === "ABBREV");
if (abbrev.length > 0) {
  console.log(`\n~ ABBREV ${abbrev.length} 条（段内截断 / 同义词，**只报告**；按 AGENTS.md 改为段对齐写法即可消除）：`);
  for (const r of abbrev.slice(0, 50)) {
    const ref = r.end !== r.start ? `${r.refFile}:${r.start}-${r.end}` : `${r.refFile}:${r.start}`;
    console.log(`   ~ ${r.doc}:${r.docLine} → ${ref}（疑似${r.kind}）：${r.candidates.join(" / ")}`);
  }
  if (abbrev.length > 50) console.log(`   … 其余 ${abbrev.length - 50} 条省略（--dump-doc-refs 看全量）`);
}
const nonloc = refs.filter((r) => r.bucket === "NONLOC");
if (nonloc.length > 0) {
  console.log(`\nℹ NONLOC ${nonloc.length} 条**未检查**（裸同名文件无法唯一定位；补上路径才进检查）：`);
  for (const r of nonloc.slice(0, 10)) console.log(`   ℹ ${r.doc}:${r.docLine} → ${r.refFile}:${r.start}`);
  if (nonloc.length > 10) console.log(`   … 其余 ${nonloc.length - 10} 条省略`);
}

// ── 未检查面（读盘失败）报告 ───────────────────────────────────────
// 三个来源**分档打印**（引用 / 目标文件 / 文档 / 目录），样例至少若干条，逐条带错误码与
// 尝试次数 —— 「有一条没查」必须指得回是哪一条，否则这个桶和没有一样（AGENTS.md 判据 #7）。
const unverifiedRefs = refs.filter((r) => r.bucket === "UNVERIFIED");
if (cov.any) {
  console.log(
    `\n⚠ UNVERIFIED 未检查面（SMB / VMware 共享目录读盘失败，**不是**文档缺陷，也**不是**检查通过）：` +
      `\n   引用 ${cov.refs} 条 ｜ 目标文件 ${cov.targets} 个 ｜ 文档 ${cov.docs} 篇 ｜ 文档目录 ${cov.dirs} 个` +
      `\n   重试 ${cov.retries} 次（每次上限 ${IO_RETRY_ATTEMPTS} 尝试、退避 ${IO_RETRY_BASE_MS}ms→${IO_RETRY_BASE_MS * IO_RETRY_FACTOR}ms）` +
      `· 累计退避 ${cov.retryMs}ms${cov.exhausted ? " ⇒ **预算已耗尽，后续读错不再重试**（盘的抖动升级为整块失联的信号）" : ""}` +
      `\n   ${REQUIRE_NO_UNVERIFIED ? "本桶**已判红**（--require-no-unverified）" : `本桶默认**只报告**（不影响退出码）；要判红请加 --require-no-unverified`}`,
  );
  for (const f of ioNotes.failures.slice(0, 12)) {
    const kindLabel = f.kind === "target" ? "目标" : f.kind === "doc" ? "文档" : "目录";
    console.log(`   ⚠ [${kindLabel}] ${f.at} —— ${f.err}，尝试 ${f.attempts} 次${f.sleptMs ? `（退避 ${f.sleptMs}ms）` : ""}`);
  }
  if (ioNotes.failures.length > 12) console.log(`   … 其余 ${ioNotes.failures.length - 12} 个读盘失败点省略（--dump-doc-refs 看逐条引用）`);
  if (unverifiedRefs.length > 0) {
    console.log(`   其中判进 UNVERIFIED 桶的引用（这些条的**行号**都没比过）：`);
    for (const r of unverifiedRefs.slice(0, 10)) {
      const ref = r.end !== r.start ? `${r.refFile}:${r.start}-${r.end}` : `${r.refFile}:${r.start}`;
      console.log(`      ⚠ ${r.doc}:${r.docLine} → ${ref} —— ${r.why}`);
    }
    if (unverifiedRefs.length > 10) console.log(`      … 其余 ${unverifiedRefs.length - 10} 条省略`);
  }
  console.log(
    `   处置：盘的瞬时抖动无需改文档；**同一批失败点反复出现**才是真问题 —— 先查挂载 / 杀软 /` +
      ` 索引器，再考虑重跑本门。未检查的条数不影响上面各桶的读数（那些是**真读过**的）。`,
  );
}
// ── 第二层（SYMDRIFT）报告面 ───────────────────────────────────────
// 语义与 ABBREV 同档：**默认只报告**，不影响退出码。判红有两条独立路径（见 `symViolations`）：
//   · `--require-symbol-match` ⇒ 整个活文档面判红（#182 的全量对账档，不看基线）；
//   · 基线已首冻（有 `symEntries`）⇒ 只有**基线外新增**判红（棘轮档）。
// 逐条必须点到「符号实际在第几行」，否则这条判据指不回真实风险（同 ABBREV 的自证要求）。
if (sym.drift.length > 0) {
  console.log(
    `\n~ SYMDRIFT 活文档面 ${sym.drift.length} 条（键面去重后 ${symRatchet.keyCount} 个条目）：引用可定位、但**所指那一行不是句中所述符号**，` +
      `行号仍在范围内 ⇒ 既有 BROKEN/STALE 全绿的那种腐烂。` +
      `${REQUIRE_SYMBOL_MATCH ? "本档**已判红**（--require-symbol-match）。" : symRatchet.armed ? `棘轮已武装 ⇒ 基线外新增 ${symRatchet.newViolations.length} 条判红。` : "棘轮**未武装**（基线无 symEntries）⇒ 本轮只报告。"}`,
  );
  for (const r of sym.drift.slice(0, 30)) {
    const ref = r.end !== r.start ? `${r.refFile}:${r.start}-${r.end}` : `${r.refFile}:${r.start}`;
    console.log(
      `   ~ ${r.doc}:${r.docLine} → ${ref} 簇 \`${r.symDrift.spans.join("` / `")}\`` +
        `（形态 ${r.symDrift.forms.join("/")}）的符号 \`${r.symDrift.symbols.join("` / `")}\`` +
        ` 不在 ${r.symDrift.window[0]}..${r.symDrift.window[1]} 行（实际 ${r.symDrift.actual.join(" / ")}）`,
    );
  }
  if (sym.drift.length > 30) console.log(`   … 其余 ${sym.drift.length - 30} 条省略（--dump-doc-refs 看全量）`);
  console.log(
    `   修法：把行号订正到「实际」那一行（或把符号名改成所指行上的真符号）。\n` +
      `   复核窗口：--sym-window=N 放大折行窗口，可分辨「折行没吃进」与「真漂移」。\n` +
      `   建议清单（零写盘）：--suggest-fix；首冻棘轮：--init-baseline（**必须等安静窗口**）。`,
  );
}
// 快照面：**永不订正**，但必须让「免检了多少」可复算（判据 #7）—— 只报告、不进棘轮、不进建议。
if (sym.driftSnapshot.length > 0) {
  console.log(
    `\nℹ SYMDRIFT 快照面 ${sym.driftSnapshot.length} 条（` +
      `docs/audits/AUDIT-*-<YYYY-MM-DD>.md 等**时点快照**，判据⑥ 排除）：` +
      `按 AGENTS.md「文档放置」，快照里的旧读数记录的是**当时的世界**，腐烂是正常且应保留的 ⇒` +
      ` 本层**不判红、不进基线、不出订正建议**。第一层（BROKEN/STALE/…）对快照仍按原口径跑。\n` +
      `   复算：--dump-doc-refs 里这些条目带「快照面」字样（合计 ${sym.face.snapshot.withSymbol} 条给出符号 / 比对 ${sym.face.snapshot.checked} / 零存在 ${sym.face.snapshot.absent}）。`,
  );
}

// ── 自动回写（任务 #202 ③）：dry-run 与写盘**分离**，写盘要显式开关 + 六道闸 ──
if (SUGGEST_FIX || FIX) {
  const plans = new Map();
  const whyCodes = {};
  let suggested = 0;
  for (const r of sym.drift) {
    const sug = suggestSymbolFix(r);
    if (!sug.ok) {
      whyCodes[sug.code] = (whyCodes[sug.code] ?? 0) + 1;
      continue;
    }
    suggested++;
    if (!plans.has(r.doc)) plans.set(r.doc, []);
    plans.get(r.doc).push({ r, sug });
  }
  console.log(
    `\n${FIX ? "✚" : "~"} 回写${SUGGEST_FIX && !FIX ? "建议（dry-run，零写盘）" : "模式"}：` +
      `活文档面 ${sym.drift.length} 条漂移 ⇒ 无歧义建议 ${suggested} 条 / 判歧义放弃 ${sym.drift.length - suggested} 条` +
      `（放弃原因 ${Object.entries(whyCodes).map(([k, v]) => `${k} ${v}`).join(" / ") || "无"}）` +
      `；快照面 ${sym.driftSnapshot.length} 条**永不进入**（判据⑥）。`,
  );
  for (const [doc, list] of [...plans.entries()].sort()) {
    for (const { r, sug } of list) console.log(`   → ${doc}:${r.docLine} ${r.refFile}:${r.start} ⇒ :${sug.to}（符号 ${sug.sym} 的声明行）`);
  }
  if (SUGGEST_FIX && !FIX) {
    console.log(`   dry-run 到此为止：一行盘都没写。要写盘请给 --fix，并同时给 --i-attest-quiet-window。`);
  }
  if (FIX) {
    // 写盘模式**当场**跑回写自检（不是「平时碰巧跑过」）：无覆盖 / 零断言 / 有失败 ⇒ 一律拒绝。
    const before = ASSERT_RUN;
    const st = runSymFixSelftest();
    const gate = writeModeGate({
      fix: true,
      attested: QUIET_WINDOW_ATTESTED,
      selftestRan: true,
      selftestAsserts: ASSERT_RUN - before,
      selftestPassed: st.passed,
      precisionOk: st.precision.ok,
      precisionWhy: st.precision.why,
      unverifiedAny: cov.any,
      armed: base.symKeys !== null,
    });
    if (!gate.ok) {
      console.error(`\n✖ --fix 拒绝写盘（退 2）：${gate.why}`);
      console.error(`   上面列出的只是**建议**；未写任何文件。判据与六道闸见文件头「自动回写」。`);
      process.exit(2);
    }
    let wrote = 0;
    const failures = [];
    for (const [doc, list] of [...plans.entries()].sort()) {
      const abs = path.join(ROOT, doc.split("/").join(path.sep));
      let raw;
      try {
        raw = fs.readFileSync(abs, "utf8");
      } catch (e) {
        failures.push(`${doc} 读盘失败：${e.code ?? e.message}`);
        continue;
      }
      const eol = raw.includes("\r\n") ? "\r\n" : "\n"; // 不改行尾（禁区 14 由别的门管，本门不许顺手换）
      const linesArr = raw.split(/\r\n|\n/);
      let touched = 0;
      for (const { r, sug } of list) {
        const idx = r.docLine - 1;
        const orig = linesArr[idx];
        const patched = patchRefStartNumber(orig, r, sug.to);
        if (!patched.ok) {
          failures.push(`${doc}:${r.docLine} 补丁失败：${patched.why}`);
          continue;
        }
        if (orig.split(/\r?\n/).length !== 1 || patched.text.split(/\r?\n/).length !== 1) {
          failures.push(`${doc}:${r.docLine} 补丁跨越行边界 ⇒ 放弃`);
          continue;
        }
        const v = verifyPatchedLine(orig, patched.text, r, sug.to);
        if (!v.ok) {
          failures.push(`${doc}:${r.docLine} 写前验证不通过：${v.why}`);
          continue;
        }
        linesArr[idx] = patched.text;
        touched++;
      }
      if (touched > 0) {
        fs.writeFileSync(abs, linesArr.join(eol), "utf8");
        wrote += touched;
      }
    }
    if (failures.length > 0) {
      console.error(`\n✖ --fix：${failures.length} 条建议在写前验证里被拒 ⇒ 已放弃**全部**写盘（不留半套改动）：`);
      for (const f of failures.slice(0, 20)) console.error(`   ✖ ${f}`);
      process.exit(2);
    }
    console.log(`\n✔ --fix 完成：写入 ${wrote} 处订正（覆盖 ${plans.size} 篇活文档；快照面一条未碰）。`);
    console.log(`   复验：重跑 node scripts/check-doc-refs.mjs —— 建议数应下降，第一层读数**必须不变**（回写不许动桶）。`);
    process.exit(0);
  }
}

if (newViolations.length > 0) {
  console.error(`\n✖ ${newViolations.length} 条基线外新增（BROKEN / STALE —— 文档引用的客观错，修法唯一：改引用）：`);
  for (const v of newViolations.slice(0, 40)) {
    const ref = v.end !== v.start ? `${v.refFile}:${v.start}-${v.end}` : `${v.refFile}:${v.start}`;
    const why = v.bucket === "STALE" ? `行号越界（目标共 ${v.lines} 行）` : "仓内不存在该文件（无截断/同义词候选）";
    console.error(`   ✖ ${v.doc}:${v.docLine} → ${ref} —— ${why}`);
  }
  if (newViolations.length > 40) console.error(`   … 其余 ${newViolations.length - 40} 条省略（--dump-doc-refs 看全量）`);
  console.error(
    `\n  · BROKEN：把引用改成**段对齐**写法（完整路径 / 唯一后缀 / 唯一 basename）；\n` +
      `  · STALE ：按目标文件当前行数订正行号（引用腐烂，编译与单测都看不见）。`,
  );
  process.exit(1);
}

// 第二层判红的**两条**路径（`symViolations` 已在上方按开关/武装状态算好）：
//   · `--require-symbol-match` ⇒ 整个活文档面（不看基线）；
//   · 基线已首冻 ⇒ 只有基线外新增的键。
// 未冻结时 `symViolations` 恒空 ⇒ **退码与 #182 落地后逐字相同**（不因为本批把靶面收窄就变红，
// 也不因为加了棘轮就凭空变红 —— 武装与否由「基线里有没有 symEntries」这一个事实决定）。
if (symViolations.length > 0) {
  console.error(
    `\n✖ SYMDRIFT 判红 ${symViolations.length} 条（活文档面，${REQUIRE_SYMBOL_MATCH ? "--require-symbol-match 全量档" : `棘轮基线外新增 ${symViolations.length} 个键`}）：`,
  );
  for (const v of symViolations.slice(0, 20)) {
    const r = v.ref;
    const ref = r.end !== r.start ? `${r.refFile}:${r.start}-${r.end}` : `${r.refFile}:${r.start}`;
    console.error(
      `   ✖ ${v.key}（${r.doc}:${r.docLine} → ${ref}，符号实际在 ${r.symDrift.actual.join(" / ")}；` +
        `修法：订正行号，或确认该引用确属活文档后跑 --update-baseline 前先修）`,
    );
  }
  if (symViolations.length > 20) console.error(`   … 其余 ${symViolations.length - 20} 条省略（--dump-doc-refs 看全量）`);
  console.error(
    `\n  · 不许把新增键写进基线来凑绿（--update-baseline 在存在新增时**拒绝执行**，这是设计不是缺陷）；\n` +
      `  · 快照（docs/audits/AUDIT-*-<日期>.md）根本不进本档 ⇒ 若这里出现快照键，说明判据⑥ 失效，请当 bug 报。`,
  );
  process.exit(1);
}

// 第三层（任务 #203）升硬开关：**只有显式给了 --require-no-unverified 才会走到这里退 1**。
// 默认不开的理由（与 `--require-symbol-match` 同型判据）：本门是巡检门，盘的瞬时抖动不是文档缺陷，
// 把它判红只会把排查方向引向不存在的引用错误（本会话已实证一次）；而 CI / `ci-check.mjs` 的
// 命令行都不带该开关 ⇒ **默认退码与改动前逐字相同**。改动前不同的是：崩溃（退 1、零输出）
// 现在变成了「有判定输出 + 未检查面自陈」。
if (unverifiedHardFail(cov, REQUIRE_NO_UNVERIFIED)) {
  console.error(
    `\n✖ --require-no-unverified：未检查面非空（引用 ${cov.refs} 条 / 目标 ${cov.targets} 个 / 文档 ${cov.docs} 篇 / 目录 ${cov.dirs} 个` +
      `，重试 ${cov.retries} 次后仍读不到）。上面已逐条点名错误码；不带该开关时本桶只报告、退码与既往逐字相同。`,
  );
  process.exit(1);
}

console.log(
  `PASS: 无新增 BROKEN / STALE` +
    (base.symKeys === null
      ? `；第二层棘轮未武装（活文档面 ${sym.drift.length} 条只报告，快照面 ${sym.driftSnapshot.length} 条不订正）`
      : `；第二层棘轮已武装（基线外新增 0 / 基线 ${base.symKeys.size} 条，靶面只收活文档）`) +
    `。`,
);
process.exit(0);
