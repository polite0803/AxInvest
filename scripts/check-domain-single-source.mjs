// SPDX-License-Identifier: AGPL-3.0-only
/**
 * check-domain-single-source.mjs — 「能力域」单一真相源门禁
 *
 * ## 为什么需要它
 *
 * 「能力域」这个概念在本仓有**一份权威定义 + 一堆手抄副本**：
 *
 * | 载体 | 位置 | 形态 |
 * |---|---|---|
 * | **权威源** | `crates/harness/src/capability.rs` 的 `CapabilityDomain` | Rust 枚举（`as_str`/`FromStr`） |
 * | **L1 域元数据声明**（2026-09-15 新增） | `crates/harness/src/domain_registry.rs` 的 `DOMAIN_NODES` | 枚举变体 + `nav_path` + `nav_order` |
 * | TS 联合类型 | `src/types/capability.ts` | 手抄 9 值 |
 * | **前端域集合**（P1-④ 后前端唯一手写 id 的点） | `src/lib/domainMeta.ts` 的 `DOMAIN_PRESENTATION` | `Record<BusinessDomain, {path,color}>` —— key 集合受 **`tsc` 穷尽检查**；**声明顺序即协议顺序** |
 * | 导航分组顺序 | `src/lib/domainMeta.ts` 的 `NAV_ORDER` | 8 个 id（同一批域的**另一种排列**，无法派生 —— 前端的第二处、也是最后一处手写） |
 * | 前端域元数据视图 | `src/lib/domainMeta.ts` 的 `CAPABILITY_DOMAIN_META` | **派生**自 `NAV_ORDER.map(…)`；P1-④ 起硬拦「写回字面量数组」 |
 * | 协议顺序 | `src/lib/domainMeta.ts` 的 `CAPABILITY_DOMAIN_PROTOCOL_ORDER` | **派生**自 `Object.keys(DOMAIN_PRESENTATION)`；P1-② 起逐项硬拦顺序，P1-④ 起硬拦「写回字面量数组」 |
 * | i18n key 公式 | `domain_registry.rs::DomainNode::label_key()` ↔ `domainMeta.ts::domainLabelKey()` | 两侧须为**同一命名空间** —— P1-③ 起公式比对硬拦 |
 * | L1 分类器 prompt | `src-tauri/src/init/state.rs` 的 prompt 模板 | **P2 起派生**自 `harness::l1_classifier_domain_list()`（读覆盖层）—— 本段只防「抄回字面量」的回流 |
 * | OPC 域包→域映射 | `src-tauri/src/commands/opc_workflows/mod.rs`（两张表，名字见 `OPC_MAPPING_TABLE_CONSTS`） | 手抄裸字符串（目标域） |
 * | i18n 域标签 | `src/i18n/locales/*.json` 的 `capabilityDomain` | 手抄 9 key × 11 语言 |
 *
 * **本脚本就是这些副本之间唯一的门禁**（2026-09 之前确实没有任何门禁）：后端加一个域时，
 * 前端 / prompt / i18n 不会因类型检查而红，而是静默退化成
 * 「新域永远不被 LLM 选中 / 下拉里没有它 / 界面显示裸 id」。
 *
 * ## `DOMAIN_NODES` 为什么不是「第 N 套词汇表」
 *
 * 它**不写 id 字符串** —— 每个条目存的是 `CapabilityDomain::<变体>`，id 由 `as_str()` 派生。
 * 因此它无法与权威源漂移（不复制那份数据），且本门禁额外把「声明覆盖全部变体」变成**硬拦**
 * —— 枚举新增变体却漏声明时，本脚本红。
 * （编译期已有一道守卫：`domain_registry::_variant_tripwire` 的穷尽 match。）
 *
 * > 已收敛的先例（同一件事的正确形态，勿重新分裂）：
 * > `harness/tool.rs` 的 `pub use CapabilityDomain as ToolDomain`（类型别名，
 * > 2026-08 收敛）；`capability_clusters.rs` 用 `domain: CapabilityDomain` 枚举字段
 * > 而非字符串，并有 4 个 `*_domain` 测试钉住派生关系。
 *
 * ## 判据分档（依据判据 #147：门禁里的软判据不得硬拦）
 *
 * | 档 | 对象 | 处置 |
 * |---|---|---|
 * | **硬拦** | 集合不一致（缺值/多值）、幽灵域（别名或映射指向未声明域）、**DOMAIN_NODES 覆盖不全 / 声明顺序 ≠ 枚举顺序 / nav 不变量违反 / 前端 `path` 与声明不一致 / P1-④ `DOMAIN_PRESENTATION` 形态漂移（缺 `} as const satisfies` 收尾、条目缺 `color`、key 重复）/ P1-④ 导航顺序（`NAV_ORDER`）与声明 `nav_order` 序不一致 / P1-④ 派生视图写回字面量数组或改从别处派生 / 别名遮蔽规范 id / 别名跨域重复 / 别名表为空或解析不出 / `serde(alias)` 未包含于声明别名表 / `serde(alias)` 解析不出（含漏条）/ P1-② 协议顺序 ≠ 声明顺序 / **P2 L1 prompt 域清单抄回字面量 / prompt 派生入口消失 / 覆盖层写回 `DOMAIN_NODES` 内置声明** **、权威源或声明解析不出、扫描面为 0 | `exit 1` / `exit 2` |
 * | **报告** | `domain.*` 命名空间与 id 不同名（camelCase vs snake_case）、`browserMock.ts` 的 mock 清单、枚举上 `serde(alias)` 已全部移除 | 只打印，`exit 0` |
 *
 * ### 为什么「顺序」在 P1-② 之后从报告档升为硬拦（#147 的反向适用）
 *
 * #147 说「软判据不得硬拦」，**前提是该判据确实软** —— `PROTOCOL_ORDER` 当初是手抄副本，
 * 其顺序可以是**有意的产品决策**（下拉排序），硬拦只会逼人改判据来灭红灯。
 *
 * P1-② 之后它的定性变了：协议顺序被**定义**为 `DOMAIN_NODES` 的声明顺序
 * （= 枚举声明顺序），同时是 L1 路由 / prompt 候选序 / 下拉项的唯一顺序来源
 * ⇒ 已无「自由决策」空间，两侧不一致即缺陷。而症状是**静默**的：
 * 同一下拉项在两次重构之间来回跳、prompt 候选序与枚举序错位、新域追加在末尾而 prompt 仍列在中间。
 * ⇒ 判据没变软，是**对象从「观点」变成了「派生量」**，于是升级为硬拦。
 *
 * ### P1-④ 之后：原来那条「`order` 字段复活」防回归去哪了
 *
 * 它随对象一起消失了：`CAPABILITY_DOMAIN_META` 改成**派生视图**后，条目字面量已不存在，
 * `order` 字段**无处可加**。取而代之的是 `parseDerivationShape` 拦的**整类回流**：
 * 任一派生视图（`PROTOCOL_ORDER` / `CAPABILITY_DOMAIN_META`）被改回字面量数组、
 * 或改从别处派生 ⇒ 硬拦。覆盖面比「一个字段名」大，且不依赖具体字段名。
 *
 * ### P2 之后：L1 prompt 域清单的**守卫迁移**（不是删除）
 *
 * P2 之前 `init/state.rs` 的 prompt 里有一串手抄的 9 个 slug，本段逐值 + 逐序硬拦它。
 * P2 起该清单由 `harness::domain_registry::l1_classifier_domain_list()` **实时派生**
 * （它读域覆盖层 ⇒ 停用一个域，该域自动从 LLM 候选集消失）—— 源码里**已不存在**
 * 可比对的字面量。于是守卫**换位置，不消失**：
 *
 * | 断言 | 原来在哪 | 现在在哪 |
 * |---|---|---|
 * | 清单 == 9 个域、顺序 == 协议顺序 | 本脚本正则比对源码文本 | **Rust 单测**对**真实函数输出**断言（`domain_registry::tests::test_l1_classifier_domain_list_covers_all_when_enabled`，期望值是**独立硬编码**的 9 个 slug） |
 * | 清单填了全部域（漏一个 ⇒ 该域永不被 LLM 选中） | 同上 | 同上（**更强**：测的是行为，不是源码文本） |
 * | prompt 不得抄回字面量清单 | —（当时它就该是字面量） | **本脚本**（防回流） |
 * | prompt 的派生入口必须存在 | — | **本脚本**（入口被换掉 ⇒ 按解析失败硬拦） |
 *
 * ⚠ 若只删掉原来那段正则比对、不接上面的替代物，就是**删断言**而不是迁移断言 ——
 * 那正是判据 #147 警告的「把红灯灭掉而不是把缺陷修掉」。
 *
 * ### 为什么「呈现表 key 顺序 vs Rust 声明顺序」不是同源恒真
 *
 * 本段有一处容易被误读成自证：门禁读 `DOMAIN_PRESENTATION` 的 key 顺序，
 * 而运行时的 `CAPABILITY_DOMAIN_PROTOCOL_ORDER` 也派生自同一处 ⇒ 看着像「自己比自己」。
 * 但**比对基准是 Rust 侧 `DOMAIN_NODES` 的声明顺序**（外部源）：比的是
 * 「TS 源码 vs Rust 源码」，与运行时那条派生链同源与否无关。
 * 要防的同源恒真是「TS 内部两处互为派生、门禁只比这两处」——本脚本从不这么做：
 * `NAV_ORDER` 与呈现表 key 序是两个**独立排列**，各自的基准都是 Rust 侧声明。
 *
 * ## 已知边界（自陈，勿高估本脚本）
 *
 *   · 抽取器只认 **snake_case**（`[a-z_]+`）的域标识。若某站点写成 camelCase
 *     （如 `id: "dataAnalysis"`），该值会被**整体跳过** ⇒ 门禁报「缺 data_analysis」
 *     而**不**报「多 dataAnalysis」。漏报方向安全（宁可报缺），诊断消息少一半而已。
 *     实测：探针首版把幽灵值写成 `communicationX`，门禁只报「缺 communication」，
 *     是探针的 `drift_says=false` 把它抓出来的 —— 幽灵分支当时**根本没被执行**。
 *   · 只做**集合**比对，不判「某域该不该存在」；那是产品决策（见 PLAN §4 三轴划界）。
 *   · 不扫 DB 存量数据与 `TaskKind`/`AnalystRole`/`DemandDiscovery` 等同名异义枚举。
 *
 * ## 退出码
 *
 *   0 全好 ｜ 1 客观错（集合不一致 / 幽灵域）｜ 2 脚本自身失效（权威源或站点抓不到）
 *
 * ## 用法
 *
 *   node scripts/check-domain-single-source.mjs              # 门禁
 *   node scripts/check-domain-single-source.mjs --list       # 打印全部站点实测
 *   node scripts/check-domain-single-source.mjs --json
 *   node scripts/check-domain-single-source.mjs --selftest
 *
 * ## 覆盖范围（自陈，判据 #16）
 *
 * 扫上表全部载体（实测报告 **9 个站点 / 17 个文件**）。**不扫**：DB 存量数据（`capability_clusters.domain` /
 * `workflow_template.route_path` / `agent_roles.active_domains` 等 —— 是运行时数据，
 * 其合法性由各写入端的 `FromStr`/`CapabilityDomain` 类型保证）；
 * `TaskKind`（rt-workflow，语义是**任务类型**不是能力域；2026-09-15 由 `TaskDomain` 改名）、
 * `AnalystRole`（analysis-engine，语义是**分析师分工**；同日由 `AnalystDomain` 改名）、
 * `DemandDiscovery` 的 `domain_*`（语义是**市场需求领域**）—— 三者属「同名不同义」，
 * 见 `docs/plans/PLAN-domain-single-source.md` 的三轴划界，**不要**把它们并进本门禁。
 *
 * ⚠ `DemandDiscovery` 的 `domain_*` 另有一层**跨语言隐式契约**，动它之前必读 PLAN §5：
 * 那些是 `workflow_template(id="demand-discovery")` 的**变量名**，Rust 侧
 * `commands/demand_discovery.rs` 的 `extract_domain_queries()` 用
 * `name.starts_with("domain_")` **按前缀通配**取用。任一侧改名都会**静默失效**
 * 并落到该函数的硬编码兜底关键词（`queries.is_empty()` 分支），**不报错**。
 * 故 2026-09-15 的整改**刻意保留**该前缀 —— 只登记、不改名（改它属行为变更，
 * 需先补「任一 `domain_*` 变量缺失时告警」的守卫）。
 */

