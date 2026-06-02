/**
 * 在全部 11 种语言文件中插入 9 个新的 stockAnalysis 翻译 key
 * 插入位置：stockAnalysis.workflow.bearCase 行之后
 */
import { readFileSync, writeFileSync } from "fs";
import { dirname, join } from "path";
import { fileURLToPath } from "url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const localesDir = join(__dirname, "..", "src", "i18n", "locales");

/** 9 个新 key（所有语言通用） */
const NEW_KEYS = [
  "stockAnalysis.workflow.bullAnalyst",
  "stockAnalysis.workflow.bearAnalyst",
  "stockAnalysis.workflow.riskAggressive",
  "stockAnalysis.workflow.riskConservative",
  "stockAnalysis.workflow.riskNeutral",
  "stockAnalysis.workflow.riskAggregation",
  "stockAnalysis.workflow.riskClassification",
  "stockAnalysis.workflow.notification",
  "stockAnalysis.workflow.fetchFailed",
];

/** 各语言的翻译值 */
const TRANSLATIONS = {
  "zh-CN": {
    "stockAnalysis.workflow.bullAnalyst": "多方研究员",
    "stockAnalysis.workflow.bearAnalyst": "空方研究员",
    "stockAnalysis.workflow.riskAggressive": "激进评估",
    "stockAnalysis.workflow.riskConservative": "保守评估",
    "stockAnalysis.workflow.riskNeutral": "中性评估",
    "stockAnalysis.workflow.riskAggregation": "风险聚合",
    "stockAnalysis.workflow.riskClassification": "风险分级",
    "stockAnalysis.workflow.notification": "结果通知",
    "stockAnalysis.workflow.fetchFailed": "数据获取失败",
  },
  "zh-TW": {
    "stockAnalysis.workflow.bullAnalyst": "多方研究員",
    "stockAnalysis.workflow.bearAnalyst": "空方研究員",
    "stockAnalysis.workflow.riskAggressive": "激進評估",
    "stockAnalysis.workflow.riskConservative": "保守評估",
    "stockAnalysis.workflow.riskNeutral": "中性評估",
    "stockAnalysis.workflow.riskAggregation": "風險聚合",
    "stockAnalysis.workflow.riskClassification": "風險分級",
    "stockAnalysis.workflow.notification": "結果通知",
    "stockAnalysis.workflow.fetchFailed": "數據獲取失敗",
  },
};

const DEFAULT_EN = {
  "stockAnalysis.workflow.bullAnalyst": "Bull Analyst",
  "stockAnalysis.workflow.bearAnalyst": "Bear Analyst",
  "stockAnalysis.workflow.riskAggressive": "Aggressive Assessment",
  "stockAnalysis.workflow.riskConservative": "Conservative Assessment",
  "stockAnalysis.workflow.riskNeutral": "Neutral Assessment",
  "stockAnalysis.workflow.riskAggregation": "Risk Aggregation",
  "stockAnalysis.workflow.riskClassification": "Risk Classification",
  "stockAnalysis.workflow.notification": "Notification",
  "stockAnalysis.workflow.fetchFailed": "Failed to fetch data",
};

const files = [
  "ar.json",
  "de.json",
  "en-US.json",
  "es.json",
  "fr.json",
  "hi.json",
  "ja.json",
  "ko.json",
  "ru.json",
  "zh-CN.json",
  "zh-TW.json",
];

for (const file of files) {
  const path = join(localesDir, file);
  const content = readFileSync(path, "utf-8");
  const lang = file.replace(".json", "");

  // 决定翻译值
  const dict = TRANSLATIONS[lang] ?? DEFAULT_EN;

  // 找到 stockAnalysis.workflow.bearCase 行
  const marker = `"stockAnalysis.workflow.bearCase"`;
  const markerIdx = content.indexOf(marker);
  if (markerIdx === -1) {
    console.error(`[SKIP] ${file}: 未找到 bearCase marker`);
    continue;
  }

  // 找到该行的末尾（换行符）
  const lineEndIdx = content.indexOf("\n", markerIdx);
  if (lineEndIdx === -1) {
    console.error(`[SKIP] ${file}: 无法定位行尾`);
    continue;
  }

  // 构造要插入的文本
  const insertLines = NEW_KEYS.map((key) => {
    const val = dict[key];
    return `  "${key}": "${val}"`;
  });

  const afterLine = content.slice(lineEndIdx + 1);
  const indent = content.slice(lineEndIdx - 1, lineEndIdx) === "\r" ? "\r\n" : "\n";

  // 如果接下来是 } 结尾，在 bearCase 行后插入
  // 否则在 bearCase 行后插入
  const insertion = ",\n" + insertLines.join(",\n");

  const updated = content.slice(0, lineEndIdx + 1) + insertion + afterLine;
  writeFileSync(path, updated, "utf-8");
  console.log(`[OK] ${file}: 已插入 ${NEW_KEYS.length} 个 key`);
}
