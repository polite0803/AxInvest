// 在每个 locale 的 timeTravel.replayWatermark 块后插入 degradedStyles 子块
// (B10) — 11 个 locale 全覆盖,en/zh-CN 已经手工补过会跳过
const fs = require("fs");
const path = require("path");
const dir = "d:/OneManager/AxInvest/src/i18n/locales";

const blocks = {
  "en-US": {
    title: "Data sources degraded under as-of truncation",
    description:
      "The following data styles were reduced in fidelity because the requested as-of date {{date}} cuts off history needed for the full signal.",
    reason: "Reason: {{reason}}",
    hint:
      "A degraded result is not a wrong result; it is a less informative one. Treat its confidence as lower than non-degraded styles.",
    vendor: "vendor: {{name}}",
  },
  "zh-CN": {
    title: "as-of 截断导致的数据降级",
    description: "以下数据风格因 as-of 截止日 {{date}} 截断了所需的全部历史,在回放模式下以低可信度输出。",
    reason: "原因: {{reason}}",
    hint: "降级不等于错误,只是信息量变少,请低于非降级风格的可信度处理。",
    vendor: "数据源: {{name}}",
  },
  "zh-TW": {
    title: "as-of 截斷導致的資料降級",
    description: "以下資料風格因 as-of 截止日 {{date}} 截斷了所需的全部歷史,在回放模式下以低可信度輸出。",
    reason: "原因: {{reason}}",
    hint: "降級不等於錯誤,只是資訊量變少,請低於非降級風格的可信度處理。",
    vendor: "資料源: {{name}}",
  },
  "ja": {
    title: "as-of 切り捨てによるデータ劣化",
    description:
      "以下のデータスタイルは as-of 基準日 {{date}} により必要な履歴が切り捨てられ、Replay モードでは低信頼度で出力されています。",
    reason: "理由: {{reason}}",
    hint: "劣化は誤りではなく情報量が減るだけなので、非劣化スタイルより低い信頼度で扱ってください。",
    vendor: "ベンダー: {{name}}",
  },
  "ko": {
    title: "as-of 잘림으로 인한 데이터 저하",
    description:
      "다음 데이터 스타일은 as-of 기준일 {{date}} 때문에 필요한 과거가 잘려, 리플레이 모드에서 낮은 신뢰도로 출력됩니다.",
    reason: "사유: {{reason}}",
    hint: "저하됨은 오류가 아니라 정보량이 줄어든 것뿐이므로 비저하 스타일보다 낮은 신뢰도로 다루십시오.",
    vendor: "벤더: {{name}}",
  },
  "fr": {
    title: "Dégradation des sources de données sous troncature as-of",
    description:
      "Les styles de données suivants ont été réduits en fidélité car la date as-of {{date}} coupe l'historique nécessaire au signal complet.",
    reason: "Raison : {{reason}}",
    hint:
      "Un résultat dégradé n'est pas un résultat erroné ; il est moins informatif. Traitez sa confiance comme inférieure aux styles non dégradés.",
    vendor: "fournisseur : {{name}}",
  },
  "de": {
    title: "Datenquellen unter as-of-Kürzung degradiert",
    description:
      "Folgende Datenstile sind in der Treue reduziert, da das as-of-Datum {{date}} die für das volle Signal benötigte Historie abschneidet.",
    reason: "Grund: {{reason}}",
    hint:
      "Ein degradiertes Ergebnis ist kein falsches Ergebnis; es ist weniger informativ. Behandeln Sie seine Konfidenz niedriger als bei nicht-degradierten Stilen.",
    vendor: "Anbieter: {{name}}",
  },
  "es": {
    title: "Fuentes de datos degradadas bajo truncamiento as-of",
    description:
      "Los siguientes estilos de datos se redujeron en fidelidad porque la fecha as-of {{date}} corta el historial necesario para la señal completa.",
    reason: "Motivo: {{reason}}",
    hint:
      "Un resultado degradado no es un resultado incorrecto; es menos informativo. Trate su confianza como inferior a la de los estilos no degradados.",
    vendor: "proveedor: {{name}}",
  },
  "ru": {
    title: "Ухудшение источников данных из-за усечения as-of",
    description:
      "Следующие стили данных были ухудшены, так как дата as-of {{date}} обрезает историю, необходимую для полного сигнала.",
    reason: "Причина: {{reason}}",
    hint:
      "Ухудшенный результат — не ошибочный; он менее информативен. Относитесь к его достоверности как к более низкой, чем у не ухудшенных стилей.",
    vendor: "источник: {{name}}",
  },
  "hi": {
    title: "as-of ट्रंकेशन के कारण डेटा स्रोतों का अवनमन",
    description:
      "निम्नलिखित डेटा शैलियों की विश्वसनीयता कम हुई क्योंकि as-of तिथि {{date}} पूर्ण संकेत के लिए आवश्यक इतिहास को काटती है।",
    reason: "कारण: {{reason}}",
    hint: "अवनत परिणाम गलत नहीं है; यह कम जानकारीपूर्ण है। इसके विश्वास को गैर-अवनत शैलियों से कम मानें।",
    vendor: "विक्रेता: {{name}}",
  },
  "ar": {
    title: "تدهور مصادر البيانات بسبب اقتطاع as-of",
    description: "تم تخفيض دقة أنماط البيانات التالية لأن تاريخ as-of {{date}} يقطع التاريخ اللازم للإشارة الكاملة.",
    reason: "السبب: {{reason}}",
    hint: "النتيجة المتدهورة ليست خاطئة؛ بل أقل إفادة. عامل ثقتها بأنها أقل من الأساليب غير المتدهورة.",
    vendor: "المورّد: {{name}}",
  },
};

let touched = 0;
let skipped = 0;
let bad = 0;
const allFiles = fs.readdirSync(dir).filter(f => f.endsWith(".json")).sort();
for (const f of allFiles) {
  const lang = f.replace(".json", "");
  const block = blocks[lang];
  if (!block) {
    console.log("SKIP(no block):", f);
    skipped++;
    continue;
  }
  const fp = path.join(dir, f);
  const raw = fs.readFileSync(fp, "utf8");
  const obj = JSON.parse(raw);
  if (!obj.timeTravel) {
    console.log("SKIP(no timeTravel):", f);
    skipped++;
    continue;
  }
  if (obj.timeTravel.degradedStyles) {
    console.log("SKIP(already has):", f);
    skipped++;
    continue;
  }
  obj.timeTravel.degradedStyles = block;
  // 序列化 — 使用 2 空格缩进与现有文件保持一致
  const newRaw = JSON.stringify(obj, null, 2) + "\n";
  fs.writeFileSync(fp, newRaw, "utf8");
  console.log("OK", f);
  touched++;
}
console.log("---");
console.log("touched:", touched, "skipped:", skipped, "bad:", bad);
process.exit(bad === 0 ? 0 : 1);
