/**
 * 数据源 Tab — Vendor 开关 + 健康检测 + 固定工具依赖融合展示。
 */
import { invoke } from "@/lib/invoke";
import { Badge, Button, Card, message, Select, Space, Spin, Switch, Tag, Tooltip } from "antd";
import { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";

interface VendorDef {
  key: string;
  enName: string;
  name: string;
  desc: string;
  capabilities: string[];
  requiresKey: boolean;
}

const VENDORS: VendorDef[] = [
  { key: "vendor_tencent", enName: "tencent", name: "腾讯财经", desc: "报价/K线", capabilities: ["quote", "klines"], requiresKey: false },
  { key: "vendor_eastmoney", enName: "eastmoney", name: "东方财富", desc: "综合数据", capabilities: ["quote", "klines", "financials", "money_flow", "dragon_tiger", "lockup", "search", "margin", "north_bound", "sector", "shareholder_trades", "dividend", "research_reports", "market_dragon_tiger", "cls_flash"], requiresKey: false },
  { key: "vendor_sina", enName: "sina", name: "新浪财经", desc: "报价/新闻", capabilities: ["quote", "news"], requiresKey: false },
  { key: "vendor_ths", enName: "ths", name: "同花顺", desc: "综合数据", capabilities: ["consensus_eps", "concept_blocks", "hot_stocks", "industry_ranking", "north_bound_flow"], requiresKey: false },
  { key: "vendor_cninfo", enName: "cninfo", name: "巨潮资讯", desc: "信息披露", capabilities: ["announcements"], requiresKey: false },
  { key: "vendor_baidu_stock", enName: "baidu_stock", name: "百度股市通", desc: "全维度", capabilities: ["quote", "klines", "financials", "news", "money_flow", "dragon_tiger", "lockup", "search", "margin", "north_bound", "sector", "shareholder_trades", "dividend", "research_reports", "concept_blocks", "hot_stocks", "industry_ranking", "north_bound_flow"], requiresKey: false },
  { key: "vendor_iwencai", enName: "iwencai", name: "问财", desc: "选股/研报", capabilities: ["search", "sector", "consensus_eps", "concept_blocks", "hot_stocks"], requiresKey: true },
  { key: "vendor_akshare", enName: "akshare", name: "AKShare", desc: "开源数据", capabilities: ["financials", "news", "consensus_eps", "cls_flash"], requiresKey: false },
  { key: "vendor_mootdx", enName: "mootdx", name: "Mootdx", desc: "通达信本地", capabilities: ["quote", "klines"], requiresKey: false },
];

/** 固定 ToolNode 定义：tool_name → 路由链 (vendors) + 显示标签 */
const FIXED_TOOLS = [
  { tool: "get_stock_kline", label: "K线", vendors: ["eastmoney", "tencent", "mootdx"] },
  { tool: "get_hot_stocks", label: "热门股", vendors: ["ths", "baidu_stock", "iwencai"] },
  { tool: "get_announcements", label: "公告", vendors: ["cninfo", "eastmoney"] },
  { tool: "get_consensus_eps", label: "一致预期", vendors: ["ths", "akshare", "iwencai"] },
  { tool: "get_stock_money_flow", label: "资金流向", vendors: ["eastmoney", "baidu_stock"] },
  { tool: "get_industry_ranking", label: "行业排名", vendors: ["ths", "baidu_stock"] },
];

/** 本地计算工具（不依赖远程 vendor）*/
const LOCAL_TOOLS = [
  { tool: "compute_scoring", label: "技术评分" },
  { tool: "compute_valuation", label: "估值计算" },
  { tool: "compute_portfolio_risk", label: "组合风险" },
];

const CAP_LABELS: Record<string, string> = {
  quote: "行情", klines: "K线", financials: "财务", news: "新闻",
  money_flow: "资金流向", dragon_tiger: "龙虎榜", lockup: "解禁", search: "搜索",
  margin: "融资融券", north_bound: "北向持仓", sector: "行业板块",
  shareholder_trades: "增减持", dividend: "分红", research_reports: "研报",
  consensus_eps: "一致预期", concept_blocks: "概念板块", announcements: "公告",
  market_dragon_tiger: "大盘龙虎", hot_stocks: "热门股", industry_ranking: "行业排名",
  cls_flash: "快讯", north_bound_flow: "北向资金",
};

type HealthStatus = "ok" | "fail" | "pending" | "idle";

export function DataVendorsTab() {
  const { t } = useTranslation();
  const [vendorValues, setVendorValues] = useState<Record<string, boolean>>({});
  const [iwencaiKey, setIwencaiKey] = useState("");
  const [health, setHealth] = useState<Record<string, HealthStatus>>({});
  const [checkingAll, setCheckingAll] = useState(false);
  const [saving, setSaving] = useState(false);
  const [loaded, setLoaded] = useState(false);

  // vendor 英文名 → 依赖的固定工具列表
  const vendorTools = useMemo(() => {
    const map: Record<string, typeof FIXED_TOOLS> = {};
    for (const ft of FIXED_TOOLS) {
      for (const vn of ft.vendors) {
        (map[vn] ??= []).push(ft);
      }
    }
    return map;
  }, []);

  const load = useCallback(async () => {
    try {
      const tmpl: any = await invoke("get_workflow_template", { id: "stock-analysis" });
      const vars: { name: string; value: any }[] = tmpl?.variables ?? [];
      const vals: Record<string, boolean> = {};
      let key = "";
      for (const v of vars) {
        if (v.name.startsWith("vendor_") && v.name !== "vendor_iwencai_key") {
          vals[v.name] = !!v.value;
        }
        if (v.name === "vendor_iwencai_key") key = typeof v.value === "string" ? v.value : "";
      }
      setVendorValues(vals);
      setIwencaiKey(key);
      setLoaded(true);
    } catch { setLoaded(true); }
  }, []);

  useEffect(() => { load(); }, [load]);

  const handleSave = useCallback(async () => {
    setSaving(true);
    try {
      await invoke("update_workflow_template", {
        id: "stock-analysis",
        input: {
          variables: [
            ...Object.entries(vendorValues).map(([k, v]) => ({ name: k, value: v })),
            { name: "vendor_iwencai_key", value: iwencaiKey },
          ],
        },
      });
      message.success(t("stockAnalysis.settings.saveSuccess"));
    } catch { message.error(t("stockAnalysis.settings.saveFailed")); } finally { setSaving(false); }
  }, [vendorValues, iwencaiKey, t]);

  const checkOne = useCallback(async (vendorName: string) => {
    setHealth((prev) => ({ ...prev, [vendorName]: "pending" }));
    try {
      await invoke("check_vendor_health", { vendor: vendorName });
      setHealth((prev) => ({ ...prev, [vendorName]: "ok" }));
    } catch { setHealth((prev) => ({ ...prev, [vendorName]: "fail" })); }
  }, []);

  const checkAll = useCallback(async () => {
    setCheckingAll(true);
    const names = ["tencent", "eastmoney", "sina", "ths", "cninfo", "baidu_stock", "iwencai", "akshare", "mootdx"];
    for (const n of names) { setHealth((prev) => ({ ...prev, [n]: "pending" })); }
    for (const n of names) { await checkOne(n); }
    setCheckingAll(false);
  }, [checkOne]);

  if (!loaded) return <div className="flex justify-center py-12"><Spin /></div>;

  return (
    <div className="flex flex-col gap-3">
      {/* 顶部操作栏 */}
      <div className="flex items-center justify-between">
        <Space size={4}>
          <span className="text-sm text-gray-500">9 个数据源 + 6 个数据工具 + 3 个本地计算</span>
        </Space>
        <Space>
          <Button size="small" loading={checkingAll} onClick={checkAll}>
            {t("stockAnalysis.settings.checkHealth")}
          </Button>
          <Button size="small" type="primary" loading={saving} onClick={handleSave}>
            {t("stockAnalysis.settings.saveConfig")}
          </Button>
        </Space>
      </div>

      {/* Vendor 卡片（含依赖的固定工具） */}
      {VENDORS.map((v) => {
        const enabled = vendorValues[v.key] ?? false;
        const status = health[v.enName] ?? "idle";
        const deps = vendorTools[v.enName] ?? [];
        // 计算依赖工具的降级链中本 vendor 的位置
        const depInfo = deps.map((ft) => {
          const idx = ft.vendors.indexOf(v.enName);
          return { ...ft, isPrimary: idx === 0 };
        });

        return (
          <Card
            key={v.key}
            size="small"
            className={enabled ? "" : "opacity-60"}
            title={
              <Space size={8}>
                <Switch
                  size="small"
                  checked={enabled}
                  onChange={(checked) => setVendorValues((prev) => ({ ...prev, [v.key]: checked }))}
                />
                <span className="font-medium text-sm">{v.name}</span>
                <span className="text-xs text-gray-400">{v.desc}</span>
                <Tooltip title={
                  status === "ok" ? t("stockAnalysis.settings.connected")
                    : status === "fail" ? t("stockAnalysis.settings.disconnected")
                    : status === "pending" ? "检测中..." : "未检测"
                }>
                  <Badge status={status === "ok" ? "success" : status === "fail" ? "error" : status === "pending" ? "processing" : "default"} />
                </Tooltip>
              </Space>
            }
            extra={
              <Space size={4}>
                {v.key === "vendor_iwencai" && (
                  <Select
                    style={{ width: 180 }}
                    size="small"
                    mode="tags"
                    maxCount={1}
                    placeholder="API Key"
                    value={iwencaiKey ? [iwencaiKey] : []}
                    onChange={(vals) => setIwencaiKey(vals[0] ?? "")}
                  />
                )}
                <Button size="small" onClick={() => checkOne(v.enName)}>
                  {t("stockAnalysis.settings.check")}
                </Button>
              </Space>
            }
          >
            {/* 能力标签 */}
            <div className="mb-2">
              <Space wrap size={[2, 4]}>
                {v.capabilities.map((cap) => (
                  <Tag key={cap} color="blue" className="text-xs m-0">{CAP_LABELS[cap] ?? cap}</Tag>
                ))}
              </Space>
            </div>
            {/* 依赖的固定工具 + 降级链 */}
            {depInfo.length > 0 && (
              <div className="border-t border-gray-100 pt-2 mt-1">
                <div className="text-xs text-gray-400 mb-1">
                  {enabled ? "为以下固定工具提供数据：" : "关闭后以下工具将降级到下一 vendor："}
                </div>
                {depInfo.map((ft) => (
                  <div key={ft.tool} className="flex items-center gap-2 text-xs py-0.5">
                    <Tag color="default" className="text-xs m-0">⚙️ {ft.tool}</Tag>
                    <span className="text-gray-400">{ft.label}</span>
                    <span className="text-gray-300">
                      {ft.vendors.map((vn, i) => (
                        <span key={vn}>
                          {i > 0 ? " → " : ""}
                          <span className={vn === v.enName ? "font-medium text-gray-600" : health[vn] === "ok" ? "text-green-600" : "text-gray-400"}>
                            {vn}
                          </span>
                        </span>
                      ))}
                    </span>
                  </div>
                ))}
              </div>
            )}
            {depInfo.length === 0 && (
              <div className="text-xs text-gray-400">无固定工具依赖此数据源</div>
            )}
          </Card>
        );
      })}

      {/* 本地计算工具 */}
      <div className="pt-2">
        <div className="text-xs text-gray-400 mb-2">本地计算工具（不依赖远程 vendor）</div>
        <Space wrap size={[4, 8]}>
          {LOCAL_TOOLS.map((lt) => (
            <Tag key={lt.tool} color="purple" className="text-xs m-0">💻 {lt.tool}</Tag>
          ))}
        </Space>
      </div>
    </div>
  );
}
