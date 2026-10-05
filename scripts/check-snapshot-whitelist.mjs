#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-only
// 每日快照白名单的一致性门（#20①，2026-10-05）。
//
// ## 为什么要有这道门
// 2026-10-05 抓到一条**活的断链**：eastmoney 已把 `get_consensus_eps` 申报
// `NoHistoricalSemantic`（P9-3），但 `SNAPSHOT_METHODS` 白名单里没有它 ⇒
// `try_stock_daily_snapshot` 被 `contains` 挡掉、恒 miss ⇒ as-of 一致预期必然降级，
// 而降级文案还写着「等每日归档」——归档从来不会采它。
// 形态复述：**申报了通道 ≠ 通道接上了**。白名单与申报是两处清单，缺一处就恒 miss。
//
// ## 规则
// R1 硬：`PER_STOCK_METHODS ⊆ SNAPSHOT_METHODS` —— 只加一边等于没接（采集侧的个股臂
//    与读取侧的 `contains` 各查一处）。
// R2 硬：每个 vendor 申报 `NoHistoricalSemantic` 的方法必须在 `SNAPSHOT_METHODS` 里；
//    例外逐条进 `EXEMPT`（带日期 + 理由，**每次运行都打印**，失效即红）。
// R3 自证：`--selftest` 用合成文本证明检测器不空转（真从 match 臂读出来，不是恒空集）。
//
// 扫描面：`crates/astock-data/src/vendors/*.rs`（申报面）+ 同 crate 的 `daily_snapshot.rs`（白名单面）。
// ⚠ 别用 `grep -B6` 邻域法抽方法名：相邻 match 臂的名字会串进来（本门第一版就是这么错的，
//    把 `get_market_dragon_tiger` / `get_quote` 混成了 NHS）。正解是按 `=>` 切臂，
//    只有 **右值** 里出现 `AsOfCapability::NoHistoricalSemantic` 的臂才算。
//
// 用法：node scripts/check-snapshot-whitelist.mjs [--selftest] [--dump]

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const VENDOR_DIR = path.join(ROOT, "src-tauri/crates/astock-data/src/vendors");
const LIST_FILE = path.join(ROOT, "src-tauri/crates/astock-data/src/daily_snapshot.rs");

/**
 * 逐条豁免：**带日期 + 理由**，每次运行都打印。
 * 失效即删 —— 留着过期豁免等于把门改软。
 */
const EXEMPT = [
  {
    method: "get_holder_count",
    since: "2026-10-05",
    why:
      "eastmoney 申报 NHS（`RPT_HOLDERNUMLATEST` 无日期参数），但**有意不归档**：" +
      "历史多期在 `RPT_F10_EH_HOLDERNUM`，计划改申报 NativeDateParam 走原生通道；" +
      "当下回放按结构性缺口留痕（见 vendors/eastmoney.rs 该臂注释）。",
  },
  {
    method: "get_peers",
    since: "2026-10-05",
    why:
      "iwencai 申报 NHS，但该申报**走不到**：调用点的 as-of 放行列表只含 " +
      "`NativeDateParam`（lib.rs `get_peers` 的 asof_probe），NHS 分支在路由层就被筛掉；" +
      "同维度 eastmoney 有原生通道 `get_peers_with_asof`。归档它此刻无意义。",
  },
];

/** 从 `pub const NAME: &[&str] = &[ ... ];` 里取引号内的字符串。 */
export function readStringArray(src, constName) {
  const start = src.indexOf(`pub const ${constName}`);
  if (start < 0) {
    return null;
  }
  const open = src.indexOf("[", src.indexOf("=", start));
  const close = src.indexOf("];", open);
  if (open < 0 || close < 0) {
    return null;
  }
  const body = src.slice(open, close);
  return [...body.matchAll(/"([A-Za-z_][A-Za-z0-9_]*)"/g)].map((m) => m[1]);
}

/**
 * 从一个 vendor 源码里抽出「申报 NoHistoricalSemantic 的方法名」。
 *
 * 实现：先取 `fn asof_capability` 的函数体（按花括号配平），再按 `=>` 切臂；
 * 只有当一段的**右值**开头出现该 enum 时，才把该段左值里引号包着的方法名算进来。
 */
export function extractNhsDeclarations(text) {
  const fnAt = text.indexOf("fn asof_capability");
  if (fnAt < 0) {
    return [];
  }
  const bodyOpen = text.indexOf("{", fnAt);
  let depth = 0;
  let i = bodyOpen;
  for (; i < text.length; i += 1) {
    const ch = text[i];
    if (ch === "{") depth += 1;
    if (ch === "}") {
      depth -= 1;
      if (depth === 0) break;
    }
  }
  const body = text.slice(bodyOpen, i + 1);

  const parts = body.split("=>");
  const names = new Set();
  for (let k = 0; k < parts.length - 1; k += 1) {
    const rhs = parts[k + 1].slice(0, 120);
    if (!rhs.includes("AsOfCapability::NoHistoricalSemantic")) {
      continue;
    }
    const lhs = parts[k];
    const cut = Math.max(lhs.lastIndexOf("}"), lhs.lastIndexOf(","));
    for (const m of lhs.slice(cut + 1).matchAll(/"([A-Za-z_][A-Za-z0-9_]*)"/g)) {
      names.add(m[1]);
    }
  }
  return [...names].sort();
}

