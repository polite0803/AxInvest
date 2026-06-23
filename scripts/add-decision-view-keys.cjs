// scripts/add-decision-view-keys.cjs
// 给除 zh-CN 外的 10 个 locale 批量添加 dualView.decision 块
// 策略:用字符串插入(不重排键顺序),保持 dprint 兼容

const fs = require("fs");
const path = require("path");

const locales = ["zh-TW", "en-US", "de", "es", "fr", "ja", "ko", "ru", "hi", "ar"];
const dir = path.resolve(__dirname, "../src/i18n/locales");

// 10 个语言的 decision 块(18 个 key,顺序与 zh-CN 完全一致)
const DECISION = {
  "zh-TW": {
    title: "決策雙視角對比",
    formula: "公式視角",
    llm: "LLM 視角",
    formulaBadge: "公式",
    llmBadge: "LLM",
    field: "欄位",
    action: "行動",
    positionPct: "倉位%",
    confidence: "信心度",
    reasoning: "推理",
    formulaReasoningOmitted: "見主決策面板",
    highAgreementHint: "高度一致,可信度高",
    midAgreementHint: "中等一致,注意分歧點",
    lowAgreementHint: "分歧較大,建議人工覆核",
    reviewRecommended: "建議人工覆核",
    llmUnavailable: "LLM 視角不可用",
    llmUnavailableHint: "本次分析未啟用 LLM 決策節點",
    llmMissingHint: "LLM 視角不可用(一致性參考自上次,score={{score}})",
  },
  "en-US": {
    title: "Decision Dual-View Comparison",
    formula: "Formula View",
    llm: "LLM View",
    formulaBadge: "Formula",
    llmBadge: "LLM",
    field: "Field",
    action: "Action",
    positionPct: "Position %",
    confidence: "Confidence",
    reasoning: "Reasoning",
    formulaReasoningOmitted: "See main decision panel",
    highAgreementHint: "Highly consistent, high confidence",
    midAgreementHint: "Moderate consistency, watch disagreements",
    lowAgreementHint: "Large disagreement, manual review recommended",
    reviewRecommended: "Manual review recommended",
    llmUnavailable: "LLM view unavailable",
    llmUnavailableHint: "LLM decision node not enabled in this analysis",
    llmMissingHint: "LLM view unavailable (agreement referenced from last, score={{score}})",
  },
  "de": {
    title: "Entscheidungs-Doppelsicht-Vergleich",
    formula: "Formelsicht",
    llm: "LLM-Sicht",
    formulaBadge: "Formel",
    llmBadge: "LLM",
    field: "Feld",
    action: "Aktion",
    positionPct: "Position %",
    confidence: "Konfidenz",
    reasoning: "Begründung",
    formulaReasoningOmitted: "Siehe Hauptentscheidungsfeld",
    highAgreementHint: "Hochkonsistent, hohe Konfidenz",
    midAgreementHint: "Mäßige Konsistenz, achte auf Abweichungen",
    lowAgreementHint: "Große Abweichung, manuelle Überprüfung empfohlen",
    reviewRecommended: "Manuelle Überprüfung empfohlen",
    llmUnavailable: "LLM-Sicht nicht verfügbar",
    llmUnavailableHint: "LLM-Entscheidungsknoten in dieser Analyse nicht aktiviert",
    llmMissingHint: "LLM-Sicht nicht verfügbar (Übereinstimmung aus letzter Analyse, Punktzahl={{score}})",
  },
  "es": {
    title: "Comparación de doble perspectiva de decisión",
    formula: "Vista de fórmula",
    llm: "Vista LLM",
    formulaBadge: "Fórmula",
    llmBadge: "LLM",
    field: "Campo",
    action: "Acción",
    positionPct: "Posición %",
    confidence: "Confianza",
    reasoning: "Razonamiento",
    formulaReasoningOmitted: "Ver panel de decisión principal",
    highAgreementHint: "Alta consistencia, alta confianza",
    midAgreementHint: "Consistencia moderada, atención a desacuerdos",
    lowAgreementHint: "Gran discrepancia, revisión manual recomendada",
    reviewRecommended: "Revisión manual recomendada",
    llmUnavailable: "Vista LLM no disponible",
    llmUnavailableHint: "Nodo de decisión LLM no habilitado en este análisis",
    llmMissingHint: "Vista LLM no disponible (acuerdo de la última, puntuación={{score}})",
  },
  "fr": {
    title: "Comparaison double perspective de décision",
    formula: "Vue formule",
    llm: "Vue LLM",
    formulaBadge: "Formule",
    llmBadge: "LLM",
    field: "Champ",
    action: "Action",
    positionPct: "Position %",
    confidence: "Confiance",
    reasoning: "Raisonnement",
    formulaReasoningOmitted: "Voir panneau de décision principal",
    highAgreementHint: "Très cohérent, haute confiance",
    midAgreementHint: "Cohérence modérée, attention aux divergences",
    lowAgreementHint: "Grand désaccord, révision manuelle recommandée",
    reviewRecommended: "Révision manuelle recommandée",
    llmUnavailable: "Vue LLM indisponible",
    llmUnavailableHint: "Nœud de décision LLM non activé dans cette analyse",
    llmMissingHint: "Vue LLM indisponible (accord de la dernière, score={{score}})",
  },
  "ja": {
    title: "意思決定のデュアルビュー比較",
    formula: "フォーミュラビュー",
    llm: "LLMビュー",
    formulaBadge: "フォーミュラ",
    llmBadge: "LLM",
    field: "フィールド",
    action: "アクション",
    positionPct: "ポジション%",
    confidence: "信頼度",
    reasoning: "推論",
    formulaReasoningOmitted: "メイン決定パネルを参照",
    highAgreementHint: "高い一貫性、信頼度高",
    midAgreementHint: "中程度の一貫性、分岐に注意",
    lowAgreementHint: "大きな分岐、手動レビュー推奨",
    reviewRecommended: "手動レビュー推奨",
    llmUnavailable: "LLMビュー利用不可",
    llmUnavailableHint: "この分析でLLM決定ノードが無効",
    llmMissingHint: "LLMビュー利用不可（前回の一致度を参照,スコア={{score}}）",
  },
  "ko": {
    title: "의사결정 듀얼 뷰 비교",
    formula: "공식 시점",
    llm: "LLM 시점",
    formulaBadge: "공식",
    llmBadge: "LLM",
    field: "필드",
    action: "행동",
    positionPct: "포지션%",
    confidence: "신뢰도",
    reasoning: "추론",
    formulaReasoningOmitted: "메인 결정 패널 참조",
    highAgreementHint: "높은 일치도, 높은 신뢰도",
    midAgreementHint: "중간 일치도, 분기 주의",
    lowAgreementHint: "큰 분기, 수동 검토 권장",
    reviewRecommended: "수동 검토 권장",
    llmUnavailable: "LLM 시점 사용 불가",
    llmUnavailableHint: "이 분석에서 LLM 결정 노드 비활성화",
    llmMissingHint: "LLM 시점 사용 불가 (마지막 일치도 참조, 점수={{score}})",
  },
  "ru": {
    title: "Сравнение двух ракурсов решения",
    formula: "Формульный взгляд",
    llm: "LLM-взгляд",
    formulaBadge: "Формула",
    llmBadge: "LLM",
    field: "Поле",
    action: "Действие",
    positionPct: "Позиция %",
    confidence: "Уверенность",
    reasoning: "Обоснование",
    formulaReasoningOmitted: "См. главную панель решения",
    highAgreementHint: "Высокая согласованность, высокая уверенность",
    midAgreementHint: "Средняя согласованность, обратите внимание на расхождения",
    lowAgreementHint: "Сильное расхождение, рекомендуется ручная проверка",
    reviewRecommended: "Рекомендуется ручная проверка",
    llmUnavailable: "LLM-взгляд недоступен",
    llmUnavailableHint: "Узел решения LLM не включён в этом анализе",
    llmMissingHint: "LLM-взгляд недоступен (ссылка на прошлое, оценка={{score}})",
  },
  "hi": {
    title: "निर्णय दोहरा दृश्य तुलना",
    formula: "सूत्र दृष्टिकोण",
    llm: "LLM दृष्टिकोण",
    formulaBadge: "सूत्र",
    llmBadge: "LLM",
    field: "फ़ील्ड",
    action: "कार्य",
    positionPct: "स्थान %",
    confidence: "विश्वास",
    reasoning: "तर्क",
    formulaReasoningOmitted: "मुख्य निर्णय पैनल देखें",
    highAgreementHint: "उच्च स्थिरता, उच्च विश्वास",
    midAgreementHint: "मध्यम स्थिरता, मतभेद पर ध्यान दें",
    lowAgreementHint: "बड़ा मतभेद, मैन्युअल समीक्षा की सिफारिश",
    reviewRecommended: "मैन्युअल समीक्षा की सिफारिश",
    llmUnavailable: "LLM दृष्टिकोण अनुपलब्ध",
    llmUnavailableHint: "इस विश्लेषण में LLM निर्णय नोड सक्षम नहीं",
    llmMissingHint: "LLM दृष्टिकोण अनुपलब्ध (पिछले से सहमति, स्कोर={{score}})",
  },
  "ar": {
    title: "مقارنة العرض المزدوج للقرار",
    formula: "منظور الصيغة",
    llm: "منظور LLM",
    formulaBadge: "الصيغة",
    llmBadge: "LLM",
    field: "الحقل",
    action: "الإجراء",
    positionPct: "المركز %",
    confidence: "الثقة",
    reasoning: "المنطق",
    formulaReasoningOmitted: "انظر لوحة القرار الرئيسية",
    highAgreementHint: "توافق عالٍ، ثقة عالية",
    midAgreementHint: "توافق متوسط، انتبه للخلافات",
    lowAgreementHint: "اختلاف كبير، مراجعة يدوية موصى بها",
    reviewRecommended: "مراجعة يدوية موصى بها",
    llmUnavailable: "منظور LLM غير متاح",
    llmUnavailableHint: "لم يتم تفعيل عقدة قرار LLM في هذا التحليل",
    llmMissingHint: "منظور LLM غير متاح (مرجع من آخر، النتيجة={{score}})",
  },
};

