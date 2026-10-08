import { readFileSync } from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";
import { normalizeDecision, normalizeHorizonPriceMap } from "../agentOutput";
import {
  ACTION_SOURCES,
  actionSourceLabelKey,
  CONFIDENCE_SOURCES,
  confidenceSourceLabelKey,
  HORIZON_CAMEL_TO_SNAKE,
  HORIZON_T_SUFFIX,
  horizonIcAbsenceKey,
  horizonSourceLabelKey,
  horizonSuffix,
  moverLabelPresentation,
  readDecisionProvenance,
  readHorizonActions,
} from "../stock-analysis-utils";

/**
 * 历史列表的档位呈现（2026-09-29）：四档 Action 取自 `decisionJson.decisionsByHorizon`，
 * 缺席必须**渲染为空**而不是回退成「四档同主档」。
 */
describe("readHorizonActions", () => {
  it("取回四档各自的 Action（camelCase，现网产出形态）", () => {
    const json = JSON.stringify({
      decisionsByHorizon: {
        ultraShort: { action: "观望" },
        short: { action: "持有" },
        mid: { action: "买入" },
        long: { action: "买入" },
      },
    });
    expect(readHorizonActions(json)).toEqual([
      { key: "ultra_short", action: "观望" },
      { key: "short", action: "持有" },
      { key: "mid", action: "买入" },
      { key: "long", action: "买入" },
    ]);
  });

  it("兼容 snake_case 旧快照，且只产出实际存在的档", () => {
    const json = JSON.stringify({ decisions_by_horizon: { mid: { action: "持有" }, long: null } });
    expect(readHorizonActions(json)).toEqual([{ key: "mid", action: "持有" }]);
  });

  it("无结构 / 坏 JSON / 空串 ⇒ 空数组（调用方不渲染，不伪造）", () => {
    expect(readHorizonActions(null)).toEqual([]);
    expect(readHorizonActions("{")).toEqual([]);
    expect(readHorizonActions(JSON.stringify({ action: "买入" }))).toEqual([]);
    expect(readHorizonActions(JSON.stringify({ decisionsByHorizon: { mid: {} } }))).toEqual([]);
  });
});

describe("档位标签与来源", () => {
  it("四档各有 i18n 后缀（与 DecisionBanner 同一批键）", () => {
    expect(Object.keys(HORIZON_T_SUFFIX).sort()).toEqual(["long", "mid", "short", "ultra_short"]);
  });

  it("来源标签覆盖值域的每个现役值，未知值不贴标签", () => {
    // 值域权威是 `crates/harness/src/holding_period.rs` 的 HORIZON_SOURCES；
    // 本函数是它的第三处载体（产出=脚本、白名单=decision.rs、标签=这里）。
    // 漏一个值的后果不是「少个标签」，而是那一代记录的「谁定的档」在界面上直接消失。
    expect(horizonSourceLabelKey("branch_pick")).toBe("stockAnalysis.horizonSourceBranchPick");
    expect(horizonSourceLabelKey("formula_no_branch"))
      .toBe("stockAnalysis.horizonSourceNoBranch");
    expect(horizonSourceLabelKey("formula")).toBe("stockAnalysis.horizonSourceFormula");
    expect(horizonSourceLabelKey("model")).toBe("stockAnalysis.horizonSourceModel");
    expect(horizonSourceLabelKey(null)).toBeNull();
    expect(horizonSourceLabelKey(undefined)).toBeNull();
    expect(horizonSourceLabelKey("")).toBeNull();
    // 值域外的脏值（脚本被改坏 / 别的系统写库）必须**不**贴标签，
    // 而不是套一个最接近的 —— 那等于把未知来源伪装成已知来源。
    expect(horizonSourceLabelKey("gut_feeling")).toBeNull();
  });
});

/**
 * 档名两族键的**单点互转**：`decisionsByHorizon` 的键是 camelCase，i18n 后缀表按 snake_case
 * 建模，而荐股链（`RecommendationPanel` / `RecoHistoryModal` / `RecoStrategyMatrix` /
 * `CompactRecommendation` / `MoverRecallPanel` / `SerenityCandidateCard`）送进来的是后端
 * `Period::as_str()` 的 snake_case ⇒ 两侧都靠 `horizonSuffix` 归一。
 * 它一旦退化（比如 camel 查不到就原样显示），标签会显示成「ultraShort」而不是「超短线」，
 * 且 11 个语言里都不会报错 —— 所以锁的是「两族输入得到同一后缀」，不是「某个后缀对不对」。
 *
 * （本块首版写的理由是 Phase F 的 `sharesPosteriorWith` 注脚；该字段已随 R-11 退役，
 *   判据本身仍然有六个现役消费方，故保留并换成真主语。）
 */
