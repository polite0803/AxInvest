#!/usr/bin/env node
/**
 * 缺陷 F 修复: 注入 stockAnalysis.empty 段 4 个 key 到 11 个 locale。
 * - replayDegradedTitle: Alert title
 * - replayDegraded: 默认 description
 * - replayDegradedWithCount: 带 count
 * - replayDegradedWithReason: 带具体 reason
 */
const fs = require("fs");
const path = require("path");

const LOCALES_DIR = "d:/OneManager/AxInvest/src/i18n/locales";

const TRANSLATIONS = {
  "zh-CN": {
    replayDegradedTitle: "⏪ 回放模式下降级",
    replayDegraded: "此数据源在回放模式下无历史语义,已被自动跳过。",
    replayDegradedWithCount: "回放模式下共 {{n}} 项数据源被跳过,本面板不显示。",
    replayDegradedWithReason: "回放模式下降级: {{reason}}",
  },
  "zh-TW": {
    replayDegradedTitle: "⏪ 回放模式下降級",
    replayDegraded: "此資料源在回放模式下無歷史語意,已被自動跳過。",
    replayDegradedWithCount: "回放模式下共 {{n}} 項資料源被跳過,本面板不顯示。",
    replayDegradedWithReason: "回放模式下降級: {{reason}}",
  },
  "en-US": {
    replayDegradedTitle: "⏪ Degraded in Replay Mode",
    replayDegraded: "This data source has no historical semantics in replay mode and was skipped automatically.",
    replayDegradedWithCount: "{{n}} data sources were skipped in replay mode; this panel is hidden.",
    replayDegradedWithReason: "Degraded in replay mode: {{reason}}",
  },
  "ja": {
    replayDegradedTitle: "⏪ 再生モードで降格",
    replayDegraded: "このデータソースは再生モードで履歴がなく、自動的にスキップされました。",
    replayDegradedWithCount: "再生モードで {{n}} 件のデータソースがスキップされました。このパネルは非表示です。",
    replayDegradedWithReason: "再生モードで降格: {{reason}}",
  },
  "ko": {
    replayDegradedTitle: "⏪ 재생 모드에서 성능 저하",
    replayDegraded: "이 데이터 소스는 재생 모드에서 기록이 없어 자동으로 건너뛰었습니다.",
    replayDegradedWithCount: "재생 모드에서 {{n}}개 데이터 소스가 건너뛰어져 이 패널이 숨겨졌습니다.",
    replayDegradedWithReason: "재생 모드에서 성능 저하: {{reason}}",
  },
  "fr": {
    replayDegradedTitle: "⏪ Dégradé en mode replay",
    replayDegraded: "Cette source de données n'a pas de sens historique en mode replay et a été ignorée.",
    replayDegradedWithCount: "{{n}} sources de données ignorées en mode replay ; ce panneau est masqué.",
    replayDegradedWithReason: "Dégradé en mode replay : {{reason}}",
  },
  "de": {
    replayDegradedTitle: "⏪ Im Replay-Modus heruntergestuft",
    replayDegraded: "Diese Datenquelle hat im Replay-Modus keinen historischen Wert und wurde übersprungen.",
    replayDegradedWithCount: "{{n}} Datenquellen wurden im Replay-Modus übersprungen; dieses Panel ist ausgeblendet.",
    replayDegradedWithReason: "Im Replay-Modus heruntergestuft: {{reason}}",
  },
  "es": {
    replayDegradedTitle: "⏪ Degradado en modo replay",
    replayDegraded: "Esta fuente de datos no tiene semántica histórica en modo replay y se omitió automáticamente.",
    replayDegradedWithCount: "{{n}} fuentes de datos omitidas en modo replay; este panel está oculto.",
    replayDegradedWithReason: "Degradado en modo replay: {{reason}}",
  },
  "ar": {
    replayDegradedTitle: "⏪ متدهور في وضع الإعادة",
    replayDegraded: "مصدر البيانات هذا ليس له دلالات تاريخية في وضع الإعادة وتم تخطيه تلقائيًا.",
    replayDegradedWithCount: "تم تخطي {{n}} من مصادر البيانات في وضع الإعادة؛ هذه اللوحة مخفية.",
    replayDegradedWithReason: "متدهور في وضع الإعادة: {{reason}}",
  },
  "ru": {
    replayDegradedTitle: "⏪ Ухудшено в режиме повтора",
    replayDegraded: "Этот источник данных не имеет исторической семантики в режиме повтора и был пропущен.",
    replayDegradedWithCount: "В режиме повтора пропущено {{n}} источников данных; эта панель скрыта.",
    replayDegradedWithReason: "Ухудшено в режиме повтора: {{reason}}",
  },
  "hi": {
    replayDegradedTitle: "⏪ रीप्ले मोड में अवनत",
    replayDegraded: "इस डेटा स्रोत का रीप्ले मोड में कोई ऐतिहासिक अर्थ नहीं है और स्वचालित रूप से छोड़ दिया गया।",
    replayDegradedWithCount: "रीप्ले मोड में {{n}} डेटा स्रोत छोड़े गए; यह पैनल छिपा है।",
    replayDegradedWithReason: "रीप्ले मोड में अवनत: {{reason}}",
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
  if (!data.stockAnalysis) {
    console.error(`[fail] ${f}: missing stockAnalysis root`);
    fail++;
    continue;
  }
  if (!data.stockAnalysis.empty) {
    data.stockAnalysis.empty = {};
  }
  let changed = false;
  for (const [k, v] of Object.entries(text)) {
    if (data.stockAnalysis.empty[k] !== v) {
      data.stockAnalysis.empty[k] = v;
      changed = true;
    }
  }
  const out = JSON.stringify(data, null, 2) + "\n";
  fs.writeFileSync(filePath, out, "utf8");
  if (changed) {
    updated++;
    console.log(`[updated] ${f}`);
  } else {
    console.log(`[ok] ${f} (no change)`);
  }
  ok++;
}

console.log(`\nDone. ok=${ok} updated=${updated} fail=${fail}`);
process.exit(fail > 0 ? 1 : 0);
