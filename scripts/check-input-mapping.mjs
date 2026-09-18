// 临时校验脚本（A9 CI 门禁原型 v2，2026-09-15）
//
// 扫描 workflow 模板种子文件里的变量源路径，校验两件事：
//   A. **形态**：多段源路径的第 2 段必须是「节点输出包裹层的真实字段名」
//      （Agent → content / Tool → result / Loop → items 等）。
//   B. **消费端可达**：消费端节点的解析器有没有 JSON 字符串穿透能力。
//      本仓有多份语义不同的 resolve_var_path —— 这是最隐蔽的一类失效。
//      **2026-09-15 变更：ToolNode 的私有严格副本已删除，改为转调共享宽松版**
//      （`tool_executor.rs:77` 全限定调用 `super::resolve_var_path`），
//      故 ToolNode 与 AgentNode/CodeNode/Loop/Switch 同属「穿透」一档。
//      当前仍在册的实现：
//        · executors::resolve_var_path      (executors/mod.rs:106-164)  **穿透**
//          AgentNode / CodeNode 的 input_mapping（agent_executor.rs:762,
//          code_executor.rs:125）· ToolNode 的 input_mapping（tool_executor.rs:77）·
//          Loop 的 iter_input_var（loop_executor.rs:166）· Switch（switch_executor.rs:45）
//          ⇒ 允许任意深度。
//        · condition_executor::resolve_var_path (condition_executor.rs:396-412) **不穿透**
//          （源码注释明写「严格模式：不做 JSON 字符串穿透」）。
//          ⇒ 消费端是 **ConditionNode** 时，**深度受限**：
//            root 是 Agent（content 是 JSON 字符串）⇒ 最深 2 段
//            root 是 Tool（result 可能是对象）      ⇒ 最深 3 段
//            root 是 Loop（items 是数组）          ⇒ 最深 2 段（无下标语法）
//            root 是 Code（result 形态不定）        ⇒ 最深 3 段
//      另注：SubWorkflow / Storage / Validation / LlmClassifier 也各有严格私有副本
//      （subworkflow_executor.rs:373 / storage_executor.rs:26 /
//       validation_executor.rs:286 / llm_classifier_executor.rs:452），
//      但这四类节点的输入映射不由本脚本的 MAKERS 解析，暂不在扫描范围内。
//
// 用法：
//   node scripts/check-input-mapping.mjs                # 扫默认生产文件
//   node scripts/check-input-mapping.mjs <file.rs> ...
//   node scripts/check-input-mapping.mjs --selftest     # 正负对照（6 组反向夹具 + 1 组正对照）
// 退出码：0 = OK / OK-with-baseline / selftest 全过；1 = FAIL（新违规，或已知基线条数上升）
//
// **为什么从 output/ 迁到 scripts/**：`.gitignore:133-134` 忽略整个 `output/`
// （注释原文即 "Generated output files"）⇒ 放在那里的门禁**对任何 clone 本仓库的人都不存在**，
// 它不是「位置不理想」，而是**从来没生效过**。同目录已有 13 个同族 `check-*.mjs`，
// 本文件按同一形态接入（聚合器 `scripts/ci-check.mjs` 用**显式 step 列表**）。
//
// **路径约定（迁到 scripts/ 的前提）**：所有相对路径按**脚本自身位置**推导
// （`ROOT = <脚本目录>/..`），不再依赖 `process.cwd()` ⇒ 从任意工作目录调用都扫得到；
// 绝对路径原样使用（`--selftest` 的临时夹具走这条）。若按 ROOT 推不出文件而按 cwd 能推出，
// 则回退 cwd，兼容既有调用方式。
//
// 注意：本脚本不参与构建，也不在 package.json 里。