describe("horizonSuffix（两族键名的单点互转）", () => {
  it("camelCase 与 snake_case 都映射到同一后缀", () => {
    expect(horizonSuffix("ultraShort")).toBe("UltraShort");
    expect(horizonSuffix("ultra_short")).toBe("UltraShort");
    for (const [camel, snake] of Object.entries(HORIZON_CAMEL_TO_SNAKE)) {
      expect(horizonSuffix(camel)).toBe(HORIZON_T_SUFFIX[snake]);
    }
  });

  it("认不出的档名返回 null，不猜档位", () => {
    expect(horizonSuffix("medium")).toBeNull();
    expect(horizonSuffix("")).toBeNull();
  });
});

describe("normalizeDecision 透传逐档口径标注", () => {
  it("scoreSource / stopSource / positionSource / 自证三件套原样到达展示层", () => {
    const raw = {
      action: "BUY",
      confidence: 62,
      decisionsByHorizon: {
        ultraShort: {
          action: "观望",
          posterior: 55.0,
          scoreSource: "daily_fallback",
          stopSource: "fallback_pct",
          positionSource: "kelly_x_position_multiplier",
          horizon: "ultra_short",
          confidenceMethod: "posterior_floor",
          exitRule: "time_stop",
          absentLegs: ["microstructure"],
        },
        short: {
          action: "观望",
          posterior: 55.0,
          scoreSource: "tier_native",
          confidenceMethod: "trend_gate_and_seal",
        },
      },
    };
    const parsed = normalizeDecision(raw);
    expect(parsed).not.toBeNull();
    const ultra = parsed!.decisionsByHorizon?.ultraShort;
    // 这些键是**结构性缺席声明 + 档位自证**本身：normalizeDecision 走白名单构造返回体，
    // 漏加一个键就等于把「该档按日线退化」「该档用的是哪种置信口径」在 IPC 之后静默丢弃。
    // ⚠ 原并列的 `sharesPosteriorWith` 随 R-11 退役（同源注脚是给伪装贴标签，不是去伪装）。
    expect(ultra?.scoreSource).toBe("daily_fallback");
    expect(ultra?.stopSource).toBe("fallback_pct");
    expect(ultra?.positionSource).toBe("kelly_x_position_multiplier");
    expect(ultra?.horizon).toBe("ultra_short");
    expect(ultra?.confidenceMethod).toBe("posterior_floor");
    expect(ultra?.exitRule).toBe("time_stop");
    expect(ultra?.absentLegs).toEqual(["microstructure"]);
    expect(parsed!.decisionsByHorizon?.short?.scoreSource).toBe("tier_native");
    expect(parsed!.decisionsByHorizon?.short?.confidenceMethod).toBe("trend_gate_and_seal");
  });
});

/**
 * rank IC 的缺席必须**分句**：四种缺席各自对应不同的下一步动作
 * （补字段 / 攒样本 / 该档取值本无差异 / 等口径换代），并成一句「暂无数据」
 * 就是把结构性缺口压成歧义。这里锁映射表本身，而不是锁某句译文。
 */
describe("horizonIcAbsenceKey（IC 缺席四分句）", () => {
  it("四种缺席各占一键，且互不相同", () => {
    const keys = [
      "no_confidence",
      "insufficient_ic_samples",
      "degenerate_variance",
      "pre_snr_regime",
    ].map((st) => horizonIcAbsenceKey(st));
    expect(keys.every((k) => k !== null)).toBe(true);
    expect(new Set(keys).size).toBe(4);
    expect(horizonIcAbsenceKey("pre_snr_regime")).toBe(
      "stockAnalysis.reflection.hitrateIcPreRegime",
    );
  });

  it("ok 不是缺席、未知状态不猜翻译 ⇒ 都返回 null", () => {
    expect(horizonIcAbsenceKey("ok")).toBeNull();
    expect(horizonIcAbsenceKey("nope_not_a_status")).toBeNull();
    expect(horizonIcAbsenceKey(null)).toBeNull();
    expect(horizonIcAbsenceKey(undefined)).toBeNull();
    // 空串（旧记录 Default 出来的形态）同样不得被猜成某一句
    expect(horizonIcAbsenceKey("")).toBeNull();
  });
});

