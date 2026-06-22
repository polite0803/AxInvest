// scripts/add-decision-overview-key.cjs
const fs = require("fs");
const path = require("path");

const locales = ["en-US", "zh-TW", "ja", "de", "fr", "es", "ar", "ru", "hi", "ko"];
const dir = path.resolve(__dirname, "../src/i18n/locales");

const KEY = "overviewTitle";
const VALUE = "Decision Chain";

let updated = 0, skipped = 0;
for (const loc of locales) {
  const fp = path.join(dir, `${loc}.json`);
  if (!fs.existsSync(fp)) { continue; }
  const obj = JSON.parse(fs.readFileSync(fp, "utf8"));
  if (!obj.stockAnalysis?.timeline) {
    skipped++;
    continue;
  }
  if (obj.stockAnalysis.timeline[KEY]) {
    skipped++;
    continue;
  }
  obj.stockAnalysis.timeline[KEY] = VALUE;
  fs.writeFileSync(fp, JSON.stringify(obj, null, 2) + "\n", "utf8");
  updated++;
}
console.log("updated:", updated, "skipped:", skipped);
