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