/**
 * 价位映射的键归一（v125 批准 ① 顺带修掉的**既有**缺陷）。
 *
 * 旧实现只做类型断言不转键：`DecisionBanner` 的 `HORIZON_KEYS` 按 camel 读，而后端
 * `horizonPriceMap` 出的是 snake ⇒ 四键里只有 `ultra_short` 一族拼写错开（short/mid/long
 * 两族同名），表现为「超短档价位行从来不显示、另外三档正常」，而库里四键齐备
 * （实证：000710 现网行的映射键 = mid / long / short / ultra_short）。
 */
describe("normalizeHorizonPriceMap", () => {
  const legacy = {
    ultra_short: { stopLossPct: 3.2, takeProfitPct: 3.2, expectedHoldingDays: 2, targetPrice: 10.5, stopLoss: 10.15 },
    short: { stopLossPct: 6.0, takeProfitPct: 12.0, expectedHoldingDays: 5, targetPrice: 11.2, stopLoss: 10.2 },
    mid: { stopLossPct: 8.0, takeProfitPct: 18.0, expectedHoldingDays: 28, targetPrice: 12.1, stopLoss: 10.0 },
    long: { stopLossPct: 12.0, takeProfitPct: 30.0, expectedHoldingDays: 90, targetPrice: 13.5, stopLoss: 9.8 },
  };

  it("存量行的 snake 键归一成 camel（超短档那一行不再静默丢失）", () => {
    const out = normalizeHorizonPriceMap(legacy);
    expect(out?.ultraShort?.stopLoss).toBe(10.15);
    expect(Object.keys(out ?? {})).toEqual(["ultraShort", "short", "mid", "long"]);
  });

  it("v125 的 camel 键原样收，两种拼写同时在场时 camel 优先", () => {
    expect(normalizeHorizonPriceMap({ mid: legacy.mid })?.mid?.stopLossPct).toBe(8.0);
    const mixed = { mid: { ...legacy.mid, stopLossPct: 9.9 }, ultra_short: legacy.ultra_short };
    const out = normalizeHorizonPriceMap(mixed);
    expect(out?.mid?.stopLossPct).toBe(9.9);
    expect(out?.ultraShort?.stopLossPct).toBe(3.2);
  });

  it("字段缺席 = null（stage1 之前的记录），映射为空 = {}（四路全缺席，口径 A）", () => {
    expect(normalizeHorizonPriceMap(undefined)).toBeNull();
    expect(normalizeHorizonPriceMap(null)).toBeNull();
    // 空对象**不是** null：它说的是「本轮四路都没产出」，而 null 说的是「没有这个信息」。
    // 把两者并成一个值，读侧就分不出这两种完全不同的缺席。
    expect(normalizeHorizonPriceMap({})).toEqual({});
    expect(normalizeHorizonPriceMap("不是对象")).toBeNull();
  });

  it("值域封闭：表外的键不得混进决策对象；单档缺席则该键不在结果里", () => {
    const out = normalizeHorizonPriceMap({ ...legacy, yesterday: { stopLossPct: 1 }, ultraShort: null });
    expect(Object.keys(out ?? {})).toEqual(["short", "mid", "long"]);
    expect(out && "yesterday" in out).toBe(false);
    expect(out && "ultraShort" in out).toBe(false);
  });
});

/**
 * 主档来历的值域三载体（§五十三 ①，v127）。
 *
 * 为什么必须有这条门：`actionSource` / `confidenceSource` 的**权威在产端**
 * （`portfolio-mgr.rhai` 的 `action_source` / `confidence_source`），第二载体是这里的
 * `ACTION_SOURCES` / `CONFIDENCE_SOURCES` + 标签函数，第三载体是 11 语言文案。
 * 三处任漏一处，`cargo check` 与 `tsc` 全绿也照样存在 —— 表现是「那一类记录的来历在界面上
 * 直接消失」（标签函数返回 null ⇒ 调用方不渲染）。000710 的抱怨正是这一族：
 * 主档=持有、标签=超短线分支选档、超短线 chip=买入，三者同屏而无人说明谁改写谁。
 */
