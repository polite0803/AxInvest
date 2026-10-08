#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-only
// 「逐档分支」的**档内纯度门**（#7 的 ②与④，PLAN §七十九 A3，2026-10-06）。
//
// ## 要拦什么
// R-11 的裁定是「一次分析产四档结论，每档只吃本档证据」。这件事有两个泄漏面：
//   ④ **变量面**：`pm-h-<档>` 分支节点（v135 起在档子模板 builder 里）或它 `include_str!` 的
//      `portfolio-mgr-h-<档>.rhai` 里读到**别的档**的东西 —— 评分节点、风险档节点、
//      解禁窗口、逐档分支 JSON、分析师逐档实例 id。任一处串档，两档结论就会恒等或互相污染，
//      而面板上四格仍各自显示 ⇒ 检不出来就是假独立。
//   ② **就绪面**：PLAN 早期写「四路同批起跑」，改图前实际是四个评分节点自身串成链
//      （`t-scoring → hour/week → month → quarter`）。v135（B-2b #36）把每档的评分节点搬进
//      各自的档子模板后**这条链在物理上断开**、四档真并行（§九十一(5) 决策点 3 自行拍定：
//      接受断开；代价是季线重复聚合月线，属运行时长而不是口径 —— 时长读数被 #48 挡着，
//      现在猜没有证据）。于是 **R3 从「错峰边逐条登记」改判为「主图与档模板里都不得出现
//      档间评分依赖」**：空登记表就是本批登记的决定本身，偷偷加回一条 ⇒ 红。
//      口径的两条判据不变：每档评分来源互不相同（R1/R5）、逐档块不读他档（R2）。
//
// ## 形状面（v135 第二步：双形 → 只认新形）
// §九十四 把门扩成「主图 + 档模板双形可认 + 过渡期逐字一致」。本批主图那份已删 ⇒ 收紧：
// - `branchBlocks` 只看档模板 builder（旧形的读取面退役，函数不再吃主图的块）；
// - 新增 **R6**：主图里 `pm-h-*` 的 CodeNode 块数必须为 **0**，且四个 `pm-h-<档>` 必须各以
//   `SubWorkflowNode` 形态存在（旧形残留＝同 id 两份定义，正是过渡期靠 `transitionDrift`
//   钉的那个状态；现在它必须是零）。
//
// ## 规则（全部现场推导，不手抄档名）
// - 档名集合取自 `harness::holding_period.rs` 里 `Period::as_str` 的 match **臂右值**
//   （按 `=>` 切臂、只认右值 —— 邻域法会被相邻臂串味）。断言恰好 4 个。
// - 四个 `t-scoring-*` 节点 id 从种子的 `tool_node(` 字面量现场收集，断言 4 个。
// - 每档「本档评分节点」= 该块 `("tier_score", "<节点>.result…")` 里的那个节点。
// - 扫描面**按域限定**：只扫四个 `pm-h-<档>` 块 + 四份 `portfolio-mgr-h-<档>.rhai`。
//   `pm-arbiter` / `portfolio-mgr` 这类合法跨档消费者**不在扫描面内** ⇒ 不需要豁免表。
// - 注释不算代码：Rhai/Rust 的 `//` 段先剥掉（文档里写「本档不是 mid」不该红，代码读 `--mid` 才该红）。
// - 档名归属取**最长命中**：`ultra_short` 里含 `short`，`cls-risk-level-ultra-short` 里也含
//   `-short` ⇒ 若按子串命中就会把超短档自己判成串到短档。规则是 token === 档名 或以
//   `_`/`-` + 档名 结尾，多个命中取档名最长者。
//
// 用法：node scripts/check-tier-purity.mjs [--selftest] [--dump]

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const SEED_REL = "src-tauri/src/commands/stock_analysis_setup/seed_stock_analysis.rs";
// B-2b（PLAN §九十一/§九十三）：四档分支正在从主图搬进这张档模板 builder。
// 过渡期两侧都有 ⇒ 由 `transitionDrift()` 钉逐字一致；改图后只剩这一侧 ⇒ 扫描面不塌。
const BUILDER_REL = "src-tauri/src/commands/stock_analysis_setup/horizon_tier_template.rs";
const HARNESS_REL = "src-tauri/crates/harness/src/holding_period.rs";
const COMMANDS_REL = "src-tauri/src/commands";
// 点号算断符：否则 `horizon_branch_json.ultra_short` 是整 token，尾部 `_short` 会被 short 抢走（最长命中失效）
const TOKEN_BREAK = "\t\r\n(){}[]<>,.;:\"'`=+*/%&|^?!~#@$\\{}";
const NL = String.fromCharCode(10);

/** 剥掉行注释（`//` 到行尾）。注释里提档名不算代码引用。 */
export function codeOnly(text) {
  return text
    .split(NL)
    .map((line) => {
      const at = line.indexOf("//");
      return at < 0 ? line : line.slice(0, at);
    })
    .join(NL);
}

