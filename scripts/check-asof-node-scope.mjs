#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-only
// as-of（回放）接线一致门 —— 防「接了参数却没接线」（2026-10-11）。
//
// ## 为什么要有这道门
// 2026-10-11 实测：趋势智选（serenity）命令**收下并解析了 `as_of_date`**，却只用裸
// `AS_OF.scope` 包住 `run_workflow`。而工作流引擎为**每个节点单独 spawn**
// （`rt-workflow/.../engine/mod.rs` 的 `JoinSet::spawn`），`tokio::task_local` 不跨
// spawn ⇒ 节点内 `current_as_of()` 读不到截止日 ⇒ 回放时全部按 live 取数，UI 上却
// 显示成回放松紧如常。主分析链在 2026-10-03（P1-8）已补 `RunOptions.node_task_scope`，
// serenity 没跟上这次收编 —— 同一形态在下一条链上还会复发。
//
// 同轮另外两处同类断裂也由本门锁住：
//   ① `AS_OF.scope` 只罩到工作流运行边界，其后的落库前取数（行情/日线）会退回 live；
//   ② 回放结果照写 `reco_picks`（`generated_at=now`、`synthetic=0`）⇒ 与实盘票混进
//      同一张历史表，面板分不出来；全局 serenity 种子还会把历史候选带进下次实盘池。
//
// ## 规则（粒度 = 顶层函数段，即从一列起的 `fn` 到下一个一列起的 `fn`）
// R1 硬：段内出现 `RunOptions {` 且带「本轮有 as-of」的任一标记
//    （`parse_asof_param(` / `with_optional_asof(` / `as_of_ctx` / `AS_OF.scope(`）
//    ⇒ 必须出现 `node_task_scope`。
// R2 硬：两类「写进实盘可见状态」的调用点，**判据不同向**（2026-10-12 裁定改了其一）——
//    `persist_reco_pick(s)(` 的前窗口内必须有**代际写入**（`BatchKey { as_of_date: .. }`）：
//    回放的候选现在要落库供反思，防污染靠代际列而不是靠跳过写库；`fn` 定义位不计入；
//    `set_serenity_seed(` 的前窗口内仍必须有 **live 守卫**（`as_of_ctx.is_none()` 等）：
//    那是进程内状态，没有代际可打，被回放写过就分不回来。
// R4 硬：`init/services.rs` 里 cron 数据分支的 spawn 任务体必须压 `declare_cron_live_mode()`
//    （下限 6 = 现网数据敏感分支数）。定时链从不接 `as_of_date`，不显式声明 live 时，
//    并发回放留在全局作用域栈上的截止日会渗进取数 —— 那是 2026-09-23 事故的镜像形态。
// R3 硬（前端）：趋势智选面板必须把「本轮口径」绑到 payload，而不是渲染时读全局时间锚点
//    （T11 复发形态）⇒ 事件侧 `setLastRunAsOfDate(` 与渲染侧 `lastRunAsOfDate` 都必须存在。
// 自证：`--selftest` 用合成骨架证明检测器不空转，共负控十三项（拆钩子 / 拆 asOfDate 键 /
//    代际写在邻域外 / 完全没传代际 / 种子守卫写在调用点之后 / 拆种子守卫 / 无调用点不误报 /
//    定义位不算调用点 / 排除定义不放过真调用点 / 快照校验只剩定义 / 调用点藏在注释里 /
//    逐条拆面板绑定 / 定时声明计数），
//    以及「token 只出现在注释或字符串里不算数」的剥离负控。
//
// ⚠ 判据两侧都在**剥注释后、空白归一**的骨架上比 —— 直接 grep 原文件会被注释里的
//   示例串骗过，也会被 rustfmt/dprint 的折行打断。
//
// 用法：node scripts/check-asof-node-scope.mjs [--selftest] [--dump]

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const COMMAND_DIR = path.join(ROOT, "src-tauri/src/commands");
const PANEL_FILE = "src/components/stock-analysis/SerenityScreeningPanel.tsx";

