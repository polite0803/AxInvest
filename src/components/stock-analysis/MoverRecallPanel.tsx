import { invoke } from "@/lib/invoke";
import { horizonSuffix } from "@/lib/stock-analysis-utils";
import type { MoverRecallEventReview, MoverRecallReviewReflection, MoverRecallView } from "@/types/stock-analysis";
import { Button, Card, Spin, Tag } from "antd";
import { Fragment, useEffect, useState } from "react";
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
  // P3：漏检明细行展开 → 懒加载补分析报告 + 逐档反思。key = 三键（票×右端日×档）。
  const [expandedKeys, setExpandedKeys] = useState<ReadonlySet<string>>(new Set());
  const [reviews, setReviews] = useState<
    Record<string, { loading: boolean; error: string | null; data: MoverRecallEventReview | null }>
  >({});

  const missKey = (stockCode: string, anchorDate: string, period: string): string =>
    `${stockCode}-${anchorDate}-${period}`;

  const toggleReview = async (key: string, stockCode: string, anchorDate: string, period: string) => {
    if (expandedKeys.has(key)) {
      setExpandedKeys((prev) => {
        const next = new Set(prev);
        next.delete(key);
        return next;
      });
      return;
    }
    setExpandedKeys((prev) => new Set(prev).add(key));
    // 已加载过（含 null/失败留痕）不重复请求；失败后想重试靠面板顶部刷新整轮。
    if (reviews[key] != null) {
      return;
    }
    setReviews((prev) => ({ ...prev, [key]: { loading: true, error: null, data: null } }));
    try {
      const data = await invoke<MoverRecallEventReview | null>("get_mover_recall_event_review", {
        stockCode,
        anchorDate,
        period,
      });
      setReviews((prev) => ({ ...prev, [key]: { loading: false, error: null, data: data ?? null } }));
    } catch (e) {
      setReviews((prev) => ({
        ...prev,
        [key]: { loading: false, error: detailOf(e), data: null },
      }));
    }
  };

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
                          <th style={{ width: 40 }} />
                        </tr>
                      </thead>
                      <tbody>
                        {data.misses.map((m) => {
                          const key = missKey(m.event.stockCode, m.event.anchorDate, m.event.period);
                          const expanded = expandedKeys.has(key);
                          const reviewState = reviews[key];
                          return (
                            <Fragment key={key}>
                              <tr>
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
                                <td style={{ textAlign: "center", padding: "3px 0" }}>
                                  <Button
                                    size="small"
                                    type="text"
                                    aria-label={t("stockAnalysis.moverRecall.review.toggle")}
                                    data-testid={`mover-review-toggle-${key}`}
                                    onClick={() =>
                                      void toggleReview(
                                        key,
                                        m.event.stockCode,
                                        m.event.anchorDate,
                                        m.event.period,
                                      )}
                                  >
                                    {expanded ? "−" : "+"}
                                  </Button>
                                </td>
                              </tr>
                              {expanded
                                ? (
                                  <tr key={`${key}-detail`}>
                                    <td colSpan={6} style={{ padding: "6px 8px 10px" }}>
                                      <MoverReviewDetail
                                        state={reviewState ?? { loading: true, error: null, data: null }}
                                        tierName={tierName}
                                        layerName={layerName}
                                      />
                                    </td>
                                  </tr>
                                )
                                : null}
                            </Fragment>
                          );
                        })}
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

// ── P3：漏检行展开详情（补分析报告 + 逐档反思，懒加载自 `get_mover_recall_event_review`）──

type ReviewState = {
  loading: boolean;
  error: string | null;
  data: MoverRecallEventReview | null;
};

/** 反思卡固定档位顺序（后端按字母序返回，前端重排为超短→长）。 */
const REVIEW_HORIZON_ORDER = ["ultra_short", "short", "mid", "long"];

const EVENT_STATUS_COLOR: Record<string, string> = {
  pending: "default",
  excluded: "default",
  running: "processing",
  analyzed: "blue",
  reflected: "green",
  failed: "error",
};

const VERDICT_COLOR: Record<string, string> = {
  correct: "green",
  partial: "gold",
  wrong: "red",
};

