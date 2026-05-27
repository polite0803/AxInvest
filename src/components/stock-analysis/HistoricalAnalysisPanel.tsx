import { invoke } from "@/lib/invoke";
import { SearchOutlined } from "@ant-design/icons";
import { Button, Card, Collapse, Empty, Input, List, message, Spin, Statistic, Tag } from "antd";
import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";

interface AnalysisRecord {
  id: string;
  stockCode: string;
  stockName: string;
  decisionJson: string | null;
  blackboardSnapshot: string | null;
  createdAt: number;
  status: string;
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
  maxDrawdown: number;
}

interface Props {
  analysisId: string;
}

export function HistoricalAnalysisPanel({ analysisId }: Props) {
  const { t } = useTranslation();
  const [records, setRecords] = useState<AnalysisRecord[]>([]);
  const [loading, setLoading] = useState(false);
  const [search, setSearch] = useState("");
  const [snapshot, setSnapshot] = useState<Record<string, string> | null>(null);
  const [btResult, setBtResult] = useState<BacktestResult | null>(null);
  const [btAllResults, setBtAllResults] = useState<BacktestResult[] | null>(null);
  const [btLoading, setBtLoading] = useState(false);

  useEffect(() => {
    setLoading(true);
    invoke<any[]>("list_stock_analyses", { limit: 30 })
      .then((list) => {
        if (Array.isArray(list)) { setRecords(list); }
      })
      .catch(() => {})
      .finally(() => setLoading(false));
  }, []);

  useEffect(() => {
    if (!analysisId) { return; }
    invoke<{ blackboardSnapshot: string | null }>("get_stock_analysis", { analysisId })
      .then((r) => {
        if (r.blackboardSnapshot) { setSnapshot(JSON.parse(r.blackboardSnapshot)); }
      })
      .catch(() => {});
  }, [analysisId]);

  const runBacktest = async (record: AnalysisRecord) => {
    setBtLoading(true);
    try {
      const r = await invoke<BacktestResult>("backtest_analysis", { analysisId: record.id });
      setBtResult(r);
    } catch {
      message.error("回测失败");
    }
    setBtLoading(false);
  };

  const runBacktestAll = async () => {
    setBtLoading(true);
    try {
      const r = await invoke<BacktestResult[]>("backtest_all_history");
      if (Array.isArray(r)) { setBtAllResults(r); }
    } catch {
      message.error("全量回测失败");
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

  const reportEntries = Object.entries(snapshot ?? {}).filter(([k]) => k.startsWith("report."));
  const debateEntries = Object.entries(snapshot ?? {}).filter(([k]) => k.startsWith("debate."));

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

      {/* 全量回测汇总 */}
      {btStats && (
        <Card size="small" title="回测汇总" styles={{ body: { padding: "4px 8px" } }}>
          <div className="grid grid-cols-3 gap-1 text-center">
            <Statistic title="总数" value={btStats.total} valueStyle={{ fontSize: 14 }} />
            <Statistic
              title="正确率"
              value={btStats.correct}
              suffix={`/${btStats.total}`}
              valueStyle={{ fontSize: 14, color: "var(--sa-green)" }}
            />
            <Statistic title="平均收益" value={btStats.avgReturn} suffix="%" valueStyle={{ fontSize: 14 }} />
          </div>
        </Card>
      )}

      {/* 单次回测结果 */}
      {btResult && (
        <Card
          size="small"
          title={`回测: ${btResult.stockCode} ${btResult.decisionAction}`}
          styles={{ body: { padding: "4px 8px" } }}
        >
          <div className="grid grid-cols-3 gap-1 text-center text-xs">
            <div>
              <span className="text-gray-400">持有天数</span>
              <br />
              <b>{btResult.holdingDays}</b>
            </div>
            <div>
              <span className="text-gray-400">收益率</span>
              <br />
              <b className={btResult.returnPct >= 0 ? "text-red-500" : "text-green-500"}>
                {btResult.returnPct >= 0 ? "+" : ""}
                {btResult.returnPct.toFixed(2)}%
              </b>
            </div>
            <div>
              <span className="text-gray-400">最大回撤</span>
              <br />
              <b>{btResult.maxDrawdown.toFixed(2)}%</b>
            </div>
            <div className="col-span-3 mt-1">
              <Tag color={btResult.wasCorrect ? "green" : "red"}>{btResult.wasCorrect ? "✓ 正确" : "✗ 错误"}</Tag>
            </div>
          </div>
        </Card>
      )}

      {/* 历史列表 */}
      <Card
        size="small"
        title="历史记录"
        styles={{ body: { padding: "6px 8px" } }}
        extra={
          <div className="flex gap-1">
            <Input
              size="small"
              prefix={<SearchOutlined />}
              placeholder="搜索"
              style={{ width: 100 }}
              value={search}
              onChange={(e) => setSearch(e.target.value)}
              allowClear
            />
            <Button size="small" loading={btLoading} onClick={runBacktestAll}>回测全部</Button>
          </div>
        }
      >
        {loading
          ? <Spin size="small" />
          : filtered.length === 0
          ? <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description="暂无记录" />
          : (
            <List
              size="small"
              dataSource={filtered}
              renderItem={(r) => {
                let decision: any = null;
                try {
                  if (r.decisionJson) { decision = JSON.parse(r.decisionJson); }
                } catch { /* */ }
                return (
                  <List.Item
                    style={{ cursor: "pointer", padding: "4px 0" }}
                    onClick={() => runBacktest(r)}
                    actions={[
                      decision && (
                        <Tag
                          key="act"
                          color={decision.action === "买入" ? "green" : decision.action === "卖出" ? "red" : "blue"}
                          className="text-xs m-0"
                        >
                          {decision.action}
                        </Tag>
                      ),
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
                        回测
                      </Button>,
                    ]}
                  >
                    <div className="flex items-center gap-2 text-xs">
                      <Tag className="m-0 text-xs">{r.stockCode}</Tag>
                      <span>{r.stockName}</span>
                    </div>
                  </List.Item>
                );
              }}
            />
          )}
      </Card>
    </div>
  );
}
