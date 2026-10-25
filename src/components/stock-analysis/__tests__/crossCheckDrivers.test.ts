import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

/**
 * 跨系统分歧「判据码 / 证据腿名」与 11 语言文案的契约（2026-10-02）。
 *
 * 同型缺陷的既有判据见 `src/lib/__tests__/stockAnalysisAction.test.ts`：后端产出的是
 * **机器可读码**，前端按 `crossCheck.driver.<码>` / `crossCheck.leg.<名>` 取文案；
 * 码加了、翻译没补 ⇒ react-i18next 未命中会**退回渲染原始码串**，值域单测查不出来。
 * 所以这里把码表从**生产者**（Rust 派生函数、rhai 因子表）现场抽出来对，而不是手抄一份
 * —— 手抄的那份正是最容易和后端不同步的那一份。
 */
const LANGS = ["zh-CN", "zh-TW", "en-US", "ja", "ko", "de", "fr", "es", "ru", "ar", "hi"];
const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..", "..", "..", "..");
const LOCALES_DIR = path.join(ROOT, "src", "i18n", "locales");
const HOOKS_RS = path.join(ROOT, "src-tauri", "src", "commands", "stock_workflow", "hooks.rs");
const PORTFOLIO_RHAI = path.join(ROOT, "src-tauri", "src", "commands", "portfolio-mgr.rhai");

/** 后端 `divergence_attribution` 实际会 push 的判据码 */
function driverCodesFromRust(): string[] {
  const src = fs.readFileSync(HOOKS_RS, "utf8");
  return [...src.matchAll(/drivers\.push\("([a-z_]+)"\)/g)].map((m) => m[1]);
}

/**
 * 决策链 `evidence.factors` 里的腿名（渲染端 `crossCheck.leg.<名>` 的取值域）。
 *
 * 腿名自 v180 起有**两种合法写法**，抽取器必须都认（只认①的那一份在本批当场误报）：
 *   ① 字面量：`#{ "name": "trend", … }`
 *   ② 具名变量：`#{ "name": f7_name, … }`，其值域写在
 *      `let f7_name = if f7_source == "risk_reward" { "risk_reward" } else { "trade_signal" };`
 *      —— f7 这条腿按 f7 输入的来历二选一，两个名字都是产端真会吐出的值。
 * 按①的字面量正则扫会把②那一格**静默漏掉**：漏的后果不是报错，而是「真缺的译名查不出来、
 * 已有译名反被说成死码」。故②按赋值右侧的字符串字面量全数收下，并**单独成一条**
 * 「未解出」清单 —— 解不出就红，不放行成「这一格不存在」。
 */
