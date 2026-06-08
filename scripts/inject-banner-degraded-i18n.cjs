// B15 配套 i18n: stockAnalysis.recommendation.bannerDegraded
const fs = require("fs");
const path = require("path");
const dir = "d:/OneManager/AxInvest/src/i18n/locales";

const vals = {
  "en-US":
    "Data styles {{styles}} were reduced in fidelity (as-of {{date}}). Treat their confidence as lower than non-degraded styles.",
  "zh-CN": "数据风格 {{styles}} 因 as-of {{date}} 截断,已以降级模式输出,请低于非降级风格的可信度处理。",
  "zh-TW": "資料風格 {{styles}} 因 as-of {{date}} 截斷,已以降級模式輸出,請低於非降級風格的可信度處理。",
  "ja":
    "データスタイル {{styles}} は as-of {{date}} 切り捨てで低信頼度出力されています。非劣化スタイルより低い信頼度で扱ってください。",
  "ko":
    "데이터 스타일 {{styles}} 은(는) as-of {{date}} 잘림으로 인해 저하되어 출력됩니다. 비저하 스타일보다 낮은 신뢰도로 다루십시오.",
  "fr":
    "Les styles de données {{styles}} ont été réduits en fidélité (as-of {{date}}). Traitez leur confiance comme inférieure aux styles non dégradés.",
  "de":
    "Datenstile {{styles}} wurden in der Treue reduziert (as-of {{date}}). Behandeln Sie ihre Konfidenz niedriger als bei nicht-degradierten Stilen.",
  "es":
    "Los estilos de datos {{styles}} se redujeron en fidelidad (as-of {{date}}). Trate su confianza como inferior a la de los estilos no degradados.",
  "ru":
    "Стили данных {{styles}} были ухудшены (as-of {{date}}). Относитесь к их достоверности как к более низкой, чем у не ухудшенных стилей.",
  "hi": "डेटा शैलियाँ {{styles}} की विश्वसनीयता कम हुई (as-of {{date}}). गैर-अवनत शैलियों से कम विश्वास के साथ व्यवहार करें।",
  "ar": "تم تخفيض دقة أنماط البيانات {{styles}} (as-of {{date}}). عامل ثقتها بأنها أقل من الأساليب غير المتدهورة.",
};

let touched = 0, skipped = 0, bad = 0;
for (const [lang, val] of Object.entries(vals)) {
  const fp = path.join(dir, lang + ".json");
  if (!fs.existsSync(fp)) {
    bad++;
    continue;
  }
  const raw = fs.readFileSync(fp, "utf8");
  const obj = JSON.parse(raw);
  if (!obj.stockAnalysis) {
    bad++;
    continue;
  }
  if (!obj.stockAnalysis.recommendation) {
    bad++;
    continue;
  }
  if (obj.stockAnalysis.recommendation.bannerDegraded) {
    skipped++;
    continue;
  }
  obj.stockAnalysis.recommendation.bannerDegraded = val;
  fs.writeFileSync(fp, JSON.stringify(obj, null, 2) + "\n", "utf8");
  console.log("OK", lang);
  touched++;
}
console.log("---", { touched, skipped, bad });
process.exit(bad === 0 ? 0 : 1);
