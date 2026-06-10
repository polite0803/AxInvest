// Restore full stockAnalysis i18n keys from git history to all 11 locales
import { execSync } from "child_process";
import { readdirSync, readFileSync, unlinkSync, writeFileSync } from "fs";
import { join } from "path";

// Extract from git
execSync("git show f2ed4103:src/i18n/locales/zh-CN.json > scripts/.orig-zh.json");
execSync("git show f2ed4103:src/i18n/locales/en-US.json > scripts/.orig-en.json");

const origCN = JSON.parse(readFileSync("scripts/.orig-zh.json", "utf8")).stockAnalysis;
const origEN = JSON.parse(readFileSync("scripts/.orig-en.json", "utf8")).stockAnalysis;

// zh-TW: simple substitution map
const twSubs = {
  "分析": "分析",
  "风险": "風險",
  "辩论": "辯論",
  "评估": "評估",
  "决策": "決策",
  "行情": "行情",
  "新闻": "新聞",
  "持仓": "持倉",
  "市值": "市值",
  "盈亏": "盈虧",
  "数据": "資料",
  "加载": "載入",
  "完成": "完成",
  "价格": "價格",
  "告警": "告警",
  "对比": "對比",
  "开始": "開始",
  "标题": "標題",
  "代码": "代碼",
  "名称": "名稱",
  "条件": "條件",
};
function toTW(text) {
  if (typeof text !== "string") { return text; }
  let result = text;
  // Only substitute if the simplified char exists
  for (const [sc, tc] of Object.entries(twSubs)) {
    result = result.split(sc).join(tc);
  }
  return result;
}

const origTW = {};
for (const [k, v] of Object.entries(origCN)) {
  origTW[k] = toTW(v);
}

// Update all locales
const dir = "src/i18n/locales";
const files = readdirSync(dir).filter((f) => f.endsWith(".json"));

for (const f of files) {
  const fp = join(dir, f);
  const j = JSON.parse(readFileSync(fp, "utf8"));
  if (!j.stockAnalysis) { j.stockAnalysis = {}; }

  let source;
  if (f === "zh-CN.json") { source = origCN; }
  else if (f === "en-US.json") { source = origEN; }
  else if (f === "zh-TW.json") { source = origTW; }
  else { source = origEN; // fallback to English for all other languages
   }

  let added = 0;
  for (const [k, v] of Object.entries(source)) {
    if (!(k in j.stockAnalysis)) {
      j.stockAnalysis[k] = v;
      added++;
    }
  }

  writeFileSync(fp, JSON.stringify(j, null, 2) + "\n");
  console.log(f + ": added " + added + " keys (now " + Object.keys(j.stockAnalysis).length + " total)");
}

unlinkSync("scripts/.orig-zh.json");
unlinkSync("scripts/.orig-en.json");
console.log("Done");
