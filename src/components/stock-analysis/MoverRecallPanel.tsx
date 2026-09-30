import { invoke } from "@/lib/invoke";
import { horizonSuffix } from "@/lib/stock-analysis-utils";
import type { MoverRecallView } from "@/types/stock-analysis";
import { Button, Card, Spin } from "antd";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

/**
 * 窗口涨幅达标漏检核查面板（`PLAN-mover-recall-attribution.md` Phase 5）。
 *
 * 四件事在 UI 上必须各自成句、不得互相冒充（「结构性缺口不得在 UI 造成歧义」）：
 * ① 不可运行（`market_daily_close` 未采集/取数失败）—— `failed` 支；
 * ② 某档判据不成立（阈值变量非正数）—— `rulesMissing` 支；
 * ③ 区间内无达标事件 —— `empty` 支；**仅当全部档位窗口已满**时才算结论，
 *    未满时以 `emptyNotYet` 说明「尚不可判定」并逐档标注（`windowOpen`）。
 *    四种「空」的原因不同，合并成一句就是歧义。
 * ④ 返回值形状不符（IPC 契约漂移 / mock 兜底返回 `{}`）—— 同走 `failed` 支：
 *    既不静默空白，也不让 `data.rates` 这类取值把整棵调用树带崩。
 *
 * 口径边界：判据是**绝对涨幅**，不含板块涨停语义 ⇒ 本组件与 i18n 一律不得出现「涨停」字样。
 */