/** 展开区：加载/错误/未落库/事件四态 + 分析摘要与逐档反思卡。 */
function MoverReviewDetail({
  state,
  tierName,
  layerName,
}: {
  state: ReviewState;
  tierName: (code: string) => string;
  layerName: (code: string) => string;
}) {
  const { t } = useTranslation();

  if (state.loading) {
    return (
      <div style={{ display: "flex", gap: 8, alignItems: "center" }} data-testid="mover-review-loading">
        <Spin size="small" />
        <span style={{ fontSize: 12 }}>{t("stockAnalysis.moverRecall.review.loading")}</span>
      </div>
    );
  }
  if (state.error != null) {
    return (
      <div className="sa-failed" style={{ fontSize: 12 }} data-testid="mover-review-error">
        {t("stockAnalysis.moverRecall.review.loadError", { detail: state.error })}
      </div>
    );
  }
  const r = state.data;
  // 实时核查视图的 misses 不落库；命令按三键查不到事件 ⇒ 提示而非留白。
  if (r == null) {
    return (
      <div style={{ fontSize: 12, color: "var(--color-text-tertiary)" }} data-testid="mover-review-not-tracked">
        {t("stockAnalysis.moverRecall.review.notTracked")}
      </div>
    );
  }

  const statusColor = EVENT_STATUS_COLOR[r.status] ?? "default";
  const sortedReflections = [...r.reflections].sort((a, b) => {
    const ia = REVIEW_HORIZON_ORDER.indexOf(a.horizon ?? "");
    const ib = REVIEW_HORIZON_ORDER.indexOf(b.horizon ?? "");
    return (ia < 0 ? 99 : ia) - (ib < 0 ? 99 : ib);
  });
  const showAnalysisBlock = r.status === "analyzed" || r.status === "reflected";
  // 补分析整行是 Model 的 serde 直出（camelCase），宽松取值；缺字段不渲染该行。
  const a = r.analysis;
  const analysisDate = a == null
    ? null
    : (typeof a.asOfDate === "string" ? a.asOfDate : null)
      ?? (typeof a.analysisDate === "string" ? a.analysisDate : null);
  const decisionAction = a == null ? null : typeof a.decisionAction === "string" ? a.decisionAction : null;
  const decisionReasoning = a == null
    ? null
    : typeof a.decisionReasoning === "string"
    ? a.decisionReasoning
    : null;

  return (
    <div
      data-testid="mover-review-detail"
      style={{
        background: "var(--color-bg-secondary, rgba(127,127,127,0.06))",
        border: "1px solid var(--color-border-secondary, rgba(127,127,127,0.2))",
        borderRadius: 6,
        padding: "8px 10px",
        fontSize: 12,
      }}
    >
      <div style={{ display: "flex", gap: 8, alignItems: "center", flexWrap: "wrap" }}>
        <Tag color={statusColor}>{t(`stockAnalysis.moverRecall.review.status_${r.status}`)}</Tag>
        {r.missLayer
          ? (
            <span style={{ color: "var(--color-text-secondary)" }}>
              {t("stockAnalysis.moverRecall.review.layerHint", { layer: layerName(r.missLayer) })}
            </span>
          )
          : null}
        <span style={{ color: "var(--color-text-tertiary)" }}>
          {t("stockAnalysis.moverRecall.review.windowFacts", {
            start: r.windowStart,
            anchor: r.anchorDate,
            days: r.windowDays,
            gain: r.cumGainPct.toFixed(1),
            max: r.maxDailyPct.toFixed(1),
          })}
        </span>
      </div>

      {r.status === "failed"
        ? (
          <div className="sa-failed" style={{ marginTop: 6 }} data-testid="mover-review-failed">
            {t("stockAnalysis.moverRecall.review.errorDetail", { detail: r.errorDetail ?? "-" })}
          </div>
        )
        : null}
      {(r.status === "pending" || r.status === "excluded" || r.status === "running") && r.errorDetail
        ? (
          <div style={{ marginTop: 6, color: "var(--color-text-tertiary)" }}>
            {t("stockAnalysis.moverRecall.review.errorDetail", { detail: r.errorDetail })}
          </div>
        )
        : null}

      {showAnalysisBlock
        ? (
          <div style={{ marginTop: 8 }} data-testid="mover-review-analysis">
            <div style={{ fontWeight: 600, marginBottom: 4 }}>
              {analysisDate
                ? t("stockAnalysis.moverRecall.review.analysisHeading", { date: analysisDate })
                : t("stockAnalysis.moverRecall.review.analysisHeading", { date: "-" })}
            </div>
            {a == null
              ? (
                <div style={{ color: "var(--color-text-tertiary)" }}>
                  {t("stockAnalysis.moverRecall.review.analysisMissing")}
                </div>
              )
              : (
                <>
                  {decisionAction
                    ? (
                      <div>
                        <strong>
                          {t("stockAnalysis.moverRecall.review.decisionLabel", { action: decisionAction })}
                        </strong>
                      </div>
                    )
                    : null}
                  {decisionReasoning
                    ? (
                      <div
                        style={{
                          marginTop: 2,
                          color: "var(--color-text-secondary)",
                          display: "-webkit-box",
                          WebkitLineClamp: 4,
                          WebkitBoxOrient: "vertical",
                          overflow: "hidden",
                        }}
                        title={decisionReasoning}
                      >
                        {decisionReasoning}
                      </div>
                    )
                    : null}
                </>
              )}
          </div>
        )
        : null}

      {showAnalysisBlock
        ? (
          <div style={{ marginTop: 8 }} data-testid="mover-review-reflections">
            <div style={{ fontWeight: 600, marginBottom: 4 }}>
              {t("stockAnalysis.moverRecall.review.reflectionHeading")}
            </div>
            {sortedReflections.length === 0
              ? (
                <div style={{ color: "var(--color-text-tertiary)" }}>
                  {t("stockAnalysis.moverRecall.review.noReflection")}
                </div>
              )
              : (
                <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
                  {sortedReflections.map((rf) => <ReflectionCard key={rf.id} rf={rf} tierName={tierName} />)}
                </div>
              )}
          </div>
        )
        : null}
    </div>
  );
}

