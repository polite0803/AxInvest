import { List } from "@/components/common/AntdList";
import { showBackendError } from "@/lib/errorI18n";
import { invoke } from "@/lib/invoke";
import {
  actionSourceLabelKey,
  confidenceSourceLabelKey,
  FAST_TEMPLATE_ID,
  getActionTagStyle,
  getActionTKey,
  HORIZON_T_SUFFIX,
  horizonSourceLabelKey,
  readDecisionProvenance,
  readHorizonActions,
  resolveDisplayAction,
} from "@/lib/stock-analysis-utils";
import { SearchOutlined } from "@ant-design/icons";
import { App, Button, Card, Checkbox, Collapse, Empty, Input, Spin, Statistic, Tag } from "antd";
import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";

interface AnalysisRecord {
  id: string;
  stockCode: string;
  stockName: string;
  analysisDate: string;
  /** 决策动作（后端直返，如 BUY/SELL/HOLD/WAIT/UNCERTAIN） */
  decisionAction: string | null;
  /** 决策仓位百分比（后端直返，0-100） */
  decisionPositionPct: number | null;
  /** 决策持仓状态轴（后端直返，v228）；null = 记录早于 v228，非 EMPTY */
  decisionPositionState: string | null;
  /** 完整决策 JSON（含 confidence 等，部分旧数据可能为 null） */
  decisionJson: string | null;
  /** 列表场景不返回，详情页通过 get_stock_analysis 单独获取 */
  blackboardSnapshot?: string | null;
  createdAt: number;
  updatedAt?: number;
  status: string;
  /** 版本化分析：指向原始记录 ID，null 表示首次分析 */
  parentAnalysisId: string | null;
  /**
   * 工作流模板 id：`"stock-analysis-fast"` = 快速 JEV 链。
   * `undefined` / `null` = 未知（本列引入前的记录或非模板产出）—— 不打标识。
   */
  templateId?: string | null;
  /**
   * 主结论所属档位（`ultra_short` / `short` / `mid` / `long`）。
   * `null` / `undefined` = 本列引入前的记录 ⇒ 按「未知周期」显示，**不得**推断成某档。
   */
  decisionTimeHorizon?: string | null;
  /** 档位来源：`formula` = 本地公式定档；`model` = 采信模型自报（历史形态）；`null` = 未知。 */
  decisionHorizonSource?: string | null;
}

interface BacktestResult {
  stockCode: string;
  analysisDate: string;
  decisionAction: string;
  decisionConfidence: number;
  entryPrice?: number;
  exitPrice: number;
  holdingDays: number;
  returnPct: number;
  wasCorrect: boolean;
  maxDrawdownPct: number;
}

interface Props {
  analysisId?: string;
}