const RHAI_PATH = path.resolve(__dirname, "../../../src-tauri/src/commands/portfolio-mgr.rhai");
const LOCALE_DIR = path.resolve(__dirname, "../../../src/i18n/locales");
const LANGS = ["ar", "de", "en-US", "es", "fr", "hi", "ja", "ko", "ru", "zh-CN", "zh-TW"];
const PROVENANCE_KEYS = [
  ...ACTION_SOURCES.map(actionSourceLabelKey),
  ...CONFIDENCE_SOURCES.map(confidenceSourceLabelKey),
  "stockAnalysis.decisionProvenanceDirect",
  "stockAnalysis.decisionProvenanceDowngraded",
].map((k) => (k ?? "").replace("stockAnalysis.", ""));

/** 取 `let <name> = …` 这条语句的完整文本（语句以行尾 `;` 收；取不到即红，不静默放宽）。 */
function rhaiStatement(src: string, name: string): string {
  const lines = src.split("\n");
  const start = lines.findIndex((l) => l.trimStart().startsWith(`let ${name} =`));
  expect(start, `portfolio-mgr.rhai 里找不到 let ${name} =`).toBeGreaterThan(-1);
  let end = start;
  while (end < lines.length && !lines[end].trimEnd().endsWith(";")) { end += 1; }
  expect(end, `let ${name} 语句没有以 ; 收尾（形态已变，须同步本门）`).toBeLessThan(lines.length);
  return lines.slice(start, end + 1).join("\n");
}

describe("主档来历：值域三载体必须同集合", () => {
  const src = readFileSync(RHAI_PATH, "utf8");

  it("Rhai 产端发出的每个值都在 TS 值域内，且 TS 值域没有产端不发的死值", () => {
    const emitted = (name: string) => new Set([...rhaiStatement(src, name).matchAll(/"([a-z_]+)"/g)].map((m) => m[1]));
    for (
      const [name, domain] of [["action_source", ACTION_SOURCES], ["confidence_source", CONFIDENCE_SOURCES]] as const
    ) {
      const got = emitted(name);
      expect([...got].length, `${name} 一个值都没抽到 ⇒ 抽取失效`);
      const notInDomain = [...got].filter((v) => !domain.includes(v));
      const neverEmitted = domain.filter((v) => !got.has(v));
      expect(notInDomain, `${name} 发出了值域外的标签：${notInDomain.join(", ")}`).toEqual([]);
      expect(neverEmitted, `值域里的死值（产端不发）：${neverEmitted.join(", ")}`).toEqual([]);
    }

    // ── 负控：往产端塞一个值域外的标签，本门必须报出来 ──
    // （不这么跑一次，「两边都全」可能只是抽取函数什么都没抽到 + 断言恒真）
    const mutated = src.replace(
      '\telse { "main_chain" };',
      '\telse if x > 0 { "invented_label" } else { "main_chain" };',
    );
    expect(mutated, "负控变异点未命中 —— action_source 语句形态已变，须同步本测试").not.toBe(src);
    const got2 = [...rhaiStatement(mutated, "action_source").matchAll(/"([a-z_]+)"/g)].map((m) => m[1]);
    expect(got2.filter((v) => !ACTION_SOURCES.includes(v))).toEqual(["invented_label"]);
  });

  it("值域每个值都有标签函数，且标签 key 真的进了 11 语言文案", () => {
    const known = new Set(PROVENANCE_KEYS);
    for (const v of ACTION_SOURCES) {
      const key = actionSourceLabelKey(v);
      expect(key, `actionSource=${v} 无标签 key`).not.toBeNull();
      expect(
        known.has((key ?? "").replace("stockAnalysis.", "")),
        `${key} 未随 11 语言落地（下一行的语言覆盖测试会一起红）`,
      ).toBe(true);
    }
    for (const v of CONFIDENCE_SOURCES) {
      const key = confidenceSourceLabelKey(v);
      expect(key, `confidenceSource=${v} 无标签 key`).not.toBeNull();
      expect(known.has((key ?? "").replace("stockAnalysis.", "")), `${key} 未落地`).toBe(true);
    }
    // 未命中必须返回 null —— 不得给旧记录（无该字段）编一个「分支选档」
    expect(actionSourceLabelKey(undefined)).toBeNull();
    expect(actionSourceLabelKey("table")).toBeNull();
    expect(confidenceSourceLabelKey(null)).toBeNull();
  });

  it("11 语言全部真译：非空、不等于键名、非中文语言不得抄 zh-CN", () => {
    const zh = JSON.parse(readFileSync(path.join(LOCALE_DIR, "zh-CN.json"), "utf8")).stockAnalysis;
    for (const lang of LANGS) {
      const sa = JSON.parse(readFileSync(path.join(LOCALE_DIR, `${lang}.json`), "utf8")).stockAnalysis;
      for (const k of PROVENANCE_KEYS) {
        expect(sa[k], `${lang} 缺译 ${k}`).toBeTruthy();
        expect(sa[k], `${lang} 的 ${k} 是占位符`).not.toBe(k);
        if (lang !== "zh-CN" && lang !== "zh-TW") {
          expect(sa[k], `${lang} 的 ${k} 抄了中文`).not.toBe(zh[k]);
        }
      }
      // 两条成句模板的插值集合必须逐语言一致（占位符门也查，这里锁本批新增的这两条）
      for (const tpl of ["decisionProvenanceDirect", "decisionProvenanceDowngraded"]) {
        const ph = (sa[tpl] as string).match(/\{\{[a-zA-Z]+\}\}/g) ?? [];
        const zhPh = (zh[tpl] as string).match(/\{\{[a-zA-Z]+\}\}/g) ?? [];
        expect(ph.sort(), `${lang} 的 ${tpl} 插值集合与 zh-CN 不一致`).toEqual(zhPh.sort());
      }
    }
  });
});