/** R2 的「前窗口」宽度（骨架字符数）：证据与调用点之间只允许隔着少量代码。 */
const GUARD_WINDOW = 600;
/** R1 的「本轮带 as-of」标记：命令侧解析，或上层已把 ctx 传进来。 */
const ASOF_MARKERS = [
  "parse_asof_param(",
  "with_optional_asof(",
  "as_of_ctx",
  "AS_OF.scope(",
];
/**
 * R2：两类「写进实盘可见状态」的调用点，判据**不同向**（2026-10-12 裁定改了其一）：
 *  - `persist_reco_pick(s)(`：回放的候选**要**落库（不落就没有样本可供反思），所以这里锁的是
 *    「写库时带上代际」—— 批次键 `BatchKey { as_of_date, .. }` 的具名字段必须落在调用点前窗口，
 *    不再是 10-11 那版的「回放跳过写库」。仍接受旧的 `is_none()` 守卫形态会让门对
 *    「忘了打标记」睁一只眼。
 *    ⚠ 检法随接线形态一起改过（2026-10-29）：这一版原先盯命令层的 `pick.insert(db)`，
 *    落库下沉到 dao 后该字面量在命令层彻底消失 ⇒ R2 只剩种子那半边在计数，把四条真实
 *    落库链（主荐股 / 趋势智选 / 时间旅行批次 / 漏检回放）全放到了证据之外，直到
 *    「命中数下限」报出「判据失效」才暴露。**下限是唯一还在起作用的部分** —— 这条教训
 *    写在这里，是为了让下一次「把证据改成现网恰好长什么样」之前先想起来问一句：
 *    现网那四种写法互不相同（`as_of_date:` / `as_of_date,` / `Some(&as_of_str)`），
 *    按链各配证据串等于把门退化成记账；所以改的是**生产形态**（统一收成必填的 BatchKey），
 *    不是把证据串写成四个别名。
 *  - `set_serenity_seed(`：进程内可变状态，没有代际可打，一旦被回放写过就分不回来
 *    ⇒ 仍必须被 live 守卫挡住。
 * `calls` 是调用点族：同一族里任一出现都算一个落库调用点。
 */
const PERSIST_CALLS = [
  {
    calls: ["persist_reco_pick(", "persist_reco_picks("],
    // `as_of_date:` = 具名赋值；`as_of_date,` = struct shorthand（`BatchKey { as_of_date, .. }`）。
    // shorthand 必须一起认：写全 `as_of_date: as_of_date` 会撞 clippy::redundant_field_names，
    // 而该仓 clippy 是 `-D warnings` —— 逼调用方为了过门改写法，等于让门和门禁互相打架。
    evidence: ["as_of_date:", "as_of_date,"],
    label: "代际写入（BatchKey 的 as_of_date 字段）",
    // 证据落在调用点**邻域**（前后各一窗）：批次键现在是调用点的实参字面量
    // （`persist_reco_picks(.., &BatchKey { as_of_date: .. })`），也可能像趋势智选那样
    // 在循环外先构好再传 `&batch_key`。两侧的具名写入都算「想过代际」。
    // ⚠ 这里放弃旧的「只看前窗口」方向性，理由要说清：旧形态下方向性防的是
    // 「先 insert 再补 as_of_date」，而那个字段现在收进了 BatchKey 的**必填字段**，
    // 顺序错了直接编译不过 —— 方向性已由类型系统承担，门只需盯「邻域内有没有具名代际」。
    side: "near",
  },
  {
    calls: ["set_serenity_seed("],
    evidence: ["as_of_ctx.is_none()", "as_of_date.is_none()"],
    label: "live 守卫",
    // 种子没有代际可打，守卫必须是**包住它的那个 if** ⇒ 只看前窗口，方向性保留。
    side: "before",
  },
];

/**
 * 证据窗口：按 spec.side 取调用点邻域。
 * `near` = 前后各 GUARD_WINDOW；`before` = 只看前 GUARD_WINDOW。
 */
function evidenceWindow(body, at, side) {
  return side === "near"
    ? body.slice(Math.max(0, at - GUARD_WINDOW), at + GUARD_WINDOW + 400)
    : body.slice(Math.max(0, at - GUARD_WINDOW), at);
}

/**
 * 调用点族里排除**定义位**：`pub async fn persist_reco_picks(` 自身含该字面量，
 * 把定义当调用点计数会让读数虚高、并在定义段里判「没带代际」（它的形参就叫
 * as_of_date，证据是巧合不是接线）。与 R4「只数包裹点、`fn cron_live<F>` 定义行不计入」
 * 同一条理由：会说谎的读数比没有读数更坏。
 */
function callSites(body, calls) {
  const hits = [];
  for (const call of calls) {
    let at = body.indexOf(call);
    while (at !== -1) {
      const isDefinition = body.slice(Math.max(0, at - 3), at) === "fn ";
      if (!isDefinition) hits.push(at);
      at = body.indexOf(call, at + call.length);
    }
  }
  return hits;
}

/**
 * 剥 Rust 注释，保留字符串与 raw string 原样。
 * 为什么要状态机而不是行注释正则：字符串里的 `//` 会把后半行当注释吃掉 ⇒
 * 被检查的调用点**静默消失**，门就成了恒真。Rust 的块注释还可嵌套，同样要计数。
 */