/** 单档反思卡：completed 展示裁决 + 三问 + 收益；pending/running/failed 只出状态。 */
function ReflectionCard({
  rf,
  tierName,
}: {
  rf: MoverRecallReviewReflection;
  tierName: (code: string) => string;
}) {
  const { t } = useTranslation();

  const horizonLabel = rf.horizon ? tierName(rf.horizon) : "-";
  // status 失败行落库形如 "failed: <reason>"。
  const failedMatch = /^failed(?::\s?(.*))?$/.exec(rf.status);
  const signals = parseSignals(rf.missedSignals);

  return (
    <div
      data-testid={`mover-review-rf-${rf.horizon ?? "unknown"}`}
      style={{
        border: "1px solid var(--color-border-secondary, rgba(127,127,127,0.2))",
        borderRadius: 4,
        padding: "6px 8px",
      }}
    >
      <div style={{ display: "flex", gap: 8, alignItems: "center", flexWrap: "wrap" }}>
        <strong>{horizonLabel}</strong>
        {rf.status === "completed" && rf.verdict
          ? (
            <Tag color={VERDICT_COLOR[rf.verdict] ?? "default"}>
              {t("stockAnalysis.moverRecall.review.verdictLabel")}
              {": "}
              {t(`stockAnalysis.moverRecall.review.verdict_${rf.verdict}`)}
            </Tag>
          )
          : null}
        {rf.status === "completed" && rf.rawReturn != null
          ? (
            <span style={{ color: "var(--color-text-tertiary)" }}>
              {t("stockAnalysis.moverRecall.review.rawReturn")}
              {": "}
              {`${rf.rawReturn >= 0 ? "+" : ""}${rf.rawReturn.toFixed(1)}%`}
            </span>
          )
          : null}
        {rf.status === "pending"
          ? <Tag>{t("stockAnalysis.moverRecall.review.rfPending")}</Tag>
          : null}
        {rf.status === "running"
          ? <Tag color="processing">{t("stockAnalysis.moverRecall.review.rfRunning")}</Tag>
          : null}
        {failedMatch
          ? (
            <Tag color="error">
              {t("stockAnalysis.moverRecall.review.rfFailed", {
                detail: failedMatch[1] || "-",
              })}
            </Tag>
          )
          : null}
      </div>

      {rf.status === "completed"
        ? (
          <div style={{ marginTop: 4, display: "flex", flexDirection: "column", gap: 3 }}>
            {rf.lessonSummary
              ? (
                <div>
                  <strong>{t("stockAnalysis.moverRecall.review.lessonSummary")}</strong>
                  {": "}
                  {rf.lessonSummary}
                </div>
              )
              : null}
            {signals.length > 0
              ? (
                <div>
                  <strong>{t("stockAnalysis.moverRecall.review.missedSignals")}</strong>
                  {": "}
                  <ul style={{ margin: "2px 0 0 18px", padding: 0 }}>
                    {signals.map((s, i) => <li key={i}>{s}</li>)}
                  </ul>
                </div>
              )
              : null}
            {rf.whatWentWrong
              ? (
                <div>
                  <strong>{t("stockAnalysis.moverRecall.review.whatWentWrong")}</strong>
                  {": "}
                  {rf.whatWentWrong}
                </div>
              )
              : null}
            {rf.fixForFuture
              ? (
                <div>
                  <strong>{t("stockAnalysis.moverRecall.review.fixForFuture")}</strong>
                  {": "}
                  {rf.fixForFuture}
                </div>
              )
              : null}
          </div>
        )
        : null}
    </div>
  );
}

/** missed_signals 落库为 JSON 数组字符串；非数组/解析失败时按单条原文展示（不丢信息）。 */
function parseSignals(raw: string | null): string[] {
  if (raw == null || raw.trim().length === 0) { return []; }
  try {
    const v: unknown = JSON.parse(raw);
    if (Array.isArray(v) && v.every((x) => typeof x === "string")) { return v as string[]; }
  } catch { /* 非 JSON ⇒ 原文 */ }
  return [raw];
}