/** 拆标识符 token（非标识符字符为界），不做任何大小写/形态归一。 */
export function tokens(text) {
  const out = [];
  let cur = "";
  for (const ch of text) {
    if (TOKEN_BREAK.indexOf(ch) >= 0) {
      if (cur !== "") {
        out.push(cur);
      }
      cur = "";
    } else {
      cur += ch;
    }
  }
  if (cur !== "") {
    out.push(cur);
  }
  return out;
}

/**
 * 一个 token 归属哪一档：`token === 档名` 或以 `_`/`-` + 档名 结尾；多命中取**最长档名**。
 * 返回 `null` = 这个 token 不点名任何档。
 */
export function tierOfToken(token, snakes, kebabs) {
  const forms = [];
  for (const [tier, snake] of snakes.entries()) {
    forms.push([tier, snake]);
  }
  for (const [tier, kebab] of kebabs.entries()) {
    forms.push([tier, kebab]);
  }
  let best = null;
  for (const [tier, form] of forms) {
    const hit =
      token === form || token.endsWith("_" + form) || token.endsWith("-" + form);
    if (hit && (best === null || form.length > best[1].length)) {
      best = [tier, form];
    }
  }
  return best === null ? null : best[0];
}

/** 从 `Period::as_str` 的 match 臂右值现场取四档 snake 名。 */
export function periodSnakesFrom(harnessText) {
  const at = harnessText.indexOf("pub fn as_str");
  if (at < 0) {
    return null;
  }
  // 只在该函数的窗口里找臂（as_str 很短；切臂按 `=>`，右值取第一个双引号串）
  const window = harnessText.slice(at, at + 1600);
  const stop = window.indexOf(NL + "    }");
  const body = stop > 0 ? window.slice(0, stop) : window;
  const out = new Map();
  for (const arm of body.split("=>").slice(1)) {
    const name = firstQuoted(arm);
    if (name !== null && name.indexOf("Period::") < 0) {
      out.set(name, name);
    }
  }
  return out;
}

/** 取片段里第一个双引号字符串（跳过 `=>` 左边残留的臂模式）。 */
function firstQuoted(text) {
  const open = text.indexOf('"');
  if (open < 0) {
    return null;
  }
  const close = text.indexOf('"', open + 1);
  if (close < 0) {
    return null;
  }
  return text.slice(open + 1, close);
}

/** 收集种子里出现过的 `t-scoring-*` 节点 id（现场推导，不硬编码四个）。 */
export function scoringNodeIds(seedText) {
  const out = new Set();
  let at = 0;
  while (true) {
    at = seedText.indexOf("t-scoring-", at);
    if (at < 0) {
      break;
    }
    let end = at + "t-scoring-".length;
    while (end < seedText.length && TOKEN_BREAK.indexOf(seedText[end]) < 0) {
      end += 1;
    }
    out.add(seedText.slice(at, end));
    at = end;
  }
  return out;
}

/**
 * R3 的载体：**声明式错峰 → v135 起声明为空表**。
 *
 * 改图前这里登记两条（`week>month`、`month>quarter`），用途是把「哪两条评分节点之间有依赖」
 * 从图里的偶然事实变成必须登记的决定（PLAN §七十九 A3-② 选「承认错峰」而不是改拓扑）。
 * v135（B-2b #36）把每档的评分节点搬进各自子模板 ⇒ 这条链在物理上断开，四档真并行
 * （§九十一(5) 决策点 3 自行拍定接受；代价 = 季线把同一段历史再拉一遍，属运行时长不属口径）。
 *
 * ⇒ 判据跟着**改判**而不是退役：本表清空，含义变成「登记过的决定 = 四路互不等」。
 *   · 谁在主图或档模板里加回一条档间评分依赖 ⇒ 现场读数非空 ⇒ 红（要么改代码，要么带着理由加进本表）；
 *   · 本表若留着那两条 ⇒ 现场读数为空 ⇒ 红（声明与实际不符，本表就成了假文档）。
 * 口径判据不在这张表上，而在 R1/R2/R5；错峰只影响时刻。
 */
export const DECLARED_SCORING_STAGGER = [];

/**
 * 从 harness 的 `pub fn <fnName>(&self)` 里按 `Period::X => "值"` 现推四臂。
 * 返回 `Map<档名 snake, 右值>`；取不到返回 null（调用侧必须判红，不能当"没这条规则"）。
 *
 * 档名由变体名 camel→snake 推得，并与 `Period::as_str` 的右值集合互相验证
 * （两者给出的四档若不一致，说明命名约定漂了 —— 那时**两套推导都不可信**，本门直接失声）。
 */