export function stripRustComments(src) {
  let out = "";
  let i = 0;
  const n = src.length;
  while (i < n) {
    const c = src[i];
    const c2 = src[i + 1];
    if (c === "/" && c2 === "/") {
      while (i < n && src[i] !== "\n") i += 1;
      continue;
    }
    if (c === "/" && c2 === "*") {
      let depth = 1;
      i += 2;
      while (i < n && depth > 0) {
        if (src[i] === "/" && src[i + 1] === "*") {
          depth += 1;
          i += 2;
        } else if (src[i] === "*" && src[i + 1] === "/") {
          depth -= 1;
          i += 2;
        } else {
          i += 1;
        }
      }
      continue;
    }
    // r#"..."# / r##"..."##：内容整段原样保留
    if (c === "r" && c2 === "#") {
      let hashes = 0;
      let j = i + 1;
      while (src[j] === "#") {
        hashes += 1;
        j += 1;
      }
      const closer = `"${"#".repeat(hashes)}`;
      const end = src.indexOf(closer, j + 1);
      const stop = end === -1 ? n : end + closer.length;
      out += src.slice(i, stop);
      i = stop;
      continue;
    }
    // 字符字面量（`'"'` / `'\''` / `'\\'` / `'\n'`）整段跳过。
    // 不处理时 `'"'` 里的双引号会被当成字符串起点 ⇒ 从那里到下一个双引号之间的
    // **代码（含文档注释）被当作字符串保留**：2026-10-30 实测，`rest.find('"')` 让其后
    // 12 行 `///` 进了骨架，代际门于是在文档里读出一个 `pick.insert(db)` 池写形态。
    // 生命周期（`'a` / `'static`）不满足闭合形态，按普通字符放行。
    if (c === "'") {
      const afterQuote = i + 1;
      if (src[afterQuote] === "\\") {
        let j = afterQuote + 2;
        while (j < n && src[j] !== "'" && src[j] !== "\n") j += 1;
        if (src[j] === "'") {
          out += src.slice(i, j + 1);
          i = j + 1;
          continue;
        }
      } else if (src[afterQuote + 1] === "'") {
        out += src.slice(i, afterQuote + 2);
        i = afterQuote + 2;
        continue;
      }
      out += c;
      i += 1;
      continue;
    }
    if (c === "\"") {
      out += c;
      i += 1;
      while (i < n) {
        if (src[i] === "\\") {
          out += src[i] + (src[i + 1] ?? "");
          i += 2;
          continue;
        }
        out += src[i];
        if (src[i] === "\"") {
          i += 1;
          break;
        }
        i += 1;
      }
      continue;
    }
    out += c;
    i += 1;
  }
  return out;
}

/** 空白归一：折行与缩进都不该影响判据。 */
export function normalize(text) {
  return text.replace(/\s+/g, " ");
}

/** 剥注释 + 归一后的骨架（仅供自证样本使用；真扫按段归一，见 splitTopLevelFns）。 */
function skeleton(src) {
  return normalize(stripRustComments(src));
}

/**
 * 按「一列起的 `fn`」把**剥注释但保留换行**的文本切成顶层函数段。
 * ⚠ 顺序不能反：先做空白归一会抹掉行首，行锚判据便恒不命中 ⇒ 段数 0 ⇒
 * 整道门空转而报绿（本门第一版就是这么错的，靠「命中数下限」断言才暴露）。
 * 归一只在**段内**做，用于后面的 token 比对。
 *
 * ⚠ 段还必须在**一列起的 `#[cfg(test)]`** 处切断（2026-10-20 实测缺陷）：行首正则认不到缩进
 * 在 `mod tests { … }` 里的测试函数 ⇒ 文件最后一个生产函数的体会一路吞掉整个测试模块，
 * 判据于是被测试文本喂饱。触发实例：给 dao `reco_pool.rs` 补了带 `as_of_date` 形参的测试助手后，
 * 把生产侧那行 `AsOfDate.is_null()` 谓词整行删掉，代际门 R1 仍然报绿。
 * 这两道门管的是**生产接线**，测试模块里的字面量不算接线 ⇒ 一律排除在段之外。
 */
export function splitTopLevelFns(stripped) {
  const starts = [];
  const re =
    /^(?:pub |pub\(crate\) )?(?:const )?(?:async )?fn ([A-Za-z0-9_]+)/gm;
  let m;
  while ((m = re.exec(stripped)) !== null) {
    starts.push({ name: m[1], pos: m.index });
  }
  const testMods = [...stripped.matchAll(/^#\[cfg\(test\)\]/gm)].map((x) => x.index);
  return starts.map((s, idx) => {
    let end = idx + 1 < starts.length ? starts[idx + 1].pos : stripped.length;
    for (const t of testMods) {
      if (t > s.pos && t < end) {
        end = t;
      }
    }
    return { name: s.name, body: normalize(stripped.slice(s.pos, end)) };
  });
}

function listRustFiles(dir) {
  const out = [];
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      out.push(...listRustFiles(p));
    } else if (entry.name.endsWith(".rs") && !entry.name.startsWith("test_")) {
      out.push(p);
    }
  }
  return out;
}