/**
 * `readDecisionProvenance`（§五十三 ①）：三种形态 + 「无字段 = 无此信息」。
 *
 * 000710 实测那一行是 `downgraded` 形态的样本（超短线分支=买入 → 高风险风控否决 → 主档=持有），
 * 夹具照它写；`branch_pick` 与 `main_chain` 各给一格，另加两格负面对照
 * （v127 之前的记录、以及降级格但读不到分支原 action ⇒ 不得编造那个「买入」）。
 */
describe("readDecisionProvenance", () => {
  const wrap = (o: Record<string, unknown>) => JSON.stringify(o);
  const tiers = { ultraShort: { action: "买入" }, short: { action: "持有" } };

  it("降级格：成句需要分支原结论与改写者", () => {
    const p = readDecisionProvenance(wrap({
      timeHorizon: "ultra_short",
      actionSource: "risk_veto_downgrade",
      confidenceSource: "branch_row",
      decisionsByHorizon: tiers,
    }));
    expect(p).toMatchObject({ kind: "downgraded", horizon: "ultra_short", branchAction: "买入" });
  });

  it("直取格与主链格：前者成句、后者只报来历", () => {
    expect(readDecisionProvenance(wrap({
      timeHorizon: "ultraShort",
      actionSource: "branch_pick",
      confidenceSource: "branch_row",
      decisionsByHorizon: tiers,
    })))?.toMatchObject({ kind: "direct", horizon: "ultra_short" });
    expect(readDecisionProvenance(wrap({ actionSource: "main_chain" })))
      ?.toMatchObject({ kind: "label" });
  });

  it("无此字段 / 坏 JSON / 空串 ⇒ null（旧记录不得被编出来历）", () => {
    expect(readDecisionProvenance(null)).toBeNull();
    expect(readDecisionProvenance("")).toBeNull();
    expect(readDecisionProvenance(wrap({ timeHorizon: "ultra_short" }))).toBeNull();
    expect(readDecisionProvenance("{坏 JSON")).toBeNull();
  });

  it("降级格但四档明细里找不到该档 ⇒ 退成 label，不编造分支原 action", () => {
    const p = readDecisionProvenance(wrap({
      timeHorizon: "mid",
      actionSource: "sim_veto_downgrade",
      decisionsByHorizon: tiers,
    }));
    expect(p?.kind).toBe("label");
    expect(p?.branchAction).toBeUndefined();
  });
});
/**
 * 妖股标签的呈现分层（#10 P7，2026-10-06）。
 *
 * 权威在产端 `mover_recall::mover_label_for`（四态 + NULL），第二载体是本文件的
 * `moverLabelPresentation`，第三载体是 11 语言文案。三处任漏一处，`tsc` 与 `cargo check`
 * 全绿也照样存在 —— 表现是「那一格的文案没了」或「未满被显示成未达标」。
 * 用户裁定的边界：**只显示标签列，不加过滤**，所以这里不测过滤，只测分层与逐句。
 */