import { existsSync, readFileSync, readdirSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const ROOT = resolve(HERE, "..");

const ARG = (n) => process.argv.includes(`--${n}`);
const LIST = ARG("list");
const JSON_OUT = ARG("json");
const SELFTEST = ARG("selftest");
const CI = ARG("ci");

// ── 站点清单（契约：每条都必须抓到，抓不到 = 代码被重构了，不是「没问题」）──

/** 权威源 */
const AUTHORITY_REL = "src-tauri/crates/harness/src/capability.rs";
/** L1 域元数据声明（2026-09-15 新增；枚举变体 + nav_path/nav_order） */
const REGISTRY_REL = "src-tauri/crates/harness/src/domain_registry.rs";
/** TS 联合类型 */
const TS_TYPE_REL = "src/types/capability.ts";
/** 前端域元数据 + 协议顺序（同一文件两处声明） */
const META_REL = "src/lib/domainMeta.ts";
/** L1 分类器 prompt 的域清单 */
const PROMPT_REL = "src-tauri/src/init/state.rs";
/** OPC 两张映射表 */
const OPC_REL = "src-tauri/src/commands/opc_workflows/mod.rs";
/** i18n locale 目录 */
const LOCALES_DIR = "src/i18n/locales";

const readText = (rel) => {
  const p = join(ROOT, rel);
  return existsSync(p) ? readFileSync(p, "utf8") : null;
};

// ══ 纯函数：抽取 ══════════════════════════════════════════════════════

/** 取从 `head` 起的那对花括号内的内容（含括号；配平失败返回 null） */
function sliceBraced(src, start) {
  const open = src.indexOf("{", start);
  if (open < 0) return null;
  let depth = 0;
  for (let i = open; i < src.length; i++) {
    if (src[i] === "{") depth++;
    else if (src[i] === "}") {
      depth--;
      if (depth === 0) return src.slice(open, i + 1);
    }
  }
  return null;
}

/**
 * 取 `impl <Type> {` 的**整块**。
 *
 * ⚠ 为什么必须先限定 impl 块，而不是直接找 `pub fn as_str`：
 *   实测 `capability.rs` 里 `pub fn as_str` 出现 **10 次**，第一次属于**另一个类型**；
 *   裸符号名锚点在「一个符号名在同一文件多处出现」时会静默取错对象
 *   （本脚本首跑即踩到：解析出的是别的类型的 as_str ⇒ 一条 `CapabilityDomain::` 都匹配不到）。
 *   类型限定是自锚定的前提，不是可选优化。
 */
export function extractImplBlock(src, implHead) {
  const start = src.indexOf(implHead);
  if (start < 0) return null;
  return sliceBraced(src, start);
}

/** 在 impl 块内取 `pub fn <name>` 的函数体 */
export function extractFnBodyIn(block, name) {
  if (block === null) return null;
  const start = block.indexOf(name);
  if (start < 0) return null;
  return sliceBraced(block, start);
}

const IMPL_CAPABILITY_DOMAIN = "impl CapabilityDomain {";
// ⚠ 此处曾有 `IMPL_FROM_STR`（`impl std::str::FromStr for CapabilityDomain {`），
// 用于解析 `FromStr` 里那张扁平别名 `match`。2026-09-15 别名迁入
// `domain_registry.rs::DOMAIN_NODES.aliases` 后该 `match` 已不存在，
// 常量随之删除 —— **不要**把它加回来用以「兼容旧形态」：那只会重建一条
// 读不到东西却报绿灯的假检查（见 `parseDeclaredAliases` 的说明）。

/**
 * 权威源：`CapabilityDomain::Xxx => "yyy"` 的 (变体名, 字符串) 对。
 *
 * 同时给出「内部域」判定的唯一来源 —— `is_system()` 用的是变体名
 * （`CapabilityDomain::System`），本脚本据此把变体名映射回字符串，
 * **不在脚本里硬编码 `"system"` 字面量**。
 */
export function parseAuthorityPairs(src) {
  const block = extractImplBlock(src, IMPL_CAPABILITY_DOMAIN);
  const body = extractFnBodyIn(block, "pub fn as_str");
  if (body === null) return null;
  const pairs = [...body.matchAll(/CapabilityDomain::(\w+)\s*=>\s*"([a-z_]+)"/g)].map((m) => [
    m[1],
    m[2],
  ]);
  return pairs.length ? pairs : null;
}

/** `is_system()` 判定的变体名（权威源里的"内部域"） */
export function parseSystemVariant(src) {
  const block = extractImplBlock(src, IMPL_CAPABILITY_DOMAIN);
  const body = extractFnBodyIn(block, "pub fn is_system");
  if (body === null) return null;
  const m = body.match(/CapabilityDomain::(\w+)/);
  return m ? m[1] : null;
}

/**
 * `domain_registry.rs::DOMAIN_NODES` 里声明的 (别名, 变体名) 对 —— 别名的**唯一**声明位置。
 *
 * ⚠ **不得改回读 `capability.rs` 的 `FromStr`**（2026-09-15 教训）：
 * 别名迁入 `DOMAIN_NODES.aliases` 后，`FromStr` 里那张扁平 `match` 已不存在，
 * 而旧实现 `parseAliasTargets` 会把「正则匹配到 0 条」返回成**空数组**（不是 `null`），
 * 主流程据此打印「历史别名 0 条 ✔」并 `exit 0` —— **fail-open 假通过**
 * （判据 #7「0 命中 ≠ 没问题」+ #216「静默兜底」）。
 *
 * 因此本函数的契约是：**解析不出、或一条别名都没有 ⇒ `null`**。
 * 别名 0 条不是合法状态 —— `active_domains` / `route_path` 等 DB 存量字符串
 * 全靠它解析（`PLAN-domain-single-source.md` §7.2）。
 */
export function parseDeclaredAliases(registrySrc) {
  const nodes = parseDomainNodes(registrySrc);
  if (nodes === null) return null;
  const out = [];
  for (const n of nodes) for (const a of n.aliases) out.push([a, n.variant]);
  return out.length ? out : null;
}

/** TS 联合类型：`export type X = | "a" | "b";` */
export function parseTsUnion(src, typeName = "CapabilityDomain") {
  const m = src.match(new RegExp(`export type\\s+${typeName}\\s*=([\\s\\S]*?);`));
  if (!m) return null;
  const vals = [...m[1].matchAll(/"([a-z_]+)"/g)].map((x) => x[1]);
  return vals.length ? vals : null;
}

/**
 * `domainMeta.ts::DOMAIN_PRESENTATION` —— 域 id 集合的**唯一**声明点（P1-④）。
 *
 * 解析契约（写在 `domainMeta.ts` 该常量的文档里）：对象字面量，**每个 key 是一个业务域
 * id**，值形如 `{ path: "…", color: "…" }`。返回顺序 = **声明顺序**（它同时就是协议顺序，
 * 因为 `CAPABILITY_DOMAIN_PROTOCOL_ORDER` 由 `Object.keys()` 派生本表）。
 *
 * ⚠ 终止标记取 `} as const satisfies`（该常量被 `Record<BusinessDomain, …>` 约束，
 * 这是「漏一个域 ⇒ `tsc` 报错」的实现手段）。形态一旦变化 ⇒ 返回 null，
 * **不得**返回部分结果 —— 部分解析会让「漏声明一个域」被当成「少一行」静默通过。
 *
 * ⚠ **漏条检测**：条目数必须等于 `path:` 出现次数，且 id 不得重复。
 * 少了这道检测，一条形态异常的条目会被正则静默跳过，「8 个域」会被当成「7 个域」
 * 报上去而没人知道（判据 #7：漏命中 ≠ 无问题）。
 */
export function parsePresentationTable(src, constName = "DOMAIN_PRESENTATION") {
  const start = src.indexOf(`const ${constName}`);
  if (start < 0) return null;
  const end = src.indexOf("} as const satisfies", start);
  if (end < 0) return null;
  const body = src.slice(start, end);
  const attrCount = [...body.matchAll(/\bpath\s*:/g)].length;
  const out = [];
  for (const m of body.matchAll(/([a-z_]+)\s*:\s*\{([^}]*)\}/g)) {
    const blk = m[2];
    const p = blk.match(/\bpath\s*:\s*"([^"]*)"/);
    const c = blk.match(/\bcolor\s*:\s*"([^"]*)"/);
    if (!p || !c) return null;
    out.push({ id: m[1], path: p[1], color: c[1] });
  }
  if (!out.length) return null;
  if (out.length !== attrCount) return null;
  // 重复 key 在对象字面量里是「后者胜出」，但源码里两个条目都数得到 ⇒
  // 上面那个计数检测抓不到，必须单独判（否则运行时集合会悄悄少一个域）。
  if (new Set(out.map((e) => e.id)).size !== out.length) return null;
  return out;
}

/**
 * `domainMeta.ts::NAV_ORDER` —— 导航分组顺序（P1-④ 之后该文件**唯一**手写 id 的地方）。
 *
 * ⚠ 它与 `DOMAIN_PRESENTATION` 的 key 顺序**不是同一件事**：前者是导航分组序、
 * 后者是协议（枚举）序。两者都合法但用途不同，混用会让侧栏分区/下拉项在两次重构
 * 之间来回跳。本函数只负责取出这个序列，比对基准由主流程给（后端 `nav_order`）。
 */
export function parseNavOrder(src, constName = "NAV_ORDER") {
  const start = src.indexOf(`const ${constName}`);
  if (start < 0) return null;
  const end = src.indexOf("]", start);
  if (end < 0) return null;
  const vals = [...src.slice(start, end).matchAll(/"([a-z_]+)"/g)].map((m) => m[1]);
  return vals.length ? vals : null;
}

/**
 * Rust 侧 i18n key 公式 —— `DomainNode::label_key()` 的**命名空间字面量**。
 *
 * 契约形态（写在 `domain_registry.rs` 的方法文档里）：
 * `format!("<命名空间>.{}", self.slug())`。返回 `"<命名空间>."`（含末尾点）。
 *
 * ⚠ 字符类用 `[^"]*` 而**不是** `[a-z_]+`：命名空间 `capabilityDomain` 含**大写**，
 * 用小写字符类会让正则静默匹配不到 ⇒ 返回 null ⇒ 门禁硬拦（方向安全，但会误报形态问题）。
 * 本仓已两次踩「值里含大写/数字被字符类静默跳过」的坑（`communicationX` / `core_v2`），
 * 故此处一律用「除引号外任意」并配合下面的**漏条检测**。
 *
 * 解析不出 ⇒ `null`（**不是**空字符串，更不是「无违规」）。
 */
export function parseLabelKeyFormula(src) {
  const heads = [...src.matchAll(/fn\s+label_key\s*\(\s*&self\s*\)/g)].length;
  if (heads === 0) return null;
  const m = src.match(
    /fn\s+label_key\s*\(\s*&self\s*\)\s*->\s*String\s*\{\s*format!\(\s*"([^"]*)\{\}"\s*,\s*self\.slug\(\)\s*\)/,
  );
  if (!m) return null;
  // 漏条检测：函数头出现多次而只抽到一个 ⇒ 说明还有别的形态没被覆盖（判据 #7：漏命中 ≠ 无问题）
  if (heads !== 1) return null;
  return m[1];
}

/**
 * TS 侧 i18n key 公式 —— `domainLabelKey()` 的**命名空间字面量**。
 *
 * 契约形态：`return \`<命名空间>.${<参数名>}\`;`。返回
 * `{ namespace, interpolated, params, simple }` —— 除命名空间外，额外校验
 * 「模板插值的变量必须是本函数自己的参数」（防止公式变成基于外部变量、
 * 从而在调用点算出与参数无关的 key）。
 *
 * ⚠ 插值抽取用 `[^}]*`（**不**限定为标识符）：若写成 `${globalThis.__d}` 这类复合表达式，
 * 限定标识符的正则会**整体匹配失败 ⇒ 返回 null ⇒ 门禁报「形态已变」** ——
 * 硬拦方向安全，但诊断消息指向错误的位置（真问题不是形态，是插值不可信）。
 * 故此处取出插值原文，再用 `simple` 标记它是否为「裸标识符」，交由调用方分档诊断。
 *
 * 解析不出 ⇒ `null`。
 */
export function parseTsLabelKeyFormula(src) {
  const header = src.match(/function\s+domainLabelKey\s*\(([^)]*)\)\s*:\s*string\s*\{/);
  if (!header) return null;
  const params = header[1]
    .split(",")
    .map((p) => p.split(":")[0].trim())
    .filter(Boolean);
  const body = src.slice(header.index);
  const m = body.match(/return\s+`([^`$]*)\$\{([^}]*)\}/);
  if (!m) return null;
  const interpolated = m[2].trim();
  return {
    namespace: m[1],
    interpolated,
    params,
    simple: /^[A-Za-z_$][\w$]*$/.test(interpolated),
  };
}