/**
 * R1/R4：把 as-of 带进工作流引擎的命令段，必须同时具备两件接线。
 * 缺 ① = 节点内读不到截止日（回放静默跑 live）；
 * 缺 ② = 前端绑了 `p.asOfDate` 却恒拿不到键 ⇒ 呈现层退回「看不出是不是回放」。
 */
export function missingEngineWirings(body) {
  const missing = [];
  if (!body.includes("node_task_scope")) missing.push("node_task_scope（节点任务作用域钩子）");
  if (!body.includes("\"asOfDate\":")) missing.push("\"asOfDate\":（把本轮口径回灌给前端）");
  return missing;
}

/** R1 + R2 */
export function checkRustSources(dump) {
  const violations = [];
  let fnCount = 0;
  let r1Hits = 0;
  let r2Hits = 0;
  const files = listRustFiles(COMMAND_DIR).filter((f) => !path.basename(f).startsWith("test"));
  for (const file of files) {
    const src = fs.readFileSync(file, "utf8");
    if (!src.includes("fn ")) continue;
    for (const fn of splitTopLevelFns(stripRustComments(src))) {
      fnCount += 1;
      const rel = path.relative(ROOT, file);
      // R1：把 as-of 带进引擎的调用点必须挂节点作用域钩子
      if (fn.body.includes("RunOptions {") && ASOF_MARKERS.some((m) => fn.body.includes(m))) {
        r1Hits += 1;
        for (const miss of missingEngineWirings(fn.body)) {
          violations.push(
            `R1 ${rel}::${fn.name} —— 带 as-of 进引擎却缺接线「${miss}」⇒ 节点读不到截止日` +
              `（回放静默跑 live），或前端拿不到本轮口径`,
          );
        }
      }
      // R2：写进实盘可见状态的调用点必须带上各自的证据（代际写入 / live 守卫）
      for (const spec of PERSIST_CALLS) {
        for (const at of callSites(fn.body, spec.calls)) {
          r2Hits += 1;
          const window = evidenceWindow(fn.body, at, spec.side);
          if (!spec.evidence.some((g) => window.includes(g))) {
            violations.push(
              `R2 ${rel}::${fn.name} —— 调用点（${spec.calls.join(" 或 ")}）的` +
                `${spec.side === "near" ? "邻域" : "前窗口"} ${GUARD_WINDOW} 字符内没有 ` +
                `${spec.label} ⇒ 回放产物会与实盘同表同种子、分不出代际`,
            );
          }
        }
      }
      if (dump) console.log(`  段 ${rel}::${fn.name} (${fn.body.length} 字符)`);
    }
  }
  // 命中面下限：读不到样本 ⇒ 是「扫描面/切段失效」，不是「恰好都合规」。
  // （2026-10-11 本门第一版就是切出 0 段却报绿 —— 靠这三条下限才暴露。）
  if (fnCount === 0) {
    violations.push("判据失效: 命令层切出 0 个顶层函数段 ⇒ 扫描面没读到任何代码");
  }
  if (r1Hits < 2) {
    violations.push(
      `判据失效: R1 只命中 ${r1Hits} 处（现网至少有主分析链与趋势智选两处 as-of + RunOptions）`,
    );
  }
  // 下限 2 的含义是「落库链 + 种子链各至少一处」，不是现网调用点点名清单。
  // （2026-10-29 校准：旧文案写「趋势智选链至少 2 处」，指的是 `pick.insert(db)` +
  //  `set_serenity_seed(`；落库下沉 dao 后前者消失，靠这条下限才把「R2 已经看不见落库」
  //  暴露出来 —— 下限本来就是为这一刻准备的，别把它调低。）
  if (r2Hits < 2) {
    violations.push(
      `判据失效: R2 只命中 ${r2Hits} 个落库/种子调用点（现网至少有 persist_reco_pick(s) 落库与 set_serenity_seed 两类）`,
    );
  }
  return { violations, fnCount, r1Hits, r2Hits, fileCount: files.length };
}

/** R3 的三条绑定：缺任何一条，回放口径就不再是「本轮」的。 */
export function panelBindFlags(skel) {
  return {
    eventBind: skel.includes("store.setLastRunAsOfDate("),
    fallbackBind: skel.includes("setLastRunAsOfDate(typeof r?.asOfDate"),
    renderBind: skel.includes("lastRunAsOfDate !== null"),
  };
}

