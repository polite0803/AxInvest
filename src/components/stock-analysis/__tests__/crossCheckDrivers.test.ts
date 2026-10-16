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

/** 决策链 `evidence.factors` 里的腿名（渲染端 `crossCheck.leg.<名>` 的取值域） */
function legNamesFromRhai(): string[] {
  const src = fs.readFileSync(PORTFOLIO_RHAI, "utf8");
  return [...src.matchAll(/#\{\s*"name":\s*"([a-z_]+)"/g)].map((m) => m[1]);
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
    expect(legNamesFromRhai().length).toBe(12);
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