// 在 dualView 节点中找最后一个 "..." 后的位置(闭合 `}` 之前),插入 decision 块
// 同步返回 dualView 节点的范围 [start, end) 用于内部 decision 检测
function findDualViewRange(content) {
  const start = content.indexOf('"dualView": {');
  if (start === -1) {
    throw new Error("dualView 节点未找到");
  }
  let i = start + '"dualView": {'.length;
  let depth = 1;
  let inString = false;
  let escape = false;
  for (; i < content.length; i++) {
    const ch = content[i];
    if (escape) {
      escape = false;
      continue;
    }
    if (ch === "\\") {
      escape = true;
      continue;
    }
    if (ch === '"') {
      inString = !inString;
      continue;
    }
    if (inString) { continue; }
    if (ch === "{") { depth++; }
    else if (ch === "}") {
      depth--;
      if (depth === 0) { break; }
    }
  }
  if (depth !== 0) {
    throw new Error("dualView 节点大括号未闭合");
  }
  return { start, end: i }; // end 指向匹配的 }
}

function insertDecisionBlock(content, decisionDict) {
  // 1) 找 dualView 节点范围
  const { start, end } = findDualViewRange(content);
  const inner = content.slice(start, end);

  // 2) 校验未重复添加:在 dualView 内部找 "decision":
  //    注意不要误匹配 "decisionMaker" 等其他 key,使用边界
  if (/(^|\n)\s*"decision"\s*:/.test(inner)) {
    return { content, inserted: false, reason: "already exists" };
  }

  // 3) 构造 decision 块
  const lines = [];
  lines.push('    "decision": {');
  const entries = Object.entries(decisionDict);
  for (let k = 0; k < entries.length; k++) {
    const [key, val] = entries[k];
    const isLast = k === entries.length - 1;
    lines.push(`      ${JSON.stringify(key)}: ${JSON.stringify(val)}${isLast ? "" : ","}`);
  }
  lines.push("    }");
  const block = lines.join("\n");

  // 4) 在 end (匹配的 } 位置) 之前插入
  //    注意:before 末尾可能含换行 + 缩进空格(下一个 key 的前导空白),需要 trimEnd 去掉
  //    然后统一以 ",\n    "decision": {...}\n  }" 形式拼接
  const beforeTrimmed = content.slice(0, end).replace(/\s+$/, "");
  const after = content.slice(end);
  // 闭合 brace 之前应保持 dualView 节点的 2 空格缩进
  const newContent = beforeTrimmed + ",\n" + block + "\n  " + after;

  return {
    content: newContent,
    inserted: true,
  };
}

let updated = 0, skipped = 0, failed = 0;
for (const loc of locales) {
  const fp = path.join(dir, `${loc}.json`);
  if (!fs.existsSync(fp)) {
    console.warn("MISSING", fp);
    failed++;
    continue;
  }
  const orig = fs.readFileSync(fp, "utf8");
  const decisionDict = DECISION[loc];
  if (!decisionDict) {
    console.warn("NO DECISION DICT for", loc);
    failed++;
    continue;
  }
  try {
    const { content, inserted, reason } = insertDecisionBlock(orig, decisionDict);
    if (!inserted) {
      console.log("SKIP", loc, reason);
      skipped++;
      continue;
    }
    // 验证 JSON 解析成功
    JSON.parse(content);
    fs.writeFileSync(fp, content, "utf8");
    console.log("OK", loc);
    updated++;
  } catch (e) {
    console.error("FAIL", loc, e.message);
    failed++;
  }
}
console.log("---");
console.log("updated:", updated, "skipped:", skipped, "failed:", failed);