/** TS/TSX 剥注释（行注释 + 块注释），保留换行。 */
function stripTsComments(src) {
  return src.replace(/\/\*[\s\S]*?\*\//g, "").replace(/\/\/[^\n]*/g, "");
}

/**
 * R4：定时链必须**自己声明模式**。cron 分支从不接 `as_of_date`，而 `current_as_of()` 在
 * task_local 不可见时回落进程级全局作用域栈 ⇒ 若恰逢别的会话在跑回放，到点的定时任务会
 * 静默按那个截止日取数并把截断产物落进实盘表（2026-09-23 事故的镜像形态）。
 * 正解：用 task_local 显式 `None` 包住 spawn 的任务体（本仓 helper `cron_live`）。
 * ⚠ 不得改用 `enter_global_asof(None)`：全局栈「最后压入者赢」，定时一压 None 会把同窗口的
 *   时间旅行批次反读成 live —— 那是用一种污染换另一种污染（R4 也直接拦这种写法）。
 * 下限 6 = 现网数据敏感分支数（荐股 / 自选股扫描 / 候选池 / 反思 / 决策回测 / 兜底 workflow）；
 * 低于下限即红 —— 读数为 0 只会是「被人删了」，不是「恰好不需要」。
 */
export function checkCronLiveMode(minimum = 6) {
  const rel = "src-tauri/src/init/services.rs";
  const text = fs.readFileSync(path.join(ROOT, rel), "utf8");
  // 判据跑在**剥注释后**的面上：权威文件自己的 `///` 里写着「不要用 enter_global_asof(None)」
  // 和「cron_live(async move {」的说明，按原文匹配会把文档当成代码（自指误报）。
  const code = stripRustComments(text);
  // 只数**包裹点**：`fn cron_live<F>` 的定义行不计入，否则读数虚高一格 = 会说谎的读数。
  const calls = countOccurrences(code, "cron_live(async move {");
  const violations = [];
  if (calls < minimum) {
    violations.push(
      `R4 ${rel} —— 定时任务体的 live 包裹只有 ${calls} 处（下限 ${minimum}）` +
        `⇒ 有分支没声明自己的模式，并发回放的全局残留会渗进定时取数`,
    );
  }
  if (code.includes("enter_global_asof(None)")) {
    violations.push(
      `R4 ${rel} —— 定时链改用了进程级 enter_global_asof(None)：会反噬同窗口的时间旅行批次` +
        `（全局栈「最后压入者赢」），必须用 cron_live 的 task_local 显式 None`,
    );
  }
  return { violations, calls, minimum };
}

/** R5 的计数判据（抽成纯函数：自测要在合成骨架上拆它，读真文件的自测与现网同源、测不到判据）。 */
export function batchSnapshotCalls(code) {
  return countOccurrences(code, "require_pool_snapshot_coverage(");
}

/**
 * R5：时间旅行批次进 guard 前必须校验「该 as-of 日有建池快照」。
 * 建池依赖的榜单维度在回放里必须有历史正门：热股榜自 2026-10-18 起可走同花顺
 * `history/v1/rank`，其余当日榜单仍以本地每日快照为准；缺快照就是空池 ⇒ 整批候选 0，
 * 而批次终态只留一句「均无候选（可能当日数据缺失或阈值过高）」——
 * 把「这个日期根本不可回放」和「阈值太高所以没选出来」挤进同一句。
 * 下限 2 = 定义 + 调用点各一处：只剩定义（死代码）比压根没写更坏，因为它让下一次摘掉
 * 调用点看起来像"逻辑还在"。
 */
export function checkBatchSnapshotGate() {
  const rel = "src-tauri/src/commands/time_travel_batch.rs";
  const code = stripRustComments(fs.readFileSync(path.join(ROOT, rel), "utf8"));
  const calls = batchSnapshotCalls(code);
  const violations = [];
  if (calls < 2) {
    violations.push(
      `R5 ${rel} —— 建池快照覆盖校验只出现 ${calls} 处（定义 + 调用点应各 1）` +
        `⇒ 不可回放的日期会重新变成「跑完静默零候选」`,
    );
  }
  return { violations, calls };
}

/** 出现次数（判据用字面计数，不做语义解析 —— 靶子是「有没有那一行」）。 */
export function countOccurrences(haystack, needle) {
  let n = 0;
  let at = haystack.indexOf(needle);
  while (at !== -1) {
    n += 1;
    at = haystack.indexOf(needle, at + needle.length);
  }
  return n;
}

/** R3：面板的回放口径必须绑本轮 payload，不读全局锚点 */
export function checkPanel(dump) {
  const violations = [];
  const file = path.join(ROOT, PANEL_FILE);
  const src = fs.readFileSync(file, "utf8");
  const skel = normalize(stripTsComments(src));
  // 事件侧与 invoke 兜底侧都要把本轮日期写进 store（缺一侧就是「按 payload 显示、按锚点落库」）
  const { eventBind, fallbackBind, renderBind } = panelBindFlags(skel);
  if (!eventBind) {
    violations.push(`R3 ${PANEL_FILE} —— completed 事件没把 payload 的 asOfDate 写进 store`);
  }
  if (!fallbackBind) {
    violations.push(`R3 ${PANEL_FILE} —— invoke 兜底路径没写 lastRunAsOfDate（两路必须同源）`);
  }
  if (!renderBind) {
    violations.push(`R3 ${PANEL_FILE} —— 渲染层没消费 lastRunAsOfDate（口径显示回退到全局锚点）`);
  }
  if (dump) {
    console.log(`  面板绑定：事件=${eventBind} 兜底=${fallbackBind} 渲染=${renderBind}`);
  }
  return { violations };
}

function selftest() {
  const violations = [];
  const fail = (msg) => violations.push(msg);

  // 剥离器：token 只在注释里 ⇒ 不算数（否则门恒真）
  const commented = skeleton("// node_task_scope 已接线\nfn a() { RunOptions { } parse_asof_param(x); }");
  if (commented.includes("// node_task_scope") || !commented.includes("RunOptions {")) {
    fail("剥离失败：注释没剥掉或代码被误剥");
  }
  // 剥离器：`//` 出现在字符串里不得吃掉后面的代码
  const urlish = skeleton(
    'fn b() { let u = "https://x//y"; persist_reco_pick(db, &p, &BatchKey { as_of_date: d }); }',
  );
  if (!urlish.includes("persist_reco_pick(db")) {
    fail("剥离失败：字符串里的 // 把后面的代码当注释吃掉了");
  }

  // 正样本要同时给出现代两种合法形态：落库**带代际写入**、种子**被 live 守卫挡住**
  const good =
    "fn run(x) { let ctx = parse_asof_param(d); let o = RunOptions { node_task_scope: Some(h) }; " +
    'let j = json!({ "asOfDate": d }); ' +
    "if let Err(e) = persist_reco_pick(db, &p, &BatchKey { generated_at: g, as_of_date: d, purpose: q }) { } " +
    "if as_of_ctx.is_none() { set_serenity_seed(s); } }";
  const mk = (body) => skeleton(body);
  const seg = (body) => splitTopLevelFns(mk(body));

  // 正样本：R1/R2 都不报
  const probe = (body, spec) => {
    const b = seg(body)[0]?.body ?? "";
    const sites = callSites(b, spec.calls);
    if (sites.length === 0) return "no-call";
    // 同族多个调用点：任一缺证据就判 unguarded（否则「一处带代际」会替另一处背书）
    return sites.every((at) => {
      const window = evidenceWindow(b, at, spec.side);
      return spec.evidence.some((g) => window.includes(g));
    })
      ? "guarded"
      : "unguarded";
  };
  if (missingEngineWirings(seg(good)[0].body).length !== 0) {
    fail(`自证失败：正样本被判缺接线 ${missingEngineWirings(seg(good)[0].body)}`);
  }
  if (probe(good, PERSIST_CALLS[0]) !== "guarded") {
    fail("自证失败：正样本的落库代际写入没读出来");
  }
  if (probe(good, PERSIST_CALLS[1]) !== "guarded") {
    fail("自证失败：正样本的 set_serenity_seed live 守卫没读出来");
  }

  // 负控 1：拆掉 node_task_scope ⇒ R1 形态必须命中
  const noHook = good.replace(" node_task_scope: Some(h)", "");
  if (noHook.includes("node_task_scope")) fail("自证失败：负控 1 没真正拆掉钩子");
  if (!missingEngineWirings(seg(noHook)[0].body).some((m) => m.includes("node_task_scope"))) {
    fail("自证失败：拆掉钩子后 R1 没报出 node_task_scope 缺失");
  }

  // 负控 1b：拆掉回灌给前端的 asOfDate 键 ⇒ 同一条 R1 必须报出（前端接线不能悬空）
  const noKey = good.replace("{ \"asOfDate\": d }", "{}");
  if (noKey.includes("\"asOfDate\":")) fail("自证失败：负控 1b 没真正拆掉 asOfDate 键");
  if (!missingEngineWirings(seg(noKey)[0].body).some((m) => m.includes("asOfDate"))) {
    fail("自证失败：拆掉 asOfDate 键后 R1 没报出（前端会绑到恒 undefined 的键）");
  }

  // 负控 2：调用点只传 `&b` 变量、具名代际写在**邻域之外** ⇒ 必须判成未带代际。
  //（这条锁的是邻域宽度：门认的是就地给出的 BatchKey 字面量，不是「某处曾经存在」）
  const genOutsideNeighbourhood =
    `fn run(x) { let b = BatchKey { as_of_date: d }; ${"let z = 0; ".repeat(400)} ` +
    "persist_reco_pick(db, &p, &b); }";
  if (probe(genOutsideNeighbourhood, PERSIST_CALLS[0]) !== "unguarded") {
    fail("自证失败：代际写在邻域之外仍被当成已带代际（邻域宽度不起作用）");
  }

  // 负控 2b：调用点邻域内根本没有 as_of_date 字样 ⇒ 必须判成未带代际
  const genAbsent = "fn run(x) { let b = make_batch(); persist_reco_picks(db, picks, &b); }";
  if (probe(genAbsent, PERSIST_CALLS[0]) !== "unguarded") {
    fail("自证失败：没传代际的落库调用点没被判出 ⇒ 回放写进实盘表的形态会静默复活");
  }

  // 负控 3：种子守卫必须**包住**调用点 ⇒ 写在调用点之后不算（before 侧的方向性仍在）
  const seedGuardAfter = "fn run(x) { set_serenity_seed(s); if as_of_ctx.is_none() { } }";
  if (probe(seedGuardAfter, PERSIST_CALLS[1]) !== "unguarded") {
    fail("自证失败：种子守卫写在调用点之后仍被当成已守卫（live 侧判据失去方向性）");
  }

  // 负控 3b：拆掉 set_serenity_seed 的 live 守卫 ⇒ 必须判成未守卫
  //（种子这半边判据没被新裁定退役，拆掉就该红）
  const seedUnguarded =
    "fn run(x) { persist_reco_pick(db, &p, &BatchKey { as_of_date: d }); set_serenity_seed(s); }";
  if (probe(seedUnguarded, PERSIST_CALLS[1]) !== "unguarded") {
    fail("自证失败：种子调用点没了 live 守卫却没被判出 ⇒ 污染实盘池的形态会静默复活");
  }

  // 负控 4：完全没有落库调用 ⇒ 检测器不得凭空报（恒真判据的另一种形态）
  if (callSites(seg("fn run(x) { let a = 1; }")[0].body, PERSIST_CALLS[0].calls).length !== 0) {
    fail("自证失败：无调用点的样本被读出落库调用点");
  }

  // 负控 4b（2026-10-29 随「排除定义位」一起补）：定义位不是调用点。
  //   `pub async fn persist_reco_picks(` 自身含该字面量 ⇒ 计入会让读数虚高一格，
  //   还会在定义段里判出「没带代际」的假阳性（它的**形参**就叫 as_of_date，那是巧合）。
  //   ⚠ 这条同时锁住反面：排除定义**不得顺手把真调用点也放过**（判 1 不是判 0）。
  const defOnly = "pub async fn persist_reco_picks(db, as_of_date: Option<&str>) { inner(); }";
  if (callSites(skeleton(defOnly), PERSIST_CALLS[0].calls).length !== 0) {
    fail("自证失败：R2 把落库函数的定义位当成调用点（读数会说谎）");
  }
  const defPlusUnguardedCall =
    "pub async fn persist_reco_picks(db, as_of_date: Option<&str>) { inner(); }\n" +
    "fn run(x) { persist_reco_picks(db, picks); }";
  const mixedSites = callSites(skeleton(defPlusUnguardedCall), PERSIST_CALLS[0].calls);
  if (mixedSites.length !== 1) {
    fail(`自证失败：定义 + 一处真调用点的读数应为 1，实得 ${mixedSites.length} ⇒ 排除逻辑过宽`);
  }

  // 负控 4c（R5）：批次建池快照校验的三种形态必须给出不同读数
  //（只剩定义 = 死代码，读数应是 1 而不是 2；调用点写在注释里不算调用）
  const gateGood =
    "fn require_pool_snapshot_coverage(db) { }\nfn run(x) { require_pool_snapshot_coverage(db); }";
  if (batchSnapshotCalls(skeleton(gateGood)) !== 2) {
    fail(`自证失败：R5 正样本读数应为 2，实得 ${batchSnapshotCalls(skeleton(gateGood))}`);
  }
  if (batchSnapshotCalls(skeleton("fn require_pool_snapshot_coverage(db) { }")) !== 1) {
    fail("自证失败：R5 摘掉调用点后读数不是 1 ⇒ 下限 2 判不出「只剩定义」这种死代码");
  }
  const gateCallInComment =
    "fn require_pool_snapshot_coverage(db) { }\n" +
    "fn run(x) { /* require_pool_snapshot_coverage(db); */ }";
  if (batchSnapshotCalls(skeleton(gateCallInComment)) !== 1) {
    fail("自证失败：R5 把注释里的调用点当成真调用（判据会被注释蒙过）");
  }

  // 负控 5（前端 R3）：真面板的三条绑定必须在位，逐条拆掉后对应标志必须翻假
  const panelSkel = normalize(stripTsComments(fs.readFileSync(path.join(ROOT, PANEL_FILE), "utf8")));
  const full = panelBindFlags(panelSkel);
  for (const k of ["eventBind", "fallbackBind", "renderBind"]) {
    if (!full[k]) fail(`自证失败：真面板缺 ${k} ⇒ 门本该红，先修代码再来自证`);
  }
  const MUT = [
    ["eventBind", "store.setLastRunAsOfDate(", "store.setX("],
    ["fallbackBind", "setLastRunAsOfDate(typeof r?.asOfDate", "setX(typeof r?.asOfDate"],
    ["renderBind", "lastRunAsOfDate !== null", "otherThing !== null"],
  ];
  for (const [k, from, to] of MUT) {
    if (panelSkel.indexOf(from) === -1) fail(`自证失败：负控 5 的拆点 ${k} 在面板里找不到`);
    if (panelBindFlags(normalize(panelSkel.replace(from, to)))[k]) {
      fail(`自证失败：拆掉 ${k} 后仍判为已绑定（R3 判据恒真）`);
    }
  }

  // 负控 6（R4）：定时 live 包裹的计数与下限
  if (countOccurrences("a(); ".repeat(6), "a(); ") !== 6) fail("计数辅助函数读不出 6 次（R4 下限判据不可信）");
  if (countOccurrences("b(); b();", "a(); ") !== 0) fail("计数辅助函数把不存在的 needle 也数进去了");
  // R4 跑在剥注释后的面上：权威文件自己的 /// 里就写着这两个词，按原文匹配会自指误报
  if (stripRustComments("/// 不要用 enter_global_asof(None)\nfn f() {}\n").includes("enter_global_asof(None)")) {
    fail("剥注释没生效：文档里的写法会被 R4 当成代码自指");
  }
  if (!stripRustComments("fn f() { g.enter_global_asof(None); }\n").includes("enter_global_asof(None)")) {
    fail("代码里的写法被剥注释误吞：R4 的反噬判据会恒假");
  }
  // 剥离器：字符字面量 `'"'` 不得开启字符串模式（2026-10-30 实测缺陷）——
  // 一处 `rest.find('"')` 让其后 12 行 `///` 被当成字符串原样保留进代码骨架，
  // 代际门 R2 于是在**文档注释**里读出一个 `pick.insert(db)` 池写形态。
  const dq = String.fromCharCode(34);
  const charLiteralThenDoc = skeleton(
    "fn f() { let q = '" + dq + "'; }\n/// 历史形态：pick.insert(db)\nfn g() {}\n",
  );
  if (charLiteralThenDoc.includes("pick.insert(db)")) {
    fail("剥离失败：字符字面量里的双引号开启了字符串模式，其后文档注释被当成代码");
  }
  if (!charLiteralThenDoc.includes("let q = '" + dq + "'")) {
    fail("剥离失败：字符字面量本体被误删（剥注释不该动代码）");
  }
  const cronReal = checkCronLiveMode();
  if (cronReal.calls < 6) fail(`自证失败：现网定时 live 包裹只有 ${cronReal.calls} 处 ⇒ 门本该红，先补代码`);

  if (violations.length > 0) {
    console.error("❌ 自证失败：");
    for (const v of violations) console.error("  - " + v);
    process.exit(1);
  }
  console.log(
    "✅ 自证通过（剥注释不吞代码；正样本读得出钩子/回灌键/代际写入；负控十三项：拆钩子 / " +
      "拆 asOfDate 键 / 代际写在邻域外 / 完全没传代际 / 种子守卫写在调用点之后 / 拆种子守卫 / " +
      "无调用点不误报 / 定义位不算调用点 / 排除定义不放过真调用点 / 快照校验只剩定义 / " +
      "调用点藏在注释里 / 面板绑定逐条拆 / 定时声明计数，均按预期）",
  );
  process.exit(0);
}

function main() {
  const args = process.argv.slice(2);
  if (args.includes("--selftest")) selftest();
  const dump = args.includes("--dump");

  const rust = checkRustSources(dump);
  const panel = checkPanel(dump);
  const cron = checkCronLiveMode();
  const batch = checkBatchSnapshotGate();
  const violations = [
    ...rust.violations,
    ...panel.violations,
    ...cron.violations,
    ...batch.violations,
  ];

  console.log(
    `扫描：Rust 命令层 ${rust.fileCount} 个文件 / ${rust.fnCount} 个顶层函数段；` +
      `R1 命中 ${rust.r1Hits} 处（as-of 标记 + RunOptions）、R2 命中 ${rust.r2Hits} 个落库调用点；` +
      `R3 面板 1 个；R4 定时 live 声明 ${cron.calls}/${cron.minimum} 处；` +
        `R5 批次建池快照校验 ${batch.calls} 处`,
  );
  if (violations.length > 0) {
    console.error(`❌ as-of 接线违规 ${violations.length} 条：`);
    for (const v of violations) console.error("  - " + v);
    process.exit(1);
  }
  console.log("✅ as-of 接线一致（节点作用域钩子、落库守卫、面板本轮口径绑定都在位）");
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  main();
}