export function armsOfPeriodFn(harnessText, fnName) {
  const at = harnessText.indexOf("pub fn " + fnName);
  if (at < 0) {
    return null;
  }
  const window = harnessText.slice(at, at + 2400);
  const stop = window.indexOf(NL + "    }");
  const body = stop > 0 ? window.slice(0, stop) : window;
  const out = new Map();
  const re = /Period::([A-Za-z]+)\s*=>\s*"([^"]+)"/g;
  let m = re.exec(body);
  while (m !== null) {
    out.set(m[1].replace(/([a-z])([A-Z])/g, "$1_$2").toLowerCase(), m[2]);
    m = re.exec(body);
  }
  return out;
}

/** 现场收集种子里的「评分节点之间」的边（`edge("id", "src", "dst")` 取后两个引号串）。 */
export function scoringStaggerEdges(seedText) {
  const scoring = scoringNodeIds(seedText);
  const out = [];
  let at = 0;
  while (true) {
    at = seedText.indexOf("edge(", at);
    if (at < 0) {
      break;
    }
    const close = seedText.indexOf(")", at);
    if (close < 0) {
      break;
    }
    const args = seedText.slice(at + "edge(".length, close);
    const quoted = [];
    let q = 0;
    while (true) {
      q = args.indexOf('"', q);
      if (q < 0) {
        break;
      }
      const end = args.indexOf('"', q + 1);
      if (end < 0) {
        break;
      }
      quoted.push(args.slice(q + 1, end));
      q = end + 1;
    }
    if (quoted.length >= 3) {
      const src = quoted[quoted.length - 2];
      const dst = quoted[quoted.length - 1];
      const isScoring = (id) => id !== "t-scoring" && scoring.has(id);
      if (isScoring(src) && isScoring(dst)) {
        out.push(src + ">" + dst);
      }
    }
    at = close;
  }
  return out;
}

/**
 * 字符串感知的括号配对：从 `open`（指向 `{` 或 `[`）找到配对闭合符，跳过 `"…"` 内的括号。
 * 找不到返回 -1（形状变了要红，不能退化成「少比几项」）。
 */
function matchBrace(text, open) {
  const opener = text[open];
  const closer = opener === "[" ? "]" : opener === "(" ? ")" : "}";
  if (opener !== "{" && opener !== "[" && opener !== "(") {
    return -1;
  }
  let depth = 0;
  let inString = false;
  let escaped = false;
  for (let i = open; i < text.length; i += 1) {
    const ch = text[i];
    if (inString) {
      if (escaped) {
        escaped = false;
      } else if (ch === "\\") {
        escaped = true;
      } else if (ch === '"') {
        inString = false;
      }
      continue;
    }
    if (ch === '"') {
      inString = true;
    } else if (ch === opener) {
      depth += 1;
    } else if (ch === closer) {
      depth -= 1;
      if (depth === 0) {
        return i;
      }
    }
  }
  return -1;
}

/**
 * 按 **`CodeNode {` 的花括号平衡**切出 `pm-h-*` 块，返回 `Map<节点 id, 块文本>`。
 *
 * 为什么不再用「到下一个 `nodes.push(` 为止」：那是**主图专用**的启发式。四档分支搬进
 * `horizon_tier_template.rs` 之后，分支节点后面跟的是 `tier_end_node(…)`（同一个 `vec![…]` 里），
 * 旧右边界会一路滑到文件末尾 ⇒ 一个块里混进另外三档的字面量 ⇒ R2 假红（PLAN §九十三(4)）。
 */
export function branchBlocksIn(text) {
  const out = new Map();
  let at = 0;
  while (true) {
    at = text.indexOf("CodeNode {", at);
    if (at < 0) {
      break;
    }
    const open = at + "CodeNode {".length - 1;
    const end = matchBrace(text, open);
    if (end < 0) {
      break;
    }
    const body = text.slice(open, end + 1);
    const idAt = body.indexOf('id: "pm-h-');
    if (idAt >= 0) {
      const nodeId = firstQuoted(body.slice(idAt));
      if (nodeId !== null) {
        out.set(nodeId, body);
      }
    }
    at = end + 1;
  }
  return out;
}

/** 块内 `input_mapping: [` 里的 `"键" ← "源路径"` 对（本门自己的口径：剥行注释后再取）。 */
export function mappingPairs(blockText) {
  const at = blockText.indexOf("input_mapping: [");
  if (at < 0) {
    return [];
  }
  const open = at + "input_mapping: [".length - 1;
  const close = matchBrace(blockText, open);
  if (close < 0) {
    return [];
  }
  const seg = codeOnly(blockText.slice(open, close + 1));
  const pairs = [];
  const re = /\(\s*"([^"]+)"\s*,\s*"([^"]+)"\s*\)/g;
  let m = re.exec(seg);
  while (m !== null) {
    pairs.push(m[1] + "<=" + m[2]);
    m = re.exec(seg);
  }
  return pairs.sort();
}