import { existsSync, mkdirSync, readFileSync, unlinkSync, writeFileSync } from 'node:fs';
import { dirname, isAbsolute, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { execFileSync } from 'node:child_process';
import { tmpdir } from 'node:os';

const SCRIPT_FILE = fileURLToPath(import.meta.url);
const SCRIPT_DIR = dirname(SCRIPT_FILE);
/** 仓库根：本脚本位于 `<root>/scripts/`（或 `<root>/output/`），其上一级即 root。 */
const ROOT = resolve(SCRIPT_DIR, '..');

const DEFAULT_FILE = 'src-tauri/src/commands/opc_workflows/seed_content_media.rs';

/**
 * 把 CLI 传入的路径解析成绝对路径。
 * 绝对路径原样返回；相对路径先按 ROOT 解析，ROOT 下不存在时回退 process.cwd()。
 * 为什么不是纯 cwd：脚本被 CI / 任意工作目录调用时必须稳定命中同一个文件。
 */
function resolveInput(rel) {
  if (isAbsolute(rel)) return rel;
  const byRoot = resolve(ROOT, rel);
  if (existsSync(byRoot)) return byRoot;
  const byCwd = resolve(process.cwd(), rel);
  return existsSync(byCwd) ? byCwd : byRoot;
}

// ── 已知悬空基线（按 (source_key, target_key) 键控，**不用行号** —— 行号必然漂）──
// 判据：即使补上正确的包裹层，字段本身也不存在（产出节点的 prompt 从未声明该键名）。
// 这类问题不是「路径写错」，需要改上游 prompt 或换数据通路。
// count = 当前预期出现次数，**只允许减少**：实际出现次数多于 count 即 FAIL（棘轮）。
//
// **2026-09-15（R1 + A1(A2) + R3 + R4 批次后）：列表已清空。**
// 清零原因：原 3 条被**真正修掉**（不是继续挂白名单、也不是抹掉记录），
// 故按棘轮「count 只允许减少」把 count 下调到 0，并把条目整体移除。
//
// 被移除的 3 条**逐字**如下（`source_key` → `target_key`，行号为改前，行号会漂）：
//   1. `lc-chapter-structure.current_chapter` → `chapter_structure`
//      `seed_content_media.rs:1097`（消费端 `lc-draft-chapter`）→ R2/A1 修
//   2. `lc-chapter-structure.current_chapter` → `chapter_structure`
//      `seed_content_media.rs:1141`（消费端 `lc-structure-checker`）→ R2/A1 修
//   3. `lc-outline.remaining` → `remaining_chapters`
//      `seed_content_media.rs:1160`（消费端 `lc-structure-adapter`）→ R3 删
// R2/A1 的修法：删掉 LLM 节点 `lc-structure-injector`，改 Loop 体内确定性 ToolNode
// `lc-chapter-instructions`，两个消费端改 4 段 `<tool>.result.content.constraints`；
// R3 的修法：该死映射直接删除。
//
// **清零前证据留档：`output/_gate-v4.txt`（该文件不要删。）**
// 它原样保留了清零前的门禁输出，可反查这 3 条确实存在过：
//   `已知悬空      : 1 条（基线，非违规）`
//   `RESULT: OK-with-baseline —— 0 条新违规，1 条已知悬空未修（基线 3）`
//   `[棘轮可收紧] 'lc-chapter-structure.current_chapter' 实际 0 次 < 基线 2 次`
//
// **同轮还修了本脚本的一个假阳**：新增 `maskComments()`（字符扫描，跳过普通字符串与
// `r"…"` / `r#"…"#` 原始字符串，把注释内容用**空格**抹平以保持偏移与行号不变）。
// 修的问题是：源码注释里为了记录历史而写下的映射字面量（例如
// `("remaining_chapters","lc-outline.remaining")`）会被 `parsePairs` 的**整文件正则**
// 当成**活配置**扫进来，于是「已删除的映射」在注释里又被报成一条悬空。
// 证据：`output/_gate-v4.txt` 是**修前**（多段源路径 35 条，含该假阳），
// 修后为 34 条；该行为已由反向夹具 `output/_fixtures-v2.mjs` 的 **E 组**锁定
// （断言 EXIT=0 **且** 多段源路径 = 34）。
// 注：`output/` 被 `.gitignore` 忽略，上面两份证据文件只在本机存在；因此本脚本内置
//     `--selftest`，把同 6 组夹具做成**可执行**形态（系统临时目录里生成、跑完即删），
//     使「门禁自己是否还在说谎」这件事在 clone 出来的仓库里也能被验证。
//
// 之所以**清空而不是留 count:0**，是因为清空后若这些路径复发，会回到规则 A 的
// 普通违规（`agent 节点(lc-outline) 的包裹层没有字段 'remaining'` → FAIL），
// 输出语义最直白；而留 count:0 会让它们的复发先以「已知悬空（基线，非违规）」
// 的名义打印出来，容易误读。棘轮机制本身保留，供将来登记新的已知悬空。
const KNOWN_DANGLING = [];

/** 各节点输出包裹层的字段（用于规则 A） */
const WRAPPER_FIELDS = {
    agent: new Set([
        'role',
        'model',
        'content',
        'thinking',
        'usage',
        'tool_calls_made',
        'node_id',
        // 2026-09-15 补齐（逐字对齐 agent_executor.rs:2582-2602 的 NodeOutput.output）：
        // 此前漏登记这两个真实字段 ⇒ 若有人写 x.streamTruncated 会被误报为违规。
        'streamTruncated',
        'truncationReason',
    ]),
    // ToolNode 的信封按**三条真实分支**取并集（2026-09-15 补齐，来源逐一核对）：
    //   · 回退回调路径（**生产实际走这条**，ToolRegistry 未注入 WorkEngine：
    //     `src/init/state.rs:660-665`；回调在 `src/init/services.rs:1913-1916`
    //     返回 `json!({"content": output.content})`）⇒ tool_node 输出
    //     `{tool_name, result:{"content":…}, node_id}`（`tool_executor.rs:177-188`）
    //     ⇒ **业务字段的实到路径是 `<tool>.result.content.<字段>`（4 段）**，
    //     不是 `executors/mod.rs:116` 注释里那个 `t-scoring.result.totalScore`（3 段）——
    //     那条先例属于下面第 1 条「未启用」的分支。
    //   · ToolRegistry 路径（未启用）：`{tool_name, result: <内容本身>, truncated, is_error, node_id}`（`:136-149`）
    //   · dry_run：`{tool_name, result, args, dry_run, node_id}`（`:100-106`）
    //   · 额外字段：`attach_investigation` 可能补 `needs_investigation`（`:198`）
    tool: new Set([
        'tool_name',
        'result',
        'node_id',
        'truncated',
        'is_error',
        'args',
        'dry_run',
        'needs_investigation',
    ]),
    loop: new Set([
        'loop_type',
        'iter_count',
        'last_iter_index',
        'resumed_from_checkpoint',
        'interrupted',
        'items',
        'iter_output_var',
        'iter_input_var',
        'node_id',
    ]),
    code: new Set(['status', 'language', 'result', 'input_params', 'node_id']),
    condition: new Set(['status', 'result', 'judge_mode', 'note', 'node_id']),
    approval: new Set(['status', 'decision', 'note', 'node_id']),
    trigger: new Set(['node_id']),
    end: new Set(['node_id']),
};

/**
 * 消费端严格性：true = 不穿透（深度受限）。
 *
 * **2026-09-15 变更**：`tool` 已从此表移除 —— ToolNode 的私有不穿透副本已删除，
 * 改为转调共享宽松版（`tool_executor.rs:77`）。若把 `tool` 留在表内，规则 B / B′
 * 会对 ToolNode 产生**假违规**（判据已失效，不是「结果不好看」）。
 *
 * 仍在册的两档说明：
 *   · condition: true —— 核实过：`condition_executor.rs:396-412` 是原地严格实现。
 *   · loop / approval —— **未逐字核实**（Loop 的 iter_input_var 实际走的是
 *     `loop_executor.rs:166` 的共享宽松版，即本档对 loop 很可能是**假阳性**；
 *     ApprovalNode 不调用 resolve_var_path，该档为空转）。
 *     本轮**刻意不动**这两档：收窄它们会凭空减少违规条数，容易被误读为
 *     「为让结果好看而放宽判据」。留待单独核实后再改。
 */
const STRICT_CONSUMER = { condition: true, approval: true, loop: true };

/** 严格消费端下，源 root 类型允许的最大段数（1 段另算，见下） */
const MAX_DEPTH_STRICT = { agent: 2, tool: 3, code: 3, loop: 2, condition: 2, approval: 2 };

const MAKERS = [
    ['make_agent_node_full', 'agent'],
    ['make_agent_node_with_inputs', 'agent'],
    ['make_agent_node', 'agent'],
    ['make_tool_node', 'tool'],
    ['make_loop_node', 'loop'],
    ['make_code_node', 'code'],
    ['make_condition_node', 'condition'],
    ['make_approval_node', 'approval'],
    ['make_trigger', 'trigger'],
    ['make_end', 'end'],
];

// ── 源码解析 ──────────────────────────────────────────────

function parseNodes(src) {
  const nodes = [];
  const re = /make_[a-z_]+\s*\(/g;
  let m;
  while ((m = re.exec(src)) !== null) {
    const fnName = m[0].replace(/\s*\($/, '');
    // 跳过 `fn make_xxx(` 的**定义体**，只认调用点
    // （否则辅助函数定义会被当成节点，`make_trigger`/`make_end` 尤其明显）
    if (/fn\s+$/.test(src.slice(Math.max(0, m.index - 3), m.index))) continue;
    const kind = MAKERS.find(([n]) => n === fnName)?.[1];
    if (!kind) continue;
    const openIdx = m.index + m[0].length - 1;
    const closeIdx = matchParen(src, openIdx);
    if (closeIdx < 0) continue;
    const argsSpan = src.slice(openIdx + 1, closeIdx);
    const args = splitTopLevel(argsSpan);
    const strings = args.map((a) => firstStringLiteral(a));
    // make_trigger / make_end 的入参只有坐标，id 在辅助函数里硬编码为 "trigger"/"end"
    const id = fnName === 'make_trigger' ? 'trigger' : fnName === 'make_end' ? 'end' : strings[0];
    if (!id) continue;

    // 注意: 各 maker 的签名里 output_var 的下标必须逐一对齐（2026-09-15 修正：
    // make_agent_node_with_inputs 曾写成 strings[6]，其签名
    // (id,title,prompt,tools,profile_id,**output_var**,inputs,x,y) 的正确下标是 5；
    // 旧值恒 null ⇒ 该节点的 output_var 别名从未建立，只是恰好被同 id 节点掩盖）。
    let outputVar = null;
    if (fnName === 'make_agent_node') outputVar = strings[5];
    else if (fnName === 'make_agent_node_with_inputs') outputVar = strings[5];
    else if (fnName === 'make_agent_node_full') outputVar = strings[5];
    else if (fnName === 'make_tool_node') outputVar = strings[4];
    else if (fnName === 'make_code_node') outputVar = strings[4];
    // make_loop_node 的 config 没有 output_var 字段，strings[5] 是 iter_output_var，
    // 仅作别名近似（LoopNode 的聚合输出 key）；无任何模板路径以该名取嵌套字段。
    else if (fnName === 'make_loop_node') outputVar = strings[5];

    // Loop 的 body_steps（第 9 个参数）：这些节点由 LoopExecutor 驱动、
    // **刻意不接 edges** ⇒ 规则 C 必须排除，否则误报「入度 0 / 不可达」。
    const bodySteps =
      fnName === 'make_loop_node'
        ? [...(args[8] ?? '').matchAll(/"([^"]+)"\s*\.to_string\(\)/g)].map((x) => x[1])
        : [];

    // CodeNode：记下脚本常量名（规则 D 要用它去取脚本正文）
    const codeIdent = fnName === 'make_code_node' ? args[2].trim() : null;

    nodes.push({
      id,
      kind,
      outputVar,
      fnName,
      codeIdent,
      bodySteps,
      start: openIdx,
      end: closeIdx,
      src,
    });
  }
  return nodes;
}

function matchParen(src, openIdx) {
  let depth = 0;
  for (let i = openIdx; i < src.length; i++) {
    const c = src[i];
    if (c === '"') {
      i = skipString(src, i) - 1;
      continue;
    }
    if (c === '(') depth++;
    else if (c === ')') {
      depth--;
      if (depth === 0) return i;
    }
  }
  return -1;
}

function skipString(src, i) {
  i++;
  while (i < src.length) {
    if (src[i] === '\\') {
      i += 2;
      continue;
    }
    if (src[i] === '"') return i + 1;
    i++;
  }
  return i;
}

/**
 * 规则前置：**把注释抹成空格**（保留长度与换行 ⇒ 所有 offset / 行号与原文一一对应）。
 *
 * 动机（2026-09-15 实测到的假阳）：为记录 R3 的「改前」形态，我在
 * `lc-structure-adapter` 的 input_mapping **注释**里写了原映射字面量
 * `("remaining_chapters", "lc-outline.remaining")`，`parsePairs` 的正则把它当成
 * **活的** input_mapping 抓了出来 ⇒ 基线里凭空多出一条已经删掉的悬空路径
 * （见 output/_gate-v4.txt）。这类「注释里的字面量被当成配置」的假阳会污染棘轮，
 * 让「基线只允许减少」这条约束失去意义（作者不敢在注释里写改前形态）。
 * 同类问题还有「注释掉的 edge 被当成真边」（parseEdges 同样全文件正则扫）。
 *
 * 实现要点：
 *   · 逐字符扫描，**字符串与原始字符串整体跳过**（否则字符串里的 `//` 会被误判成注释）；
 *   · 原始字符串语法 `r"…"` / `r#"…"#` / `r##"…"##`（结束定界符的 `#` 个数必须与
 *     开始一致）—— 本文件里 4 个 Rhai 脚本常量都是 `r#"…"#`，必须跳过，
 *     否则 `codeConstBody`（规则 D 要用它取脚本正文）就取不到正文了；
 *   · 注释内容替换为**空格**而不是删除，保证行号不漂。
 */
function maskComments(src) {
  const out = src.split('');
  const n = src.length;
  const blank = (from, to) => {
    const end = Math.min(to, n);
    for (let k = from; k < end; k++) if (out[k] !== '\n') out[k] = ' ';
  };
  let i = 0;
  while (i < n) {
    const c = src[i];
    // 原始字符串 r"…" / r#"…"# / r##"…"##（也覆盖 br#"…"#，因为扫到 r 时同样命中）
    if (c === 'r') {
      const m = /^r(#*)"/.exec(src.slice(i, i + 12));
      if (m) {
        const term = '"' + m[1];
        const body = i + m[0].length;
        const close = src.indexOf(term, body);
        i = close < 0 ? n : close + term.length;
        continue;
      }
    }
    if (c === '"') {
      i = skipString(src, i);
      continue;
    }
    if (c === '/' && src[i + 1] === '/') {
      let j = i;
      while (j < n && src[j] !== '\n') j++;
      blank(i, j);
      i = j;
      continue;
    }
    if (c === '/' && src[i + 1] === '*') {
      const close = src.indexOf('*/', i + 2);
      const end = close < 0 ? n : close + 2;
      blank(i, end);
      i = end;
      continue;
    }
    i++;
  }
  return out.join('');
}

function splitTopLevel(s) {
  const out = [];
  let depth = 0;
  let start = 0;
  for (let i = 0; i < s.length; i++) {
    const c = s[i];
    if (c === '"') {
      i = skipString(s, i) - 1;
      continue;
    }
    if (c === '(' || c === '[' || c === '{') depth++;
    else if (c === ')' || c === ']' || c === '}') depth--;
    else if (c === ',' && depth === 0) {
      out.push(s.slice(start, i));
      start = i + 1;
    }
  }
  out.push(s.slice(start));
  return out;
}

function firstStringLiteral(s) {
  const m = /"((?:[^"\\]|\\.)*)"/.exec(s);
  return m ? m[1] : null;
}

/** 节点调用内的 input_mapping 对（排除 td("工具名","描述") 这类非 mapping 对） */
function parsePairs(node) {
  const span = node.src.slice(node.start, node.end);
  const pairs = [];
  const re = /\("((?:[^"\\]|\\.)*)"\s*,\s*"((?:[^"\\]|\\.)*)"\)/g;
  let m;
  while ((m = re.exec(span)) !== null) {
    const before = span.slice(Math.max(0, m.index - 4), m.index);
    if (/td\($/.test(before)) continue;
    pairs.push({
      target: m[1],
      source: m[2],
      line: node.src.slice(0, node.start + m.index).split('\n').length,
    });
  }
  return pairs;
}

/** ConditionNode 的 var_path（同样是消费端严格的源路径） */
function parseVarPaths(node) {
  const span = node.src.slice(node.start, node.end);
  const out = [];
  const re = /var_path\s*:\s*"((?:[^"\\]|\\.)*)"/g;
  let m;
  while ((m = re.exec(span)) !== null) {
    out.push({
      target: '(var_path)',
      source: m[1],
      line: node.src.slice(0, node.start + m.index).split('\n').length,
    });
  }
  return out;
}

