import { List } from "@/components/common/AntdList";
import { invoke } from "@/lib/invoke";
import { FAST_TEMPLATE_ID } from "@/lib/stock-analysis-utils";
import { useStockAnalysisStore } from "@/stores";
import { App, Button, Input, Segmented, Tag, Tooltip } from "antd";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

interface ParsedIntent {
  rawInput: string;
  stockQuery: string | null;
  stockCode: string | null;
  timeHorizon: string | null;
  actionType: string;
  success: boolean;
  description: string;
}

const HORIZON_LABELS: Record<string, string> = {
  ultra_short: "ultraShort",
  short: "short",
  mid: "mid",
  long: "long",
};

/**
 * 从搜索框文本解析出股票代码。
 *
 * 存在的理由（2026-09-22 实测缺陷，DB 实证）：输入框文本（searchKeyword）与待分析标的
 * （store.stockCode）是**两套独立状态**，二者唯一的同步点是「点下拉项」和「回车且 NL 解析成功」。
 * 用户输入代码后直接点「开始分析」时，按钮用的是**上一次**的 stockCode
 * ⇒ 输入 688315 却分析了上一次的 300642（实证：stock_analyses 里 688315 零记录，
 * 而当天该次点击后多出一条 300642）。故点击必须以输入框为准。
 *
 * 解析顺序：括号内代码（预填格式「名称 (代码)」）→ 纯 6 位代码 → 后端模糊搜索（优先同名）。
 */
async function resolveCodeFromKeyword(keyword: string): Promise<string | null> {
  const kw = keyword.trim();
  if (!kw) { return null; }
  const inParen = /[（(]\s*(\d{6})\s*[)）]/.exec(kw);
  if (inParen?.[1]) { return inParen[1]; }
  if (/^\d{6}$/.test(kw)) { return kw; }
  try {
    const hits = await invoke<Array<{ code: string; name: string }>>("search_stock", { keyword: kw });
    if (!Array.isArray(hits) || hits.length === 0) { return null; }
    const exact = hits.find((h) => h.name === kw);
    return (exact ?? hits[0])?.code ?? null;
  } catch {
    return null;
  }
}

