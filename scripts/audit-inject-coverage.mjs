#!/usr/bin/env node
// 注入覆盖审计：某个 CodeNode(rhai) 脚本引用的变量，是否都有注入来源。
//
// 为什么需要它（A4 的前置论证 + A1 同型缺陷的通用检法）：
//   本项目已出过一次「数据到了门口没人开门」——`serenity_context` 早由 input_mapping
//   注入，但脚本侧零引用，核心方法论因此长期不参与决策（审计报告 A1）。
//   其**镜像形态**同样致命且更隐蔽：脚本 `present(X)` 引用了 X，但 X 无任何注入来源
//   ⇒ X 恒为 `()` ⇒ 对应因子**永远不激活**，且因为 `present()` 守卫的存在，
//   它**不报错、不打日志、静默降级**，与「数据缺失」在观感上完全一致。
//
// 判据（与 seed_stock_analysis.rs:27 的成文完备性判据同源）：
//   ① 差集 = present() 引用集合 − 脚本内自给(let/const) − 注入来源(手写 input_mapping
//      ∪ PORTFOLIO_MGR_TUNABLE_PARAMS 派生的同名映射) ⇒ **应为空**
//   ② 倒查 = 注入了但脚本全文零提及的 target ⇒ 悬空映射（冗余，低危；v51 删
//      stock_sector 即此类，有先例）
//
// 用法：
//   node scripts/audit-inject-coverage.mjs                 # 默认审计 portfolio-mgr
//   node scripts/audit-inject-coverage.mjs --strict        # ① 非空则退出码 1
//   node scripts/audit-inject-coverage.mjs --selftest      # 扫描器正负对照
//
// ⚠ 定位：这是**审计工具，不是 CI 门禁**，故默认不接 ci-check.mjs —— 它同时产出
//   低危的悬空映射清单，接成硬门禁会用噪声逼人关掉它。需要卡 ① 时用 `--strict`。
//
// ── 与 `check-input-mapping.mjs` 的分工（勿误判为重复实现）──
//   两个脚本都在审 `input_mapping`，但**审的是相反的一侧**：
//     · `check-input-mapping.mjs` 审 **source 侧（value）**：
//       `("target", "上游路径")` 里的路径形态对不对、消费端节点解析器能否穿透 JSON。
//       默认扫 `seed_content_media.rs`，已接 `ci-check.mjs`（硬门禁 + 悬空基线棘轮）。
//     · 本脚本审 **target 侧（key）**：
//       脚本 `present(X)` 引用的 X 有没有人注入；以及注入了的 target 有没有人用。
//   一方查「路对不对」，一方查「两端是否互指」⇒ 互补，合起来才覆盖映射表 ↔ 脚本的
//   双向一致。**不要**把本脚本改造成 source 侧检查（那才是真重复）。
//
// ── 本脚本的开发史（三次自证，留作「审计脚本自身会撒谎」的实证）──
//   ① 首版用 `indexOf("];")` 截数组体 ⇒ 区块注释里的 `[0,40)/[40,60)` 使配对错位；
//   ② 二版只找 `input_mapping: [` ⇒ 实际是**块表达式** `input_mapping: { .. }`，
//      定位失败静默退化为全文匹配；
//   ③ 三版元组正则写成 `\s*\)` ⇒ rustfmt 拆行的长名元组（`"src",\n)`）漏匹配，
//      产出 5 个**假的**「无注入来源」，与真缺陷混在一张表里无法区分。
//   三处都不是被测对象的问题。故本脚本内建 `--selftest`，并把「扫描面为 0」当失败。

import fs from "node:fs";
import path from "node:path";
import process from "node:process";

const ROOT = path.resolve(import.meta.dirname, "..");

const DEFAULTS = {
  script: "src-tauri/src/commands/portfolio-mgr.rhai",
  seed: "src-tauri/src/commands/stock_analysis_setup/seed_stock_analysis.rs",
  constName: "PORTFOLIO_MGR_TUNABLE_PARAMS",
  // seed 里定位本节点区块的锚：从 include_str! 本脚本处，到 nodes.push(<var>) 处
  includeMarker: 'include_str!("../portfolio-mgr.rhai")',
  pushMarker: "nodes.push(pm)",
};