/**
 * **单形**扫描（v135 第二步）：`pm-h-*` 分支块只认档子模板 builder 那一份。
 *
 * §九十四 建的是「双形可认 + 过渡期逐字一致」，用途是让改图那天两侧都不失去读取面。
 * 本批主图那份已删 ⇒ 按 §九十一(6) 的两步写法收紧到只认新形；旧形是否真的归零，
 * 由下面的 [`seedLegacyBranchBlocks`] 单独检（不检的话「两侧都扫」会永远绿着放过残留）。
 */
export function branchBlocks(builderText) {
  const out = new Map();
  for (const [id, text] of branchBlocksIn(builderText).entries()) {
    out.set(id, { text, label: BUILDER_REL, where: "档模板" });
  }
  return out;
}

/**
 * R6 的读取面：主图里**残留**的 `pm-h-*` CodeNode 块（应为零）。
 *
 * 返回值是「还在主图里的旧形节点 id」。旧形残留 = 同 id 有两份定义（主图一份、档模板一份），
 * 正是过渡期靠逐字一致门钉住的那个状态；本批之后它必须是零，否则两份会各演进一份。
 */
export function seedLegacyBranchBlocks(seedText) {
  return [...branchBlocksIn(seedText).keys()];
}

/**
 * 一个节点 id 在主图里由哪种构造器拥有（`subWorkflow` / `code` / `null` = 找不到）。
 *
 * 判据取「id 之前**最近**的那个构造器标记」，而不是「文件里出现过 SubWorkflowNode」——
 * 后者会让「主图既有扇出、又留着旧的 CodeNode 块」这种半新半旧状态混过去。
 * 两种新形写法都要认：`WorkflowNode::SubWorkflow(SubWorkflowNode {` 与
 * `let mut fanout = SubWorkflowNode {`（后一种是为了用 helper 追加带档分析师键）。
 */
export function nodeShapeOf(text, nodeId) {
  const pat = 'id: "' + nodeId + '"';
  let at = text.indexOf(pat);
  while (at >= 0) {
    const sw = text.lastIndexOf("SubWorkflowNode {", at);
    const code = text.lastIndexOf("CodeNode {", at);
    if (sw >= 0 || code >= 0) {
      if (sw > code) {
        return "subWorkflow";
      }
      return "code";
    }
    at = text.indexOf(pat, at + 1);
  }
  return null;
}

/** 从块里读 `("tier_score", "<节点>.result…")` 的节点名（本档评分来源）。 */
export function tierScoreSource(blockText) {
  const at = blockText.indexOf('("tier_score"');
  if (at < 0) {
    return null;
  }
  const after = blockText.slice(at, at + 400);
  // 引号位序：1 个在 `tier_score` 前、2 个在它后（闭合），3/4 包住来源路径 ⇒ 取第 3、第 4 个
  const q1 = after.indexOf('"');
  const q2 = after.indexOf('"', q1 + 1);
  const q3 = after.indexOf('"', q2 + 1);
  const q4 = after.indexOf('"', q3 + 1);
  if (q1 < 0 || q2 < 0 || q3 < 0 || q4 < 0) {
    return null;
  }
  const source = after.slice(q3 + 1, q4);
  const dot = source.indexOf(".");
  return dot < 0 ? source : source.slice(0, dot);
}

function kebabOf(snake) {
  return snake.split("_").join("-");
}

/**
 * 主检查。返回问题清单（空 = 绿）。
 * 断言分五组：R1 四档评分来源互不相同；R2 块内/脚本内不点名他档；R3 主图与档模板都不得有
 * 档间评分依赖（v135 起声明为空表）；R5 档↔尺度↔节点对齐权威 `Period::scoring_node_id`；
 * R6 主图那份旧形（`pm-h-*` 的 CodeNode 块）必须归零，四个 `pm-h-<档>` 必须是扇出形态。
 *
 * `builderText` 是分支块的**唯一**来源（§九十一(6) 第二步）；缺省为空 ⇒ 定位不到四个块即红。
 */
