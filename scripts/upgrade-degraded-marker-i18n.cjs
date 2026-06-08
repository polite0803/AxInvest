#!/usr/bin/env node
/**
 * 升级 timeTravel.degradedMarker 段:把 label 改为 labelWithCount 模板,
 * 保留原 label 字符串作为降级面板描述(已存在则跳过注入)。
 * 缺陷 E 修复配套 i18n。
 */
const fs = require("fs");
const path = require("path");

const LOCALES_DIR = "d:/OneManager/AxInvest/src/i18n/locales";

const TRANSLATIONS = {
  "zh-CN": {
    label: "已降级 · 部分数据无历史",
    labelWithCount: "已降级 · {{n}} 项",
    tooltip: "回放模式下资金流向/融资融券/北向持仓等数据无历史语义,已被跳过。详见推荐面板的降级提示。",
  },
  "zh-TW": {
    label: "已降級 · 部分資料無歷史",
    labelWithCount: "已降級 · {{n}} 項",
    tooltip: "回放模式下資金流向/融資融券/北向持倉等資料無歷史語意,已被跳過。詳見推薦面板的降級提示。",
  },
  "en-US": {
    label: "Degraded · Some data has no history",
    labelWithCount: "Degraded · {{n}} items",
    tooltip:
      "In replay mode, money flow / margin / north-bound holding etc. have no historical semantics and were skipped.",
  },
  "ja": {
    label: "降格 · 一部データに履歴なし",
    labelWithCount: "降格 · {{n}} 件",
    tooltip: "再生モードでは資金フロー/融資融券/北向保有などに履歴がなくスキップされました。",
  },
  "ko": {
    label: "성능 저하 · 일부 데이터에 기록 없음",
    labelWithCount: "성능 저하 · {{n}}건",
    tooltip: "재생 모드에서 자금 흐름/마진/북향 보유 등에 기록이 없어 건너뛰었습니다.",
  },
  "fr": {
    label: "Dégradé · Certaines données sans historique",
    labelWithCount: "Dégradé · {{n}} éléments",
    tooltip:
      "En mode replay, flux monétaire/marge/détention nord-bound etc. n'ont pas de sens historique et ont été ignorés.",
  },
  "de": {
    label: "Heruntergestuft · Einige Daten ohne Verlauf",
    labelWithCount: "Heruntergestuft · {{n}} Elemente",
    tooltip:
      "Im Replay-Modus haben Geldfluss/Marge/Nordbound-Bestand etc. keine historische Bedeutung und wurden übersprungen.",
  },
  "es": {
    label: "Degradado · Algunos datos sin historial",
    labelWithCount: "Degradado · {{n}} elementos",
    tooltip:
      "En modo replay, flujo de dinero/margen/tenencia norte-bound etc. no tienen semántica histórica y se omitieron.",
  },
  "ar": {
    label: "متدهور · بعض البيانات بلا سجل",
    labelWithCount: "متدهور · {{n}} عناصر",
    tooltip: "في وضع الإعادة، تدفق الأموال/الهامش/حيازة الشمال ليس لها دلالات تاريخية وتم تخطيها.",
  },
  "ru": {
    label: "Ухудшено · Часть данных без истории",
    labelWithCount: "Ухудшено · {{n}} элементов",
    tooltip:
      "В режиме повтора поток средств/маржа/северные активы и т.д. не имеют исторической семантики и были пропущены.",
  },
  "hi": {
    label: "अवनत · कुछ डेटा का इतिहास नहीं",
    labelWithCount: "अवनत · {{n}} आइटम",
    tooltip: "रीप्ले मोड में मनी फ्लो/मार्जिन/नॉर्थ-बाउंड होल्डिंग आदि का कोई ऐतिहासिक अर्थ नहीं है, छोड़ दिया गया।",
  },
};

const files = fs
  .readdirSync(LOCALES_DIR)
  .filter((f) => f.endsWith(".json"))
  .sort();

let ok = 0;
let fail = 0;
let updated = 0;
for (const f of files) {
  const filePath = path.join(LOCALES_DIR, f);
  const lang = path.basename(f, ".json");
  const text = TRANSLATIONS[lang];
  if (!text) {
    console.warn(`[skip] ${f}: no translation defined`);
    continue;
  }
  const raw = fs.readFileSync(filePath, "utf8");
  let data;
  try {
    data = JSON.parse(raw);
  } catch (e) {
    console.error(`[fail] ${f}: invalid JSON: ${e.message}`);
    fail++;
    continue;
  }
  if (!data.timeTravel || !data.timeTravel.degradedMarker) {
    console.error(`[fail] ${f}: missing timeTravel.degradedMarker`);
    fail++;
    continue;
  }
  const before = JSON.stringify(data.timeTravel.degradedMarker);
  data.timeTravel.degradedMarker = text;
  const after = JSON.stringify(data.timeTravel.degradedMarker);
  if (before !== after) {
    const out = JSON.stringify(data, null, 2) + "\n";
    fs.writeFileSync(filePath, out, "utf8");
    updated++;
  }
  console.log(`[ok] ${f}`);
  ok++;
}

console.log(`\nDone. ok=${ok} updated=${updated} fail=${fail}`);
process.exit(fail > 0 ? 1 : 0);