/** 剥 Rust 行注释 / 块注释（字符串内容保留，故元组路径串不受影响）。 */
function stripRustComments(src) {
  let out = "";
  let i = 0;
  const n = src.length;
  while (i < n) {
    const c = src[i];
    const c2 = src[i + 1];
    if (c === "/" && c2 === "/") {
      while (i < n && src[i] !== "\n") {
        out += " ";
        i++;
      }
      continue;
    }
    if (c === "/" && c2 === "*") {
      i += 2;
      while (i < n && !(src[i] === "*" && src[i + 1] === "/")) {
        out += src[i] === "\n" ? "\n" : " ";
        i++;
      }
      i += 2;
      continue;
    }
    if (c === '"') {
      out += c;
      i++;
      while (i < n) {
        if (src[i] === "\\") {
          out += src[i] + (src[i + 1] ?? "");
          i += 2;
          continue;
        }
        out += src[i];
        if (src[i] === '"') {
          i++;
          break;
        }
        i++;
      }
      continue;
    }
    out += c;
    i++;
  }
  return out;
}

/** 括号配对（注释已剥 ⇒ 块内括号只可能来自真语法）。支持 `[..]` 与 `{..}`。 */
function bracketEnd(s, openIdx, openCh, closeCh) {
  let depth = 0;
  for (let i = openIdx; i < s.length; i++) {
    if (s[i] === openCh) depth++;
    else if (s[i] === closeCh) {
      depth--;
      if (depth === 0) return i;
    }
  }
  return -1;
}

/** 抽取「脚本引用的变量」与「脚本内自给变量」与「函数形参」。 */
export function scanRhaiscript(rhaiText) {
  // 函数形参：`fn present(x)` 的参数名会被 present(X) 正则误抓（X 就是形参本身）。
  const fnParams = new Set();
  for (const m of rhaiText.matchAll(/^[ \t]*fn\s+([A-Za-z_]\w*)\s*\(([^)]*)\)/gm)) {
    for (const p of m[2].split(",")) {
      const name = p.trim().replace(/^mut\s+/, "").split(/[\s:]/)[0];
      if (name) fnParams.add(name);
    }
  }
  // 脚本内自给：`present(X)` 语义是「X 存在且非 unit」，脚本自己 let 的变量恒满足。
  const localVars = new Set();
  for (const m of rhaiText.matchAll(/^[ \t]*(?:let|const)\s+(?:mut\s+)?([A-Za-z_]\w*)/gm)) {
    localVars.add(m[1]);
  }
  const presentRaw = new Set();
  for (const m of rhaiText.matchAll(/\bpresent\(\s*([A-Za-z_]\w*)\s*\)/g)) presentRaw.add(m[1]);

  const referenced = [...presentRaw].filter((v) => !fnParams.has(v)).sort();
  return { presentRaw, fnParams, localVars, referenced };
}

/** 按**顶层逗号**切分实参（尊重嵌套括号与两种引号字符串）。 */
function splitArgs(body) {
  const out = [];
  let depth = 0;
  let cur = "";
  let i = 0;
  while (i < body.length) {
    const c = body[i];
    if (c === '"' || c === "`") {
      cur += c;
      i++;
      while (i < body.length) {
        if (body[i] === "\\") {
          cur += body[i] + (body[i + 1] ?? "");
          i += 2;
          continue;
        }
        cur += body[i];
        if (body[i] === c) {
          i++;
          break;
        }
        i++;
      }
      continue;
    }
    if (c === "(" || c === "[" || c === "{") depth++;
    else if (c === ")" || c === "]" || c === "}") depth--;
    if (c === "," && depth === 0) {
      out.push(cur.trim());
      cur = "";
      i++;
      continue;
    }
    cur += c;
    i++;
  }
  if (cur.trim() !== "") out.push(cur.trim());
  return out;
}

/**
 * 抽取「可调用定义」：`fn name(a, b) {` 与 `let name = |a, b| {`。
 * 返回 `{ name, params, body }`，body 用于判断形参是否被 `present(形参)` 守卫。
 */