export function checkAll(seedText, harnessText, scriptsByText, builderText = "") {
  const problems = [];
  const snakes = periodSnakesFrom(harnessText);
  if (snakes === null || snakes.size !== 4) {
    return ["取不到 `Period::as_str` 的四档 snake 名（现场推导失败 ⇒ 判据不存在，不是绿）"];
  }
  const kebabs = new Map();
  for (const snake of snakes.keys()) {
    kebabs.set(snake, kebabOf(snake));
  }

  const blocks = branchBlocks(builderText);
  if (blocks.size !== 4) {
    return [
      "档模板 builder 里只定位到 " + blocks.size + " 个 `pm-h-*` 分支块（应为 4 ⇒ 改名/搬走/漏登记）",
    ];
  }
  // ── R6：主图那份旧形必须归零，且四个 `pm-h-<档>` 必须以扇出形态存在 ──
  const legacy = seedLegacyBranchBlocks(seedText);
  if (legacy.length > 0) {
    problems.push(
      "主图里仍残留 " + legacy.join(" / ") + " 的 CodeNode 分支块 ⇒ 同一 id 两份定义，" +
        "本批要求旧形归零（§九十一(6) 第二步：门已收紧到只认新形，残留不是「兼容」而是分叉的前置）",
    );
  }
  for (const [tier, kebab] of kebabs.entries()) {
    const nodeId = "pm-h-" + kebab;
    const shape = nodeShapeOf(seedText, nodeId);
    if (shape === null) {
      problems.push("主图里找不到 " + nodeId + "（四档扇出被删？父侧 `portfolio-mgr` 仍按它取值）");
    } else if (shape !== "subWorkflow") {
      problems.push("主图里 " + nodeId + " 的构造器是 " + shape + "，本批之后必须是 SubWorkflow 扇出");
    }
  }
  // 档↔尺度↔模板名的归属：`scoring_node_id` 由 R5 对，模板名由播种侧的 Rust 门对
  // （`tier_fanout_inputs_are_complete` 的 ④），本门不重复判 —— 但**扫描面**要如实打印。
  const scoringNodes = scoringNodeIds(seedText + NL + builderText);
  if (scoringNodes.size < 4) {
    return ["主图 + 档模板里只收集到 " + scoringNodes.size + " 个 `t-scoring-*` 节点 id"];
  }

  // ── R1：每档评分来源存在、互不相同、且都是 t-scoring-* 节点 ──
  const ownByTier = new Map();
  for (const [nodeId, info] of blocks.entries()) {
    const tier = nodeId.split("pm-h-")[1].split("-").join("_");
    if (!snakes.has(tier)) {
      problems.push("分支块 " + nodeId + " 的档位名 " + tier + " 不在四档权威里");
      continue;
    }
    const src = tierScoreSource(info.text);
    if (src === null) {
      problems.push(nodeId + " 块里读不到 `(\"tier_score\", …)` 映射 ⇒ 断言失去对象");
      continue;
    }
    if (!scoringNodes.has(src) && src !== "t-scoring") {
      problems.push(nodeId + " 的 tier_score 来源 " + src + " 不是 t-scoring-* 评分节点");
    }
    ownByTier.set(tier, src);
  }
  const values = [...ownByTier.values()];
  if (new Set(values).size !== values.length) {
    problems.push("四档的 tier_score 来源出现重复（" + values.join(" / ") + "）⇒ 两档评分输入恒等，方向永远一样");
  }

  // ── R5：每档的评分来源必须等于权威 `Period::scoring_node_id` 指的那个节点 ──
  //
  // R1 只保证「四路互不相同」，保证不了「对得上档位」——两档互换来源 R1 照样绿。
  // 这条把 §九十二 量到的那类缺陷（四档实际取同一尺度、各处字面量互相印证而无权威可对）
  // 变成改一臂就红：档↔尺度↔节点 三张臂必须在 harness 里同处一致，种子必须服从它。
  const wantNode = armsOfPeriodFn(harnessText, "scoring_node_id");
  if (wantNode === null || wantNode.size !== 4) {
    return ["取不到 `Period::scoring_node_id` 的四臂（权威缺失 ⇒ 判据不存在，不是绿）"];
  }
  for (const snake of snakes.keys()) {
    if (!wantNode.has(snake)) {
      problems.push(
        "harness 的 `scoring_node_id` 推导不出档名 " + snake + " ⇒ 变体名 camel→snake 与 `as_str` 右值两套推导已经不一致，本门不可信",
      );
    }
  }
  for (const [tier, src] of ownByTier.entries()) {
    if (wantNode.has(tier) && wantNode.get(tier) !== src) {
      problems.push(
        "R5：" + tier + " 的 tier_score 来源是 " + src + "，而权威 `Period::scoring_node_id` 给的是 " +
          wantNode.get(tier) + " ⇒ 档位与尺度串了（这一档的评分实际来自别的尺度）",
      );
    }
  }

  // ── R3：主图与档模板里都不得出现档间评分依赖（声明表已随 v135 清空，见其文档） ──
  const foundStagger = scoringStaggerEdges(seedText + NL + builderText).sort();
  const wantStagger = [...DECLARED_SCORING_STAGGER].sort();
  if (foundStagger.join(",") !== wantStagger.join(",")) {
    problems.push(
      "评分节点之间的依赖与声明不符：现场 [" + foundStagger.join(" / ") + "]，声明 [" +
        wantStagger.join(" / ") + "] —— v135 起四档真并行、声明为空表；" +
        "加回一条档间评分边要连本表一起改（带理由），否则四档重新变成同一份输入" +
        "（错峰影响的是时刻不是口径；口径由 R1/R2/R5 管）",
    );
  }

  // ── R2：块内与脚本内不得点名他档 ──
  const units = [];
  for (const [nodeId, info] of blocks.entries()) {
    const tier = nodeId.split("pm-h-")[1].split("-").join("_");
    if (ownByTier.has(tier)) {
      units.push([
        info.label + "（" + info.where + "）的 " + nodeId + " 块",
        tier,
        codeOnly(info.text),
        ownByTier.get(tier),
      ]);
    }
  }
  for (const [entry] of scriptsByText.entries()) {
    const m = entry.split("portfolio-mgr-h-")[1].split(".rhai")[0];
    const tier = m.split("-").join("_");
    if (snakes.has(tier)) {
      units.push([entry, tier, codeOnly(scriptsByText.get(entry)), ownByTier.get(tier)]);
    }
  }

  for (const [label, tier, text, ownScoring] of units) {
    const offenders = new Map();
    for (const tok of tokens(text)) {
      const hit = tierOfToken(tok, snakes, kebabs);
      if (hit !== null && hit !== tier) {
        offenders.set(hit, tok);
      }
    }
    for (const [other, tok] of offenders.entries()) {
      problems.push(label + "（本档 " + tier + "）出现了他档 token：" + tok + "（指向 " + other + "）");
    }
    // 评分来源：块里出现的其它 t-scoring-* 节点（日线 `t-scoring` 是刻意共享的 σ_daily 来源）
    for (const node of scoringNodes) {
      if (node === ownScoring) {
        continue;
      }
      let at = 0;
      while (true) {
        at = text.indexOf(node, at);
        if (at < 0) {
          break;
        }
        const tail = text[at + node.length];
        if (tail === undefined || TOKEN_BREAK.indexOf(tail) >= 0) {
          problems.push(label + "（本档评分源 " + ownScoring + "）却引用了 " + node + " ⇒ 跨档取评分");
          break;
        }
        at += node.length;
      }
    }
  }
  return problems;
}