function collectDeclarations() {
  const out = new Map(); // method -> [vendor]
  for (const f of fs.readdirSync(VENDOR_DIR).sort()) {
    if (!f.endsWith(".rs")) continue;
    const decls = extractNhsDeclarations(fs.readFileSync(path.join(VENDOR_DIR, f), "utf8"));
    for (const m of decls) {
      if (!out.has(m)) out.set(m, []);
      out.get(m).push(f.replace(/\.rs$/, ""));
    }
  }
  return out;
}

function main() {
  const args = process.argv.slice(2);
  const listSrc = fs.readFileSync(LIST_FILE, "utf8");
  const snapshot = readStringArray(listSrc, "SNAPSHOT_METHODS");
  const perStock = readStringArray(listSrc, "PER_STOCK_METHODS");
  if (!snapshot || !perStock) {
    console.error("❌ 白名单解析失败：daily_snapshot.rs 里没找到 SNAPSHOT_METHODS / PER_STOCK_METHODS");
    process.exit(1);
  }

  if (args.includes("--selftest")) {
    const fixture = `
      fn asof_capability(&self, method: &str) -> AsOfCapability {
        match method {
          "get_alpha" | "get_beta" => { AsOfCapability::NoHistoricalSemantic }
          "get_gamma" => AsOfCapability::Fallthrough,
          _ => AsOfCapability::Fallthrough,
        }
      }`;
    const got = extractNhsDeclarations(fixture);
    const ok = got.length === 2 && got.includes("get_alpha") && got.includes("get_beta") &&
      !got.includes("get_gamma");
    if (!ok) {
      console.error("❌ 自证失败：检测器没正确读出 NHS 臂", got);
      process.exit(1);
    }
    // 负控：把同一段改成 Fallthrough ⇒ 必须一个都不报（否则检测器是恒真的）
    const neg = extractNhsDeclarations(fixture.replaceAll("NoHistoricalSemantic", "Fallthrough"));
    if (neg.length !== 0) {
      console.error("❌ 自证失败：全 Fallthrough 的样例仍被报出", neg);
      process.exit(1);
    }
    console.log("✅ 自证通过（识别器真读 match 臂；全 Fallthrough 负控为空）");
    process.exit(0);
  }

  const snapshotSet = new Set(snapshot);
  const exempt = new Map(EXEMPT.map((e) => [e.method, e]));
  const violations = [];

  // R1
  for (const m of perStock) {
    if (!snapshotSet.has(m)) {
      violations.push(`R1: PER_STOCK_METHODS 里的 ${m} 不在 SNAPSHOT_METHODS（只加一边等于没接）`);
    }
  }

  // R2
  const decls = collectDeclarations();
  for (const [m, vendors] of decls) {
    if (snapshotSet.has(m) || exempt.has(m)) {
      continue;
    }
    violations.push(
      `R2: ${vendors.join("/")} 申报 NoHistoricalSemantic 的 ${m} 不在 SNAPSHOT_METHODS，` +
        `也不在豁免表 ⇒ as-of 必然恒 miss`,
    );
  }

  // 豁免表失效检测：已进白名单或已无申报 ⇒ 豁免该删了
  for (const e of EXEMPT) {
    if (snapshotSet.has(e.method)) {
      violations.push(`R2(豁免失效): ${e.method} 已进 SNAPSHOT_METHODS ⇒ 删除 EXEMPT 里 ${e.since} 那条`);
    } else if (!decls.has(e.method)) {
      violations.push(`R2(豁免失效): ${e.method} 已无 vendor 申报 NHS ⇒ 删除 EXEMPT 里 ${e.since} 那条`);
    }
  }

  console.log(
    `扫描：白名单 ${snapshot.length} 项（个股级 ${perStock.length} 项）；` +
      `vendor 申报 NHS 共 ${decls.size} 个方法`,
  );
  for (const e of EXEMPT) {
    console.log(`  豁免(${e.since})：${e.method} —— ${e.why}`);
  }
  if (args.includes("--dump")) {
    for (const [m, v] of decls) console.log(`  ${m} ← ${v.join(", ")}`);
  }
  if (violations.length > 0) {
    console.error(`❌ ${violations.length} 处违规：`);
    for (const v of violations) console.error("  " + v);
    process.exit(1);
  }
  console.log("✅ 白名单与申报一致（豁免见上，逐条带日期与理由）");
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  main();
}
