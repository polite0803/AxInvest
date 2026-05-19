// Post-merge script: re-add AxInvest stockAnalysis i18n keys + merge allowlist
import { readdirSync, readFileSync, writeFileSync } from "fs";
import { join } from "path";

const dir = "src/i18n/locales";

const zhCN = {
  actionBuy: "买入",
  actionSell: "卖出",
  actionIncrease: "增持",
  actionHold: "持有",
  actionReduce: "减持",
  recentAnalysis: "最近分析",
  targetPriceNote: "目标¥{{price}}",
  totalMarketValue: "总市值",
  unrealizedPnl: "浮动盈亏",
  concentration: "集中度",
  holdings: "持仓",
  sharesUnit: "只",
  wanUnit: "万",
  analystCount: "{{current}}/{{total}} 分析师",
  debateRounds: "{{current}}/{{total}} 轮辩论",
  riskAssessCount: "{{count}} 项评估",
  offlineMode: "离线模式 (LLM Driver 未连接，占位数据)",
  charCount: "{{count}} 字",
  "settings.saveFailed": "设置保存失败",
  "settings.quoteTag": "行情",
  "settings.financialKlineTag": "财务/K线",
  "settings.newsTag": "新闻",
  "settings.tencentFinance": "腾讯财经",
  "settings.eastmoney": "东方财富",
  "settings.sinaFinance": "新浪财经",
};

const langMap = {
  "zh-CN.json": zhCN,
  "zh-TW.json": {
    actionBuy: "買入",
    actionSell: "賣出",
    actionIncrease: "增持",
    actionHold: "持有",
    actionReduce: "減持",
    recentAnalysis: "最近分析",
    targetPriceNote: "目標¥{{price}}",
    totalMarketValue: "總市值",
    unrealizedPnl: "浮動盈虧",
    concentration: "集中度",
    holdings: "持倉",
    sharesUnit: "檔",
    wanUnit: "萬",
    analystCount: "{{current}}/{{total}} 分析師",
    debateRounds: "{{current}}/{{total}} 輪辯論",
    riskAssessCount: "{{count}} 項評估",
    offlineMode: "離線模式",
    charCount: "{{count}} 字",
    "settings.saveFailed": "設定儲存失敗",
    "settings.quoteTag": "行情",
    "settings.financialKlineTag": "財務/K線",
    "settings.newsTag": "新聞",
    "settings.tencentFinance": "騰訊財經",
    "settings.eastmoney": "東方財富",
    "settings.sinaFinance": "新浪財經",
  },
  "en-US.json": {
    actionBuy: "Buy",
    actionSell: "Sell",
    actionIncrease: "Increase",
    actionHold: "Hold",
    actionReduce: "Reduce",
    recentAnalysis: "Recent Analysis",
    targetPriceNote: "Target ¥{{price}}",
    totalMarketValue: "Total Mkt Value",
    unrealizedPnl: "Unrealized P&L",
    concentration: "Concentration",
    holdings: "Holdings",
    sharesUnit: "",
    wanUnit: "0k",
    analystCount: "{{current}}/{{total}} analysts",
    debateRounds: "{{current}}/{{total}} rounds",
    riskAssessCount: "{{count}} assessments",
    offlineMode: "Offline mode",
    charCount: "{{count}} chars",
    "settings.saveFailed": "Failed to save",
    "settings.quoteTag": "Quotes",
    "settings.financialKlineTag": "Financial/K-line",
    "settings.newsTag": "News",
    "settings.tencentFinance": "Tencent Finance",
    "settings.eastmoney": "East Money",
    "settings.sinaFinance": "Sina Finance",
  },
};

// Generate other languages from en-US (fallback)
const fallback = langMap["en-US.json"];
const otherLangs = ["ja.json", "ko.json", "ar.json", "de.json", "es.json", "fr.json", "hi.json", "ru.json"];
otherLangs.forEach(f => {
  langMap[f] = { ...fallback };
});

// Add stockAnalysis keys
for (const [file, keys] of Object.entries(langMap)) {
  const filepath = join(dir, file);
  if (!readdirSync(dir).includes(file)) { continue; }
  const json = JSON.parse(readFileSync(filepath, "utf8"));
  json.stockAnalysis = keys;
  writeFileSync(filepath, JSON.stringify(json, null, 2) + "\n");
}
console.log("11 locales updated");

// Merge allowlist: upstream base + AxInvest entries
const al = JSON.parse(readFileSync("scripts/.i18n-allowlist.json", "utf8"));
const existing = {};
al.entries.forEach(e => {
  if (!existing[e.file]) { existing[e.file] = new Set(); }
  (e.lines || "").split(",").forEach(l => {
    if (l.trim()) { existing[e.file].add(l.trim()); }
  });
});

const axInvest = [
  ["src/components/stock-analysis/DecisionBanner.tsx", ["8", "9", "10", "11", "12", "27", "28", "29", "30", "31"]],
  ["src/components/stock-analysis/TradePanel.tsx", ["9", "10"]],
  ["src/types/stock-analysis.ts", ["82", "83", "84", "85", "86", "87", "88", "89", "90", "91", "92", "93", "94", "95"]],
  ["src/stores/feature/stockAnalysisStore.ts", ["295"]],
  ["src/components/stock-analysis/__tests__/DecisionBanner.test.tsx", ["37", "39", "40", "47"]],
  ["src/components/stock-analysis/__tests__/TradePanel.test.tsx", ["40", "49", "51"]],
  ["src/stores/__tests__/stockAnalysisStore.test.ts", [
    "46",
    "83",
    "102",
    "104",
    "161",
    "166",
    "179",
    "183",
    "184",
    "196",
  ]],
  ["src/components/stock-analysis/AnalystReportCard.tsx", ["77", "79"]],
  ["src/components/stock-analysis/RiskMatrix.tsx", ["5", "14"]],
  ["src/components/chat/InputArea.tsx", ["167"]],
  ["src/pages/SettingsPage.tsx", ["81"]],
  ["src/components/stock-analysis/KLineChart.tsx", ["10"]],
  ["src/components/settings/SettingsSidebar.tsx", ["78"]],
  ["src/components/settings/StockAnalysisSettings.tsx", ["79"]],
];

for (const [file, lines] of axInvest) {
  if (!existing[file]) { existing[file] = new Set(); }
  lines.forEach(l => existing[file].add(l));
}

al.entries = [];
for (const [file, linesSet] of Object.entries(existing)) {
  al.entries.push({
    file,
    lines: Array.from(linesSet).sort((a, b) => parseInt(a) - parseInt(b)).join(","),
    reason: "硬编码中文字符串",
    phase: 3,
  });
}
al.total_entries = al.entries.length;
al.total_files = al.entries.length;

writeFileSync("scripts/.i18n-allowlist.json", JSON.stringify(al, null, 2) + "\n");
console.log("Allowlist:", al.entries.length, "entries");
