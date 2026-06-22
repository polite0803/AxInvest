// scripts/add-multimodelvote-keys.cjs
// 给除 zh-CN/en-US 外的 9 个 locale 批量添加 chat.multiModelVote 块
// 使用英文 fallback 文案(后续可由翻译者优化)

const fs = require("fs");
const path = require("path");

const locales = ["zh-TW", "ja", "de", "fr", "es", "ar", "ru", "hi", "ko"];
const dir = path.resolve(__dirname, "../src/i18n/locales");

const en = {
  button: "Aggregate Vote",
  title: "Multi-Model Decision Vote",
  strategy: "Aggregation Strategy",
  strategyMajority: "Majority Vote",
  strategyWeighted: "Confidence-Weighted",
  strategyConsensus: "Unanimous Consensus",
  modelCount: "Models Participated",
  validCount: "Valid Decisions",
  finalAction: "Final Action",
  winnerModel: "Winner Model",
  avgConfidence: "Avg. Confidence",
  voteBreakdown: "Vote Breakdown",
  allAgree: "All models agree",
  disagreement: "Disagreement detected",
  noDecision: "No decision to aggregate",
};

let updated = 0;
let skipped = 0;
for (const loc of locales) {
  const fp = path.join(dir, `${loc}.json`);
  if (!fs.existsSync(fp)) {
    console.warn("MISSING", fp);
    continue;
  }
  const obj = JSON.parse(fs.readFileSync(fp, "utf8"));
  if (obj.chat?.multiModelVote) {
    console.log("ALREADY HAS multiModelVote:", loc);
    skipped++;
    continue;
  }
  if (!obj.chat) { obj.chat = {}; }
  obj.chat.multiModelVote = { ...en };
  fs.writeFileSync(fp, JSON.stringify(obj, null, 2) + "\n", "utf8");
  console.log("OK", loc);
  updated++;
}
console.log("---");
console.log("updated:", updated, "skipped:", skipped);