function legNamesFrom(src: string): { names: string[]; sites: number; unresolved: string[] } {
  const names: string[] = [];
  const unresolved: string[] = [];
  let sites = 0;
  for (const m of src.matchAll(/#\{\s*"name":\s*(?:"([a-z_]+)"|([A-Za-z_][A-Za-z0-9_]*))/g)) {
    sites += 1;
    if (m[1]) {
      names.push(m[1]);
      continue;
    }
    const domain = assignLiteralsOf(src, m[2]);
    if (domain.length === 0) {
      unresolved.push(m[2]);
    } else {
      for (const n of domain) {
        if (!names.includes(n)) {
          names.push(n);
        }
      }
    }
  }
  return { names, sites, unresolved };
}

/** `let <ident> = <expr>;` 右侧的**分支体**字面量 —— 即该腿名的取值域（分支给几个收几个） */
function assignLiteralsOf(src: string, ident: string): string[] {
  const start = src.indexOf(`let ${ident} =`);
  if (start < 0) { return []; }
  const end = src.indexOf("\n", start);
  const rhs = src.slice(start, end < 0 ? undefined : end);
  // 只收 `{ "x" }` 形态（分支结果），**不收**比较式里的字面量：
  // `let f7_name = if f7_source == "analyst_votes" { "analyst_vote" } else { "trade_signal" };`
  // 里 `analyst_votes` 是 f7_source 的取值、不是腿名 —— 全行扫描会把它错当成第二条腿，
  // 后果是「要求译出一个永不产出的键」+「真正的腿名被淹没在多余项里」。
  return [...rhs.matchAll(/\{\s*"([a-z_]+)"\s*\}/g)].map((m) => m[1]);
}

function legNamesFromRhai(): string[] {
  return legNamesFrom(fs.readFileSync(PORTFOLIO_RHAI, "utf8")).names;
}

function dig(dict: unknown, key: string): unknown {
  return key.split(".").reduce<unknown>(
    (cur, seg) => (cur && typeof cur === "object" ? (cur as Record<string, unknown>)[seg] : undefined),
    dict,
  );
}

describe("分歧归因的判据码必须有 11 语言文案", () => {
  it("码表从生产者抽出来非空（防退化成空集合 ⇒ 断言恒真）", () => {
    expect(driverCodesFromRust().length).toBeGreaterThan(0);
    // 12 个 name 站点（腿数没变），其中 f7 那格按来历贡献两个腿名 ⇒ 值域 13 个
    expect(legNamesFromRhai().length).toBe(13);
  });

  /**
   * 抽取器自证：站点数与「未解出清单」各断一条。
   * 光断条数会同时放过两种相反的错误 —— 站点被删（腿少了一条）与站点解析不出（腿名躲在变量后）。
   */
  it("抽取器对每个 name 站点都解出了值域（解不出＝这一格被静默跳过，门会失去对照意义）", () => {
    const r = legNamesFrom(fs.readFileSync(PORTFOLIO_RHAI, "utf8"));
    expect(r.sites).toBe(12);
    expect(r.unresolved).toEqual([]);
  });

  it("f7 腿名是按 f7_source 二选一的变量，两个分支值都必须在取值域里", () => {
    const names = new Set(legNamesFromRhai());
    expect(names.has("trade_signal"), "含交易观点口径（f7 读 trader）的腿名被漏收").toBe(true);
    expect(names.has("analyst_vote"), "v183 生产口径（分析师结构票）的腿名被漏收").toBe(true);
  });

  /**
   * 负控：分支**条件**里的字面量不是腿名。
   * `f7_source == "analyst_votes"` 里那个复数形态永远不会作为腿名产出；若抽取器按整行扫
   * 字面量就会把它收进取值域 ⇒ 门会要求译出一个永不产出的键（本测试头一次红正是这个形态）。
   */
  it("负控：比较式里的字面量不得被当成腿名", () => {
    const fake = 'let f7_name = if f7_source == "analyst_votes" { "analyst_vote" } else { "trade_signal" };';
    expect(assignLiteralsOf(fake, "f7_name")).toEqual(["analyst_vote", "trade_signal"]);
  });

  it("负控：变量形态的腿名若查不到赋值 ⇒ 进未解出清单，不算「这格不存在」", () => {
    const fake = '{"a": #{ "name": "trend" }, "b": #{ "name": ghost_name } }';
    const r = legNamesFrom(fake);
    expect(r.sites).toBe(2);
    expect(r.names).toEqual(["trend"]);
    expect(r.unresolved).toEqual(["ghost_name"]);
  });

  it("每个判据码在 11 语言里都有非空文案", () => {
    for (const lang of LANGS) {
      const dict = JSON.parse(fs.readFileSync(path.join(LOCALES_DIR, `${lang}.json`), "utf8"));
      for (const code of driverCodesFromRust()) {
        const v = dig(dict, `stockAnalysis.crossCheck.driver.${code}`);
        expect(typeof v === "string" && v.trim() !== "", `${lang} 缺 driver.${code}`).toBe(true);
      }
    }
  });

  it("每个证据腿名在 11 语言里都有非空文案", () => {
    for (const lang of LANGS) {
      const dict = JSON.parse(fs.readFileSync(path.join(LOCALES_DIR, `${lang}.json`), "utf8"));
      for (const name of legNamesFromRhai()) {
        const v = dig(dict, `stockAnalysis.crossCheck.leg.${name}`);
        expect(typeof v === "string" && v.trim() !== "", `${lang} 缺 leg.${name}`).toBe(true);
      }
    }
  });

  it("反向：locale 里不得躺着后端不会再产出的死码（死键会让门失去对照意义）", () => {
    const codes = new Set(driverCodesFromRust());
    const names = new Set(legNamesFromRhai());
    const dict = JSON.parse(fs.readFileSync(path.join(LOCALES_DIR, "zh-CN.json"), "utf8"));
    const driverKeys = Object.keys(dig(dict, "stockAnalysis.crossCheck.driver") as object);
    const legKeys = Object.keys(dig(dict, "stockAnalysis.crossCheck.leg") as object);
    expect(driverKeys.filter((k) => !codes.has(k))).toEqual([]);
    expect(legKeys.filter((k) => !names.has(k))).toEqual([]);
  });
});

const DECISION_RS = path.join(ROOT, "src-tauri", "src", "commands", "stock_workflow", "decision.rs");
const HORIZON_LIST_TSX = path.join(
  ROOT,
  "src",
  "components",
  "stock-analysis",
  "HorizonAgreementList.tsx",
);

/** B2 逐档归因：`attribute_horizon_divergence` 实际 push 的判据码（同一个派生法，换生产文件） */
function horizonDriverCodesFromRust(): string[] {
  const src = fs.readFileSync(DECISION_RS, "utf8");
  return [...src.matchAll(/row\.drivers\.push\("([a-z_]+)"\)/g)].map((m) => m[1]);
}

/** `driverParams` 传进 t() 的参数袋键名 —— 译文里出现它没给的槽，界面就会显示裸 `{{槽}}` */
function paramBagKeysFromComponent(): string[] {
  const src = fs.readFileSync(HORIZON_LIST_TSX, "utf8");
  const start = src.indexOf("const driverParams");
  if (start < 0) {
    throw new Error("抽取器前提：driverParams 还在组件里");
  }
  // 切片必须从 driverParams **之后**找 `return (`：组件顶部那条 not-produced 早退也是 `return (`，
  // 用全局 indexOf 会得到 end < start ⇒ 空切片 ⇒ 除手工补的三个键外全部漏收（本测试头一次红就是这样）
  const end = src.indexOf("return (", start);
  const body = src.slice(start, end);
  const keys = [...body.matchAll(/^\s+([a-zA-Z][a-zA-Z0-9]*):/gm)].map((m) => m[1]);
  // legContribution 的三个槽在嵌套调用里，缩进与外层不同，一并由上面的通用规则收（收不到则补）
  for (const k of ["leg", "bias", "weight"]) {
    if (!keys.includes(k)) {
      keys.push(k);
    }
  }
  return keys;
}

describe("逐档分歧归因（horizonDivergence）的码表与文案契约", () => {
  it("码表从 decision.rs 现场抽出来非空（防退化成空集合 ⇒ 断言恒真）", () => {
    // 下限写 7：这是 B2 立码表时的条数，条数掉了说明派生函数被改而前端没跟上
    expect(horizonDriverCodesFromRust().length).toBeGreaterThanOrEqual(7);
  });

  it("每个逐档判据码在 11 语言里都有非空文案", () => {
    for (const lang of LANGS) {
      const dict = JSON.parse(fs.readFileSync(path.join(LOCALES_DIR, `${lang}.json`), "utf8"));
      for (const code of horizonDriverCodesFromRust()) {
        const v = dig(dict, `stockAnalysis.horizonDivergence.driver.${code}`);
        expect(typeof v === "string" && v.trim() !== "", `${lang} 缺 horizonDivergence.driver.${code}`).toBe(true);
      }
      // 空驱动项那句是呈现层的「不是两侧一致」声明，缺了会退化成静默
      const none = dig(dict, "stockAnalysis.horizonDivergence.none");
      expect(typeof none === "string" && none.trim() !== "", `${lang} 缺 horizonDivergence.none`).toBe(true);
    }
  });

  it("反向：horizonDivergence.driver 里不得躺着产端不会再产出的死码", () => {
    const codes = new Set(horizonDriverCodesFromRust());
    const dict = JSON.parse(fs.readFileSync(path.join(LOCALES_DIR, "zh-CN.json"), "utf8"));
    const keys = Object.keys(dig(dict, "stockAnalysis.horizonDivergence.driver") as object);
    expect(keys.filter((k) => !codes.has(k))).toEqual([]);
  });

  it("译文用到的插值槽，组件参数袋必须真的传（漏一个槽＝界面显示裸 {{槽}}）", () => {
    const bag = new Set(paramBagKeysFromComponent());
    for (const lang of LANGS) {
      const dict = JSON.parse(fs.readFileSync(path.join(LOCALES_DIR, `${lang}.json`), "utf8"));
      const group = dig(dict, "stockAnalysis.horizonDivergence") as Record<string, unknown>;
      const values = [
        group.none as string,
        group.legContribution as string,
        ...Object.values(group.driver as Record<string, string>),
      ];
      for (const v of values) {
        for (const m of String(v).matchAll(/\{\{([a-zA-Z_][a-zA-Z0-9_]*)\}\}/g)) {
          expect(bag.has(m[1]), `${lang} 用了组件没传的槽 {{${m[1]}}}`).toBe(true);
        }
      }
    }
  });
});
