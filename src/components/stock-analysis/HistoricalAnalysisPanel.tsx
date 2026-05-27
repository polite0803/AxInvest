import { invoke } from "@/lib/invoke";
import { SearchOutlined } from "@ant-design/icons";
import { Card, Collapse, Empty, Input, List, Spin, Tag } from "antd";
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

interface Props {
  analysisId: string;
}

export function HistoricalAnalysisPanel({ analysisId }: Props) {
  const { t } = useTranslation();
  const [records, setRecords] = useState<AnalysisRecord[]>([]);
  const [loading, setLoading] = useState(false);
  const [search, setSearch] = useState("");
  const [snapshot, setSnapshot] = useState<Record<string, string> | null>(null);
  const [backtest, setBacktest] = useState<
    { decision: string; priceThen: number; priceNow: number; correct: boolean } | null
  >(null);

  // 加载历史列表
  useEffect(() => {
    setLoading(true);
    invoke<any[]>("list_stock_analyses", { limit: 30 })
      .then((list) => {
        if (Array.isArray(list)) { setRecords(list); }
      })
      .catch(() => {})
      .finally(() => setLoading(false));
  }, []);

  // 当前分析详情
  useEffect(() => {
    if (!analysisId) { return; }
    invoke<{ blackboardSnapshot: string | null }>("get_stock_analysis", { analysisId })
      .then((r) => {
        if (r.blackboardSnapshot) { setSnapshot(JSON.parse(r.blackboardSnapshot)); }
      })
      .catch(() => {});
  }, [analysisId]);

  // 回测：对比分析决策 vs 最新价格
  const runBacktest = async (record: AnalysisRecord) => {
    if (!record.decisionJson) { return; }
    try {
      const d = JSON.parse(record.decisionJson);
      const quote = await invoke<any>("get_stock_quote", { stockCode: record.stockCode });
      const direction = d.action === "买入" || d.action === "增持" ? "bull" : "bear";
      const priceThen = d.targetPrice ?? 0;
      const priceNow = quote?.price ?? 0;
      const correct = direction === "bull" ? priceNow > priceThen : priceNow < priceThen;
      setBacktest({ decision: d.action, priceThen, priceNow, correct });
    } catch { /* skip */ }
  };

  const filtered = useMemo(() => {
    if (!search) { return records; }
    const q = search.toLowerCase();
    return records.filter((r) =>
      r.stockCode.toLowerCase().includes(q)
      || r.stockName.toLowerCase().includes(q)
      || (r.decisionJson && r.decisionJson.toLowerCase().includes(q))
    );
  }, [records, search]);

  // 当前分析的详情展示
  const reportEntries = Object.entries(snapshot ?? {}).filter(([k]) => k.startsWith("report."));
  const debateEntries = Object.entries(snapshot ?? {}).filter(([k]) => k.startsWith("debate."));

  return (
    <div className="flex flex-col gap-2">
      {/* 当前分析详情 */}
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

      {/* 历史列表 + 搜索 */}
      <Card
        size="small"
        title="历史记录"
        styles={{ body: { padding: "6px 8px" } }}
        extra={
          <Input
            size="small"
            prefix={<SearchOutlined />}
            placeholder="搜索代码/名称"
            style={{ width: 130 }}
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            allowClear
          />
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
                    ]}
                  >
                    <div className="flex items-center gap-2 text-xs">
                      <Tag className="m-0 text-xs">{r.stockCode}</Tag>
                      <span>{r.stockName}</span>
                      {backtest && backtest.priceThen > 0 && (
                        <span className={backtest.correct ? "text-green-500" : "text-red-400"}>
                          {backtest.correct ? "✓ 正确" : "✗ 偏差"}
                        </span>
                      )}
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
