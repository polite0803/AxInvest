// scripts/add-elapsedhint-keys.cjs
// 给所有 locale 加 chat.workflow.elapsedHint + chat.workflow.lastUpdatedHint

const fs = require("fs");
const path = require("path");

const locales = ["en-US", "zh-TW", "ja", "de", "fr", "es", "ar", "ru", "hi", "ko"];
const dir = path.resolve(__dirname, "../src/i18n/locales");

const en = {
  elapsedHint: "Node runtime, auto-refreshes every 1s",
  lastUpdatedHint: "Last workflow status fetch time",
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
  if (!obj.chat?.workflow) {
    console.log("NO chat.workflow:", loc);
    skipped++;
    continue;
  }
  if (obj.chat.workflow.elapsedHint) {
    console.log("ALREADY:", loc);
    skipped++;
    continue;
  }
  obj.chat.workflow.elapsedHint = en.elapsedHint;
  obj.chat.workflow.lastUpdatedHint = en.lastUpdatedHint;
  fs.writeFileSync(fp, JSON.stringify(obj, null, 2) + "\n", "utf8");
  console.log("OK", loc);
  updated++;
}
console.log("---");
console.log("updated:", updated, "skipped:", skipped);