/**
 * `domain_registry.rs` 的 `DOMAIN_NODES` 条目。
 *
 * 解析契约写在被解析文件自己的模块头（`# 解析契约`）：每个条目须是
 * `DomainNode { … }` 字面量，字段名逐字为 `domain` / `nav_path` / `nav_order` / `aliases`。
 * **不要求单行或多行**（`cargo fmt` 会按列宽折叠，实测同一数组里两种形态并存）。
 *
 * 任一字段形态变了 ⇒ 返回 null（**不是**跳过该条）—— 部分解析会让「漏声明一个域」
 * 被当成「少一行」而静默通过。
 *
 * ⚠ 返回的是**枚举变体名**（`General`），不是 id（`general`）。
 * 变体名 → id 的映射取自权威源的 `as_str()`，本函数**不**持有该映射
 * （否则就是又一份会腐烂的副本）。
 */
export function parseDomainNodes(src) {
  const start = src.indexOf("pub const DOMAIN_NODES");
  if (start < 0) return null;
  const end = src.indexOf("\n];", start);
  if (end < 0) return null;
  const body = src.slice(start, end);
  const out = [];
  for (const m of body.matchAll(/DomainNode\s*\{([^}]*)\}/g)) {
    const blk = m[1];
    const d = blk.match(/domain:\s*CapabilityDomain::(\w+)/);
    const p = blk.match(/nav_path:\s*(Some\("([^"]*)"\)|None)/);
    const o = blk.match(/nav_order:\s*(Some\((\d+)\)|None)/);
    const a = blk.match(/aliases:\s*&\[([^\]]*)\]/);
    if (!d || !p || !o || !a) return null;
    out.push({
      variant: d[1],
      navPath: p[2] === undefined ? null : p[2],
      navOrder: o[2] === undefined ? null : Number(o[2]),
      aliases: [...a[1].matchAll(/"([a-z_]+)"/g)].map((x) => x[1]),
    });
  }
  return out.length ? out : null;
}

/**
 * 权威源里 `#[serde(alias = "…")]` 的别名（**存量反序列化契约**的额外子集）。
 *
 * 这些属性是第二批别名副本：`capability.rs` 的枚举变体上带了 3 条 `serde(alias)`，
 * 与 `DOMAIN_NODES` 的 `aliases` 字段并存。门禁只做**单向包含**校验
 * （serde 别名必须出现在声明里）—— 因为 serde 属性在**反序列化路径上是独立生效的**，
 * 少一条 = 存量 DB 字符串解析失败，而这一点 `FromStr` 的别名表**覆盖不到**。
 */