/**
 * 通用：按 maker 函数名取出全部调用点的**原始顶层参数串**（不做字面量提取）。
 *
 * 为何需要它（而不是把构造器都塞进 MAKERS）：`make_data_transformer_node`
 * 刻意**不进** MAKERS —— DataTransformer 的输出**没有信封**
 * （data_transformer_executor.rs:215-223 直接返回裸值），其顶层字段不可枚举，
 * 登记成一种 kind 反而会对合法的 `("chapter_text","lc-chapter-bare.chapter_text")`
 * 产生假阳。但它的 `input_var` / `output_var` 是**接线契约**
 * （`input_var` 与上一步的 `output_var` 必须对齐，否则 `input` 为 unit ⇒ 抛错），
 * 必须能被断言 ⇒ 用本函数取它的参数。
 *
 * 返回**原始串**（而不是只取首个字符串字面量）的关键理由：有很多实参不是字符串 ——
 * 例如 `vec!["a".to_string(), "b".to_string()]`（body_steps）、`LC_XXX_RHAI`
 * （脚本常量名）、`LoopType::ForEach`。只取字面量会把这些参数退化成 null。
 */
function parseMakerRawArgs(src, fnName) {
  const out = [];
  const re = new RegExp(`${fnName}\\s*\\(`, 'g');
  let m;
  while ((m = re.exec(src)) !== null) {
    // 跳过 `fn make_xxx(` 的定义体，只认调用点
    if (/fn\s+$/.test(src.slice(Math.max(0, m.index - 3), m.index))) continue;
    const openIdx = m.index + m[0].length - 1;
    const closeIdx = matchParen(src, openIdx);
    if (closeIdx < 0) continue;
    out.push(splitTopLevel(src.slice(openIdx + 1, closeIdx)));
  }
  return out;
}