const MOVER_RS_PATH = path.resolve(
  __dirname,
  "../../../src-tauri/crates/analysis-engine/src/mover_recall.rs",
);

describe("moverLabelPresentation（妖股标签四分句）", () => {
  it("达标 / 未达标 / 三种无从判定各自成句", () => {
    expect(moverLabelPresentation("mover")).toEqual({
      kind: "yes",
      i18nKey: "stockAnalysis.reflection.moverYes",
    });
    expect(moverLabelPresentation("normal")).toEqual({
      kind: "no",
      i18nKey: "stockAnalysis.reflection.moverNo",
    });
    const absence: [string, string][] = [
      ["window_incomplete", "stockAnalysis.reflection.moverWindowIncomplete"],
      ["no_market_data", "stockAnalysis.reflection.moverNoMarketData"],
      ["rule_unavailable", "stockAnalysis.reflection.moverRuleUnavailable"],
    ];
    for (const [label, key] of absence) {
      expect(moverLabelPresentation(label)).toEqual({ kind: "absence", i18nKey: key });
    }
  });

  it("负控：三种「无从判定」都不得映射成「未达标」", () => {
    for (const label of ["window_incomplete", "no_market_data", "rule_unavailable"]) {
      expect(moverLabelPresentation(label).kind).not.toBe("no");
      expect(moverLabelPresentation(label).i18nKey).not.toBe("stockAnalysis.reflection.moverNo");
    }
  });

  it("NULL / 空串 = 未复盘，与「算过且未达标」不同句", () => {
    for (const v of [null, undefined, ""]) {
      expect(moverLabelPresentation(v).kind).toBe("notReflected");
      expect(moverLabelPresentation(v).i18nKey).toBe("stockAnalysis.reflection.moverNotReflected");
    }
  });

  it("未登记的标签值 ⇒ unknown 且不回退成任何一句现成文案", () => {
    expect(moverLabelPresentation("mover_v2")).toEqual({ kind: "unknown", i18nKey: null });
  });

  it("反手抄：后端 mover_label_for 发出的每个字面量都必须在 TS 里有分层", () => {
    const rs = readFileSync(MOVER_RS_PATH, "utf8");
    const start = rs.indexOf("pub fn mover_label_for");
    expect(start, "mover_recall.rs 里找不到 mover_label_for ⇒ 值域权威搬家了，须同步本门").toBeGreaterThan(-1);
    const body = rs.slice(start);
    const close = body.indexOf(String.fromCharCode(10) + "}");
    expect(close, "mover_label_for 的收尾形态变了").toBeGreaterThan(-1);
    const emitted = [
      ...new Set(
        [...body.slice(0, close).matchAll(/Some\("([a-z_]+)"\)/g)].map((m) => m[1]),
      ),
    ];
    expect(emitted.length, "产端一个字面量也没解析到 ⇒ 判据失效").toBeGreaterThan(0);
    for (const label of emitted) {
      expect(moverLabelPresentation(label).kind, `TS 侧没登记 ${label}`).not.toBe("unknown");
    }
  });

  it("11 语言全部真译（非空、不等于键名、非中文不抄 zh-CN）", () => {
    const keys = [
      "colMover",
      "moverYes",
      "moverNo",
      "moverWindowIncomplete",
      "moverNoMarketData",
      "moverRuleUnavailable",
      "moverNotReflected",
    ];
    const zh = JSON.parse(readFileSync(path.join(LOCALE_DIR, "zh-CN.json"), "utf8")).stockAnalysis.reflection;
    for (const lang of LANGS) {
      const refl = JSON.parse(readFileSync(path.join(LOCALE_DIR, lang + ".json"), "utf8")).stockAnalysis.reflection;
      for (const k of keys) {
        expect(refl[k], lang + " 缺 key " + k).toBeTruthy();
        expect(refl[k], lang + " 的 " + k + " 是占位符").not.toBe(k);
        if (lang !== "zh-CN" && lang !== "zh-TW") {
          expect(refl[k], lang + " 的 " + k + " 抄了中文").not.toBe(zh[k]);
        }
      }
    }
  });
});
