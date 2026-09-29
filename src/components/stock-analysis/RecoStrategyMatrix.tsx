import { invoke } from "@/lib/invoke";
import { horizonIcAbsenceKey, horizonSuffix } from "@/lib/stock-analysis-utils";
import type { BacktestComparisonResponse, RecoIcCell, RecoIcStats, StrategyStats } from "@/types/stock-analysis";
import { Card, Empty, Segmented, Spin, Tag, Tooltip } from "antd";
import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";

const STYLE_KEYS = ["trend", "value", "capital", "reversion"] as const;
/** 四档全枚举。此前缺 `ultra_short` ⇒ 该档的回测统计在矩阵里静默不存在，用户读不出是「没这档」还是「这档没问题」。 */
const PERIOD_KEYS = ["ultra_short", "short", "mid", "long"] as const;

/** 色标辅助 */
function rateColor(rate: number): string {
  if (rate >= 55) { return "var(--sa-red)"; }
  if (rate >= 45) { return "var(--sa-warning, #faad14)"; }
  return "var(--sa-green)";
}

function rateBg(rate: number): string {
  if (rate >= 55) { return "rgba(226, 75, 74, 0.10)"; }
  if (rate >= 45) { return "rgba(250, 173, 20, 0.10)"; }
  return "rgba(82, 196, 26, 0.10)";
}

interface RecoStrategyMatrixProps {
  /** 外部传入的回测数据（可选）；不传时组件自己加载 */
  data?: BacktestComparisonResponse | null;
  /** 选中策略回调 */
  onSelectStrategy?: (strategyId: string | null) => void;
}

