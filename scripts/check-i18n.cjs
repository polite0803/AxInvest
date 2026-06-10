// 验证 11 个 locale JSON 合法
const fs = require("fs");
const path = require("path");
const dir = "d:/OneManager/AxInvest/src/i18n/locales";
let bad = 0;
fs.readdirSync(dir).filter(f => f.endsWith(".json")).sort().forEach(f => {
  const raw = fs.readFileSync(path.join(dir, f), "utf8");
  try {
    const obj = JSON.parse(raw);
    if (!obj.timeTravel || !obj.timeTravel.pageAnchor) {
      console.log("MISS pageAnchor:", f);
      bad++;
      return;
    }
    const pa = obj.timeTravel.pageAnchor;
    if (!pa.live || !pa.replay || !pa.untilDate) {
      console.log("INCOMPLETE pageAnchor:", f, pa);
      bad++;
      return;
    }
    console.log("OK", f);
  } catch (e) {
    console.log("BAD", f, e.message);
    bad++;
  }
});
console.log("---");
console.log("bad count:", bad);
process.exit(bad === 0 ? 0 : 1);