export function parseSerdeAliases(src) {
  const start = src.indexOf("pub enum CapabilityDomain");
  if (start < 0) return null;
  const end = src.indexOf("\n}", start);
  if (end < 0) return null;
  const body = src.slice(start, end);
  const attrs = [...body.matchAll(/#\[serde\(alias\s*=/g)].length;
  const out = [...body.matchAll(/#\[serde\(alias\s*=\s*"([a-z_]+)"\)\]/g)].map((m) => m[1]);
  // ⚠ **漏条检测**：若某条属性写成 `#[serde(alias = "core_v2")]`（含数字）或
  // `"coreX"`（含大写），值字符类 `[a-z_]+` 匹配不上 ⇒ 正则**静默跳过该条**，
  // 门禁只看到「剩下的那些」并报绿。故比对「属性出现次数」与「抽到的条数」，
  // 不等即判解析失败（判据 #7：漏命中 ≠ 无问题）。
  if (out.length !== attrs) return null;
  return out;
}

/**
 * 常量声明的**形态**：是「手抄字面量数组」还是「派生表达式」（P1-④）。
 *
 * 返回 `{ expr, isLiteralArray, from }`：
 *   · `expr`           —— 等号右侧原文（已 trim）
 *   · `isLiteralArray` —— 是否以 `[` 开头（= 手抄字面量）
 *   · `from`           —— 派生来源标识符（`Object.keys(X)` → `X`；`X.map(…)` → `X`）
 *
 * 用途 = **防回流**。P1-④ 把 `CAPABILITY_DOMAIN_PROTOCOL_ORDER` / `CAPABILITY_DOMAIN_META`
 * 从手抄数组改成派生视图后，集合维度才收敛为单点。否则下次有人为了「改起来直观」
 * 把字面量数组写回去，集合就又变回两个会各自腐烂的手写点 —— 而且是**静默**的：
 * 编译通过、运行正常、门禁若只比集合也照样绿（两份恰好一致时）。
 *
 * 解析不出 ⇒ `null`（不得当作「形态没问题」）。
 */
export function parseDerivationShape(src, constName) {
  const m = src.match(new RegExp(`const\\s+${constName}\\b[^=]*=([\\s\\S]*?);`));
  if (!m) return null;
  const expr = m[1].trim();
  if (expr.startsWith("[")) return { expr, isLiteralArray: true, from: null };
  const keys = expr.match(/Object\.keys\(\s*([A-Za-z_$][\w$]*)\s*[,)]/);
  if (keys) return { expr, isLiteralArray: false, from: keys[1] };
  const map = expr.match(/([A-Za-z_$][\w$]*)\s*\.map\s*\(/);
  if (map) return { expr, isLiteralArray: false, from: map[1] };
  return { expr, isLiteralArray: false, from: null };
}

/**
 * `init/state.rs` 的 L1 分类器 prompt 里那串域标识。
 *
 * 锚点用 prompt 的中文原话（`L1 域路由分类器`），再取「…标点：」之后的逗号列表。
 * **LLM 只认得这里列出的值** —— 漏一个 ⇒ 该域永远不会被模型兜底选中。
 */
export function parsePromptDomainList(src) {
  const anchor = src.indexOf("L1 域路由分类器");
  if (anchor < 0) return null;
  const window = src.slice(anchor, anchor + 900);
  const m = window.match(/：\\?\s*([a-z_][a-z_,\s]*)\s*";/);
  if (!m) return null;
  const vals = m[1]
    .split(",")
    .map((s) => s.trim())
    .filter(Boolean);
  return vals.length ? vals : null;
}

/**
 * P2：判定 prompt 站点（`init/state.rs`）的域清单**形态**。
 *
 * P2 之前这里必须是一串手抄的 9 个 slug，本门禁逐值 + 逐序比对。
 * P2 起清单由 `harness::l1_classifier_domain_list()` 从覆盖层实时派生
 * ⇒ 正确形态**不再**是字面量。于是本函数把「形态判定」做成可测的纯函数
 * （门禁自身的惯例：判据要能被 selftest 直接喂输入）。
 *
 * 返回 `{ kind, list, entry }`，`kind` 取值：
 *
 * | kind | 含义 | 门禁处置 |
 * |---|---|---|
 * | `missing_anchor` | 连 prompt 锚点都没了（prompt 被搬走 / 改名） | 硬拦（解析失败） |
 * | `literal` | **回流**：清单又变回手抄字面量 | 硬拦（防回流） |
 * | `derived` | 正确：调用派生入口 | 通过 + 站点报告 |
 * | `unknown_entry` | 无字面量清单、也找不到派生入口（入口被改名的典型形态） | 硬拦（解析失败） |
 */
export function classifyPromptDomainSite(src) {
  if (typeof src !== "string" || !src.includes("L1 域路由分类器")) {
    return { kind: "missing_anchor", list: null, entry: null };
  }
  const list = parsePromptDomainList(src);
  const entryMatch = src.match(/l1_classifier_domain_list\s*\(/);
  const entry = entryMatch ? entryMatch[0].replace(/\s*\($/, "") : null;
  if (list) return { kind: "literal", list, entry };
  if (!entry) return { kind: "unknown_entry", list: null, entry: null };
  return { kind: "derived", list: null, entry };
}

/**
 * OPC 两张映射表的**常量名**（会被重命名 ⇒ 改名时先看这里）。
 *
 * 2026-09-15 实测：外部重构把 `OPC_INDUSTRY_DOMAIN` 改名为 `OPC_DOMAIN_PACK_DOMAIN`
 * （OPC「行业包 → 域包」术语统一），门禁随即报「解析失败」——**报得对**（它没被绕过），
 * 但诊断消息当时只说「两张表须都存在且非空」，没点名缺哪个常量，排查要回读源码。
 * ⇒ 常量名集中在这里 + 报错消息列出期望名，改名时一眼可见。
 */
export const OPC_MAPPING_TABLE_CONSTS = ["OPC_CAPABILITY_PACK_DOMAIN", "OPC_WF_SEGMENT_DOMAIN"];

/** OPC 两张映射表的**第二个元素**（目标能力域） */
export function parseOpcMappingTargets(src, constNames = OPC_MAPPING_TABLE_CONSTS) {
  const out = [];
  let found = 0;
  for (const name of constNames) {
    const start = src.indexOf(`const ${name}`);
    if (start < 0) continue;
    const end = src.indexOf("];", start);
    if (end < 0) continue;
    found++;
    for (const m of src.slice(start, end).matchAll(/\("[^"]+",\s*"([a-z_]+)"\)/g)) out.push(m[1]);
  }
  return found === constNames.length && out.length ? out : null;
}

/** i18n locale 里 `capabilityDomain` 命名空间的 key 集合 */
export function parseI18nDomainKeys(jsonText) {
  try {
    const obj = JSON.parse(jsonText);
    const seg = obj?.capabilityDomain;
    if (!seg || typeof seg !== "object" || Array.isArray(seg)) return null;
    const keys = Object.keys(seg);
    return keys.length ? keys : null;
  } catch {
    return null;
  }
}

// ══ 纯函数：比较（必须能判失败 —— selftest 有专门用例）════════════════

/** 集合差：返回 `{ missing, extra }`（按 found / expected 各自的顺序） */
export function diffSets(found, expected) {
  const f = new Set(found);
  const e = new Set(expected);
  return {
    missing: expected.filter((x) => !f.has(x)),
    extra: found.filter((x) => !e.has(x)),
  };
}

// ══ 主流程 ════════════════════════════════════════════════════════════

const problems = [];
const reports = [];
const info = [];
const siteReport = [];
let readCount = 0;

function loadSite(rel) {
  const t = readText(rel);
  if (t === null) {
    problems.push({ where: rel, why: "站点文件不存在（清单是契约，勿当作通过）" });
    return null;
  }
  readCount++;
  return t;
}

function checkSet(label, found, expected, where) {
  if (found === null) {
    problems.push({ where, why: `${label} 解析失败 —— 模式腐烂或结构被改（**勿当作通过**）` });
    return;
  }
  const d = diffSets(found, expected);
  if (d.missing.length || d.extra.length) {
    problems.push({
      where,
      why:
        `${label} 与权威源不一致` +
        (d.missing.length ? `｜缺 ${d.missing.join(", ")}` : "") +
        (d.extra.length ? `｜多（幽灵域）${d.extra.join(", ")}` : ""),
    });
  }
  siteReport.push({ site: where, kind: label, found: `${found.length} 值`, note: found.join(", ") });
}

function main() {
  // ── 1. 权威源 ──
  const authoritySrc = loadSite(AUTHORITY_REL);
  if (authoritySrc === null) return 2;

  const pairs = parseAuthorityPairs(authoritySrc);
  const systemVariant = parseSystemVariant(authoritySrc);
  if (pairs === null || systemVariant === null) {
    console.error("✖ 权威源解析失败：capability.rs 的 as_str()/is_system() 形态已变。");
    console.error("  门禁拒绝按「0 违规」通过（判据：权威源解析不出 ⇒ 非 0 退出）。");
    return 2;
  }

  const authority = pairs.map(([, s]) => s);
  const variantToStr = new Map(pairs);
  const systemStr = variantToStr.get(systemVariant);
  if (!systemStr) {
    console.error(`✖ 权威源自洽失败：is_system() 指向 ${systemVariant}，但 as_str() 未声明它。`);
    return 2;
  }
  const business = authority.filter((d) => d !== systemStr);

  info.push(`权威源：${authority.length} 个域（含内部域 \`${systemStr}\`）＝ ${authority.join(", ")}`);
  info.push(`业务域（不含内部域）：${business.length} 个 ＝ ${business.join(", ")}`);

  // ⚠ 别名检查**不在此处** —— 别名的唯一声明位置是 `domain_registry.rs::DOMAIN_NODES.aliases`，
  // 故必须等声明解析完成后再验（见下 1b 段末）。此处历史上曾读 `capability.rs` 的
  // `FromStr` 扁平 `match`，别名迁出后该实现退化成「匹配 0 条 ⇒ 报 0 条 ✔」的
  // fail-open 假通过（判据 #7/#216），已删除。

  // ── 1b. L1 域元数据声明（DOMAIN_NODES）—— 2026-09-15 新增 ──
  //
  // 本段是「域元数据（导航路径/顺序）从此有了声明位置」的门禁面：
  //   ① 声明必须覆盖 as_str() 的**全部**变体（缺一个 = 硬拦，这是本段的核心）
  //   ② 声明顺序必须等于 as_str() 顺序（它被当作「协议顺序」的权威值）
  //   ③ nav 不变量：业务域有路径+顺序且唯一连续；内部域两者皆无
  const registrySrc = loadSite(REGISTRY_REL);
  let declaredNodes = null;
  // ⚠ 提升到本段**外层**：声明顺序是「协议顺序」的权威值，下游有两处要比对它
  // （前端 `PROTOCOL_ORDER`、L1 分类器 prompt 域清单）。此前它只在 `else` 块内，
  // 下游只能退而对比 `authority`（枚举顺序）—— 两者当前相等，但**耦合方向是错的**：
  // 「声明顺序 ≠ 枚举顺序」的漂移会被本段拦下，可下游拿 `authority` 比对时
  // **看不出任何差异**，等于下游校验的是另一件事。
  let declaredSlugs = null;
  if (registrySrc !== null) {
    declaredNodes = parseDomainNodes(registrySrc);
    if (declaredNodes === null) {
      problems.push({
        where: REGISTRY_REL,
        why: "DOMAIN_NODES 解析失败 —— 形态已变（解析契约见该文件模块头「# 解析契约」）。勿当作通过。",
      });
      declaredNodes = null;
    } else {
      const declaredVariants = declaredNodes.map((n) => n.variant);

      const ghostVariants = declaredVariants.filter((v) => !variantToStr.has(v));
      if (ghostVariants.length) {
        problems.push({
          where: REGISTRY_REL,
          why: `DOMAIN_NODES 声明了权威源不存在的变体：${ghostVariants.join(", ")}`,
        });
      }

      declaredSlugs = declaredVariants.map((v) => variantToStr.get(v) ?? `?${v}`);

      const dcov = diffSets(declaredSlugs, authority);
      if (dcov.missing.length || dcov.extra.length) {
        problems.push({
          where: REGISTRY_REL,
          why:
            `DOMAIN_NODES 未与权威源对齐` +
            (dcov.missing.length ? `｜缺 ${dcov.missing.join(", ")}` : "") +
            (dcov.extra.length ? `｜多 ${dcov.extra.join(", ")}` : "") +
            `\n      （新增域必须在 domain_registry.rs 声明节点，否则导航/门禁都看不到它）`,
        });
      }

      if (declaredSlugs.join(",") !== authority.join(",")) {
        problems.push({
          where: REGISTRY_REL,
          why:
            `DOMAIN_NODES 顺序必须等于 as_str() 顺序（它被当作「协议顺序」的权威值）\n` +
            `      权威顺序：${authority.join(", ")}\n` +
            `      声明顺序：${declaredSlugs.join(", ")}`,
        });
      }

      // nav 不变量
      const businessNodes = declaredNodes.filter((n) => variantToStr.get(n.variant) !== systemStr);
      const systemNodes = declaredNodes.filter((n) => variantToStr.get(n.variant) === systemStr);
      for (const n of businessNodes) {
        if (!n.navPath) {
          problems.push({ where: REGISTRY_REL, why: `业务域 ${n.variant} 缺 nav_path` });
        } else if (!n.navPath.startsWith("/")) {
          problems.push({ where: REGISTRY_REL, why: `域 ${n.variant} 的 nav_path 须以 / 开头：${n.navPath}` });
        }
        if (n.navOrder === null) {
          problems.push({ where: REGISTRY_REL, why: `业务域 ${n.variant} 缺 nav_order` });
        }
      }
      for (const n of systemNodes) {
        if (n.navPath !== null || n.navOrder !== null) {
          problems.push({
            where: REGISTRY_REL,
            why: `内部域 ${n.variant} 不得有 nav_path/nav_order（不变量：内部域永不进入导航）`,
          });
        }
      }
      const orders = businessNodes.map((n) => n.navOrder).filter((x) => x !== null);
      const sortedOrders = [...orders].sort((a, b) => a - b);
      const expectOrders = businessNodes.map((_, i) => i);
      if (sortedOrders.join(",") !== expectOrders.join(",")) {
        problems.push({
          where: REGISTRY_REL,
          why: `业务域 nav_order 必须是 0..${businessNodes.length - 1} 连续无重复，实测：${orders.join(", ")}`,
        });
      }
      const paths = businessNodes.map((n) => n.navPath).filter(Boolean);
      if (new Set(paths).size !== paths.length) {
        problems.push({ where: REGISTRY_REL, why: `导航路径重复：${paths.join(", ")}` });
      }

      // ── 别名（唯一声明位置 = DOMAIN_NODES.aliases，故在此验而非读权威源）──
      //
      // 三类硬拦，每类都对应一种**静默**故障：
      //   ① 幽灵域：别名指向未声明变体 ⇒ 解析永远失败（等价于该别名不存在）
      //   ② 遮蔽：别名与某个**规范 id 同名** ⇒ `from_str` 先比 slug，该别名永不生效
      //   ③ 跨域重复：同一别名挂在两个域上 ⇒ 后声明者永远选不到（依赖声明顺序）
      const declaredAliases = parseDeclaredAliases(registrySrc);
      if (declaredAliases === null) {
        problems.push({
          where: REGISTRY_REL,
          why: "DOMAIN_NODES.aliases 解析失败，或一条别名都没有 —— **别名 0 条不是合法状态**\n" +
            "      （`active_domains` / `route_path` 等 DB 存量字符串全靠它解析）。勿当作通过。",
        });
      } else {
        const ghostAliasList = declaredAliases.filter(([, v]) => !variantToStr.has(v));
        if (ghostAliasList.length) {
          problems.push({
            where: REGISTRY_REL,
            why: `别名指向未声明变体（幽灵域）：${ghostAliasList.map(([k, v]) => `${k}→${v}`).join(", ")}`,
          });
        }
        const canonicalIds = new Set(authority);
        const shadowAliases = declaredAliases.filter(([a]) => canonicalIds.has(a)).map(([a]) => a);
        if (shadowAliases.length) {
          problems.push({
            where: REGISTRY_REL,
            why: `别名与规范 id 同名（会被 slug 分支遮蔽，永不生效）：${shadowAliases.join(", ")}`,
          });
        }
        const aliasOwner = new Map();
        const dupAliases = [];
        for (const [a, v] of declaredAliases) {
          if (aliasOwner.has(a)) dupAliases.push(`${a}（${aliasOwner.get(a)} 与 ${v}）`);
          else aliasOwner.set(a, v);
        }
        if (dupAliases.length) {
          problems.push({
            where: REGISTRY_REL,
            why: `别名跨域重复（后声明者永远选不到）：${dupAliases.join(", ")}`,
          });
        }
        if (!ghostAliasList.length && !shadowAliases.length && !dupAliases.length) {
          info.push(`历史别名 ${declaredAliases.length} 条（声明于 DOMAIN_NODES），无幽灵/遮蔽/重复 ✔`);
        }

        // serde(alias) 是**第二条解析路径**（反序列化不经 `from_str`），故单独校验：
        // 它必须整体包含在声明别名表里 —— 否则会漂移成「serde 能解析、`from_str` 不能」
        // 这类只在部分调用点暴露的隐性契约。
        //
        // ⚠ 本校验此前**只存在于注释里**：`capability.rs` 的枚举文档写着「由门禁校验
        // 『必须出现在别名表里』」，而脚本里 `parseSerdeAliases` **零调用**
        // —— 又一处「文档声称有守卫、实际没有」（判据 #7 的变体）。
        const serdeAliases = parseSerdeAliases(authoritySrc);
        if (serdeAliases === null) {
          problems.push({
            where: AUTHORITY_REL,
            why: "`#[serde(alias)]` 解析失败 —— 枚举块形态已变，**或**有条目取值不合 `[a-z_]`\n" +
              "      （含数字/大写，会被抽取器整条跳过）。两种情况都不得当作「0 条」通过",
          });
        } else if (serdeAliases.length === 0) {
          reports.push({
            where: AUTHORITY_REL,
            why: "枚举上已无 `#[serde(alias)]`。若是有意放弃对存量 `core`/`invest`/`opc` 字符串的兼容，\n" +
              "      请同步删除该文件枚举文档里的相关说明；若无意，则存量 DB 字符串会在反序列化路径静默失败",
          });
        } else {
          const declaredAliasNames = new Set(declaredAliases.map(([a]) => a));
          const missingSerde = serdeAliases.filter((a) => !declaredAliasNames.has(a));
          if (missingSerde.length) {
            problems.push({
              where: AUTHORITY_REL,
              why: `serde(alias) 未出现在 DOMAIN_NODES.aliases 里：${missingSerde.join(", ")}\n` +
                "      （serde 反序列化路径独立于 from_str，两边不一致会让同一字符串只在部分调用点解析成功）",
            });
          } else {
            info.push(`serde(alias) ${serdeAliases.length} 条，均已包含于声明别名表 ✔`);
          }
        }
      }

      siteReport.push({
        site: REGISTRY_REL,
        kind: "DOMAIN_NODES 声明",
        found: `${declaredNodes.length} 个节点`,
        note: `业务域 ${businessNodes.length} + 内部域 ${systemNodes.length}`,
      });
      info.push(`域元数据声明：${declaredNodes.length} 个节点（含导航路径/顺序）`);
    }
  }

  // ── 2. TS 联合类型（应 == 全量域，含内部域）──
  const tsSrc = loadSite(TS_TYPE_REL);
  if (tsSrc !== null) {
    checkSet("TS 联合类型", parseTsUnion(tsSrc), authority, TS_TYPE_REL);
  }

  // ── 3/4. 前端域元数据：集合单点 + 两个派生视图 + 导航顺序（P1-④ 重写）──
  //
  // P1-④ 之前本段读的是**两个手抄数组**：`CAPABILITY_DOMAIN_META` 的 8 个
  // `{id,path,color}` 条目 + `CAPABILITY_DOMAIN_PROTOCOL_ORDER` 的 8 个 id。
  // 同一批 id 出现两次、各自独立手写 ⇒ 集合维度有两个会各自腐烂的手写点，
  // 且数组漏一个元素仍是合法数组（`tsc` 拦不住），只有本门禁事后能守。
  //
  // 现在的形态（`src/lib/domainMeta.ts` 模块头有同一张图）：
  //   `DOMAIN_PRESENTATION`  Record，**key 集合 = 域集合**，受 `tsc` **穷尽检查**
  //     ├─ key 声明顺序 → `CAPABILITY_DOMAIN_PROTOCOL_ORDER`（`Object.keys` 派生）
  //     └─ 值（path/color）→ `CAPABILITY_DOMAIN_META` 的 path/color
  //   `NAV_ORDER`            同一批 id 的**另一种排列**（无法派生，该文件唯一手写点）
  //
  // ⇒ 本段除了集合/顺序/path 比对，还新增两类**防回流**硬拦：
  //   两个派生视图不得写回字面量数组、也不得改从别处派生。否则集合维度会静默
  //   退回「两处手写」——编译通过、运行正常、只比集合也照样绿（两份恰好一致时）。
  const metaSrc = loadSite(META_REL);
  if (metaSrc !== null) {
    const presentation = parsePresentationTable(metaSrc);
    if (presentation === null) {
      problems.push({
        where: META_REL,
        why:
          "DOMAIN_PRESENTATION 解析失败 —— 形态已变（须为 `{ <域 id>: { path, color } }`\n" +
          "      对象字面量并以 `} as const satisfies` 收尾；该 `satisfies` 是「漏一个域 ⇒ tsc 报错」的实现手段）",
      });
    } else {
      checkSet(
        "前端域集合（DOMAIN_PRESENTATION key）",
        presentation.map((e) => e.id),
        business,
        META_REL,
      );
    }

    const navOrder = parseNavOrder(metaSrc);
    checkSet("导航顺序集合（NAV_ORDER）", navOrder, business, `${META_REL} (NAV_ORDER)`);

    // ── 防回流 ①：`PROTOCOL_ORDER` 必须仍是 `Object.keys(DOMAIN_PRESENTATION)` 派生 ──
    const protocolDeriv = parseDerivationShape(metaSrc, "CAPABILITY_DOMAIN_PROTOCOL_ORDER");
    if (protocolDeriv === null) {
      problems.push({
        where: `${META_REL} (PROTOCOL_ORDER)`,
        why: "声明形态解析失败 —— 勿当作通过（本检查防「改回手抄数组」这类静默回流）",
      });
    } else if (protocolDeriv.isLiteralArray) {
      problems.push({
        where: `${META_REL} (PROTOCOL_ORDER)`,
        why:
          "协议顺序被改回**字面量数组**（P1-④ 硬拦）\n" +
          "      该常量必须由 `Object.keys(DOMAIN_PRESENTATION)` 派生：域集合只在\n" +
          "      `DOMAIN_PRESENTATION` 出现一次（且受 tsc 穷尽检查）。写回数组等于恢复\n" +
          "      「同一批 id 两处手写」的旧形态，而编译与运行都不会报错。",
      });
    } else if (protocolDeriv.from !== "DOMAIN_PRESENTATION") {
      problems.push({
        where: `${META_REL} (PROTOCOL_ORDER)`,
        why: `协议顺序必须派生自 DOMAIN_PRESENTATION，实测派生自：${protocolDeriv.from ?? "（无法识别）"}`,
      });
    }

    // ── 防回流 ②：`CAPABILITY_DOMAIN_META` 必须仍是 `NAV_ORDER.map(…)` 派生 ──
    const metaDeriv = parseDerivationShape(metaSrc, "CAPABILITY_DOMAIN_META");
    if (metaDeriv === null) {
      problems.push({
        where: META_REL,
        why: "CAPABILITY_DOMAIN_META 声明形态解析失败 —— 勿当作通过",
      });
    } else if (metaDeriv.isLiteralArray) {
      problems.push({
        where: META_REL,
        why:
          "CAPABILITY_DOMAIN_META 被改回**字面量数组**（P1-④ 硬拦）\n" +
          "      id 应来自 `NAV_ORDER`、path/color 应来自 `DOMAIN_PRESENTATION`",
      });
    } else if (metaDeriv.from !== "NAV_ORDER") {
      problems.push({
        where: META_REL,
        why: `CAPABILITY_DOMAIN_META 必须派生自 NAV_ORDER（导航顺序），实测派生自：${metaDeriv.from ?? "（无法识别）"}`,
      });
    }

    // ── 协议顺序：逐项 == DOMAIN_NODES 声明顺序（P1-② 硬拦）──
    //
    // 升级的依据不是「顺序变重要了」，而是**顺序的定性变了**：
    //   · 升级前：`PROTOCOL_ORDER` 是手抄副本，其顺序**可能是有意的产品决策**（下拉排序）
    //     ⇒ 硬拦会逼人改判据来灭红灯（违反 #147），故只报告。
    //   · 升级后：协议顺序被**定义为** `DOMAIN_NODES` 的声明顺序（= 枚举声明顺序），
    //     且它同时是 L1 路由 / prompt 候选清单 / 下拉项的顺序来源 ⇒ 已无「自由决策」空间。
    //     两侧不一致就是缺陷，而症状**静默**：同一下拉项在两次重构之间来回跳、
    //     prompt 候选序与代码枚举序错位、新增域插在末尾而 prompt 里仍在中间。
    //
    // 比对基准取 `declaredSlugs`（**声明顺序**）而非 `authority`（枚举顺序）：
    // 二者被 1b 段强制相等，但对下游而言「声明」才是它真正引用的上游 ——
    // 拿 `authority` 比等于在比另一件事（漂移方向看不出来）。
    //
    // P1-④ 起被比对的**值**来自呈现表的 key 顺序（= `Object.keys` 的运行结果，
    // 由上面防回流 ① 保证同源）。注意这**不构成同源恒真**：基准是 Rust 声明，
    // 属于外部源 —— 门禁读 TS 侧源码 key 顺序，比的是「TS 源码 vs Rust 源码」。
    if (presentation !== null && declaredSlugs !== null) {
      const protocolOrder = presentation.map((e) => e.id);
      const declaredBusiness = declaredSlugs.filter((d) => d !== systemStr);
      if (protocolOrder.join(",") !== declaredBusiness.join(",")) {
        problems.push({
          where: `${META_REL} (PROTOCOL_ORDER)`,
          why:
            `协议顺序必须逐项等于 domain_registry.rs 的 DOMAIN_NODES 声明顺序（P1-②）\n` +
            `      ⇒ 它 = DOMAIN_PRESENTATION 的 key 顺序，故**调整该表条目顺序即改协议顺序**\n` +
            `      声明顺序：${declaredBusiness.join(", ")}\n` +
            `      实测顺序：${protocolOrder.join(", ")}`,
        });
      } else {
        info.push(`协议顺序逐项等于声明顺序 ✔（${protocolOrder.length} 项）`);
      }
    }

    // ── 导航顺序：`NAV_ORDER` 逐项 == DOMAIN_NODES 按 nav_order 排出的序列（硬拦）──
    //
    // P1-④ 之前这里比的是「每条 `CAPABILITY_DOMAIN_META` 条目的**数组下标**是否等于
    // 该域的 `nav_order`」——那时导航顺序由条目顺序表达。现在条目顺序已被 `NAV_ORDER`
    // 取代（`CAPABILITY_DOMAIN_META` 是它的派生），故基准改为「按声明 `nav_order`
    // 排序后的域序列」。等强，且**额外**能抓 `NAV_ORDER` 内部乱序以外的形态。
    if (navOrder !== null && declaredNodes !== null) {
      const declaredNavSeq = [...declaredNodes]
        .filter((n) => variantToStr.get(n.variant) !== systemStr)
        .sort((a, b) => (a.navOrder ?? 0) - (b.navOrder ?? 0))
        .map((n) => variantToStr.get(n.variant));
      if (navOrder.join(",") !== declaredNavSeq.join(",")) {
        problems.push({
          where: `${META_REL} (NAV_ORDER)`,
          why:
            `导航分组顺序必须逐项等于 DOMAIN_NODES 的 nav_order 顺序\n` +
            `      后端 nav_order 序：${declaredNavSeq.join(", ")}\n` +
            `      实测 NAV_ORDER ：${navOrder.join(", ")}`,
        });
      }
    }

    // ── path：逐值比对 DOMAIN_NODES（硬拦）──
    //
    // **刻意不比对 color** —— 颜色是纯前端呈现决策，后端没有任何渲染路径读它，
    // 把它搬进 Rust 声明只会造出「零读取端字段」（判据 #183）。
    if (presentation !== null && declaredNodes !== null) {
      const byId = new Map(presentation.map((e) => [e.id, e]));
      for (const n of declaredNodes) {
        const slug = variantToStr.get(n.variant);
        // 内部域不进前端元数据（其 nav_path/nav_order 为 None，已由 1b 段校验）
        if (!slug || slug === systemStr) continue;
        const e = byId.get(slug);
        // 集合层面的缺失/多余已由上面的 checkSet 报过，此处不重复报
        if (!e) continue;
        if (e.path !== n.navPath) {
          problems.push({
            where: META_REL,
            why: `域 ${slug} 的 path 与 DOMAIN_NODES 声明不一致：前端 "${e.path}" ≠ 声明 "${n.navPath}"`,
          });
        }
      }
      siteReport.push({
        site: `${META_REL} (path/顺序)`,
        kind: "域路径与顺序",
        found: `${presentation.length} 条`,
        note: "path 逐值比对声明；协议顺序 = 呈现表 key 序、导航顺序 = NAV_ORDER，均逐项比对（color 除外）",
      });
    }
  }

  // ── 4b. i18n key 命名空间：Rust ↔ TS 跨语言字符串契约（P1-③，2026-09-15 新增）──
  //
  // 「域标签的 i18n key」被两侧各算一次：
  //   · 声明端 `domain_registry.rs::DomainNode::label_key()` → `capabilityDomain.<slug>`
  //   · 实际查表端 `src/lib/domainMeta.ts::domainLabelKey()` → 同公式
  //
  // **没有任何类型系统能连接这两者** —— 一侧改了命名空间（或后缀来源），
  // 界面会把域名显示成裸 key（i18n 查不到该 key），而 `cargo` / `tsc` 双双沉默：
  // 两侧各自都是合法代码，只是算出来的字符串不同。⇒ 本段是这条契约的**唯一**守卫。
  //
  // ⚠ 本段是补上一个**假声称**：`domainMeta.ts` 的注释此前写着「门禁**硬拦**本函数的公式被
  // 改成别的命名空间」，而实测门禁里**没有**这条检查（判据：文档声称有守卫 ⇒ 必须回查守卫存在）。
  const rustLabelNs = registrySrc === null ? null : parseLabelKeyFormula(registrySrc);
  const tsLabel = metaSrc === null ? null : parseTsLabelKeyFormula(metaSrc);
  if (registrySrc !== null && rustLabelNs === null) {
    problems.push({
      where: REGISTRY_REL,
      why:
        "`DomainNode::label_key()` 公式解析失败 —— 形态已变（契约见该方法文档「# 形态约束」）。\n" +
        "      勿当作通过：探针 ⑭（真实文件注入）证明本段会拦住命名空间漂移。",
    });
  }
  if (metaSrc !== null && tsLabel === null) {
    problems.push({
      where: META_REL,
      why:
        "`domainLabelKey()` 公式解析失败 —— 形态已变（须为 `return `<命名空间>.${参数}`;`）",
    });
  }
  if (rustLabelNs !== null && tsLabel !== null) {
    if (rustLabelNs !== tsLabel.namespace) {
      problems.push({
        where: `${REGISTRY_REL} ↔ ${META_REL}`,
        why:
          `i18n key 命名空间两侧不一致（**跨语言字符串契约破裂**，P1-③）\n` +
          `      Rust label_key()   ：${rustLabelNs}\n` +
          `      TS domainLabelKey()：${tsLabel.namespace}\n` +
          `      ⇒ 界面显示裸 key（i18n 查不到），且两侧编译器都不会报错`,
      });
    }
    if (!tsLabel.simple || !tsLabel.params.includes(tsLabel.interpolated)) {
      problems.push({
        where: META_REL,
        why:
          `domainLabelKey() 的模板插值 \`\${${tsLabel.interpolated}}\` 不可信` +
          `（须为裸参数名；本函数参数：${tsLabel.params.join(", ") || "无"}）\n` +
          `      ⇒ 算出的 key 与传入的域 id 无关，界面显示裸 key`,
      });
    }
    siteReport.push({
      site: `${REGISTRY_REL} ↔ ${META_REL}`,
      kind: "i18n key 命名空间（跨语言公式）",
      found: `${rustLabelNs} ↔ ${tsLabel.namespace}`,
      note: rustLabelNs === tsLabel.namespace ? "两侧公式一致" : "两侧不一致（已硬拦）",
    });
  }

  // ── 5. L1 分类器 prompt 域清单（P2：已改为**派生** ⇒ 本段改为防回流）──
  //
  // P2 之前这份清单是 `init/state.rs` 里手抄的 9 个 slug，本段逐值 + 逐序比对。
  // P2 起它由 `harness::domain_registry::l1_classifier_domain_list()` 从**域覆盖层**
  // 实时派生（停用域自动从候选集消失）⇒ 源码里已不存在字面量清单可比对。
  //
  // ⚠ 守卫**换位置，不消失**（详见脚本头部「P2 之后：L1 prompt 域清单的守卫迁移」）：
  //   · 「清单 == 9 个域 && 顺序 == 协议顺序」→ Rust 单测对**真实函数输出**断言
  //     （`domain_registry::tests::test_l1_classifier_domain_list_covers_all_when_enabled`，
  //      期望值是独立硬编码的 9 个 slug）。比正则比对源码文本**更强**。
  //   · 本段保留的职责 = **防回流**：谁把字面量清单抄回 `init/state.rs`，谁就被拦。
  const promptSrc = loadSite(PROMPT_REL);
  if (promptSrc !== null) {
    const site = classifyPromptDomainSite(promptSrc);
    if (site.kind === "missing_anchor") {
      // 连锚点都没了 ⇒ 无法判断「清单被删了」还是「prompt 被搬走了」，
      // 只能按解析失败硬拦（判据：解析不出 ≠ 0 违规）。
      problems.push({
        where: PROMPT_REL,
        why:
          `L1 分类器 prompt 的锚点「L1 域路由分类器」在文件里找不到 —— ` +
          `prompt 被搬走了？本段按【解析失败】硬拦，不按 0 违规通过。\n` +
          `      若确实搬了位置，同步更新本脚本的 PROMPT_REL / parsePromptDomainList 锚点。`,
      });
    } else if (site.kind === "literal") {
      problems.push({
        where: PROMPT_REL,
        why:
          `L1 分类器 prompt 的域清单又变回**手抄字面量**了（P2 起必须派生）\n` +
          `      实测清单：${site.list.join(", ")}\n` +
          `      正确写法：调用 axagent_harness::l1_classifier_domain_list()（它读域覆盖层 ⇒\n` +
          `      「停用域」自动从 LLM 候选集消失）。抄回字面量会让「停用域」重新被 LLM 选中，\n` +
          `      而且**编译与运行都不报错** —— 只有本段能拦。`,
      });
    } else if (site.kind === "unknown_entry") {
      problems.push({
        where: PROMPT_REL,
        why:
          `L1 分类器 prompt 既没有字面量清单、也找不到派生入口 ` +
          `\`l1_classifier_domain_list()\`。\n` +
          `      ⇒ 清单来源被换成了别的东西（或入口被改名）—— 改名的同时必须更新本段正则，\n` +
          `      否则这里会长期假绿（本段按【解析失败】硬拦）。`,
      });
    } else {
      siteReport.push({
        site: PROMPT_REL,
        kind: "L1 分类器 prompt 域清单",
        found: "派生",
        note: `调用 ${site.entry}()（读域覆盖层）；完整性/顺序由 domain_registry 单测守`,
      });
    }
  }

  // ── 5b. P2：内置声明不可被「覆盖层」就地改写 ──
  //
  // 覆盖层的正确形态是**并列于内置声明的一层**（`OVERRIDES` + `apply_domain_overrides`），
  // `DOMAIN_NODES` 保持编译期常量。若有人图省事把它改成 `static mut` /
  // `OnceLock<Vec<DomainNode>>` 并在原地改它，两件事同时坏掉：
  //   · 本脚本后续所有「逐值比对内置声明」的断言会开始比对**运行时可变**的东西
  //     （今天绿明天红，且红的时候不知道该信库还是信代码）；
  //   · 「出厂默认」这个概念消失 —— 清空覆盖也回不到初始状态。
  // 注：`registrySrc` 在本函数更早处已加载（声明段），此处直接复用，不重复读盘。
  if (registrySrc !== null) {
    if (!/pub\s+const\s+DOMAIN_NODES\s*:\s*&\[DomainNode\]\s*=\s*&\[/.test(registrySrc)) {
      problems.push({
        where: REGISTRY_REL,
        why:
          `DOMAIN_NODES 的声明形态已变 —— 期望 \`pub const DOMAIN_NODES: &[DomainNode] = &[\`\n` +
          `      （编译期**内置默认**）。覆盖层必须并列于它（见该文件 §运行时覆盖层），\n` +
          `      不得把内置声明改成可变容器：那会让本脚本后续「逐值比对内置声明」的断言\n` +
          `      失去稳定基准，且「出厂默认」不再存在（清空覆盖也回不到初始状态）。`,
      });
    } else {
      siteReport.push({
        site: REGISTRY_REL,
        kind: "覆盖层边界",
        found: "内置声明为 const 切片",
        note: "覆盖层并列于内置默认，未就地改写",
      });
    }
  }

  // ── 6. OPC 映射表目标值（应 ⊆ 全量域）──
  const opcSrc = loadSite(OPC_REL);
  if (opcSrc !== null) {
    const targets = parseOpcMappingTargets(opcSrc);
    if (targets === null) {
      problems.push({
        where: OPC_REL,
        why:
          `OPC 映射表解析失败 —— 期望的常量在文件里找不到或为空：` +
          `${OPC_MAPPING_TABLE_CONSTS.join(", ")}\n` +
          `      最常见原因：**表被改名**（2026-09-15 实测 \`OPC_INDUSTRY_DOMAIN\` → \`OPC_DOMAIN_PACK_DOMAIN\`）。\n` +
          `      ⇒ 改名时同步更新本脚本的 \`OPC_MAPPING_TABLE_CONSTS\`，而不是去改解析正则。`,
      });
    } else {
      const ghost = [...new Set(targets.filter((t) => !authority.includes(t)))];
      if (ghost.length) {
        problems.push({
          where: OPC_REL,
          why: `映射目标域未在权威源声明（会静默落到「未分类」）：${ghost.join(", ")}`,
        });
      }
      siteReport.push({
        site: OPC_REL,
        kind: "OPC 映射目标域",
        found: `${targets.length} 条`,
        note: `去重 ${[...new Set(targets)].length} 个目标域`,
      });
    }
  }

  // ── 7. i18n（每语言的 capabilityDomain key 集合 == 全量域）──
  const localesAbs = join(ROOT, LOCALES_DIR);
  if (!existsSync(localesAbs)) {
    problems.push({ where: LOCALES_DIR, why: "locale 目录不存在" });
  } else {
    const files = readdirSync(localesAbs).filter((f) => f.endsWith(".json"));
    if (files.length === 0) {
      problems.push({ where: LOCALES_DIR, why: "一个 locale 文件都没扫到" });
    }
    let bad = 0;
    for (const f of files) {
      const rel = `${LOCALES_DIR}/${f}`;
      const text = readText(rel);
      readCount++;
      const keys = text === null ? null : parseI18nDomainKeys(text);
      if (keys === null) {
        problems.push({ where: rel, why: "capabilityDomain 命名空间缺失或解析失败" });
        bad++;
        continue;
      }
      const d = diffSets(keys, authority);
      if (d.missing.length || d.extra.length) {
        problems.push({
          where: rel,
          why:
            `capabilityDomain key 与权威源不一致` +
            (d.missing.length ? `｜缺 ${d.missing.join(", ")}` : "") +
            (d.extra.length ? `｜多 ${d.extra.join(", ")}` : ""),
        });
        bad++;
      }
    }
    siteReport.push({
      site: `${LOCALES_DIR}/*.json`,
      kind: "i18n capabilityDomain key",
      found: `${files.length} 个语言文件`,
      note: bad === 0 ? "全部与权威源一致" : `${bad} 个不一致`,
    });
  }

  // ── 报告档：同名不同义的命名空间（需产品裁决，不拦）──
  const zh = readText(`${LOCALES_DIR}/zh-CN.json`);
  if (zh !== null) {
    try {
      const ns = Object.keys(JSON.parse(zh)?.domain ?? {});
      const labelKeys = ns.filter((k) => k !== "description" && k !== "empty");
      const camel = labelKeys.filter((k) => k !== k.toLowerCase());
      if (camel.length) {
        reports.push({
          where: `${LOCALES_DIR}/zh-CN.json`,
          why:
            `\`domain.*\` 命名空间用 camelCase key（${camel.join(", ")}），而权威域 id 是 snake_case\n      ⇒ 同一批域标签存在两套 key（\`capabilityDomain.<snake>\` 与 \`domain.<camel>\`），见 PLAN-domain-single-source.md §4`,
        });
      }
    } catch {
      /* 报告档解析失败不拦 */
    }
  }

  const mock = readText("src/lib/browserMock.ts");
  if (mock !== null && /domainRules/.test(mock)) {
    reports.push({
      where: "src/lib/browserMock.ts",
      why: "`domainRules` 是浏览器模式的 mock 副本（可接受，但**不随权威源自动更新**，改域时勿忘）",
    });
  }

  // ── 输出 ──
  if (readCount === 0) {
    console.error("✖ 扫描面为 0：一个文件都没读到 ⇒ 脚本自身失效");
    return 2;
  }

  if (JSON_OUT) {
    console.log(
      JSON.stringify({ authority, business, system: systemStr, sites: siteReport, problems, reports }, null, 2),
    );
  } else {
    console.log("── 权威源 ──");
    for (const l of info) console.log(`   ${l}`);
    console.log("\n── 副本站点 ──");
    for (const s of siteReport) console.log(`   ${s.site}\n     ${s.kind}｜${s.found}｜${s.note}`);
    if (LIST) {
      console.log("\n── --list：完整清单 ──");
      console.log(`   权威域：${authority.join(", ")}`);
      console.log(`   业务域：${business.join(", ")}`);
    }
    if (reports.length) {
      console.log("\n── 报告档（不拦，需人判）──");
      for (const r of reports) console.log(`   ⚠ ${r.where}\n     ${r.why}`);
    }
    console.log(`\n扫描面：读取文件 ${readCount} 个｜站点 ${siteReport.length} 个`);
    if (problems.length === 0) {
      console.log("\n✅ 能力域单一真相源：全部副本与权威源一致");
    } else {
      console.log(`\n✖ ${problems.length} 处不一致（硬拦）`);
      for (const p of problems) console.log(`   ${p.where}\n     ${p.why}`);
    }
  }

  if (problems.length) return 1;
  if (reports.length && CI) return 0;
  return 0;
}

// ══ selftest ══════════════════════════════════════════════════════════

function selftest() {
  const results = [];
  const t = (name, fn) => {
    try {
      const r = fn();
      results.push({ name, ok: r === true, detail: r === true ? "" : String(r) });
    } catch (e) {
      results.push({ name, ok: false, detail: `抛错：${e.message}` });
    }
  };

  // 正样本片段（与真实文件形态一致，但独立于磁盘 —— selftest 不依赖仓库状态）
  const AUTH = `
impl CapabilityDomain {
    pub fn as_str(&self) -> &'static str {
        match self {
            CapabilityDomain::General => "general",
            CapabilityDomain::Finance => "finance",
            CapabilityDomain::System => "system",
        }
    }
    pub fn is_system(&self) -> bool {
        matches!(self, CapabilityDomain::System)
    }
}
impl std::str::FromStr for CapabilityDomain {
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s.to_lowercase().as_str() {
            "general" => CapabilityDomain::General,
            "core" => CapabilityDomain::General,
            _ => return Err(()),
        })
    }
}`;

  t("parseAuthorityPairs 正：抽到 (变体,字符串) 对", () => {
    const p = parseAuthorityPairs(AUTH);
    return p && p.length === 3 && p[1][0] === "Finance" && p[1][1] === "finance"
      ? true
      : `得到 ${JSON.stringify(p)}`;
  });
  t("parseAuthorityPairs 负：无 as_str ⇒ null（不得静默返 []）", () => {
    const p = parseAuthorityPairs("pub fn other() {}");
    return p === null ? true : `得到 ${JSON.stringify(p)}`;
  });
  t("★parseAuthorityPairs 不被同文件其它类型的 as_str 误导（锚点须 impl 限定）", () => {
    // 真实回归：capability.rs 里 `pub fn as_str` 出现 10 次，首跑误取了另一个类型的实现
    const withDecoy = `impl CapabilityKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            CapabilityKind::Tool => "tool",
            CapabilityKind::Skill => "skill",
            CapabilityKind::Template => "template",
        }
    }
}
${AUTH}`;
    const p = parseAuthorityPairs(withDecoy);
    return p && p.length === 3 && p[1][1] === "finance"
      ? true
      : `得到 ${JSON.stringify(p)} —— 锚点未做 impl 限定，取了诱饵类型`;
  });
  t("parseSystemVariant 正：识别内部域变体", () => {
    return parseSystemVariant(AUTH) === "System" ? true : "未识别 System";
  });
  t("parseSerdeAliases 正：抽枚举上的 serde(alias)", () => {
    const src =
      `pub enum CapabilityDomain {\n    #[serde(alias = "core")]\n    General,\n    #[serde(alias = "invest")]\n    Finance,\n}`;
    const v = parseSerdeAliases(src);
    return v && v.length === 2 && v[0] === "core" && v[1] === "invest"
      ? true
      : `得到 ${JSON.stringify(v)}`;
  });
  t("parseSerdeAliases 负：无 enum ⇒ null（不得当作「0 条」通过）", () => {
    return parseSerdeAliases("pub struct X;") === null ? true : "应返回 null";
  });
  t("★parseSerdeAliases 负：含数字/大写的条目 ⇒ null（不得静默跳过该条）", () => {
    // 回归钉子：值字符类 `[a-z_]+` 匹配不上 `core_v2` / `coreX` 时会**整条跳过**，
    // 门禁只看到剩余条目并报绿 —— 实测正是这个形态让探针 ⑬ 拿到 `drift_exit=0` 假失败。
    const src =
      `pub enum CapabilityDomain {\n    #[serde(alias = "core")]\n    General,\n    #[serde(alias = "core_v2")]\n    Finance,\n}`;
    const v = parseSerdeAliases(src);
    return v === null ? true : `得到 ${JSON.stringify(v)}`;
  });
  // parseDeclaredAliases 的用例依赖合成常量 REG，故置于 REG 定义之后（见「域元数据声明」段）。
  t("parseTsUnion 正：抽到联合类型值", () => {
    const v = parseTsUnion('export type CapabilityDomain =\n  | "general"\n  | "finance";');
    return v && v.join(",") === "general,finance" ? true : `得到 ${JSON.stringify(v)}`;
  });
  t("parseTsUnion 负：无该类型 ⇒ null", () => {
    return parseTsUnion("export type Other = 1;") === null ? true : "应返回 null";
  });
  // ── 2026-09-15 P1-④：域集合单点（`DOMAIN_PRESENTATION`）+ 两个派生视图 ──
  //
  // 合成源码刻意与 `domainMeta.ts` **同形**（含 dprint 折行后的 `Object.keys(\n X,\n)`）：
  // 合成输入与真实文件不同形，测试通过就没有意义（本仓已踩过：合成的 `REG` 缺
  // 新增的 `aliases` 字段 ⇒ 3 条用例假红）。
  const PRES = `
const DOMAIN_PRESENTATION = {
  general: { path: "/general", color: "#8c8c8c" },
  finance: { path: "/finance", color: "#d4380d" },
} as const satisfies Record<BusinessDomain, { readonly path: string; readonly color: string }>;
const NAV_ORDER = [
  "finance",
  "general",
] as const satisfies readonly BusinessDomain[];
export const CAPABILITY_DOMAIN_PROTOCOL_ORDER: readonly CapabilityDomain[] = Object.keys(
  DOMAIN_PRESENTATION,
) as BusinessDomain[];
export const CAPABILITY_DOMAIN_META: readonly CapabilityDomainMeta[] = NAV_ORDER.map((id) => ({
  id,
  path: DOMAIN_PRESENTATION[id].path,
  color: DOMAIN_PRESENTATION[id].color,
}));`;

  t("parsePresentationTable 正：抽 (id, path)，顺序 = 声明顺序", () => {
    const v = parsePresentationTable(PRES);
    return v &&
        v.length === 2 &&
        v[0].id === "general" &&
        v[0].path === "/general" &&
        v[1].id === "finance" &&
        v[1].path === "/finance"
      ? true
      : `得到 ${JSON.stringify(v)}`;
  });
  t("parsePresentationTable 负：常量不存在 ⇒ null", () => {
    return parsePresentationTable("const X = [];") === null ? true : "应返回 null";
  });
  t("★parsePresentationTable 负：缺 `} as const satisfies` 收尾 ⇒ null（防形态漂移）", () => {
    // 少了 `satisfies` 就等于少了「漏一个域 tsc 报错」的实现手段 ⇒ 必须红。
    const bad = PRES.replace("} as const satisfies", "};");
    return parsePresentationTable(bad) === null ? true : "应返回 null";
  });
  t("★parsePresentationTable 漏条检测：条目缺 color ⇒ null，不得静默少一条", () => {
    const bad = PRES.replace('{ path: "/finance", color: "#d4380d" }', '{ path: "/finance" }');
    return parsePresentationTable(bad) === null ? true : "应返回 null（否则会把 2 个域报成 1 个）";
  });
  t("★parsePresentationTable 重复 key ⇒ null（对象字面量里后者胜出，源码里两处都数得到）", () => {
    const bad = PRES.replace(
      '  general: { path: "/general", color: "#8c8c8c" },',
      '  general: { path: "/general", color: "#8c8c8c" },\n  general: { path: "/g2", color: "#000" },',
    );
    const v = parsePresentationTable(bad);
    return v === null ? true : `应返回 null，得到 ${JSON.stringify(v)}`;
  });

  t("parseNavOrder 正：抽导航顺序序列", () => {
    const v = parseNavOrder(PRES);
    return v && v.join(",") === "finance,general" ? true : `得到 ${JSON.stringify(v)}`;
  });
  t("parseNavOrder 负：常量不存在 ⇒ null", () => {
    return parseNavOrder("const X = [];") === null ? true : "应返回 null";
  });

  t("parseDerivationShape：字面量数组 ⇒ isLiteralArray=true", () => {
    const v = parseDerivationShape(PRES, "NAV_ORDER");
    return v && v.isLiteralArray === true && v.from === null ? true : `得到 ${JSON.stringify(v)}`;
  });
  t("★parseDerivationShape：`Object.keys(X)` 跨行形态 ⇒ from=X（dprint 会折行）", () => {
    const v = parseDerivationShape(PRES, "CAPABILITY_DOMAIN_PROTOCOL_ORDER");
    return v && v.isLiteralArray === false && v.from === "DOMAIN_PRESENTATION"
      ? true
      : `得到 ${JSON.stringify(v)}`;
  });
  t("★parseDerivationShape：`X.map(...)` ⇒ from=X", () => {
    const v = parseDerivationShape(PRES, "CAPABILITY_DOMAIN_META");
    return v && v.isLiteralArray === false && v.from === "NAV_ORDER"
      ? true
      : `得到 ${JSON.stringify(v)}`;
  });
  t("★parseDerivationShape 能判出「派生改回手抄数组」（P1-④ 防回流的判别力）", () => {
    const bad = PRES.replace(
      /export const CAPABILITY_DOMAIN_PROTOCOL_ORDER[\s\S]*?as BusinessDomain\[\];/,
      'export const CAPABILITY_DOMAIN_PROTOCOL_ORDER: readonly CapabilityDomain[] = [\n  "general",\n  "finance",\n];',
    );
    const v = parseDerivationShape(bad, "CAPABILITY_DOMAIN_PROTOCOL_ORDER");
    return v && v.isLiteralArray === true ? true : `得到 ${JSON.stringify(v)}`;
  });
  t("★parseDerivationShape：改从别处派生 ⇒ from 可判出", () => {
    const bad = PRES.replace(
      "Object.keys(\n  DOMAIN_PRESENTATION,\n)",
      "Object.keys(\n  NAV_ORDER,\n)",
    );
    const v = parseDerivationShape(bad, "CAPABILITY_DOMAIN_PROTOCOL_ORDER");
    return v && v.from === "NAV_ORDER" ? true : `得到 ${JSON.stringify(v)}`;
  });
  t("parseDerivationShape 负：常量不存在 ⇒ null（不得当作「形态没问题」）", () => {
    return parseDerivationShape("const X = [];", "NOPE") === null ? true : "应返回 null";
  });
  t("parsePromptDomainList 正：跨续行符抽出域清单", () => {
    const src = `你是 L1 域路由分类器。根据用户输入，从以下业务域标识中选最匹配的一个，\\
               只输出该标识，不要解释、不要引号、不要标点：\\
                general, finance, system";`;
    const v = parsePromptDomainList(src);
    return v && v.join(",") === "general,finance,system" ? true : `得到 ${JSON.stringify(v)}`;
  });
  t("parsePromptDomainList 负：无锚点 ⇒ null", () => {
    return parsePromptDomainList("const SYS = \"foo\";") === null ? true : "应返回 null";
  });

  // ── P2：prompt 站点形态判定（4 个分支各一条）──
  // 样本刻意与 `init/state.rs` 的真实形态同形（含续行符 `\` 与 `{}` 占位符）。
  const PROMPT_DERIVED = `let sys = format!(
                "你是 L1 域路由分类器。根据用户输入，从以下业务域标识中选最匹配的一个，\\
                 只输出该标识，不要解释、不要引号、不要标点：{}",
                axagent_harness::l1_classifier_domain_list()
            );`;
  const PROMPT_LITERAL = `const SYS: &str = "你是 L1 域路由分类器。根据用户输入，从以下业务域标识中选最匹配的一个，\\
                只输出该标识，不要解释、不要引号、不要标点：\\
                general, devops, ai_media";`;

  t("★classifyPromptDomainSite：派生形态 ⇒ derived（P2 的正确形态）", () => {
    const v = classifyPromptDomainSite(PROMPT_DERIVED);
    return v.kind === "derived" && v.entry === "l1_classifier_domain_list"
      ? true
      : `得到 ${JSON.stringify(v)}`;
  });
  t("★classifyPromptDomainSite：抄回字面量 ⇒ literal（防回流）", () => {
    const v = classifyPromptDomainSite(PROMPT_LITERAL);
    return v.kind === "literal" && v.list.join(",") === "general,devops,ai_media"
      ? true
      : `得到 ${JSON.stringify(v)}`;
  });
  t("classifyPromptDomainSite 负：锚点缺失 ⇒ missing_anchor（不得当作通过）", () => {
    const v = classifyPromptDomainSite("const SYS = \"foo\";");
    return v.kind === "missing_anchor" ? true : `得到 ${JSON.stringify(v)}`;
  });
  t("classifyPromptDomainSite 负：无清单也找不到派生入口 ⇒ unknown_entry", () => {
    // 有锚点、无字面量清单、也没调用派生入口 = 清单来源被换掉（典型：入口改名）
    const src = "你是 L1 域路由分类器：{}  axagent_harness::some_other_list()";
    const v = classifyPromptDomainSite(src);
    return v.kind === "unknown_entry" ? true : `得到 ${JSON.stringify(v)}`;
  });
  t("classifyPromptDomainSite：真实文件形态 = derived（回归：P2 落地后本段不该红）", () => {
    const real = loadSite(PROMPT_REL);
    if (real === null) return "文件读不到";
    const v = classifyPromptDomainSite(real);
    return v.kind === "derived" ? true : `真实文件被判为 ${v.kind}：${JSON.stringify(v)}`;
  });
  t("parseOpcMappingTargets 正：取每对第二个元素", () => {
    const src = `const OPC_CAPABILITY_PACK_DOMAIN: &[(&str, &str)] = &[\n    ("accounting", "finance"),\n    ("security", "devops"),\n];\nconst OPC_WF_SEGMENT_DOMAIN: &[(&str, &str)] = &[\n    ("fin", "finance"),\n];`;
    const v = parseOpcMappingTargets(src);
    return v && v.join(",") === "finance,devops,finance" ? true : `得到 ${JSON.stringify(v)}`;
  });
  t("parseOpcMappingTargets 负：少一张表 ⇒ null（不得只报半张）", () => {
    const v = parseOpcMappingTargets('const OPC_CAPABILITY_PACK_DOMAIN = &[("a", "finance")];');
    return v === null ? true : `得到 ${JSON.stringify(v)}`;
  });
  t("parseI18nDomainKeys 正：取命名空间 key", () => {
    const v = parseI18nDomainKeys('{"capabilityDomain":{"general":"通用","finance":"金融"}}');
    return v && v.join(",") === "general,finance" ? true : `得到 ${JSON.stringify(v)}`;
  });
  t("parseI18nDomainKeys 负：命名空间缺失 ⇒ null", () => {
    return parseI18nDomainKeys('{"other":{}}') === null ? true : "应返回 null";
  });
  t("parseI18nDomainKeys 负：非法 JSON ⇒ null（不抛）", () => {
    return parseI18nDomainKeys("{oops") === null ? true : "应返回 null";
  });

  // ── 2026-09-15 新增：域元数据声明 ──
  //
  // ⚠ 本合成串必须与被解析文件的**真实形态**同形（含 `aliases`）——
  // 2026-09-15 给 `parseDomainNodes` 加上 `aliases` 必需字段后忘了同步这里，
  // 于是 3 个用例报「解析出 null」：**合成输入比真实文件更严或更松，都是假信号**。
  const REG = `
pub const DOMAIN_NODES: &[DomainNode] = &[
    DomainNode {
        domain: CapabilityDomain::General,
        nav_path: Some("/general"),
        nav_order: Some(0),
        aliases: &["core", "device"],
    },
    DomainNode {
        domain: CapabilityDomain::System,
        nav_path: None,
        nav_order: None,
        aliases: &[],
    },
];`;

  t("parseDomainNodes 正：抽到变体 + nav_path + nav_order", () => {
    const v = parseDomainNodes(REG);
    return v &&
        v.length === 2 &&
        v[0].variant === "General" &&
        v[0].navPath === "/general" &&
        v[0].navOrder === 0 &&
        v[1].navPath === null &&
        v[1].navOrder === null
      ? true
      : `得到 ${JSON.stringify(v)}`;
  });
  t("★parseDomainNodes 负：字段形态变了 ⇒ null（不得跳过该条）", () => {
    // 回归：若解析器对形态不符的条目「跳过继续」，漏声明一个域就会退化成「少一行」而静默通过
    const broken = REG.replace("nav_order: Some(0),", "order: 0,");
    return parseDomainNodes(broken) === null ? true : `得到 ${JSON.stringify(parseDomainNodes(broken))}`;
  });
  t("parseDomainNodes 负：无 DOMAIN_NODES ⇒ null", () => {
    return parseDomainNodes("pub const OTHER: &[u8] = &[];") === null ? true : "应返回 null";
  });
  t("parseDeclaredAliases 正：从声明抽 (别名 → 变体)", () => {
    const a = parseDeclaredAliases(REG);
    return a && a.length === 2 && a[0][0] === "core" && a[0][1] === "General" && a[1][0] === "device"
      ? true
      : `得到 ${JSON.stringify(a)}`;
  });
  t("★parseDeclaredAliases 负：一条别名都没有 ⇒ null（不得返回空数组）", () => {
    // 回归钉子：别名迁出 `FromStr` 后，旧实现（`parseAliasTargets`）正则会匹配到 0 条并
    // 返回 `[]`，主流程据此打印「历史别名 0 条 ✔」+ exit 0 —— fail-open 假通过（判据 #7/#216）。
    // 本用例锁死「0 条 ⇒ null ⇒ 主流程硬拦」这条契约。
    const noAlias = REG.replace(/aliases:\s*&\[[^\]]*\]/g, "aliases: &[]");
    const v = parseDeclaredAliases(noAlias);
    return v === null ? true : `得到 ${JSON.stringify(v)}`;
  });
  t("parseDeclaredAliases 负：声明解析不出 ⇒ null", () => {
    return parseDeclaredAliases("pub const OTHER: &[u8] = &[];") === null ? true : "应返回 null";
  });
  // ⚠ 此处曾有 `parseMetaEntries` 的三条用例（id/path + **数组下标** + `order` 字段复活探测）。
  // 2026-09-15 P1-④ 把 `CAPABILITY_DOMAIN_META` 从字面量数组改为**派生视图**后，
  // 其源码里已没有 `{ id, path, color }` 条目，该解析器随之删除 —— 连同「`order` 字段
  // 复活」这条防回归一起。字段已无处可加；防回归改由 `parseDerivationShape` 承担，
  // 它拦的是「派生视图写回手抄数组」这一整类回流（覆盖面更大，不只是 order 一个字段）。

  // ── P1-③：跨语言 i18n key 公式（Rust `label_key()` ↔ TS `domainLabelKey()`）──
  const LABEL_RS = `
impl DomainNode {
    pub fn slug(&self) -> &'static str {
        self.domain.as_str()
    }
    pub fn label_key(&self) -> String {
        format!("capabilityDomain.{}", self.slug())
    }
}`;
  t("parseLabelKeyFormula 正：抽 Rust 侧命名空间", () => {
    const ns = parseLabelKeyFormula(LABEL_RS);
    return ns === "capabilityDomain." ? true : `得到 ${JSON.stringify(ns)}`;
  });
  t("parseLabelKeyFormula 负：无该函数 ⇒ null（不得当作「无违规」）", () => {
    return parseLabelKeyFormula("impl DomainNode { pub fn slug(&self) {} }") === null
      ? true
      : "应返回 null";
  });
  t("★parseLabelKeyFormula：命名空间含大写/数字照抽（不吃 [a-z_]+ 的亏）", () => {
    // 本仓已两次踩「值含大写/数字被字符类静默跳过」（`communicationX` / `core_v2`）。
    // `capabilityDomain` 本身就含大写 —— 若解析器用小写字符类，**真实文件**会解析不出，
    // 而 selftest 若只用小写样本就永远发现不了。
    const ns = parseLabelKeyFormula(LABEL_RS.replace("capabilityDomain.", "CapabilityDomain.v2_"));
    return ns === "CapabilityDomain.v2_" ? true : `得到 ${JSON.stringify(ns)}`;
  });
  t("★parseLabelKeyFormula：改成 push_str 拼接 ⇒ null（形态约束不可绕）", () => {
    const src = LABEL_RS.replace(
      'format!("capabilityDomain.{}", self.slug())',
      'let mut s = String::from("capabilityDomain."); s.push_str(self.slug()); s',
    );
    return parseLabelKeyFormula(src) === null ? true : "应返回 null（否则漏掉形态漂移）";
  });
  t("parseTsLabelKeyFormula 正：命名空间 + 插值变量 + 参数名", () => {
    const src = "export function domainLabelKey(id: CapabilityDomain): string {\n  return `capabilityDomain.${id}`;\n}";
    const v = parseTsLabelKeyFormula(src);
    return v && v.namespace === "capabilityDomain." && v.interpolated === "id" && v.params.includes("id")
      ? true
      : `得到 ${JSON.stringify(v)}`;
  });
  t("parseTsLabelKeyFormula 负：无该函数 ⇒ null", () => {
    return parseTsLabelKeyFormula("export const x = 1;") === null ? true : "应返回 null";
  });
  t("★端到端：两侧命名空间不同 ⇒ 可判出（用真实解析结果驱动）", () => {
    const rsNs = parseLabelKeyFormula(LABEL_RS);
    const ts = parseTsLabelKeyFormula(
      "export function domainLabelKey(id: CapabilityDomain): string {\n  return `domainLabel.${id}`;\n}",
    );
    if (!rsNs || !ts) return `解析失败 rs=${JSON.stringify(rsNs)} ts=${JSON.stringify(ts)}`;
    return rsNs !== ts.namespace
      ? true
      : `两侧比较失去判别力：Rust ${rsNs} 与 TS ${ts.namespace} 竟判为一致`;
  });
  t("★端到端：两侧同名 ⇒ 判为一致（不得恒报错）", () => {
    const rsNs = parseLabelKeyFormula(LABEL_RS);
    const ts = parseTsLabelKeyFormula(
      "export function domainLabelKey(id: CapabilityDomain): string {\n  return `capabilityDomain.${id}`;\n}",
    );
    return rsNs && ts && rsNs === ts.namespace
      ? true
      : `得到 rs=${JSON.stringify(rsNs)} ts=${JSON.stringify(ts)}`;
  });
  t("★parseTsLabelKeyFormula：复合插值（非裸参数）⇒ simple=false 且可检出", () => {
    const v = parseTsLabelKeyFormula(
      "export function domainLabelKey(id: CapabilityDomain): string {\n  return `capabilityDomain.${globalThis.__d}`;\n}",
    );
    if (!v) return "解析失败（复合插值应被取出后判定，而不是让解析器整体失败）";
    return v.simple === false && !v.params.includes(v.interpolated)
      ? true
      : `判别力不足：simple=${v.simple} 插值=${v.interpolated} 参数=[${v.params.join(", ")}]`;
  });
  t("★parseTsLabelKeyFormula：裸参数插值 ⇒ simple=true（不得恒 false，否则正例会误报）", () => {
    const v = parseTsLabelKeyFormula(
      "export function domainLabelKey(id: CapabilityDomain): string {\n  return `capabilityDomain.${id}`;\n}",
    );
    return v && v.simple === true ? true : `得到 ${JSON.stringify(v)}`;
  });
  t("★端到端：DOMAIN_NODES 漏声明一个域 ⇒ 检出缺失", () => {
    const authority = parseAuthorityPairs(AUTH).map(([, s]) => s);
    const nodes = parseDomainNodes(REG); // 只有 General + System，缺 Finance
    const variantToStr = new Map(parseAuthorityPairs(AUTH));
    const declaredSlugs = nodes.map((n) => variantToStr.get(n.variant)).filter(Boolean);
    const d = diffSets(declaredSlugs, authority);
    return d.missing.join(",") === "finance" ? true : `得到 ${JSON.stringify(d)}`;
  });
  t("★端到端：声明顺序与 as_str() 顺序不同 ⇒ 顺序比较能判出", () => {
    // 用真实解析结果驱动（而非比较两个字面量）：REG 的声明顺序是 general→system，
    // 权威源是 general→finance→system ⇒ 顺序比较**必须**判为不一致。
    const authority = parseAuthorityPairs(AUTH).map(([, s]) => s);
    const variantToStr = new Map(parseAuthorityPairs(AUTH));
    const declared = parseDomainNodes(REG).map((n) => variantToStr.get(n.variant));
    const mismatch = declared.join(",") !== authority.join(",");
    return mismatch === true
      ? true
      : `顺序比较失去判别力：声明 ${declared.join(",")} 竟与权威源 ${authority.join(",")} 判为一致`;
  });
  t("★端到端：PROTOCOL_ORDER「集合相同但顺序不同」⇒ 可判出（P1-②）", () => {
    // **刻意构造成等集合、反顺序** —— 若两边集合本就不同（长度不等），
    // `join() !==` 必然为真，断言恒真、毫无判别力（判据 #174 的原型）。
    // 故先断言「集合相同」，把用例自身有效性钉住，再验顺序比较。
    const variantToStr = new Map(parseAuthorityPairs(AUTH));
    const systemStr = variantToStr.get(parseSystemVariant(AUTH));
    const REG_ORDERED = `pub const DOMAIN_NODES: &[DomainNode] = &[
    DomainNode { domain: CapabilityDomain::Finance, nav_path: Some("/finance"), nav_order: Some(0), aliases: &[] },
    DomainNode { domain: CapabilityDomain::General, nav_path: Some("/general"), nav_order: Some(1), aliases: &[] },
    DomainNode { domain: CapabilityDomain::System, nav_path: None, nav_order: None, aliases: &[] },
];`;
    const declaredBusiness = parseDomainNodes(REG_ORDERED)
      .map((n) => variantToStr.get(n.variant))
      .filter((d) => d !== systemStr);
    const front = ["general", "finance"];
    if ([...front].sort().join(",") !== [...declaredBusiness].sort().join(",")) {
      return `用例自身构造错误：集合不同（front=${front.join(",")} declared=${declaredBusiness.join(",")}）⇒ 断言会恒真`;
    }
    return front.join(",") !== declaredBusiness.join(",")
      ? true
      : `顺序比较失去判别力：${front.join(",")} 竟与 ${declaredBusiness.join(",")} 判为一致`;
  });

  // ★ 比较器必须能判失败
  t("★diffSets 能判「缺值」", () => {
    const d = diffSets(["a"], ["a", "b"]);
    return d.missing.join(",") === "b" && d.extra.length === 0 ? true : `得到 ${JSON.stringify(d)}`;
  });
  t("★diffSets 能判「多值（幽灵域）」", () => {
    const d = diffSets(["a", "zzz"], ["a"]);
    return d.extra.join(",") === "zzz" && d.missing.length === 0 ? true : `得到 ${JSON.stringify(d)}`;
  });
  t("★diffSets 全等 ⇒ 无差异（不得恒报错）", () => {
    const d = diffSets(["a", "b"], ["b", "a"]);
    return d.missing.length === 0 && d.extra.length === 0 ? true : "集合相同却报差异";
  });

  // 端到端（合成）：站点缺一个域 ⇒ 必须被检出
  t("端到端：站点少一个域 ⇒ 检出 missing", () => {
    const authority = parseAuthorityPairs(AUTH).map(([, s]) => s);
    const site = parseTsUnion('export type CapabilityDomain = | "general" | "finance";');
    const d = diffSets(site, authority);
    return d.missing.join(",") === "system" ? true : `得到 ${JSON.stringify(d)}`;
  });
  t("端到端：站点多一个域 ⇒ 检出 extra", () => {
    const authority = parseAuthorityPairs(AUTH).map(([, s]) => s);
    const site = parseTsUnion('export type CapabilityDomain = | "general" | "finance" | "system" | "ops";');
    const d = diffSets(site, authority);
    return d.extra.join(",") === "ops" ? true : `得到 ${JSON.stringify(d)}`;
  });
  t("端到端：OPC 映射目标拼错 ⇒ 检出幽灵域", () => {
    const authority = parseAuthorityPairs(AUTH).map(([, s]) => s);
    const targets = parseOpcMappingTargets('const OPC_CAPABILITY_PACK_DOMAIN = &[("a", "finace")];\nconst OPC_WF_SEGMENT_DOMAIN = &[("b", "finance")];');
    const ghost = [...new Set(targets.filter((x) => !authority.includes(x)))];
    return ghost.join(",") === "finace" ? true : `得到 ${JSON.stringify(ghost)}`;
  });

  const failed = results.filter((r) => !r.ok);
  for (const r of results) {
    console.log(`${r.ok ? "  ✔" : "  ✖"} ${r.name}${r.ok ? "" : `\n      ${r.detail}`}`);
  }
  console.log(`\nselftest：${results.length - failed.length} passed / ${failed.length} failed`);
  return failed.length === 0 ? 0 : 1;
}

process.exit(SELFTEST ? selftest() : main());
