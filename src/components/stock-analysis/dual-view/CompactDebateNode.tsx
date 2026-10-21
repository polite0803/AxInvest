/**
 * CompactDebateNode — DebatePanel 在 chat 中的紧凑版本
 * 输入:debate 数据(多空双方观点)
 * 输出:情绪比例 + 1 条最强看多 + 1 条最强看空
 */
import { horizonLabelKey } from "@/lib/stock-analysis-utils";
import { useMemo } from "react";
import { useTranslation } from "react-i18next";

interface DebateRound {
  /** 档位；null/缺省 = 分档之前的历史单份记录 */
  tier?: string | null;
  round: number;
  bull: string;
  bear: string;
}

interface CompactDebateNodeProps {
  data: DebateRound[] | { rounds: DebateRound[] } | unknown;
}

function normalizeRounds(data: CompactDebateNodeProps["data"]): DebateRound[] {
  if (Array.isArray(data)) { return data; }
  if (data && typeof data === "object" && "rounds" in data) {
    const r = (data as { rounds: DebateRound[] }).rounds;
    return Array.isArray(r) ? r : [];
  }
  return [];
}

interface ParsedClaim {
  claim: string;
  strength?: number;
  category?: string;
}

function tryExtractClaims(text: string, isBull: boolean): ParsedClaim | null {
  if (!text) { return null; }
  // 提取 ```json 块
  const m = text.match(/```(?:json)?\s*([\s\S]+?)```/);
  const candidate = m ? m[1].trim() : text.trim();
  let parsed: Record<string, unknown> | null = null;
  try {
    parsed = JSON.parse(candidate);
  } catch { /* not json */ }
  if (!parsed) { return null; }

  if (isBull) {
    const arr = parsed.core_arguments;
    if (Array.isArray(arr) && arr.length > 0) {
      // 找 strength 最高的论点
      const sorted = [...arr].sort((a, b) => (b?.strength ?? 0) - (a?.strength ?? 0));
      const top = sorted[0];
      return {
        claim: String(top?.claim ?? "").slice(0, 100),
        strength: typeof top?.strength === "number" ? top.strength : undefined,
        category: top?.category,
      };
    }
    const resonance = parsed.resonance_points;
    if (Array.isArray(resonance) && resonance.length > 0) {
      const top = resonance[0];
      return { claim: String(top?.point ?? "").slice(0, 100), strength: top?.weight };
    }
  } else {
    // bear: 共用同一 JSON 结构,看多看空都从 core_arguments 提取
    // 但 bear 角色通常也会输出 core_arguments(多空都列)
    const arr = parsed.core_arguments;
    if (Array.isArray(arr) && arr.length > 0) {
      const sorted = [...arr].sort((a, b) => (b?.strength ?? 0) - (a?.strength ?? 0));
      const top = sorted[0];
      return {
        claim: String(top?.claim ?? "").slice(0, 100),
        strength: typeof top?.strength === "number" ? top.strength : undefined,
        category: top?.category,
      };
    }
  }
  return null;
}

export function CompactDebateNode({ data }: CompactDebateNodeProps) {
  const { t } = useTranslation();
  const rounds = useMemo(() => normalizeRounds(data), [data]);

  const summary = useMemo(() => {
    if (rounds.length === 0) { return null; }
    // 情绪:对比所有 round 的 bull/bear 文本长度（v161+ 是四档合计）
    const bullLen = rounds.reduce((acc, r) => acc + (r.bull?.length ?? 0), 0);
    const bearLen = rounds.reduce((acc, r) => acc + (r.bear?.length ?? 0), 0);
    const total = bullLen + bearLen;
    const ratio = total > 0 ? bullLen / total : 0.5;

    // v161+ 每档一份记录：按档各取最后一轮，不能只取全局最后一行
    // （那只是四档里最后完成的那个，其余三档会被静默丢掉）。
    const byTier = new Map<string | null, DebateRound>();
    for (const r of rounds) { byTier.set(r.tier ?? null, r); }
    const tierLines = Array.from(byTier.entries()).map(([tier, r]) => ({
      tier,
      bull: tryExtractClaims(r.bull ?? "", true),
      bear: tryExtractClaims(r.bear ?? "", false),
    }));

    return {
      ratio,
      tierLines,
      tiered: tierLines.some((line) => line.tier !== null),
      totalRounds: rounds.length,
    };
  }, [rounds]);

  if (!summary) {
    return (
      <div className="text-[12px] italic" style={{ color: "var(--muted)" }}>
        {t("stockAnalysis.compact.noDebateData")}
      </div>
    );
  }

  const pct = Math.round(summary.ratio * 100);
  const sentiment = summary.ratio > 0.55
    ? t("stockAnalysis.compact.bullish")
    : summary.ratio < 0.45
    ? t("stockAnalysis.compact.bearish")
    : t("stockAnalysis.compact.neutral");

  return (
    <div className="space-y-1 text-[12px]">
      <div className="flex items-baseline gap-2 flex-wrap">
        <span
          className="px-1.5 py-0.5 rounded text-[10px] font-medium"
          style={{
            background: summary.ratio > 0.55
              ? "var(--sa-red-bg, #fee2e2)"
              : summary.ratio < 0.45
              ? "var(--sa-green-bg, #dcfce7)"
              : "var(--muted-bg, #e5e7eb)",
            color: summary.ratio > 0.55
              ? "var(--sa-red, #dc2626)"
              : summary.ratio < 0.45
              ? "var(--sa-green, #16a34a)"
              : "var(--muted, #6b7280)",
          }}
        >
          {sentiment} · {pct}%
        </span>
        <span style={{ color: "var(--muted)" }}>
          {summary.tiered ? summary.tierLines.length : summary.totalRounds}
          {summary.tiered ? t("stockAnalysis.compact.tiers") : t("stockAnalysis.compact.rounds")}
        </span>
      </div>

      {summary.tiered
        ? summary.tierLines.map((line) => (
          <div key={line.tier ?? "legacy"} className="text-[11px] leading-snug">
            {line.tier && (
              <span style={{ color: "var(--color-text-secondary)", fontWeight: 600 }}>
                {t(horizonLabelKey(line.tier) ?? line.tier)}：
              </span>
            )}
            {line.bull?.claim && (
              <span style={{ color: "var(--sa-red, #dc2626)" }}>
                {t("stockAnalysis.compact.bull")}: {line.bull.claim}
              </span>
            )}
            {line.bull?.claim && line.bear?.claim && <span style={{ color: "var(--muted)" }}>·</span>}
            {line.bear?.claim && (
              <span style={{ color: "var(--sa-green, #16a34a)" }}>
                {t("stockAnalysis.compact.bear")}: {line.bear.claim}
              </span>
            )}
          </div>
        ))
        : (
          <>
            {summary.tierLines[0]?.bull?.claim && (
              <div className="text-[11px] leading-snug flex gap-1">
                <span style={{ color: "var(--sa-red, #dc2626)", fontWeight: 600 }}>
                  {t("stockAnalysis.compact.bull")}:
                </span>
                <span style={{ color: "var(--color-text-secondary)" }}>{summary.tierLines[0].bull.claim}</span>
              </div>
            )}
            {summary.tierLines[0]?.bear?.claim && (
              <div className="text-[11px] leading-snug flex gap-1">
                <span style={{ color: "var(--sa-green, #16a34a)", fontWeight: 600 }}>
                  {t("stockAnalysis.compact.bear")}:
                </span>
                <span style={{ color: "var(--color-text-secondary)" }}>{summary.tierLines[0].bear.claim}</span>
              </div>
            )}
          </>
        )}
    </div>
  );
}