export function RecoStrategyMatrix({ data: externalData, onSelectStrategy }: RecoStrategyMatrixProps) {
  const { t } = useTranslation();
  const [internalData, setInternalData] = useState<BacktestComparisonResponse | null>(null);
  const [loading, setLoading] = useState(false);
  // eslint-disable-next-line @typescript-eslint/no-unused-vars
  const [error, _setError] = useState<string | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  // Phase R-E：逐 (风格, 档位) rank IC —— 只报数，不回写任何权重
  const [ic, setIc] = useState<RecoIcStats | null>(null);
  const [group, setGroup] = useState<"positive" | "negative">("positive");

  useEffect(() => {
    if (externalData) { return; }
    let cancelled = false;
    Promise.resolve().then(() => {
      if (cancelled) { return; }
      setLoading(true);
      return invoke<BacktestComparisonResponse>("backtest_reco_strategies", { group });
    })
      .then((data) => {
        if (!cancelled) { setInternalData(data ?? null); }
      })
      .catch((e) => {
        if (!cancelled) { console.error("[RecoStrategyMatrix]", e); }
      })
      .finally(() => {
        if (!cancelled) { setLoading(false); }
      });
    return () => {
      cancelled = true;
    };
  }, [group, externalData]);

  // IC 与回测同源不同表：单独取，失败静默（面板退化成「只有胜率」而不报错）
  useEffect(() => {
    let cancelled = false;
    // 用 async IIFE 而不是 .then：invoke 被 mock 成同步返回 undefined 时，
    // 直接 .then 会抛 TypeError 把整张表带崩（观测面拿不到不该影响主表）。
    void (async () => {
      try {
        const r = await invoke<RecoIcStats>("reco_ic_stats");
        if (!cancelled) { setIc(r ?? null); }
      } catch { /* 观测面不可得时不干扰主表 */ }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  const icCells = useMemo(() => {
    const map: Record<string, Record<string, RecoIcCell>> = {};
    for (const st of ic?.styles ?? []) {
      map[st.style] = {};
      for (const c of st.cells) { map[st.style][c.period] = c; }
    }
    return map;
  }, [ic]);

  const halfLifeOf = (style: string): string | null => {
    const st = ic?.styles.find((x) => x.style === style);
    if (!st) { return null; }
    if (st.halfLifeDays != null) {
      return t("stockAnalysis.reflection.hitrateHalfLifeDays", { days: st.halfLifeDays.toFixed(1) });
    }
    return t("stockAnalysis.reflection.hitrateHalfLifeNone", { n: 0 });
  };

  // 数据源
  const data = externalData ?? internalData;

  // 按 (style, period) 重组
  const matrix = useMemo(() => {
    if (!data) { return null; }
    const map: Record<string, Record<string, StrategyStats>> = {};
    const grp = group === "positive" ? data.positive : data.negative;
    for (const [, s] of Object.entries(grp.strategies)) {
      if (!map[s.style]) { map[s.style] = {}; }
      map[s.style][s.period] = s;
    }
    return map;
  }, [data, group]);

  if (loading) {
    return <Spin size="small" style={{ display: "block", margin: "24px auto" }} />;
  }

  if (error) {
    return (
      <div className="text-xs text-gray-500 text-center py-8">
        {error}
      </div>
    );
  }

  if (!matrix) {
    return (
      <Empty
        image={Empty.PRESENTED_IMAGE_SIMPLE}
        description={t("stockAnalysis.backtest.strategyEmpty")}
      />
    );
  }

  return (
    <Card
      size="small"
      title={t("stockAnalysis.backtest.matrixTitle")}
      extra={
        <Segmented
          size="small"
          value={group}
          onChange={(v) => {
            setGroup(v as "positive" | "negative");
            setSelected(null);
            onSelectStrategy?.(null);
          }}
          options={[
            { label: t("stockAnalysis.backtest.groupPositive"), value: "positive" },
            { label: t("stockAnalysis.backtest.groupNegative"), value: "negative" },
          ]}
        />
      }
      styles={{ body: { padding: "8px 10px" } }}
    >
      <table style={{ width: "100%", borderCollapse: "collapse", fontSize: 12 }}>
        <thead>
          <tr style={{ borderBottom: "0.5px solid var(--color-border-tertiary)" }}>
            <th style={{ padding: "8px 10px", textAlign: "left", fontWeight: 500, width: 100 }}>
              {t("stockAnalysis.backtest.colStyle")}
            </th>
            {PERIOD_KEYS.map((p) => {
              const suffix = horizonSuffix(p);
              return (
                <th key={p} style={{ padding: "8px 10px", textAlign: "center", fontWeight: 500 }}>
                  {suffix ? t(`stockAnalysis.timeHorizon${suffix}`) : p}
                </th>
              );
            })}
          </tr>
        </thead>
        <tbody>
          {STYLE_KEYS.map((style) => (
            <tr key={style} style={{ borderBottom: "0.5px solid var(--color-border-tertiary)" }}>
              <td style={{ padding: "8px 10px", fontWeight: 500 }}>
                {t(`stockAnalysis.recommendation.style${style.charAt(0).toUpperCase() + style.slice(1)}`)}
                {(() => {
                  const hl = halfLifeOf(style);
                  if (!hl) {
                    return null;
                  }
                  const st = ic?.styles.find((x) =>
                    x.style === style
                  );
                  return (
                    <div
                      style={{ fontSize: 10, fontWeight: 400, color: "var(--color-text-tertiary)" }}
                      data-testid="reco-ic-half-life"
                    >
                      {`${t("stockAnalysis.reflection.hitrateHalfLife")}: ${hl}`}
                      {st && st.halfLifeDays == null ? `（${st.halfLifeStatus}）` : ""}
                    </div>
                  );
                })()}
              </td>
              {PERIOD_KEYS.map((period) => {
                const s = matrix[style]?.[period];
                const sid = `${style}_${period}`;
                const isSelected = selected === sid;

                // reversion_long 不存在
                if (style === "reversion" && period === "long") {
                  return (
                    <td key={period} style={{ padding: "8px 10px", textAlign: "center" }}>
                      <span style={{ color: "var(--color-text-tertiary)", fontSize: 11 }}>—</span>
                    </td>
                  );
                }

                if (!s || s.totalSignals === 0) {
                  return (
                    <td key={period} style={{ padding: "8px 10px", textAlign: "center" }}>
                      <span style={{ color: "var(--color-text-tertiary)", fontSize: 11 }}>—</span>
                    </td>
                  );
                }

                return (
                  <td
                    key={period}
                    style={{
                      padding: "6px 8px",
                      textAlign: "center",
                      cursor: "pointer",
                      borderRadius: 6,
                      background: isSelected ? rateBg(s.winRatePct) : "transparent",
                      outline: isSelected ? `2px solid ${rateColor(s.winRatePct)}` : "none",
                      outlineOffset: -2,
                    }}
                    onClick={() => {
                      const next = isSelected ? null : sid;
                      setSelected(next);
                      onSelectStrategy?.(next);
                    }}
                  >
                    <Tooltip
                      title={
                        <div style={{ fontSize: 11, lineHeight: 1.8 }}>
                          <div>
                            {`${t("stockAnalysis.backtest.colWinRate")}: ${s.winRatePct.toFixed(1)}%`}
                          </div>
                          <div>
                            {`${t("stockAnalysis.backtest.colAvgReturn")}: ${s.avgReturnPct.toFixed(2)}%`}
                          </div>
                          <div>{`Sharpe: ${s.sharpeRatio != null ? s.sharpeRatio.toFixed(2) : "—"}`}</div>
                          <div>{`Profit Factor: ${s.profitFactor != null ? s.profitFactor.toFixed(2) : "—"}`}</div>
                          <div>{`${t("stockAnalysis.backtest.colSignalCount")}: ${s.totalSignals}`}</div>
                          <div>
                            {`${t("stockAnalysis.backtest.colMaxLossStreak")}: ${s.maxConsecutiveLosses}`}
                          </div>
                        </div>
                      }
                    >
                      <span
                        style={{
                          display: "inline-block",
                          padding: "4px 12px",
                          borderRadius: 4,
                          background: rateBg(s.winRatePct),
                          color: rateColor(s.winRatePct),
                          fontWeight: 500,
                          fontSize: 13,
                        }}
                      >
                        {s.winRatePct.toFixed(1)}%
                      </span>
                    </Tooltip>
                    <div style={{ fontSize: 10, color: "var(--color-text-tertiary)", marginTop: 2 }}>
                      {`S ${s.sharpeRatio != null ? s.sharpeRatio.toFixed(1) : "—"} · ${s.totalSignals}`}
                    </div>
                    {(() => {
                      const cell = icCells[style]?.[period];
                      if (!cell) {
                        // 一格都没有实现样本：与「样本不够」不同，得说不清的方向
                        return (
                          <div style={{ fontSize: 10, color: "var(--color-text-tertiary)" }} data-testid="reco-ic-none">
                            {t("stockAnalysis.backtest.icNoValidatedSample")}
                          </div>
                        );
                      }
                      if (cell.rankIc != null) {
                        return (
                          <div style={{ fontSize: 10, color: "var(--color-text-tertiary)" }}>
                            {`IC ${cell.rankIc.toFixed(3)} · n=${cell.samples}`}
                          </div>
                        );
                      }
                      const key = horizonIcAbsenceKey(cell.icStatus);
                      return (
                        <div style={{ fontSize: 10, color: "var(--sa-warning, #faad14)" }}>
                          {key ? t(key) : cell.icStatus}
                        </div>
                      );
                    })()}
                  </td>
                );
              })}
            </tr>
          ))}
        </tbody>
      </table>

      {data?.skipped && data.skipped.length > 0 && (
        <div style={{ marginTop: 8, display: "flex", gap: 4, flexWrap: "wrap" }}>
          {data.skipped.map((reason, i) => (
            <Tag key={i} color="warning" style={{ fontSize: 10, margin: 0 }}>
              ⏭️ {reason}
            </Tag>
          ))}
        </div>
      )}
    </Card>
  );
}
