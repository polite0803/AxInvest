#!/usr/bin/env python3
"""Add 2 new i18n keys to stockAnalysis namespace for DualView fallback labels."""

import json
from pathlib import Path

LOCALE_DIR = Path(r"D:\OneManager\AxInvest\src\i18n\locales")

NEW_KEYS = {
    "zh-CN": {"llmViewUnavailable": "LLM 视角不可用", "formulaFallbackHint": "回退显示公式决策 — 暂无双视角对比"},
    "zh-TW": {"llmViewUnavailable": "LLM 視角不可用", "formulaFallbackHint": "回退顯示公式決策 — 暫無雙視角對比"},
    "en-US": {"llmViewUnavailable": "LLM view unavailable", "formulaFallbackHint": "Falling back to formula decision — no dual-view comparison"},
    "ja": {"llmViewUnavailable": "LLM ビュー利用不可", "formulaFallbackHint": "フォーミュラ決定にフォールバック — デュアルビュー比較なし"},
    "ko": {"llmViewUnavailable": "LLM 보기 불가", "formulaFallbackHint": "수식 결정으로 대체 — 이중 보기 비교 없음"},
    "de": {"llmViewUnavailable": "LLM-Ansicht nicht verfügbar", "formulaFallbackHint": "Rückfall auf Formelentscheidung — kein Dual-View-Vergleich"},
    "fr": {"llmViewUnavailable": "Vue LLM indisponible", "formulaFallbackHint": "Repli sur la décision de formule — pas de comparaison double vue"},
    "es": {"llmViewUnavailable": "Vista LLM no disponible", "formulaFallbackHint": "Retroceso a decisión de fórmula — sin comparación de doble vista"},
    "ar": {"llmViewUnavailable": "عرض LLM غير متاح", "formulaFallbackHint": "التراجع إلى قرار الصيغة — لا توجد مقارنة ثنائية الرؤية"},
    "hi": {"llmViewUnavailable": "LLM दृश्य अनुपलब्ध", "formulaFallbackHint": "सूत्र निर्णय पर वापसी — कोई दोहरा दृश्य तुलना नहीं"},
    "ru": {"llmViewUnavailable": "Представление LLM недоступно", "formulaFallbackHint": "Возврат к решению по формуле — сравнения двойного вида нет"},
}

for locale_file in sorted(LOCALE_DIR.glob("*.json")):
    lang = locale_file.stem
    if lang not in NEW_KEYS:
        continue
    with open(locale_file, "r", encoding="utf-8") as f:
        data = json.load(f)
    sa = data.get("stockAnalysis")
    if not isinstance(sa, dict):
        print(f"  WARN {lang}: no stockAnalysis")
        continue
    for k, v in NEW_KEYS[lang].items():
        sa[k] = v
    data["stockAnalysis"] = sa
    with open(locale_file, "w", encoding="utf-8") as f:
        json.dump(data, f, ensure_ascii=False, indent=2)
        f.write("\n")
    print(f"  OK   {lang}")

print("Done")
