import { type DegradationSummary, summarizeDegradations, useTimeAnchorStore } from "@/stores/feature/timeAnchorStore";
import { DatePicker, Segmented, Space, Tag, Tooltip } from "antd";
import dayjs, { type Dayjs } from "dayjs";
import { AlertTriangle, Clock, Zap } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";

/** 严重度 → AntD Tag 颜色：只有真故障才是红/橙，结构性不适用退回中性灰 */
const DEGRADATION_TAG_COLOR: Record<"failure" | "noData" | "structuralGap", string> = {
  failure: "red",
  noData: "orange",
  structuralGap: "default",
};

/**
 * Tooltip 按档分节（每档最多列 5 条，避免长列表撑爆浮层）。
 * 节标题走 i18n `degradedMarker.kindLabels.*`；条目仍是 `method: reason`，
 * 因为 reason 已由后端 `AsofProbe::reason` 写成完整可行动的一句话。
 */
function renderDegradationTooltip(summary: DegradationSummary, t: (k: string) => string) {
  if (summary.grouped.length === 0) {
    return t("timeTravel.degradedMarker.tooltip");
  }
  return (
    <div style={{ whiteSpace: "pre-line", maxWidth: 420 }}>
      {summary.grouped.map((g) => (
        <div key={g.kind} style={{ marginBottom: 6 }}>
          <div style={{ fontWeight: 600 }}>
            {t(`timeTravel.degradedMarker.kindLabels.${g.kind}`)} · {g.items.length}
          </div>
          {g.items.slice(0, 5).map((e) => (
            <div key={`${e.method}-${e.reason}`}>
              {e.method}: {e.reason}
            </div>
          ))}
        </div>
      ))}
    </div>
  );
}

/**
 * PageTimeAnchor — 页面级时间锚点(嵌入 `sa-header`)
 *
 * spec §9.2:StockAnalysisPage 顶部加 2 个组件:
 *   - `Segmented` 切换:`实时分析` | `历史回放` 二选一
 *   `DatePicker`(仅 replay 显示):与 AsOfDatePicker 共用约束
 *
 * 与全局 `ModeSwitch`(AppHeader Pill)共享 `timeAnchorStore`,
 * 模式改变会同步触发其他页面的回放遮罩 / 角标。
 */
export function PageTimeAnchor() {
  const { t } = useTranslation();
  const mode = useTimeAnchorStore((s) => s.mode);
  const asOfDate = useTimeAnchorStore((s) => s.asOfDate);
  const enterReplay = useTimeAnchorStore((s) => s.enterReplay);
  const enterLive = useTimeAnchorStore((s) => s.enterLive);
  const degradationCount = useTimeAnchorStore((s) => s.degradationCount);
  const degradationLog = useTimeAnchorStore((s) => s.degradationLog);
  const summary = summarizeDegradations(degradationLog);

  const [pending, setPending] = useState<Dayjs | null>(null);
  // 用户点击"回放"但尚未选日期时，显示 DatePicker 让用户选择
  const [showPicker, setShowPicker] = useState(false);

  const isLive = mode === "live";
  const isReplay = mode === "replay" || mode === "backtest_sweep" || showPicker;

  const today = dayjs();
  const disabledDate = (d: Dayjs) => d.isSame(today) || d.isAfter(today);

  const onChangeMode = (v: string | number) => {
    if (v === "live") {
      if (!isLive) {
        enterLive();
      }
      setShowPicker(false);
    } else {
      // 切到 replay:若已有 as_of_date 立即生效,否则显示 DatePicker 让用户选
      if (!asOfDate && !pending) {
        setShowPicker(true);
        return;
      }
      const date = (pending?.format("YYYY-MM-DD")) ?? asOfDate;
      if (date) {
        enterReplay(date);
      }
    }
  };

  const onPickDate = (d: Dayjs | null) => {
    setPending(d);
    if (d) {
      setShowPicker(false);
      enterReplay(d.format("YYYY-MM-DD"));
    }
  };

  const onCancelPicker = () => {
    setPending(null);
    setShowPicker(false);
  };

  return (
    <Space size="small" data-testid="page-time-anchor">
      <Segmented
        size="small"
        value={isLive ? "live" : "replay"}
        onChange={onChangeMode}
        data-testid="time-anchor-segmented"
        options={[
          {
            label: (
              <span style={{ display: "inline-flex", alignItems: "center", gap: 4 }}>
                <Zap size={11} />
                {t("timeTravel.pageAnchor.live")}
              </span>
            ),
            value: "live",
          },
          {
            label: (
              <span style={{ display: "inline-flex", alignItems: "center", gap: 4 }}>
                <Clock size={11} />
                {t("timeTravel.pageAnchor.replay")}
              </span>
            ),
            value: "replay",
          },
        ]}
      />
      {isReplay && (
        <>
          <DatePicker
            size="small"
            value={pending ?? (asOfDate ? dayjs(asOfDate) : null)}
            onChange={onPickDate}
            disabledDate={disabledDate}
            format="YYYY-MM-DD"
            allowClear={false}
            placeholder={t("timeTravel.datePicker.placeholder")}
            style={{ width: 150 }}
            data-testid="asof-date-picker"
          />
          {showPicker && !asOfDate && (
            <button
              type="button"
              onClick={onCancelPicker}
              style={{
                background: "none",
                border: "none",
                cursor: "pointer",
                color: "var(--text-tertiary)",
                fontSize: 12,
                padding: "0 4px",
              }}
              title={t("common.cancel")}
            >
              ✕
            </button>
          )}
          {asOfDate && (
            <Tag color="purple" data-testid="page-time-anchor-tag">
              ⏪ {t("timeTravel.pageAnchor.untilDate", { date: asOfDate })}
            </Tag>
          )}
          {
            /* 缺陷 E 修复: replay 模式下显示具体降级计数。
              实时通过 timeAnchorStore 拉取(每 3s 一次),数字就是"被跳过的方法数"。
              0 时不显示,避免噪声。
              T14(2026-09-27): 颜色取「最重的一档」——只有结构性不适用时不再标橙，
              否则「个股没有场内期权」和「接口 301 挂了」看起来是同一件事。 */
          }
          {isReplay && asOfDate && degradationCount > 0 && (
            <Tooltip title={renderDegradationTooltip(summary, t)}>
              <Tag
                color={DEGRADATION_TAG_COLOR[summary.worst ?? "failure"]}
                icon={<AlertTriangle size={11} />}
                data-testid="page-time-anchor-degraded"
              >
                {t("timeTravel.degradedMarker.labelWithCount", {
                  n: degradationCount,
                })}
              </Tag>
            </Tooltip>
          )}
        </>
      )}
    </Space>
  );
}