export function HistoricalAnalysisPanel({ analysisId = "" }: Props) {
  const { message } = App.useApp();
  const { t } = useTranslation();
  const [records, setRecords] = useState<AnalysisRecord[]>([]);
  const [loading, setLoading] = useState(false);
  const [search, setSearch] = useState("");
  const [snapshot, setSnapshot] = useState<Record<string, string> | null>(null);
  const [btResult, setBtResult] = useState<BacktestResult | null>(null);
  const [btAllResults, setBtAllResults] = useState<BacktestResult[] | null>(null);
  const [btLoading, setBtLoading] = useState(false);
  const [selectMode, setSelectMode] = useState(false);
  const [selectedIds, setSelectedIds] = useState<string[]>([]);
  const [deleting, setDeleting] = useState(false);

  useEffect(() => {
    let cancelled = false;
    Promise.resolve().then(() => {
      if (cancelled) { return; }
      setLoading(true);
      return invoke("list_stock_analyses", { limit: 30 }) as Promise<AnalysisRecord[]>;
    })
      .then((list) => {
        if (cancelled || !list) { return; }
        if (Array.isArray(list)) { setRecords(list); }
      })
      .catch(() => {})
      .finally(() => {
        if (!cancelled) { setLoading(false); }
      });
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    if (!analysisId) { return; }
    let cancelled = false;
    invoke<{ blackboardSnapshot: string | null }>("get_stock_analysis", { analysisId })
      .then((r) => {
        if (cancelled) { return; }
        if (r.blackboardSnapshot) { setSnapshot(JSON.parse(r.blackboardSnapshot)); }
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, [analysisId]);

  const runBacktest = async (record: AnalysisRecord) => {
    setBtLoading(true);
    try {
      const r = await invoke<BacktestResult>("backtest_analysis", { analysisId: record.id });
      setBtResult(r);
    } catch {
      message.error(t("stockAnalysis.backtest.failed"));
    }
    setBtLoading(false);
  };

  const runBacktestAll = async () => {
    setBtLoading(true);
    try {
      const r = await invoke<BacktestResult[]>("backtest_all_history");
      if (Array.isArray(r)) { setBtAllResults(r); }
    } catch {
      message.error(t("stockAnalysis.backtest.allFailed"));
    }
    setBtLoading(false);
  };

  const filtered = useMemo(() => {
    if (!search) { return records; }
    const q = search.toLowerCase();
    return records.filter((r) =>
      r.stockCode.toLowerCase().includes(q) || r.stockName.toLowerCase().includes(q)
      || (r.decisionJson && r.decisionJson.toLowerCase().includes(q))
    );
  }, [records, search]);

  // 按 stockCode 分组：组名 "股票名称(股票代码)"，组内按时间倒序
  const grouped = useMemo(() => {
    const map = new Map<string, { stockName: string; stockCode: string; items: AnalysisRecord[] }>();
    for (const r of filtered) {
      if (!map.has(r.stockCode)) {
        map.set(r.stockCode, { stockName: r.stockName, stockCode: r.stockCode, items: [] });
      }
      map.get(r.stockCode)!.items.push(r);
    }
    for (const g of map.values()) {
      g.items.sort((a, b) => b.createdAt - a.createdAt);
    }
    return Array.from(map.values());
  }, [filtered]);

  const reportEntries = Object.entries(snapshot ?? {}).filter(([k]) => k.startsWith("report."));
  const debateEntries = Object.entries(snapshot ?? {}).filter(([k]) => k.startsWith("debate."));

  // V66 修复(2026-07-29): 提取数据质量诊断，与 DecisionBanner 展示能力对齐
  const dataQualityInfo = useMemo(() => {
    if (!snapshot) { return null; }
    const raw = snapshot["data_quality_summary"];
    if (!raw) { return null; }
    try {
      // snapshot 中的 data_quality_summary 可能是 JSON 字符串或对象
      const parsed = typeof raw === "string" ? JSON.parse(raw) : raw;
      if (parsed && typeof parsed === "object" && typeof parsed.grade === "string") {
        return {
          grade: parsed.grade as string,
          score: Number(parsed.score) || 0,
          missingFactors: Array.isArray(parsed.missing_factors) ? parsed.missing_factors as string[] : [],
          summary: typeof parsed.summary === "string" ? parsed.summary : "",
        };
      }
    } catch {
      // 解析失败返回 null
    }
    return null;
  }, [snapshot]);

  // 全量回测汇总
  const btStats = btAllResults && btAllResults.length > 0
    ? {
      total: btAllResults.length,
      correct: btAllResults.filter((r) => r.wasCorrect).length,
      avgReturn: (btAllResults.reduce((s, r) => s + r.returnPct, 0) / btAllResults.length).toFixed(2),
    }
    : null;

  return (
    <div className="flex flex-col gap-2">
      {reportEntries.length > 0 && (
        <Card size="small" title={t("stockAnalysis.history")} styles={{ body: { padding: "6px 8px" } }}>
          <Collapse
            size="small"
            items={[
              ...reportEntries.slice(0, 6).map(([key, value]) => ({
                key,
                label: (
                  <span className="text-xs">
                    {key.replace("report.", "")}
                    <Tag style={{ marginLeft: 6, fontSize: 10 }}>
                      {t("stockAnalysis.charCount", { count: value.length })}
                    </Tag>
                  </span>
                ),
                children: (
                  <pre
                    className="text-xs"
                    style={{ whiteSpace: "pre-wrap", maxHeight: 200, overflow: "auto", margin: 0 }}
                  >{value}</pre>
                ),
              })),
              ...(debateEntries.length > 0
                ? [{
                  key: "debates",
                  label: <span className="text-xs">{t("stockAnalysis.debateHistory")}</span>,
                  children: (
                    <pre
                      className="text-xs"
                      style={{ whiteSpace: "pre-wrap", maxHeight: 200, overflow: "auto", margin: 0 }}
                    >{debateEntries.map(([k, v]) => `### ${k}\n${v}`).join("\n\n")}</pre>
                  ),
                }]
                : []),
            ]}
          />
        </Card>
      )}

      {/* V66 修复(2026-07-29): 数据质量诊断展示，与 DecisionBanner 对齐 */}
      {dataQualityInfo && (
        <Card size="small" title={t("stockAnalysis.dataQuality") as string} styles={{ body: { padding: "4px 8px" } }}>
          <div className="flex items-center gap-2 flex-wrap text-xs">
            <Tag
              color={dataQualityInfo.grade === "A"
                ? "green"
                : dataQualityInfo.grade === "B"
                ? "blue"
                : dataQualityInfo.grade === "C"
                ? "gold"
                : dataQualityInfo.grade === "D"
                ? "orange"
                : "red"}
            >
              {dataQualityInfo.grade}
              {t("stockAnalysis.gradeSuffix")}
            </Tag>
            <span style={{ color: "var(--muted)" }}>
              {t("stockAnalysis.scoreLabel", { score: dataQualityInfo.score })}
            </span>
            {dataQualityInfo.missingFactors.length > 0 && (
              <span style={{ color: "var(--sa-amber, #f59e0b)" }}>
                {t("stockAnalysis.missingFactorsLabel", { factors: dataQualityInfo.missingFactors.join("、") })}
              </span>
            )}
          </div>
          {dataQualityInfo.summary && (
            <div className="mt-1 text-xs" style={{ color: "var(--muted)", lineHeight: 1.5 }}>
              {dataQualityInfo.summary}
            </div>
          )}
        </Card>
      )}

      {/* 全量回测汇总 */}
      {btStats && (
        <Card size="small" title={t("stockAnalysis.backtest.summary")} styles={{ body: { padding: "4px 8px" } }}>
          <div className="grid grid-cols-3 gap-1 text-center">
            <Statistic
              title={t("stockAnalysis.backtest.total")}
              value={btStats.total}
              styles={{ content: { fontSize: 14 } }}
            />
            <Statistic
              title={t("stockAnalysis.backtest.accuracy")}
              value={btStats.correct}
              suffix={`/${btStats.total}`}
              styles={{ content: { fontSize: 14, color: "var(--sa-green)" } }}
            />
            <Statistic
              title={t("stockAnalysis.backtest.avgReturn")}
              value={btStats.avgReturn}
              suffix="%"
              styles={{ content: { fontSize: 14 } }}
            />
          </div>
        </Card>
      )}

      {/* 单次回测结果 */}
      {btResult && (
        <Card
          size="small"
          title={t("stockAnalysis.backtest.title", { code: btResult.stockCode, action: btResult.decisionAction })}
          styles={{ body: { padding: "4px 8px" } }}
        >
          <div className="grid grid-cols-3 gap-1 text-center text-xs">
            <div>
              <span className="text-gray-400">{t("stockAnalysis.backtest.holdingDays")}</span>
              <br />
              <b>{btResult.holdingDays}</b>
            </div>
            <div>
              <span className="text-gray-400">{t("stockAnalysis.backtest.returnRate")}</span>
              <br />
              <b className={btResult.returnPct >= 0 ? "text-red-500" : "text-green-500"}>
                {btResult.returnPct >= 0 ? "+" : ""}
                {btResult.returnPct.toFixed(2)}%
              </b>
            </div>
            <div>
              <span className="text-gray-400">{t("stockAnalysis.backtest.maxDrawdown")}</span>
              <br />
              <b>{btResult.maxDrawdownPct.toFixed(2)}%</b>
            </div>
            <div className="col-span-3 mt-1">
              <Tag color={btResult.wasCorrect ? "green" : "red"}>
                {btResult.wasCorrect ? t("stockAnalysis.backtest.correct") : t("stockAnalysis.backtest.wrong")}
              </Tag>
            </div>
          </div>
        </Card>
      )}

      {/* 历史列表 */}
      <Card
        size="small"
        title={t("stockAnalysis.history")}
        styles={{ body: { padding: "6px 8px" } }}
        extra={
          <div className="flex gap-1">
            <Input
              size="small"
              prefix={<SearchOutlined />}
              placeholder={t("stockAnalysis.search")}
              style={{ width: 100 }}
              value={search}
              onChange={(e) => setSearch(e.target.value)}
              allowClear
            />
            {selectMode
              ? (
                <>
                  <Button
                    size="small"
                    onClick={() => {
                      setSelectMode(false);
                      setSelectedIds([]);
                    }}
                  >
                    {t("stockAnalysis.historyBatchExit")}
                  </Button>
                  {selectedIds.length > 0 && (
                    <Button
                      size="small"
                      danger
                      loading={deleting}
                      onClick={async () => {
                        setDeleting(true);
                        try {
                          await invoke("batch_delete_stock_analyses", { analysisIds: selectedIds });
                          message.success(t("stockAnalysis.historyDeleteSuccess", { count: selectedIds.length }));
                          setRecords((prev) => prev.filter((r) => !selectedIds.includes(r.id)));
                          setSelectedIds([]);
                          setSelectMode(false);
                        } catch (e) {
                          showBackendError(message, e);
                        }
                        setDeleting(false);
                      }}
                    >
                      {t("stockAnalysis.historyBatchDelete")}
                    </Button>
                  )}
                </>
              )
              : (
                <>
                  <Button size="small" loading={btLoading} onClick={runBacktestAll}>
                    {t("stockAnalysis.backtest.runAll")}
                  </Button>
                  <Button size="small" onClick={() => setSelectMode(true)}>
                    {t("stockAnalysis.historySelectMode")}
                  </Button>
                </>
              )}
          </div>
        }
      >
        {loading
          ? <Spin size="small" />
          : filtered.length === 0
          ? <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description={t("stockAnalysis.noRecords")} />
          : grouped.map((g) => (
            <div key={g.stockCode} className="mb-2">
              {/* 组标题：股票名称(股票代码) - 名称用主题色，代码用次要色 */}
              <div
                className="text-xs font-semibold flex items-center gap-1 px-1 py-1"
                style={{
                  borderBottom: "1px solid var(--color-border, #333)",
                }}
              >
                <span style={{ color: "var(--accent, #7c3aed)" }}>{g.stockName}</span>
                <span style={{ color: "var(--color-text-secondary, #888)", fontSize: 10 }}>
                  ({g.stockCode})
                </span>
              </div>
              {/* 组内记录：日期为记录名 */}
              <List
                size="small"
                dataSource={g.items}
                renderItem={(r) => {
                  // 优先使用后端直返字段 decisionAction / decisionPositionPct，
                  // decisionJson 仅用于提取 confidence 等额外字段（兼容旧数据）。
                  // 2026-09-22: 展示档 = 方向档（历史 P1-2/V76 的按仓位派生已废除，
                  // 原因见 `AUDIT-300642-run-variance-2026-09-22.md`：循环判据）。
                  // 下方 state/pct 仅为兼容签名，不参与判定。
                  const displayOf = (a: string, state?: string | null, pct?: number | null) =>
                    resolveDisplayAction(a, state ?? undefined, pct == null ? null : pct);
                  let action = r.decisionAction
                    ? displayOf(r.decisionAction, r.decisionPositionState, r.decisionPositionPct)
                    : "";
                  let posPct: number | null = r.decisionPositionPct;
                  let conf: number | null = null;
                  if (r.decisionJson) {
                    try {
                      const d = JSON.parse(r.decisionJson) as Record<string, unknown>;
                      if (!action && d.action) {
                        action = displayOf(
                          d.action as string,
                          typeof d.positionState === "string" ? d.positionState : null,
                          typeof d.positionPct === "number" ? d.positionPct : null,
                        );
                      }
                      if (posPct == null && typeof d.positionPct === "number") { posPct = d.positionPct; }
                      if (typeof d.confidence === "number") { conf = d.confidence; }
                    } catch { /* */ }
                  }
                  const hasDecision = !!action;
                  return (
                    <List.Item
                      style={{ cursor: "pointer", padding: "4px 0 4px 12px" }}
                      onClick={() => {
                        if (selectMode) {
                          setSelectedIds((prev) =>
                            prev.includes(r.id) ? prev.filter((id) => id !== r.id) : [...prev, r.id]
                          );
                        } else {
                          runBacktest(r);
                        }
                      }}
                      actions={[
                        selectMode
                          ? (
                            <Checkbox
                              key="select"
                              checked={selectedIds.includes(r.id)}
                              onClick={(e) => e.stopPropagation()}
                              onChange={() => {
                                setSelectedIds((prev) =>
                                  prev.includes(r.id) ? prev.filter((id) => id !== r.id) : [...prev, r.id]
                                );
                              }}
                            />
                          )
                          : (
                            <>
                              {hasDecision && (
                                <Tag
                                  key="act"
                                  style={getActionTagStyle(action)}
                                >
                                  {t(getActionTKey(action))}
                                </Tag>
                              )}
                              <Button
                                key="bt"
                                size="small"
                                type="link"
                                className="text-xs px-1"
                                loading={btLoading}
                                onClick={(e) => {
                                  e.stopPropagation();
                                  runBacktest(r);
                                }}
                              >
                                {t("stockAnalysis.backtest.run")}
                              </Button>
                            </>
                          ),
                      ]}
                    >
                      <div className="flex flex-col gap-0.5">
                        <div className="flex items-center gap-2 text-xs">
                          <span>{r.analysisDate || new Date(r.createdAt).toLocaleDateString()}</span>
                          {r.parentAnalysisId && (
                            <Tag
                              className="m-0"
                              style={{
                                margin: 0,
                                fontSize: 10,
                                lineHeight: "16px",
                                padding: "0 4px",
                                borderRadius: 3,
                                border: "1px solid var(--color-accent)",
                                color: "var(--color-accent)",
                                background: "color-mix(in oklch, var(--color-accent) 12%, transparent)",
                              }}
                            >
                              ↻
                            </Tag>
                          )}
                          {
                            /* 链路标识（2026-09-24）：快速 JEV 链记录与完整链在此列表中
                              混排且形态一致，需与 ↻ 同款小标签把链路点出来。
                              templateId 为 null（本列引入前 / 非模板产出）时不打。 */
                          }
                          {r.templateId === FAST_TEMPLATE_ID && (
                            <Tag
                              className="m-0"
                              style={{
                                margin: 0,
                                fontSize: 10,
                                lineHeight: "16px",
                                padding: "0 4px",
                                borderRadius: 3,
                                border: "1px solid var(--color-t-tertiary, #888)",
                                color: "var(--color-t-tertiary, #888)",
                                background: "transparent",
                              }}
                            >
                              {t("stockAnalysis.fastAnalysis")}
                            </Tag>
                          )}
                          {
                            /* 主档 + 来源（2026-09-29）：列表里唯一那条 Action 必须说明它属于
                              哪一档、档位由何而来。缺列的历史记录显式标「未知周期」，不回填成某档。 */
                          }
                          <Tag
                            className="m-0"
                            style={{
                              margin: 0,
                              fontSize: 10,
                              lineHeight: "16px",
                              padding: "0 4px",
                              borderRadius: 3,
                              border: "1px solid var(--color-t-tertiary, #888)",
                              color: "var(--color-t-tertiary, #888)",
                              background: "transparent",
                            }}
                          >
                            {r.decisionTimeHorizon && HORIZON_T_SUFFIX[r.decisionTimeHorizon]
                              ? t(`stockAnalysis.timeHorizon${HORIZON_T_SUFFIX[r.decisionTimeHorizon]}`)
                              : t("stockAnalysis.reflection.horizonUnknown")}
                            {(() => {
                              const srcKey = horizonSourceLabelKey(r.decisionHorizonSource);
                              return srcKey ? ` · ${t(srcKey)}` : "";
                            })()}
                          </Tag>
                        </div>
                        {r.createdAt > 0 && (
                          <div className="text-[10px]" style={{ color: "var(--color-t-tertiary, #888)" }}>
                            {t("stockAnalysis.analysisTime")}{" "}
                            {new Date(r.createdAt).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}
                          </div>
                        )}
                        {/* 重要要素：仓位 + 置信度 */}
                        {(() => {
                          const parts: string[] = [];
                          if (posPct != null) {
                            parts.push(`${t("stockAnalysis.decision.positionPct")}${posPct}%`);
                          }
                          if (conf != null) {
                            parts.push(`${t("stockAnalysis.decision.confidence")}${conf.toFixed(0)}%`);
                          }
                          if (parts.length === 0) { return null; }
                          return (
                            <div className="text-[10px]" style={{ color: "var(--color-t-tertiary, #888)" }}>
                              {parts.join(" · ")}
                            </div>
                          );
                        })()}
                        {
                          /* 主档来历（§五十三 ①，v127）：000710 实测「主档=持有 + 标签=超短线分支选档
                             + 超短线 chip=买入」三处同屏互斥，唯一解释藏在 reasoning 的中文句子里。
                             现在成句说明「哪一档选的、分支原结论、被谁改写」。
                             v127 之前的记录没有这两个字段 ⇒ 整行不渲染（不编造来历）。 */
                        }
                        {(() => {
                          const prov = readDecisionProvenance(r.decisionJson);
                          if (!prov) { return null; }
                          const horizonName = prov.horizon && HORIZON_T_SUFFIX[prov.horizon]
                            ? t(`stockAnalysis.timeHorizon${HORIZON_T_SUFFIX[prov.horizon]}`)
                            : "";
                          const srcLabel = actionSourceLabelKey(prov.actionSource);
                          if (!srcLabel) { return null; }
                          const actionText = t(getActionTKey(action));
                          const body = prov.kind === "downgraded"
                            ? t("stockAnalysis.decisionProvenanceDowngraded", {
                              horizon: horizonName,
                              branchAction: t(getActionTKey(prov.branchAction ?? "")),
                              reason: t(srcLabel),
                              action: actionText,
                            })
                            : prov.kind === "direct"
                            ? t("stockAnalysis.decisionProvenanceDirect", {
                              horizon: horizonName,
                              action: actionText,
                            })
                            : t(srcLabel);
                          // 置信口径只在**不是**所选档时点名（那才是读者会误读的那一格）
                          const confSrcKey = prov.confidenceSource === "main_chain_posterior"
                            ? confidenceSourceLabelKey(prov.confidenceSource)
                            : null;
                          return (
                            <div className="text-[10px]" style={{ color: "var(--color-t-tertiary, #888)" }}>
                              {body}
                              {confSrcKey ? ` · ${t(confSrcKey)}` : ""}
                            </div>
                          );
                        })()}
                        {
                          /* 四档 Action（2026-09-29）：一次分析同时产四档决策，历史行原先只显示
                            主档那条 ⇒ 四档明细虽在 decisionJson 里下发却无人渲染。此处逐档并列；
                            旧记录没有该结构时**不渲染**（缺席不伪造）。 */
                        }
                        {(() => {
                          const tiers = readHorizonActions(r.decisionJson);
                          if (tiers.length === 0) { return null; }
                          return (
                            <div className="flex items-center gap-1 flex-wrap">
                              {tiers.map((x) => (
                                <Tag
                                  key={x.key}
                                  className="m-0"
                                  style={{
                                    ...getActionTagStyle(x.action),
                                    margin: 0,
                                    fontSize: 10,
                                    lineHeight: "14px",
                                    padding: "0 3px",
                                  }}
                                >
                                  {t(`stockAnalysis.timeHorizon${HORIZON_T_SUFFIX[x.key]}`)}
                                  {": "}
                                  {t(getActionTKey(x.action))}
                                </Tag>
                              ))}
                            </div>
                          );
                        })()}
                      </div>
                    </List.Item>
                  );
                }}
              />
            </div>
          ))}
      </Card>
    </div>
  );
}