/** 同 parseMakerRawArgs，但每个实参取「首个字符串字面量」（便捷版） */
function parseMakerArgs(src, fnName) {
  return parseMakerRawArgs(src, fnName).map((args) => args.map(firstStringLiteral));
}

/** 解析 edges：edge("id","src","tgt") / edge_cond("id","src","handle","tgt",EdgeType) */
function parseEdges(src) {
  const out = [];
  const re = /edge(?:_cond)?\s*\(/g;
  let m;
  while ((m = re.exec(src)) !== null) {
    // 跳过 `fn edge(` / `fn edge_cond(` 的定义体
    if (/fn\s+$/.test(src.slice(Math.max(0, m.index - 3), m.index))) continue;
    const openIdx = m.index + m[0].length - 1;
    const closeIdx = matchParen(src, openIdx);
    if (closeIdx < 0) continue;
    const s = splitTopLevel(src.slice(openIdx + 1, closeIdx)).map(firstStringLiteral);
    const isCond = m[0].startsWith('edge_cond');
    const source = s[1];
    const target = isCond ? s[3] : s[2];
    if (source && target) {
      out.push({
        source,
        target,
        line: src.slice(0, m.index).split('\n').length,
        index: m.index,
      });
    }
  }
  return out;
}

/**
 * 按 `fn build_xxx(` 切出各个模板的构建函数区间。
 * 一个种子文件里有 **4 个互相独立的 DAG**（各自的 trigger/end 同名），
 * 规则 C 必须逐模板建图 —— 混成一张图会得到 4 棵"不可达"的假树。
 */
function parseRegions(src) {
  const re = /fn\s+(build_[a-z_0-9]+)\s*\(/g;
  const hits = [];
  let m;
  while ((m = re.exec(src)) !== null) hits.push({ name: m[1], start: m.index });
  return hits.map((h, i) => ({
    name: h.name,
    start: h.start,
    end: i + 1 < hits.length ? hits[i + 1].start : src.length,
  }));
}

/** 取 CodeNode 引用的脚本常量正文（`const NAME: &str = r#"..."#;`） */
function codeConstBody(src, ident) {
  if (!ident || !/^[A-Za-z_][A-Za-z0-9_]*$/.test(ident)) return null;
  const re = new RegExp(`const\\s+${ident}\\s*:\\s*&str\\s*=\\s*r#"([\\s\\S]*?)"#;`);
  const m = re.exec(src);
  return m ? m[1] : null;
}

/** 脚本返回值里 map 字面量 `#{ k: v, ... }` 的键集合（拿不到就不判，避免误报） */
function returnedKeys(body) {
  if (!body) return null;
  const keys = new Set();
  const re = /#\{([^}]*)\}/g;
  let m;
  let found = false;
  while ((m = re.exec(body)) !== null) {
    found = true;
    for (const km of m[1].matchAll(/([A-Za-z_][A-Za-z0-9_]*)\s*:/g)) keys.add(km[1]);
  }
  return found && keys.size > 0 ? keys : null;
}

// ── 规则 ────────────────────────────────────────────────

function checkOne(src, rel, consumerKind, entry, byId, danglingKeys, stats) {
  const { source, target, line } = entry;
  const parts = source.split('.');

  if (parts.length === 1) {
    stats.single++;
    return null;
  }
  stats.multi++;

  const root = byId.get(parts[0]);
  if (!root) {
    return {
      rel,
      line,
      target,
      source,
      msg: `首段 '${parts[0]}' 未匹配任何已知节点 ID / output_var`,
    };
  }

  const second = parts[1];
  const fields = WRAPPER_FIELDS[root.kind] ?? new Set();

  // 规则 A：第 2 段必须是该节点包裹层上的真实字段
  if (!fields.has(second)) {
    const known = [...fields].join('/');
    const key = `${source}|${target}`;
    if (danglingKeys.has(key)) {
      stats.dangling.push({ rel, line, target, source });
      return null;
    }
    return {
      rel,
      line,
      target,
      source,
      msg: `${root.kind} 节点(${root.id}) 的包裹层没有字段 '${second}'（可用: ${known}）`,
    };
  }

  // 规则 B：消费端可达性（严格消费端有最大深度）
  if (STRICT_CONSUMER[consumerKind]) {
    const max = MAX_DEPTH_STRICT[root.kind];
    if (max == null) {
      return {
        rel,
        line,
        target,
        source,
        msg: `消费端 ${consumerKind} 是严格解析器，但源 root 是 ${root.kind}，包裹层形态未登记`,
      };
    }
    if (parts.length > max) {
      return {
        rel,
        line,
        target,
        source,
        msg:
          `消费端 ${consumerKind} 用不穿透的 resolve_var_path，` +
          `${root.kind} 源最深 ${max} 段，实际 ${parts.length} 段 ⇒ 中间值 '${parts[1]}' 是 JSON 字符串，` +
          `.get('${parts[2]}') 恒 None，整个路径静默解析为 None`,
      };
    }
  }

  if (second === 'content' || second === 'result') stats.ok++;
  else stats.okOther++;

  // 规则 B′：**严格**消费端 + **Tool 源** ⇒ 3 段的第 3 段只能是 result 自己的字段
  // （生产走 ToolResolver 回调路径，`result` 实测只有 `{"content": …}`，见
  //  WRAPPER_FIELDS 上方注释）⇒ 业务字段必须写 4 段 `<tool>.result.content.<字段>`，
  //  而严格解析器穿不透 content 字符串 ⇒ 恒 None。
  //  ⚠️ 2026-09-15：ToolNode 已不在严格档（转调共享宽松版），故本规则现在只对
  //  ConditionNode 等仍在册的严格消费端生效 —— 它们**能**穿透 4 段路径。
  if (STRICT_CONSUMER[consumerKind] && root.kind === 'tool' && parts.length === 3) {
    const RESULT_FIELDS = new Set(['content', 'truncated', 'is_error']);
    if (!RESULT_FIELDS.has(parts[2])) {
      return {
        rel,
        line,
        target,
        source,
        msg:
          `消费端 ${consumerKind} 用不穿透的 resolve_var_path，Tool 源在生产（回调路径）下 ` +
          `result 里只有 content ⇒ 业务字段的实到路径是 4 段 ` +
          `<tool>.result.content.${parts[2]}，严格消费端穿不透 content 字符串，` +
          `当前 3 段 \`${source}\` 解析为 None`,
      };
    }
  }

  return null;
}

/**
 * 规则 C：DAG 连通性（**逐模板**建图）。
 * 新增节点 / 改边最容易犯的错不是「路径写错」而是「漏接边」—— 孤立、不可达、
 * 或走不到 end。这类缺陷在静态形态校验里完全隐形，只有把 edges 真的建图才看得见。
 * Loop 的 body_steps 节点由 LoopExecutor 驱动、刻意不接 edges（dag_store.rs:130-141），
 * 故先剔除再判连通性。
 */
function checkDagIntegrity(scope, nodes, edges, violations, stats) {
  const bodySet = new Set(nodes.flatMap((n) => n.bodySteps));
  const graphNodes = nodes.filter((n) => !bodySet.has(n.id));
  const ids = graphNodes.map((n) => n.id);
  const idSet = new Set(ids);
  const inDeg = new Map(ids.map((i) => [i, 0]));
  const adj = new Map(ids.map((i) => [i, []]));
  const radj = new Map(ids.map((i) => [i, []]));
  let edgeCount = 0;

  for (const e of edges) {
    if (!idSet.has(e.source) || !idSet.has(e.target)) {
      // Loop body 节点之间的边本就不存在，这里只在两端都是「非 body」时才报
      if (bodySet.has(e.source) || bodySet.has(e.target)) continue;
      violations.push({
        rel: scope,
        line: e.line,
        target: e.target,
        source: e.source,
        msg: `规则 C：边引用了本模板不存在的节点（${!idSet.has(e.source) ? 'source' : 'target'}）`,
      });
      continue;
    }
    edgeCount++;
    inDeg.set(e.target, inDeg.get(e.target) + 1);
    adj.get(e.source).push(e.target);
    radj.get(e.target).push(e.source);
  }

  let noInbound = 0;
  for (const id of ids) {
    if (id === 'trigger') continue;
    if (inDeg.get(id) === 0) {
      violations.push({
        rel: scope,
        line: '—',
        target: id,
        source: `(${id})`,
        msg: '规则 C：非 trigger 节点入度为 0 ⇒ 永不执行（漏接边）',
      });
      noInbound++;
    }
  }

  const walk = (seed, graph) => {
    const seen = new Set([seed]);
    const stack = [seed];
    while (stack.length > 0) {
      for (const t of graph.get(stack.pop()) ?? []) {
        if (!seen.has(t)) {
          seen.add(t);
          stack.push(t);
        }
      }
    }
    return seen;
  };

  const reach = walk('trigger', adj);
  const coReach = walk('end', radj);
  let unreachable = 0;
  let deadEnd = 0;
  for (const id of ids) {
    if (!reach.has(id)) {
      violations.push({
        rel: scope,
        line: '—',
        target: id,
        source: `(${id})`,
        msg: '规则 C：从 trigger 不可达 ⇒ 死节点',
      });
      unreachable++;
    }
    if (!coReach.has(id)) {
      violations.push({
        rel: scope,
        line: '—',
        target: id,
        source: `(${id})`,
        msg: '规则 C：从该节点走不到 end ⇒ 死端（流程永远不结束）',
      });
      deadEnd++;
    }
  }

  stats.dagNoInbound += noInbound;
  stats.dagUnreachable += unreachable;
  stats.dagDeadEnd += deadEnd;
  stats.regions.push({
    scope,
    nodes: ids.length,
    edges: edgeCount,
    body: bodySet.size,
    noInbound,
    unreachable,
    deadEnd,
  });
}

/**
 * 规则 D：CodeNode 返回值 ↔ 下游读取键的契约。
 * 严格消费端读 `<code节点>.result.<key>`，若脚本返回的 map 里根本没有 `<key>`，
 * 该路径恒 None（与规则 B 同一类静默失效）。脚本正文取不到时**不判**（宁漏勿误）。
 */
function checkCodeReturnKeys(rel, nodes, src, violations, stats) {
  const byId = new Map();
  for (const n of nodes) {
    byId.set(n.id, n);
    if (n.outputVar && !byId.has(n.outputVar)) byId.set(n.outputVar, n);
  }
  for (const n of nodes) {
    for (const e of [...parsePairs(n), ...parseVarPaths(n)]) {
      const parts = e.source.split('.');
      if (parts.length !== 3 || parts[1] !== 'result') continue;
      const root = byId.get(parts[0]);
      if (!root || root.kind !== 'code') continue;
      const keys = returnedKeys(codeConstBody(src, root.codeIdent));
      if (!keys) continue;
      stats.retKeyChecked++;
      if (!keys.has(parts[2])) {
        violations.push({
          rel,
          line: e.line,
          target: e.target,
          source: e.source,
          msg:
            `规则 D：CodeNode '${root.id}' 返回的 map 键是 [${[...keys].join(', ')}]，` +
            `没有 '${parts[2]}' ⇒ 消费端恒 None`,
        });
      }
    }
  }
}

// ── 自检（正负对照）──────────────────────────────────────────────
// 为什么必须有：扫描器自身会撒谎（漏文件 / 剥错注释 / 规则前提被引擎改动后静默失效）。
// 沿同目录同族脚本的 `--selftest` 形态：**先证明它能红，再相信它的绿**。
// 6 组单点改动夹具（与 `output/_fixtures-v2.mjs` 同源，断言逐条一致）
// + 1 组正对照（生产文件必须 EXIT=0）。
//
// 夹具全部生成在系统临时目录，跑完即删 —— 不写仓库内任何文件。
const FIXTURES = [
  {
    name: '规则 A（第 2 段字段名错）',
    from: 'vec![("chapters_raw", "lc-outline.content")]',
    to: 'vec![("chapters_raw", "lc-outline.contents")]',
    expectContains: '包裹层没有字段',
  },
  {
    name: '规则 B（ConditionNode 3 段穿不透 Agent content 字符串）',
    from: 'var_path: "lc-conceive.content".to_string(),',
    to: 'var_path: "lc-conceive.content.genre".to_string(),',
    expectContains: '用不穿透的 resolve_var_path',
  },
  {
    name: '规则 B′（严格消费端读 Tool 源 3 段业务字段）',
    from: 'var_path: "lc-conceive.content".to_string(),',
    to: 'var_path: "lc-chapter-structure.result.genre".to_string(),',
    expectContains: '业务字段的实到路径是 4 段',
  },
  {
    name: '规则 C（删边 ⇒ 从该节点走不到 end）',
    from: 'edge("e-extract-fulltext-tolerance", "lc-extract-fulltext", "lc-tolerance-agent"),\n',
    to: '',
    expectContains: '规则 C',
  },
  {
    name: '规则 D（Rhai 返回键 full_text → fulltext）',
    from: '#{ full_text: text, char_count: text.len }',
    to: '#{ fulltext: text, char_count: text.len }',
    expectContains: '规则 D',
  },
  {
    name: '规则 E（注释里的 mapping 字面量不参与扫描）',
    from:
      'vec!["lc-conceive"],\n            1100.0,\n            -400.0,\n        ),',
    to:
      'vec!["lc-conceive"],\n            1100.0,\n            -400.0,\n        ),\n' +
      '        // 反向夹具：下面这行是**注释**，不得被当成活的 input_mapping\n' +
      '        // ("remaining_chapters", "lc-outline.remaining"),',
    expectContains: null, // null ⇒ 期望「零违规、EXIT=0」
    expectSameMultiAsBaseline: true,
  },
];

/** 以子进程方式跑本脚本（不传 --selftest，避免递归） */
function runGateOn(absPath) {
  try {
    const out = execFileSync(process.execPath, [SCRIPT_FILE, absPath], {
      encoding: 'utf8',
      stdio: ['ignore', 'pipe', 'pipe'],
    });
    return { exit: 0, out };
  } catch (e) {
    return { exit: e.status ?? -1, out: `${e.stdout ?? ''}${e.stderr ?? ''}` };
  }
}

/** 从门禁输出里取「多段源路径」条数 */
function multiOf(out) {
  return (out.match(/多段源路径\s*:\s*(\d+)/) || [])[1];
}

function runSelftest() {
  const srcPath = resolveInput(DEFAULT_FILE);
  const base = readFileSync(srcPath, 'utf8');
  const baseRun = runGateOn(srcPath);
  const baselineMulti = multiOf(baseRun.out);

  const lines = [];
  let failed = 0;
  const mark = (ok) => (ok ? '[PASS]' : '[FAIL]');

  // 正对照：生产文件必须零违规
  const posOk = baseRun.exit === 0;
  if (!posOk) failed++;
  lines.push(`${mark(posOk)} 正对照：生产文件当前零违规`);
  lines.push(`    期望: EXIT=0    实到: EXIT=${baseRun.exit}，多段源路径=${baselineMulti} 条`);

  const dir = resolve(tmpdir(), 'ax-check-input-mapping-selftest');
  mkdirSync(dir, { recursive: true });

  for (const f of FIXTURES) {
    const hits = base.split(f.from).length - 1;
    if (hits !== 1) {
      failed++;
      lines.push(`${mark(false)} ${f.name}`);
      lines.push(
        `    夹具锚点失效：在 ${DEFAULT_FILE} 中命中 ${hits} 次（期望恰好 1 次）—— ` +
          '源码形态已变，请同步更新夹具锚点',
      );
      continue;
    }
    const abs = resolve(dir, `${f.name.replace(/[^A-Za-z0-9]+/g, '_')}.rs`);
    writeFileSync(abs, base.replace(f.from, f.to));
    const { exit, out } = runGateOn(abs);
    try {
      unlinkSync(abs);
    } catch {
      /* 忽略 */
    }

    const wantFail = f.expectContains !== null;
    const exitOk = wantFail ? exit === 1 : exit === 0;
    const hitOk = !wantFail || out.includes(f.expectContains);
    const multiActual = multiOf(out);
    const multiOk =
      !f.expectSameMultiAsBaseline || String(multiActual) === String(baselineMulti);
    const ok = exitOk && hitOk && multiOk;
    if (!ok) failed++;

    lines.push(`${mark(ok)} ${f.name}`);
    lines.push(
      `    期望: ${wantFail ? `EXIT=1 且命中「${f.expectContains}」` : 'EXIT=0 且零违规'}` +
        (f.expectSameMultiAsBaseline ? `，多段源路径=${baselineMulti} 条` : ''),
    );
    lines.push(
      `    实到: EXIT=${exit}，多段源路径=${multiActual} 条` +
        (wantFail ? `，命中=${out.includes(f.expectContains)}` : ''),
    );
    const first = (out.match(/\[VIOLATION\][^\n]*\n[^\n]*/) || out.match(/RESULT:[^\n]*/) || [
      '<无>',
    ])[0];
    lines.push(`    首条: ${first.split('\n').join(' | ')}`);
  }

  console.log('── 自检：门禁正负对照（--selftest）──');
  console.log(lines.join('\n'));
  console.log('');
  console.log(
    failed === 0
      ? 'RESULT: PASS —— 6 组反向夹具全部按预期分类，且生产文件零违规'
      : `RESULT: FAIL —— ${failed} 条不符合预期`,
  );
  console.log('含义: PASS 只证明「对已知 6 种缺陷形态仍具检出能力」+「生产文件当前零违规」；');
  console.log('      它**不是** Rust 编译证据，也不覆盖本脚本规则之外的缺陷。');
  process.exit(failed > 0 ? 1 : 0);
}

function main() {
  const argv = process.argv.slice(2);
  if (argv.includes('--selftest')) {
    runSelftest();
    return;
  }
  const files = argv.filter((a) => !a.startsWith('--'));
  const targets = files.length > 0 ? files : [DEFAULT_FILE];

  const danglingKeys = new Set(KNOWN_DANGLING.map((d) => `${d.source}|${d.target}`));
  let violations = [];
  const stats = {
    multi: 0,
    single: 0,
    ok: 0,
    okOther: 0,
    dangling: [],
    regions: [],
    dagNoInbound: 0,
    dagUnreachable: 0,
    dagDeadEnd: 0,
    retKeyChecked: 0,
  };

  for (const rel of targets) {
    // maskComments 保持长度 ⇒ 行号与原文一致；所有下游解析统一用屏蔽后的源码
    const src = maskComments(readFileSync(resolveInput(rel), 'utf8'));
    const nodes = parseNodes(src);
    const byId = new Map();
    for (const n of nodes) {
      byId.set(n.id, n);
      if (n.outputVar && !byId.has(n.outputVar)) byId.set(n.outputVar, n);
    }
    for (const n of nodes) {
      const entries = [...parsePairs(n), ...parseVarPaths(n)];
      for (const e of entries) {
        const v = checkOne(src, rel, n.kind, e, byId, danglingKeys, stats);
        if (v) violations.push(v);
      }
    }
    // 规则 C / D：逐模板把 edges 建图 + CodeNode 返回值契约
    // （形态校验看不见的两类失效）
    const edges = parseEdges(src);
    for (const r of parseRegions(src)) {
      const regionNodes = nodes.filter((n) => n.start >= r.start && n.start < r.end);
      if (regionNodes.length === 0) continue;
      const regionEdges = edges.filter((e) => e.index >= r.start && e.index < r.end);
      checkDagIntegrity(`${rel}#${r.name}`, regionNodes, regionEdges, violations, stats);
    }
    checkCodeReturnKeys(rel, nodes, src, violations, stats);
  }

  // ── 棘轮：已知悬空出现次数只允许减少 ──
  for (const d of KNOWN_DANGLING) {
    const actual = stats.dangling.filter(
      (x) => x.source === d.source && x.target === d.target,
    ).length;
    if (actual > d.count) {
      violations.push({
        rel: targets.join(','),
        line: '—',
        target: d.target,
        source: d.source,
        msg: `棘轮：已知悬空 '${d.source}' 出现 ${actual} 次 > 基线 ${d.count} 次（只允许减少，新增同类须先修）`,
      });
    } else if (actual < d.count) {
      console.log(
        `[棘轮可收紧] '${d.source}' 实际 ${actual} 次 < 基线 ${d.count} 次 —— 请把基线 count 下调到 ${actual}`,
      );
    }
  }

  for (const v of violations) {
    console.error(
      `[VIOLATION] ${v.rel}:${v.line}  target='${v.target}'  source='${v.source}'\n` +
        `            原因: ${v.msg}\n` +
        `            期望: <node>.content.<field> (Agent 源 / 穿透消费端任意深；严格消费端 ≤2 段) / ` +
        `<node>.result.<field> (Tool|Code 源) / <loop>.items (Loop 源)`,
    );
  }

  console.log('── input_mapping / var_path 源路径校验 ──');
  console.log(`扫描文件      : ${targets.join(', ')}`);
  console.log(`多段源路径    : ${stats.multi} 条`);
  console.log(`  形态+可达均过: ${stats.ok} 条（第 2 段 = content/result）`);
  console.log(`  包裹层其它字段: ${stats.okOther} 条（Loop items 等，形态合法）`);
  console.log(`单段源路径    : ${stats.single} 条（节点 ID / 变量平键，按规则跳过）`);
  console.log(`已知悬空      : ${stats.dangling.length} 条（基线，非违规）`);
  for (const d of stats.dangling) {
    const k = KNOWN_DANGLING.find((x) => x.source === d.source && x.target === d.target);
    console.log(`  - ${d.rel}:${d.line}  '${d.source}'  ← target='${d.target}'`);
    console.log(`      ${k ? k.reason : ''}`);
  }
  console.log('── 规则 C：DAG 连通性（逐模板建图）──');
  for (const r of stats.regions) {
    console.log(
      `  ${r.scope.replace(/^.*#/, '')}: 节点 ${r.nodes}（Loop body ${r.body} 不计）` +
        ` / 边 ${r.edges} / 入度0=${r.noInbound} 不可达=${r.unreachable} 死端=${r.deadEnd}`,
    );
  }
  console.log(
    `合计: 入度0=${stats.dagNoInbound} 不可达=${stats.dagUnreachable} 死端=${stats.dagDeadEnd}（须全为 0）`,
  );
  console.log('── 规则 D：CodeNode 返回值契约 ──');
  console.log(`<code>.result.<key> 读取: ${stats.retKeyChecked} 处（键均已由脚本返回）`);

  const baselineTotal = KNOWN_DANGLING.reduce((a, d) => a + d.count, 0);
  console.log('');
  if (violations.length > 0) {
    console.log(`RESULT: FAIL —— ${violations.length} 条违规（未白名单）`);
    console.log('含义: 出现新违规，或已知基线条数上升。必须修判据/修数据，**不许加白名单**。');
    process.exit(1);
  }
  if (stats.dangling.length > 0) {
    console.log(`RESULT: OK-with-baseline —— 0 条新违规，${stats.dangling.length} 条已知悬空未修（基线 ${baselineTotal}）`);
    console.log('含义: 通过，但仍有已知缺陷挂在基线上 —— 基线只允许减少。');
    process.exit(0);
  }
  console.log('RESULT: OK —— 零违规、零基线残留');
  process.exit(0);
}

// 直接执行时才跑 main；被 `import` 时只导出解析器（供 output/_render-test.mjs
// 复用同一套解析逻辑 —— 避免「门禁」与「运行时形状测试」各写一份解析器而漂移）。
//
// ⚠️ Windows 上必须按**解析后的绝对路径 + 大小写不敏感**比较：`import.meta.url`
// 由 ESM loader 规范化，而 `process.argv[1]` 原样来自命令行，盘符大小写可能不一致
// （本仓踩过 vitest 的 `d:\` vs `D:\` 双实例坑）。
const __isMain = (() => {
  try {
    return (
      resolve(fileURLToPath(import.meta.url)).toLowerCase() ===
      resolve(process.argv[1] ?? '').toLowerCase()
    );
  } catch {
    return false;
  }
})();

export {
  maskComments,
  parseNodes,
  parseEdges,
  parsePairs,
  parseVarPaths,
  parseRegions,
  parseMakerArgs,
  parseMakerRawArgs,
  codeConstBody,
  returnedKeys,
  matchParen,
  splitTopLevel,
  firstStringLiteral,
  checkOne,
  checkDagIntegrity,
  checkCodeReturnKeys,
  KNOWN_DANGLING,
  WRAPPER_FIELDS,
  STRICT_CONSUMER,
  MAX_DEPTH_STRICT,
};

if (__isMain) main();