export function StockSearchBar() {
  const { t } = useTranslation();
  const searchKeyword = useStockAnalysisStore((s) => s.searchKeyword);
  const searchResults = useStockAnalysisStore((s) => s.searchResults);
  const searchStock = useStockAnalysisStore((s) => s.searchStock);
  const startAnalysis = useStockAnalysisStore((s) => s.startAnalysis);
  const getStockQuote = useStockAnalysisStore((s) => s.getStockQuote);
  const getStockKline = useStockAnalysisStore((s) => s.getStockKline);
  const status = useStockAnalysisStore((s) => s.status);
  const stockCode = useStockAnalysisStore((s) => s.stockCode);
  const stockName = useStockAnalysisStore((s) => s.stockName);
  const reportLanguage = useStockAnalysisStore((s) => s.reportLanguage);
  const setReportLanguage = useStockAnalysisStore((s) => s.setReportLanguage);
  const [intent, setIntent] = useState<ParsedIntent | null>(null);
  const { message } = App.useApp();

  const isRunning = status === "loading" || status === "running";

  /**
   * 启动分析 —— **以输入框文本为准**。
   *
   * 修复：此前直接 `startAnalysis(stockCode)`，而 stockCode 是「上一次选中的标的」，
   * 与输入框内容可以完全无关 ⇒ 输入 688315 却分析了上一次的 300642。
   * 现在：输入框与当前标的不一致时先解析输入框；解析不出就明确提示并**拒绝启动**，
   * 绝不退回「静默分析上一只股票」。
   *
   * `templateId` 缺省走完整分析链；传 `stock-analysis-fast` 走 Jev 判定快速链。
   * 两条链的落库与后续读取路径完全一致，仅图结构不同。
   */
  const handleStartAnalysis = useCallback(async (templateId?: string) => {
    const kw = searchKeyword.trim();
    const currentDisplay = stockName ? `${stockName} (${stockCode})` : stockCode;
    let code = stockCode;

    if (kw && kw !== stockCode && kw !== currentDisplay) {
      const resolved = await resolveCodeFromKeyword(kw);
      if (!resolved) {
        message.warning(t("stockAnalysis.searchUnrecognized", { keyword: kw }));
        return;
      }
      code = resolved;
      if (resolved !== stockCode) {
        getStockQuote(resolved);
        getStockKline(resolved, "daily", 120);
      }
    }

    if (code) {
      void startAnalysis(code, templateId ? { templateId } : undefined);
    }
  }, [searchKeyword, stockCode, stockName, message, t, getStockQuote, getStockKline, startAnalysis]);

  // Ctrl+K / Cmd+K 聚焦到搜索框
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.key === "k") {
        e.preventDefault();
        const el = document.querySelector<HTMLInputElement>(".stock-search-input input");
        el?.focus();
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, []);

  // P1: 自然语言意图解析 — 借鉴 TradingAgents intent parser
  // 当用户输入"调研茅台短线""分析宁德时代中线"时自动识别
  const handleSearch = useCallback(async (value: string) => {
    setIntent(null);

    // 先尝试 NL 意图解析
    try {
      const parsed = await invoke<ParsedIntent>("parse_analysis_intent", { input: value });
      if (parsed.success && parsed.stockCode) {
        setIntent(parsed);
        // 直接填充分析参数
        useStockAnalysisStore.setState({
          stockCode: parsed.stockCode,
          stockName: parsed.stockQuery || parsed.stockCode,
          searchKeyword: value,
        });
        // 如果有时间周期，传递到 store
        if (parsed.timeHorizon) {
          useStockAnalysisStore.setState({ selectedHorizon: parsed.timeHorizon } as never);
        }
        // 如果查询词就是股票代码/名称，直接获取行情
        getStockQuote(parsed.stockCode);
        getStockKline(parsed.stockCode, "daily", 120);
        return;
      }
    } catch {
      // NL 解析失败，回退到普通搜索
    }

    searchStock(value, true);
  }, [searchStock, getStockQuote, getStockKline]);

  return (
    <div className="flex flex-col gap-2">
      <div className="flex gap-2 items-center">
        <Input.Search
          className="stock-search-input"
          data-testid="stock-analysis-search-input"
          placeholder={`${t("stockAnalysis.searchPlaceholder")} (Ctrl+K)`}
          value={searchKeyword}
          onChange={(e) => {
            useStockAnalysisStore.setState({ searchKeyword: e.target.value });
            if (e.target.value.length >= 2) {
              searchStock(e.target.value);
            }
          }}
          onSearch={handleSearch}
          style={{ maxWidth: 360 }}
          loading={status === "loading"}
        />
        <Button
          type="primary"
          // 输入框有内容即可点击（解析失败会明确提示），不再要求 store.stockCode 非空 ——
          // 否则「输入了代码但没选下拉项」时按钮直接变灰，用户会以为界面坏了
          disabled={(!stockCode && !searchKeyword.trim()) || isRunning}
          loading={isRunning}
          onClick={() => {
            void handleStartAnalysis();
          }}
        >
          {isRunning ? t("stockAnalysis.analyzing") : t("stockAnalysis.startAnalysis")}
        </Button>
        <Tooltip title={t("stockAnalysis.fastAnalysisHint")}>
          {
            /* 快速分析：走 `stock-analysis-fast` 模板（Jev 判定链），不产出叙述文本，
              只给结构化决策，用于快速看结论。落库与完整链同一路径。 */
          }
          <Button
            data-testid="stock-analysis-fast-button"
            disabled={(!stockCode && !searchKeyword.trim()) || isRunning}
            loading={isRunning}
            onClick={() => {
              void handleStartAnalysis(FAST_TEMPLATE_ID);
            }}
          >
            {t("stockAnalysis.fastAnalysis")}
          </Button>
        </Tooltip>
        <Tooltip title={t("stockAnalysis.reportLanguageHint")}>
          <Segmented
            size="small"
            value={reportLanguage}
            onChange={(val) => setReportLanguage(val as "zh" | "en")}
            options={[
              { label: t("stockAnalysis.lang.zh"), value: "zh" },
              { label: "EN", value: "en" },
            ]}
          />
        </Tooltip>
      </div>

      {/* P1: 意图解析结果展示 */}
      {intent && intent.success && (
        <div className="flex gap-1 items-center text-xs" style={{ color: "var(--color-text-secondary)" }}>
          <span>{t("stockAnalysis.parsedAs")}:</span>
          <Tag color="blue" className="text-xs">{intent.stockQuery || intent.stockCode}</Tag>
          {intent.timeHorizon && (
            <Tag color="green" className="text-xs">
              {t(`stockAnalysis.period.${HORIZON_LABELS[intent.timeHorizon]}`) || intent.timeHorizon}
            </Tag>
          )}
          <span className="text-xs">{intent.description}</span>
        </div>
      )}

      {searchResults.length > 0 && (
        <List
          size="small"
          bordered
          dataSource={searchResults}
          style={{ maxWidth: 300 }}
          renderItem={(item) => (
            <List.Item
              style={{ cursor: "pointer" }}
              onClick={() => {
                getStockQuote(item.code);
                getStockKline(item.code, "daily", 120);
                // 输入框回显为「名称 (代码)」，让「输入框显示 = 实际待分析标的」这个不变量可见 ——
                // 否则输入框停在用户键入的原始文本上，界面无从判断按钮会跑哪只股票
                useStockAnalysisStore.setState({
                  searchResults: [],
                  searchKeyword: `${item.name} (${item.code})`,
                });
              }}
            >
              {item.code} — {item.name}
            </List.Item>
          )}
        />
      )}
    </div>
  );
}