function readUnits() {
  const seedText = fs.readFileSync(path.join(ROOT, SEED_REL), "utf8");
  const builderText = fs.readFileSync(path.join(ROOT, BUILDER_REL), "utf8");
  const harnessText = fs.readFileSync(path.join(ROOT, HARNESS_REL), "utf8");
  const scripts = new Map();
  for (const snake of periodSnakesFrom(harnessText).keys()) {
    const rel = COMMANDS_REL + "/portfolio-mgr-h-" + snake.split("_").join("-") + ".rhai";
    scripts.set(rel, fs.readFileSync(path.join(ROOT, rel), "utf8"));
  }
  return [seedText, builderText, harnessText, scripts];
}

function selftest() {
  const harnessStub =
    "impl Period {" +
    NL +
    "    pub fn as_str(&self) -> &'static str {" +
    NL +
    "        match self {" +
    NL +
    '            Period::UltraShort => "ultra_short",' +
    NL +
    '            Period::Short => "short",' +
    NL +
    '            Period::Mid => "mid",' +
    NL +
    '            Period::Long => "long",' +
    NL +
    "        }" +
    NL +
    "    }" +
    NL +
    "    pub fn scoring_node_id(&self) -> &'static str {" +
    NL +
    "        match self {" +
    NL +
    '            Period::UltraShort => "t-scoring-hour",' +
    NL +
    '            Period::Short => "t-scoring-week",' +
    NL +
    '            Period::Mid => "t-scoring-month",' +
    NL +
    '            Period::Long => "t-scoring-quarter",' +
    NL +
    "        }" +
    NL +
    "    }" +
    NL +
    "}";
  const kebab = (t) => t.split("_").join("-");
  // 夹具用**生产同形**（`CodeNode { … }` 花括号平衡）：门按花括号切块，
  // 夹具若长得跟生产不一样，绿了就只证明夹具自洽。
  const block = (tier, scoring) =>
    "WorkflowNode::Code(CodeNode {" +
    NL +
    '            base: WorkflowNodeBase { id: "pm-h-' +
    kebab(tier) +
    '".into(), },' +
    NL +
    '            config: CodeNodeConfig { input_mapping: [' +
    NL +
    '                ("tier_score", "' +
    scoring +
    '.result.content.totalScore"),' +
    NL +
    '                ("overall_risk", "cls-risk-level-' +
    kebab(tier) +
    '.result.category"),' +
    NL +
    "            ].into_iter().collect(), }," +
    NL +
    "        })";
  const TIERS = [
    ["ultra_short", "t-scoring-hour"],
    ["short", "t-scoring-week"],
    ["mid", "t-scoring-month"],
    ["long", "t-scoring-quarter"],
  ];
  // v135 第二步的夹具形状：主图那份是**扇出**（`SubWorkflowNode {` 拥有 `id: "pm-h-<档>"`），
  // 分支块只在档模板 builder 里。夹具必须与生产同形 —— 夹具长得不像生产，绿了只证明夹具自洽。
  const fanout = (tier) =>
    "WorkflowNode::SubWorkflow(SubWorkflowNode {" +
    NL +
    '            base: WorkflowNodeBase { id: "pm-h-' +
    kebab(tier) +
    '".into(), },' +
    NL +
    "        })";
  // 快速链仍按 id 引用那几个评分节点（`FAST_BRIEF_INPUTS` 的值），所以 `t-scoring-*` 字面量
  // 还在主图文件里 ⇒ 夹具要覆盖「有 id 文本、但没有档间边」这一形。
  const tailLines =
    '("algo_scoring_week", "t-scoring-week.result.content"),' +
    NL +
    '("algo_scoring_month", "t-scoring-month.result.content"),';
  const seedOf = (tiers) => tiers.map(([t]) => fanout(t)).join(NL) + NL + tailLines;
  const cleanSeed = seedOf(TIERS);
  // tail=true ⇒ 分支节点后面跟 `tier_end_node(…)`（档模板 builder 的真实尾巴）。
  // 旧启发式「到下一个 `nodes.push(` 为止」在这种尾巴上会滑到文件末尾，
  // 把后面三档的字面量一起吃进同一个块 ⇒ R2 假红。夹具必须覆盖这个形状。
  const fourBlocks = (tail) =>
    TIERS.map(
      ([t, s]) => block(t, s) + (tail ? NL + '        tier_end_node("h_' + t + '", 4200.0),' : ""),
    ).join(NL);
  const cleanBuilder = fourBlocks(true);

  const SCRIPT_ULTRA = "src-tauri/src/commands/portfolio-mgr-h-ultra-short.rhai";
  const SCRIPT_SHORT = "src-tauri/src/commands/portfolio-mgr-h-short.rhai";
  const emptyScripts = new Map();
  const cases = [
    ["干净夹具必须绿（主图=扇出四块、档模板=分支四块、档间无评分边）",
      checkAll(cleanSeed, harnessStub, emptyScripts, cleanBuilder).length === 0],
    [
      "R6：主图残留旧的 CodeNode 分支块必须红（同 id 两份定义＝本批要求归零的那个状态）",
      checkAll(
        cleanSeed + NL + block("mid", "t-scoring-month"),
        harnessStub,
        emptyScripts,
        cleanBuilder,
      ).some((p) => p.indexOf("残留") >= 0),
    ],
    [
      "R6：少一个扇出必须点名是哪个档（不能由「四个块都在」顶替）",
      checkAll(
        seedOf([["ultra_short", "t-scoring-hour"], ["short", "t-scoring-week"], ["long", "t-scoring-quarter"]]),
        harnessStub,
        emptyScripts,
        cleanBuilder,
      ).some((p) => p.indexOf("pm-h-mid") >= 0),
    ],
    [
      "R6 的形状判据认得出 CodeNode 形态（不是靠文件里出现过 SubWorkflowNode 蒙对）",
      nodeShapeOf(block("mid", "t-scoring-month"), "pm-h-mid") === "code" &&
        nodeShapeOf(cleanSeed, "pm-h-mid") === "subWorkflow",
    ],
    [
      "R3：档模板里加回一条档间评分依赖必须红（四路互不等是本批登记的决定）",
      checkAll(
        cleanSeed,
        harnessStub,
        emptyScripts,
        cleanBuilder + NL + 'direct_edge("e-x", "t-scoring-hour", "t-scoring-week")',
      ).some((p) => p.indexOf("评分节点之间的依赖") >= 0),
    ],
    [
      "R3：主图里出现档间评分边同样红（扫描面含主图，不给旧形留口子）",
      checkAll(
        cleanSeed + NL + 'edge("e-y", "t-scoring-month", "t-scoring-quarter")',
        harnessStub,
        emptyScripts,
        cleanBuilder,
      ).some((p) => p.indexOf("评分节点之间的依赖") >= 0),
    ],
    ["取不到四档名必须红（判据不存在≠绿）",
      checkAll(cleanSeed, "no as_str", emptyScripts, cleanBuilder).length === 1],
    [
      "短档块里出现他档风险节点必须红",
      checkAll(
        cleanSeed,
        harnessStub,
        emptyScripts,
        cleanBuilder.replace(block("short", "t-scoring-week"), block("short", "t-scoring-week").replace("cls-risk-level-short", "cls-risk-level-mid")),
      ).length > 0,
    ],
    [
      "两档共用同一评分节点必须红",
      checkAll(
        cleanSeed,
        harnessStub,
        emptyScripts,
        cleanBuilder.replace('("tier_score", "t-scoring-week.result', '("tier_score", "t-scoring-month.result'),
      ).length > 0,
    ],
    [
      "脚本里读他档分析师实例必须红",
      checkAll(cleanSeed, harnessStub, new Map([[SCRIPT_SHORT, 'let x = a_hot_money["--mid"];']]), cleanBuilder).some((p) => p.indexOf("--mid") >= 0),
    ],
    [
      "超短档脚本里的 ultra_short 不得被当成 short（最长命中）",
      checkAll(
        cleanSeed,
        harnessStub,
        new Map([[SCRIPT_ULTRA, 'let h = "ultra_short"; let s = branch_json["ultra_short"];']]),
        cleanBuilder,
      ).length === 0,
    ],
    [
      "注释里提他档不算代码（剥注释）",
      checkAll(cleanSeed, harnessStub, new Map([[SCRIPT_SHORT, "// 本档不是 mid，别照抄"]]), cleanBuilder).length === 0,
    ],
    ["定位不到四个块必须红",
      checkAll(cleanSeed, harnessStub, emptyScripts, 'id: "pm-h-short".into').length > 0],
    [
      "花括号切块不吃进兄弟档（旧 `nodes.push(` 右边界在 tier_end_node 尾巴上会滑到文件尾）",
      (function () {
        const ultra = branchBlocksIn(cleanBuilder).get("pm-h-ultra-short");
        if (ultra === undefined) {
          return false;
        }
        const pairs = mappingPairs(ultra);
        return (
          pairs.length === 2 &&
          pairs.join(",").indexOf("t-scoring-hour") >= 0 &&
          pairs.join(",").indexOf("mid") < 0 &&
          pairs.join(",").indexOf("cls-risk-level-short") < 0
        );
      })(),
    ],
    // ── R5：档↔尺度↔节点 三张臂必须由权威对齐 ──
    [
      "权威改了而档模板没跟必须红（R1 只查互不相同，查不出「对错档位」）",
      checkAll(
        cleanSeed,
        harnessStub.replace('Period::Long => "t-scoring-quarter"', 'Period::Long => "t-scoring-month"'),
        emptyScripts,
        cleanBuilder,
      ).some((p) => p.indexOf("R5") >= 0),
    ],
    [
      "取不到 scoring_node_id 四臂必须红（权威缺失＝判据不存在，不许当绿）",
      checkAll(
        cleanSeed,
        harnessStub.replace("pub fn scoring_node_id", "pub fn other_thing"),
        emptyScripts,
        cleanBuilder,
      ).length === 1,
    ],
    [
      "变体名 camel→snake 与 as_str 右值不一致时不可信（缺档名必须点名）",
      checkAll(
        cleanSeed,
        harnessStub.replace(
          'Period::UltraShort => "t-scoring-hour"',
          'Period::UltraShortX => "t-scoring-hour"',
        ),
        emptyScripts,
        cleanBuilder,
      ).some((p) => p.indexOf("ultra_short") >= 0),
    ],
  ];
  const failed = cases.filter((c) => !c[1]).map((c) => c[0]);
  if (failed.length > 0) {
    console.error("❌ 自证失败：" + failed.join(" / "));
    process.exit(1);
  }
  console.log("✅ 自证通过（" + cases.length + " 条对照，含「最长命中」「剥注释」「判据不存在」「旧形残留必须红」四类负控）");
  process.exit(0);
}