export function scanCallables(text) {
  const defs = [];
  const add = (name, paramsRaw, braceStart) => {
    const end = bracketEnd(text, braceStart, "{", "}");
    if (!name || end < 0) return;
    const params = paramsRaw
      .split(",")
      .map((p) => p.trim().replace(/^mut\s+/, "").split(/[\s:]/)[0])
      .filter(Boolean);
    defs.push({ name, params, body: text.slice(braceStart, end + 1) });
  };
  for (const m of text.matchAll(/^[ \t]*fn\s+([A-Za-z_]\w*)\s*\(([^)]*)\)\s*\{/gm)) {
    add(m[1], m[2], m.index + m[0].length - 1);
  }
  for (const m of text.matchAll(/^[ \t]*(?:let|const)\s+([A-Za-z_]\w*)\s*=\s*\|([^|]*)\|\s*\{/gm)) {
    add(m[1], m[2], m.index + m[0].length - 1);
  }
  return defs;
}

/** 收集某定义的全部调用点实参列表（`name.call(…)` 与 `name(…)`，跳过定义自身）。 */
function callSiteArgs(text, name) {
  const out = [];
  const re = new RegExp(`\\b${name}(?:\\.call)?\\s*\\(`, "g");
  let m;
  while ((m = re.exec(text)) !== null) {
    const before = text.slice(0, m.index);
    if (/(?:^|\s)(?:fn|let|const)\s+$/.test(before)) continue; // 定义行，不是调用点
    const open = m.index + m[0].length - 1;
    const close = bracketEnd(text, open, "(", ")");
    if (close < 0) continue;
    out.push(splitArgs(text.slice(open + 1, close)));
  }
  return out;
}

/**
 * 判据 ③：形参不能只被「排除」，要反过来查它的**实参来源**。
 *
 * 背景（2026-09-29 实测）：`ts` 是 `horizon_decision` 闭包的形参，被 `present(ts)` 守卫。
 * 旧版扫描器只识别 `fn` 形参、不识别闭包形参 ⇒ ① 恒报 `ts`「无注入来源」，
 * 而**真链路是齐的**（三个调用点传 `totalScore_short/mid/long`，都有 input_mapping）。
 * 只把闭包形参加进排除名单会变成「用假阳性换假阴性」：若某天调用点全改传字面量，
 * `present(ts)` 恒假、该档评分静默退化成日线评分 —— 正是本脚本要抓的形态。
 * 故：名单排除**以「至少一个调用点的对应实参是已注入变量或脚本内自给变量」为条件**。
 */
export function checkParams(fullText, defs, presentNames, sources, localVars) {
  const sourced = [];
  const unwired = [];
  for (const d of defs) {
    if (!d.params.some((p) => presentNames.has(p))) continue;
    const sites = callSiteArgs(fullText, d.name);
    d.params.forEach((p, idx) => {
      if (!presentNames.has(p)) return; // 未被 present() 守卫 ⇒ 不属本判据
      const got = sites.map((args) => args[idx]).filter((a) => a !== undefined && a !== "");
      const hit = got.filter((a) => sources.has(a) || localVars.has(a));
      if (hit.length > 0) {
        sourced.push({ name: d.name, param: p, from: [...new Set(hit)].sort() });
      } else {
        unwired.push({
          name: d.name,
          param: p,
          why: got.length === 0 ? "无调用点或实参个数不足" : `实参均非注入变量（${[...new Set(got)].join(" | ")}）`,
        });
      }
    });
  }
  return { sourced, unwired };
}

export function scanSeed(seedText, opts) {
  const lines = seedText.split(/\r?\n/);
  const startIdx = lines.findIndex((l) => l.includes(opts.includeMarker));
  const endIdx = lines.findIndex((l, i) => i > startIdx && l.includes(opts.pushMarker));
  if (startIdx < 0 || endIdx < 0) {
    return { ok: false, reason: `区块锚定位失败 start=${startIdx} end=${endIdx}` };
  }
  const block = lines.slice(startIdx, endIdx + 1).join("\n");
  const clean = stripRustComments(block);

  const decl = clean.match(/input_mapping:\s*([[{])/);
  if (!decl) return { ok: false, reason: "区块内未见 input_mapping:" };
  const openCh = decl[1];
  const closeCh = openCh === "[" ? "]" : "}";
  const openIdx = clean.indexOf(openCh, decl.index);
  const arrEnd = bracketEnd(clean, openIdx, openCh, closeCh);
  if (arrEnd <= openIdx) return { ok: false, reason: "input_mapping 括号配对失败" };
  const body = clean.slice(openIdx, arrEnd + 1);

  const mappingTargets = new Set();
  // ⚠ 必须容忍尾随逗号：rustfmt 对超 100 列的长名会把元组拆成
  //   `(\n  "long_name",\n  "src",\n),` —— `"src"` 与 `)` 之间隔着 `,`。
  for (const m of body.matchAll(/\(\s*"([^"]+)"\s*,\s*"([^"]+)"\s*,?\s*\)/g)) {
    mappingTargets.add(m[1]);
  }

  // 由常量派生的同名映射（`m.extend(CONST.iter().map(|n| (*n, *n)))`）
  const tm = seedText.match(
    new RegExp(`${opts.constName}:\\s*\\[&str;\\s*(\\d+)\\]\\s*=\\s*\\[([\\s\\S]*?)\\n\\];`),
  );
  const tunables = tm ? [...tm[2].matchAll(/"([^"]+)"/g)].map((m) => m[1]) : [];
  const declaredCount = tm ? Number(tm[1]) : -1;

  return { ok: true, mappingTargets, tunables, declaredCount, shape: openCh === "[" ? "数组字面量" : "块表达式" };
}

export function audit(rhaiText, seedText, opts) {
  const rhai = scanRhaiscript(rhaiText);
  const seed = scanSeed(seedText, opts);
  if (!seed.ok) return { ok: false, reason: seed.reason };

  const effective = new Set([...seed.mappingTargets, ...seed.tunables]);
  // 定义/调用点解析跑在**剥注释**后的文本上：注释里的示例 `fn`/`present(x)` 不是代码。
  const stripped = stripRustComments(rhaiText);
  const defs = scanCallables(stripped);
  const { sourced: paramSourced, unwired: paramUnwired } = checkParams(
    stripped,
    defs,
    rhai.presentRaw,
    effective,
    rhai.localVars,
  );
  // ① 的排除名单只收「已被 ③ 证明实参有来源」的形参 ⇒ 不是拿假阳性换假阴性。
  const provenParams = new Set(paramSourced.map((x) => x.param));
  const referenced = rhai.referenced.filter((v) => !provenParams.has(v));

  const selfProvided = referenced.filter((v) => rhai.localVars.has(v)).sort();
  const uncovered = referenced.filter((v) => !effective.has(v) && !rhai.localVars.has(v));
  // 倒查用「脚本全文是否提过这个名字」，而非只看 present()：有些变量无守卫直接使用。
  const dangling = [...seed.mappingTargets]
    .filter((t) => !new RegExp(`\\b${t}\\b`).test(rhaiText))
    .sort();

  return {
    ok: true,
    rhai,
    seed,
    effective,
    defs,
    referenced,
    selfProvided,
    uncovered,
    paramSourced,
    paramUnwired,
    dangling,
  };
}

function report(result, opts) {
  const { rhai, seed, effective, referenced, selfProvided, uncovered, paramSourced, paramUnwired, dangling } = result;
  console.log(`脚本: ${opts.script}`);
  console.log(`  ${opts.script.split("/").pop()}：${referenced.length} 个被引用变量`);
  console.log(`  函数形参（已排除）: ${[...rhai.fnParams].sort().join(", ") || "(无)"}`);
  console.log(`  present() 原始命中 ${rhai.presentRaw.size} 个 → 排除形参后 ${referenced.length} 个`);
  console.log(`input_mapping 形态: ${seed.shape} | 手写 target ${seed.mappingTargets.size} 个`);
  console.log(
    `${opts.constName}: 声明 ${seed.declaredCount} 项 / 解析到 ${seed.tunables.length} 项`,
  );
  console.log(`有效注入 target 合计: ${effective.size} 个`);

  console.log(`\n=== ① 被引用但无任何注入来源（差集，应为空）===`);
  if (uncovered.length === 0) {
    console.log("  (空) ✅ 全部有来源（注入 或 脚本内自给）");
  } else {
    uncovered.forEach((v) => console.log(`  ✗ ${v}`));
    console.log(
      `  ⇒ 这 ${uncovered.length} 个变量恒为 () ⇒ 对应分支静默不激活。`,
    );
  }
  console.log(`  （已排除脚本内自给 ${selfProvided.length} 个：${selfProvided.join(", ") || "(无)"}）`);

  console.log(`\n=== ② 注入但脚本全文零提及（悬空映射，仅供清理参考）===`);
  if (dangling.length === 0) {
    console.log("  (空) ✅ 无悬空映射");
  } else {
    dangling.forEach((v) => console.log(`  · ${v}`));
    console.log(`  ⇒ ${dangling.length} 个映射悬空（白注入，无功能危害；v51 曾同心处理 stock_sector）。`);
  }
  console.log(`
=== ③ 被 present() 守卫的形参，其实参是否有注入来源 ===`);
  if (paramSourced.length > 0) {
    paramSourced.forEach((x) => console.log(`  ✓ ${x.name}.${x.param} ← ${x.from.join(", ")}`));
  }
  if (paramUnwired.length === 0) {
    console.log("  (无悬空形参) ✅");
  } else {
    paramUnwired.forEach((x) => console.log(`  ✗ ${x.name}.${x.param} —— ${x.why}`));
    console.log(`  ⇒ 这 ${paramUnwired.length} 个形参上的 present() 恒假 ⇒ 该分支静默退化（它们已从 ① 剔除，不会重复出现在上面）。`);
  }

  return { uncovered, paramUnwired, dangling };
}

// ── 正负对照：证明扫描器**会告警**，而不是恒报空 ──
function selftest() {
  const opts = { ...DEFAULTS };
  const cases = [];
  const fakeSeed = (targets) => `
    let pm_code = include_str!("../portfolio-mgr.rhai").to_string();
    let pm = WorkflowNode::Code(CodeNode {
        config: CodeNodeConfig {
            input_mapping: {
                let mut m: Vec<(&str, &str)> = vec![
${targets.map((t) => `                    ("${t}", "src.${t}"),`).join("\n")}
                ];
                m.extend(PORTFOLIO_MGR_TUNABLE_PARAMS.iter().map(|n| (*n, *n)));
                m.into_iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
            },
        },
    });
    nodes.push(pm);
    pub(crate) const PORTFOLIO_MGR_TUNABLE_PARAMS: [&str; 1] = [
        "tunable_one",
    ];
  `;
  const run = (name, rhaiText, seedText, expectUncovered, expectDangling, expectParam = []) => {
    const r = audit(rhaiText, seedText, opts);
    if (!r.ok) {
      cases.push({ name, pass: false, note: `audit 失败: ${r.reason}` });
      return;
    }
    const gotU = r.uncovered.sort().join(",");
    const gotD = r.dangling.sort().join(",");
    const gotP = r.paramUnwired.map((x) => `${x.name}.${x.param}`).sort().join(",");
    const pass =
      gotU === [...expectUncovered].sort().join(",") &&
      gotD === [...expectDangling].sort().join(",") &&
      gotP === [...expectParam].sort().join(",");
    const pass = gotU === [...expectUncovered].sort().join(",") && gotD === [...expectDangling].sort().join(",");
    cases.push({
      name,
      pass,
    note: pass ? "ok" : `uncovered=[${gotU}] dangling=[${gotD}] paramUnwired=[${gotP}]`,
    });
  };

  // 负控：引用与注入齐备 ⇒ 两侧都应为空
  run(
    "负控 引用齐备 ⇒ 零告警",
    `fn present(x) { type_of(x) != "()" }\nlet a = 1;\nif present(b) { b }\n`,
    fakeSeed(["b"]),
    [],
    [],
  );
  // 正控 ①：引用了 X 但无注入 ⇒ 必须报 X
  run(
    "正控① 引用无注入 ⇒ 报缺失",
    `fn present(x) { type_of(x) != "()" }\nif present(orphan_var) { orphan_var }\n`,
    fakeSeed(["b"]),
    ["orphan_var"],
    ["b"],
  );
  // 正控 ②：脚本内自给 ⇒ 不得报缺失
  run(
    "正控② 脚本内 let 自给 ⇒ 不报缺失",
    `fn present(x) { type_of(x) != "()" }\nlet f7_signal = 0.5;\nif present(f7_signal) { f7_signal }\n`,
    fakeSeed(["b"]),
    [],
    ["b"],
  );
  // 正控 ③：函数形参 ⇒ 不得当成被引用变量
  run(
    "正控③ 函数形参 ⇒ 不报缺失",
    `fn present(x) { type_of(x) != "()" }\nfn safe_parse(raw) { if !present(raw) { return (); } raw }\n`,
    fakeSeed(["b"]),
    [],
    ["b"],
  );
  // ★ 回归护栏：rustfmt 拆行的长名元组必须被认到（首版在此漏匹配，凭空造出假缺失）
  run(
    "★护栏 多行元组(尾随逗号) ⇒ 认到注入",
    `fn present(x) { type_of(x) != "()" }\nif present(valuation_dcf_inapplicable_reason) { 1 }\n`,
    fakeSeed([]).replace(
      `                ];`,
      `                    (
                        "valuation_dcf_inapplicable_reason",
                        "t-valuation.result.content.x",
                    ),
                ];`,
    ),
    [],
    [],
  );

  // 正控④：闭包形参被 present() 守卫、调用点实参是已注入变量 ⇒ ①③ 都不报（`ts` 的真实形态）
  run(
    "正控④ 闭包形参实参有注入 ⇒ 不报",
    `fn present(x) { type_of(x) != "()" }
let decide = |h, sc| { if present(sc) { sc } else { 0.0 } };
let r = decide.call("mid", totalScore_mid);
`,
    fakeSeed(["totalScore_mid"]),
    [],
    [],
    [],
  );
  // 正控⑤：同形参但调用点改传字面量 ⇒ 必须报 decide.sc（排除形参不得退化成假阴性）
  run(
    "正控⑤ 闭包形参实参全字面量 ⇒ 报悬空形参",
    `fn present(x) { type_of(x) != "()" }
let decide = |h, sc| { if present(sc) { sc } else { 0.0 } };
let r = decide.call("mid", 50.0);
`,
    fakeSeed(["totalScore_mid"]),
    [],
    ["totalScore_mid"],
    ["decide.sc"],
  );
  // 正控⑥：守卫形参所在闭包无任何调用点 ⇒ 同样报出
  run(
    "正控⑥ 守卫形参无调用点 ⇒ 报悬空形参",
    `fn present(x) { type_of(x) != "()" }
let dead = |v| { if present(v) { v } else { 0.0 } };
`,
    fakeSeed(["b"]),
    [],
    ["b"],
    ["dead.v"],
  );

  console.log("=== 扫描器自检（正负对照）===");
  let failed = 0;
  for (const c of cases) {
    console.log(`  ${c.pass ? "✅" : "❌"} ${c.name}${c.pass ? "" : ` — ${c.note}`}`);
    if (!c.pass) failed++;
  }
  // ★ 护栏：扫描面不得为 0（路径形态错会扫 0 文件而不自知）
  const real = audit(
    fs.readFileSync(path.join(ROOT, DEFAULTS.script), "utf8"),
    fs.readFileSync(path.join(ROOT, DEFAULTS.seed), "utf8"),
    opts,
  );
  if (!real.ok || real.rhai.referenced.length === 0 || real.seed.mappingTargets.size === 0) {
    console.log("  ❌ ★护栏 真实扫描面为 0（路径/锚点已失效，结论不可信）");
    failed++;
  } else {
    console.log(
      `  ✅ ★护栏 真实扫描面非零（${real.rhai.referenced.length} 引用 / ${real.seed.mappingTargets.size} 手写 target）`,
    );
  }
  console.log(failed === 0 ? "\nSELFTEST PASS" : `\nSELFTEST FAIL (${failed})`);
  return failed === 0 ? 0 : 1;
}

function main() {
  const argv = process.argv.slice(2);
  if (argv.includes("--selftest")) process.exit(selftest());
  const strict = argv.includes("--strict");
  const opts = { ...DEFAULTS };
  for (let i = 0; i < argv.length; i++) {
    if (argv[i] === "--script") opts.script = argv[++i];
    else if (argv[i] === "--seed") opts.seed = argv[++i];
  }
  const result = audit(
    fs.readFileSync(path.join(ROOT, opts.script), "utf8"),
    fs.readFileSync(path.join(ROOT, opts.seed), "utf8"),
    opts,
  );
  if (!result.ok) {
    console.log(`❌ 审计无法进行: ${result.reason}`);
    process.exit(2);
  }
  const { uncovered, paramUnwired } = report(result, opts);
  if (strict && (uncovered.length > 0 || paramUnwired.length > 0)) {
    console.log(`\nSTRICT FAIL: ${uncovered.length} 个被引用变量无注入来源`);
    process.exit(1);
  }
  console.log("\n（默认非门禁模式：退出码 0。要卡 ① 请加 --strict）");
}

if (process.argv[1] && process.argv[1].endsWith("audit-inject-coverage.mjs")) main();