export function MoverRecallPanel() {
  const { t } = useTranslation();
  const [data, setData] = useState<MoverRecallView | null>(null);
  const [failed, setFailed] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [nonce, setNonce] = useState(0);

  useEffect(() => {
    let cancelled = false;
    // async IIFE 而非 .then：invoke 被 mock 成同步返回 undefined 时不至于把整棵树带崩
    void (async () => {
      setLoading(true);
      setFailed(null);
      try {
        const r = await invoke<MoverRecallView>("analyze_mover_recall", {});
        if (!cancelled) {
          const missing = missingFields(r);
          if (missing.length === 0) {
            setData(r);
          } else {
            setData(null);
            setFailed(t("stockAnalysis.moverRecall.malformed", { fields: missing.join(", ") }));
          }
        }
      } catch (e) {
        if (!cancelled) {
          setData(null);
          setFailed(detailOf(e));
        }
      } finally {
        if (!cancelled) { setLoading(false); }
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [nonce]);

  const tierName = (period: string): string => {
    const suffix = horizonSuffix(period);
    return suffix ? t(`stockAnalysis.timeHorizon${suffix}`) : period;
  };

  const layerName = (layer: string): string => t(`stockAnalysis.moverRecall.layer.${layer}`, { defaultValue: layer });

  const marketName = (type: string): string =>
    t(`stockAnalysis.moverRecall.marketType.${type}`, { defaultValue: type });

  return (
    <Card
      size="small"
      title={t("stockAnalysis.moverRecall.title")}
      extra={
        <Button size="small" loading={loading} onClick={() => setNonce((n) => n + 1)}>
          {t("stockAnalysis.moverRecall.refresh")}
        </Button>
      }
      styles={{ body: { padding: "8px 10px", fontSize: 12 } }}
    >
      {loading && !data
        ? <Spin size="small" style={{ display: "block", margin: "16px auto" }} />
        : null}

      {failed
        ? (
          <div style={{ color: "var(--sa-warning, #faad14)" }} data-testid="mover-recall-failed">
            {t("stockAnalysis.moverRecall.failed", { detail: failed })}
          </div>
        )
        : null}

      {data
        ? (
          <>
            {/* ① 数据边界显式声明：起点之前未采集，不做回填推断 */}
            <div
              style={{ color: "var(--color-text-secondary)", marginBottom: 4 }}
              data-testid="mover-recall-boundary"
            >
              {t("stockAnalysis.moverRecall.boundary", {
                from: data.from,
                to: data.to,
                since: data.dataSince,
              })}
            </div>

            {/* 枚举域：确认度不足时显式声明「池外大涨股可能不在样本内」 */}
            <div style={{ marginBottom: 6 }} data-testid="mover-recall-universe">
              {data.universeConfirmed < data.universeSize
                ? t("stockAnalysis.moverRecall.universeDegraded", {
                  size: data.universeSize,
                  confirmed: data.universeConfirmed,
                })
                : t("stockAnalysis.moverRecall.universe", {
                  size: data.universeSize,
                  confirmed: data.universeConfirmed,
                })}
            </div>

            {/* 三个率：null 显示「样本不足，算不出」，绝不显示成 0% */}
            <div style={{ display: "flex", gap: 24, flexWrap: "wrap", margin: "10px 0" }}>
              <RateStat
                label={t("stockAnalysis.moverRecall.rateReachability")}
                value={data.rates.reachability}
              />
              <RateStat
                label={t("stockAnalysis.moverRecall.rateCoverage")}
                value={data.rates.coverage}
              />
              <RateStat
                label={t("stockAnalysis.moverRecall.rateUnexplained")}
                value={data.rates.unexplainedShare}
              />
              <div
                style={{
                  fontSize: 11,
                  color: "var(--color-text-tertiary)",
                  alignSelf: "flex-end",
                }}
              >
                {t("stockAnalysis.moverRecall.events", {
                  events: data.rates.events,
                  misses: data.rates.misses,
                })}
              </div>
            </div>

            {data.rates.events === 0
              ? (
                <div
                  style={{ color: "var(--color-text-tertiary)" }}
                  data-testid="mover-recall-empty"
                >
                  {data.rules.every((r) => data.collectedDays >= r.windowDays)
                    ? t("stockAnalysis.moverRecall.empty")
                    : t("stockAnalysis.moverRecall.emptyNotYet", { have: data.collectedDays })}
                </div>
              )
              : null}

            {/* ② 四档判据：阈值来源（模板变量名）逐档点名 */}
            <div style={{ fontSize: 11, color: "var(--color-text-tertiary)" }}>
              {t("stockAnalysis.moverRecall.rulesTitle")}
            </div>
            <ul style={{ margin: "2px 0 8px", paddingLeft: 18 }}>
              {data.rules.map((r) => (
                <li key={r.period} data-testid={`mover-rule-${r.period}`}>
                  {t("stockAnalysis.moverRecall.rule", {
                    tier: tierName(r.period),
                    gain: r.gainPct,
                    days: r.windowDays,
                    var: r.varName,
                  })}
                  {data.collectedDays < r.windowDays
                    ? (
                      <span
                        style={{ color: "var(--sa-warning, #faad14)", marginLeft: 6 }}
                        data-testid={`mover-window-open-${r.period}`}
                      >
                        {t("stockAnalysis.moverRecall.windowOpen", {
                          have: data.collectedDays,
                          need: r.windowDays,
                        })}
                      </span>
                    )
                    : null}
                </li>
              ))}
            </ul>
            {data.rules.length < 4
              ? (
                <div
                  style={{ color: "var(--sa-warning, #faad14)", marginBottom: 6 }}
                  data-testid="mover-recall-rules-missing"
                >
                  {t("stockAnalysis.moverRecall.rulesMissing", { count: 4 - data.rules.length })}
                </div>
              )
              : null}

            {/* 归因分层：每层计数 + 按板块分组（避免混池假象） */}
            <div style={{ fontSize: 11, color: "var(--color-text-tertiary)" }}>
              {t("stockAnalysis.moverRecall.layersTitle")}
            </div>
            <table style={{ width: "100%", borderCollapse: "collapse", marginBottom: 8 }}>
              <tbody>
                {data.layers.map((row) => (
                  <tr
                    key={row.layer}
                    style={{ borderBottom: "0.5px solid var(--color-border-tertiary)" }}
                  >
                    <td style={{ padding: "3px 0" }}>{layerName(row.layer)}</td>
                    <td style={{ padding: "3px 0", textAlign: "right", fontWeight: 500 }}>
                      {row.count}
                    </td>
                    <td
                      style={{
                        padding: "3px 0 3px 12px",
                        fontSize: 11,
                        color: "var(--color-text-tertiary)",
                      }}
                    >
                      {row.byMarketType
                        .map(([mt, n]) => `${marketName(mt)} ${n}`)
                        .join(" · ")}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>

            {/* 漏检明细（有留痕的才可归因；截断留痕风格逐条点名） */}
            {data.misses.length > 0
              ? (
                <>
                  <div style={{ fontSize: 11, color: "var(--color-text-tertiary)" }}>
                    {t("stockAnalysis.moverRecall.missesTitle")}
                  </div>
                  <div style={{ maxHeight: 260, overflowY: "auto" }}>
                    <table
                      style={{ width: "100%", borderCollapse: "collapse", fontSize: 11 }}
                      data-testid="mover-miss-table"
                    >
                      <thead>
                        <tr style={{ color: "var(--color-text-tertiary)" }}>
                          <th style={{ textAlign: "left", padding: "3px 0" }}>
                            {t("stockAnalysis.moverRecall.colName")}
                          </th>
                          <th style={{ textAlign: "center", padding: "3px 0" }}>
                            {t("stockAnalysis.moverRecall.colTier")}
                          </th>
                          <th style={{ textAlign: "right", padding: "3px 0" }}>
                            {t("stockAnalysis.moverRecall.colCum")}
                          </th>
                          <th style={{ textAlign: "right", padding: "3px 0" }}>
                            {t("stockAnalysis.moverRecall.colMaxDaily")}
                          </th>
                          <th style={{ textAlign: "left", padding: "3px 0 3px 10px" }}>
                            {t("stockAnalysis.moverRecall.colLayer")}
                          </th>
                        </tr>
                      </thead>
                      <tbody>
                        {data.misses.map((m) => (
                          <tr key={`${m.event.stockCode}-${m.event.period}`}>
                            <td style={{ padding: "3px 0" }}>
                              {`${m.event.stockName} ${m.event.stockCode}`}
                            </td>
                            <td style={{ textAlign: "center", padding: "3px 0" }}>
                              {tierName(m.event.period)}
                            </td>
                            <td
                              style={{
                                textAlign: "right",
                                padding: "3px 0",
                                color: "var(--sa-red)",
                              }}
                            >
                              {`+${m.event.cumGainPct.toFixed(1)}%`}
                            </td>
                            <td style={{ textAlign: "right", padding: "3px 0" }}>
                              {`+${m.event.maxDailyPct.toFixed(1)}%`}
                            </td>
                            <td style={{ padding: "3px 0 3px 10px" }}>
                              {layerName(m.layer)}
                              {m.scoredOutStyles.length > 0
                                ? (
                                  <div
                                    style={{ fontSize: 10, color: "var(--color-text-tertiary)" }}
                                    data-testid="mover-scored-out"
                                  >
                                    {t("stockAnalysis.moverRecall.scoredOut", {
                                      styles: m.scoredOutStyles.join("、"),
                                    })}
                                  </div>
                                )
                                : null}
                            </td>
                          </tr>
                        ))}
                      </tbody>
                    </table>
                  </div>
                </>
              )
              : null}

            {/* 说明段：三种「非算法缺陷」的口径边界，逐条成句 */}
            <div
              style={{
                marginTop: 8,
                fontSize: 10,
                color: "var(--color-text-tertiary)",
                lineHeight: 1.7,
              }}
            >
              <div>{t("stockAnalysis.moverRecall.noteByDesign")}</div>
              <div>{t("stockAnalysis.moverRecall.notePoolDegraded")}</div>
              <div>{t("stockAnalysis.moverRecall.noteShadow")}</div>
            </div>
          </>
        )
        : null}
    </Card>
  );
}

/** 一块率读数：`null` = 分母为 0（算不出）≠ 0%。 */
function RateStat({ label, value }: { label: string; value: number | null }) {
  const { t } = useTranslation();
  return (
    <div style={{ minWidth: 130 }}>
      <div style={{ fontSize: 11, color: "var(--color-text-tertiary)" }}>{label}</div>
      <div
        style={{
          fontSize: 16,
          fontWeight: 500,
          color: value == null ? "var(--color-text-tertiary)" : undefined,
        }}
      >
        {value == null
          ? t("stockAnalysis.moverRecall.rateUnavailable")
          : `${value.toFixed(1)}%`}
      </div>
    </div>
  );
}

/** 面板渲染所必需的顶层标量字段（`null` 视同缺）。 */
const REQUIRED_SCALARS = ["from", "to", "dataSince", "collectedDays", "rates"] as const;
/** 面板要逐行遍历的顶层数组字段。 */
const REQUIRED_ARRAYS = ["rules", "layers", "misses"] as const;

/**
 * 形状守卫：返回缺失/类型不符的顶层字段名（空数组 ⇒ 形状可用，交由 TS 的 `MoverRecallView` 断言）。
 *
 * 为什么需要它：`invoke<T>` 的 `T` 只是编译期断言，管不住后端实际返回了什么；
 * 浏览器 mock 的 default 兜底与测试里的通用 `mockResolvedValue` 都会给出 `{}` / `[]`，
 * 此时若直接把载荷当视图渲染，`data.rates.reachability` 会把整棵调用树带崩（矩阵页白屏）。
 */
function missingFields(r: unknown): string[] {
  if (r == null || typeof r !== "object") {
    return [...REQUIRED_SCALARS, ...REQUIRED_ARRAYS];
  }
  const o = r as Record<string, unknown>;
  const missing: string[] = REQUIRED_SCALARS.filter((k) => o[k] == null);
  for (const k of REQUIRED_ARRAYS) {
    if (!Array.isArray(o[k])) { missing.push(k); }
  }
  return missing;
}

/** 后端错误按 `detail` 成句呈现（已是完整说明），非 JSON 时退回消息原文。 */
function detailOf(e: unknown): string {
  const msg = e instanceof Error ? e.message : String(e);
  try {
    const parsed = JSON.parse(msg) as { code?: unknown; detail?: unknown };
    if (typeof parsed.detail === "string" && parsed.detail.length > 0) { return parsed.detail; }
    if (typeof parsed.code === "string") { return parsed.code; }
  } catch { /* 非 JSON ⇒ 原文 */ }
  return msg;
}