function main() {
  const args = process.argv.slice(2);
  if (args.includes("--selftest")) {
    selftest();
  }
  const [seedText, builderText, harnessText, scripts] = readUnits();
  const problems = checkAll(seedText, harnessText, scripts, builderText);
  const snakes = [...periodSnakesFrom(harnessText).keys()];
  console.log("扫描：档名现场推导 " + snakes.join(" / ") + "（" + snakes.length + " 档）");
  console.log("扫描面：分支块只认档模板 " + BUILDER_REL + "；主图 " + SEED_REL + " 只检旧形归零与扇出形态");
  if (args.includes("--dump")) {
    console.log(
      "  评分节点之间的依赖：现场 = [" +
        (scoringStaggerEdges(seedText + NL + builderText).join(" / ") || "无") +
        "]｜声明 = [" + DECLARED_SCORING_STAGGER.join(" / ") + "]",
    );
    console.log("  主图残留的旧形分支块：" + (seedLegacyBranchBlocks(seedText).join(" / ") || "无（应为无）"));
    const blocks = branchBlocks(builderText);
    for (const [nodeId, info] of blocks.entries()) {
      const tier = nodeId.split("pm-h-")[1].split("-").join("_");
      console.log("  " + nodeId + "（读自 " + info.where + "）本档评分源 = " + tierScoreSource(info.text) + "（脚本 portfolio-mgr-h-" + nodeId.split("pm-h-")[1] + ".rhai）");
    }
  }
  if (problems.length > 0) {
    console.error("❌ " + problems.length + " 处串档：");
    for (const p of problems) {
      console.error("  " + p);
    }
    process.exit(1);
  }
  console.log("✅ 四个逐档分支（档子模板块 + Rhai 脚本）各自只读本档、四档评分来源互不相同，且主图那份旧形已归零");
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  main();
}
