import { useTimeAnchorStore } from "@/stores/feature/timeAnchorStore";
import { Modal, Tooltip, App } from "antd";
import { Clock, Zap } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { AsOfDatePicker } from "./AsOfDatePicker";
import { TimeAnchorTour } from "./TimeAnchorTour";

/**
 * ModeSwitch — L1 全局时间锚点 Pill（放在 AppHeader）
 *
 * 视觉规则（参考 design spec §10.1 HCI 4 层视觉信号）：
 *   - `live`      → 绿色脉冲点 + "LIVE" 标签（zap icon）
 *   - `replay`    → 橙色脉冲点 + "Replay YYYY-MM-DD"（clock icon）
 *   - `backtest_sweep` → 蓝色脉冲点 + "Sweep"（同 replay 视觉但加蓝条）
 *
 * 交互：
 *   - 点击 Pill → 展开/收起 AsOfDatePicker
 *   - Replay → Live 切换要求 Modal 二次确认
 */
export function ModeSwitch() {
  const { message } = App.useApp();
  const { t } = useTranslation();
  const mode = useTimeAnchorStore((s) => s.mode);
  const asOfDate = useTimeAnchorStore((s) => s.asOfDate);
  const enterReplay = useTimeAnchorStore((s) => s.enterReplay);
  const confirmPendingLive = useTimeAnchorStore((s) => s.confirmPendingLive);
  const cancelPendingLive = useTimeAnchorStore((s) => s.cancelPendingLive);
  const pendingLiveConfirm = useTimeAnchorStore((s) => s.pendingLiveConfirm);

  const [pickerOpen, setPickerOpen] = useState(false);
  const [confirmOpen, setConfirmOpen] = useState(false);

  const handleClick = () => {
    if (mode === "live") {
      setPickerOpen((v) => !v);
    } else {
      // replay/backtest → 请求切回 live（带 Modal 二次确认）
      setConfirmOpen(true);
    }
  };

  const onConfirmLive = () => {
    confirmPendingLive();
    setConfirmOpen(false);
    message.success(t("timeTravel.modeSwitch.switchedToLive"));
  };

  const onCancelLive = () => {
    cancelPendingLive();
    setConfirmOpen(false);
  };

  const isLive = mode === "live";
  const isSweep = mode === "backtest_sweep";

  const dotColor = isLive
    ? "var(--ax-success, #22c55e)"
    : isSweep
    ? "var(--ax-info, #3b82f6)"
    : "var(--ax-warning, #f59e0b)";

  const label = isLive
    ? t("timeTravel.modeSwitch.live")
    : isSweep
    ? t("timeTravel.modeSwitch.sweep")
    : t("timeTravel.modeSwitch.replay", { date: asOfDate ?? "" });

  return (
    <div style={{ position: "relative", display: "inline-flex", alignItems: "center", gap: 6 }}>
      <Tooltip title={isLive ? t("timeTravel.modeSwitch.tooltipLive") : t("timeTravel.modeSwitch.tooltipReplay")}>
        <button
          type="button"
          onClick={handleClick}
          aria-label={label}
          data-testid="mode-switch"
          style={{
            display: "inline-flex",
            alignItems: "center",
            gap: 6,
            padding: "3px 10px",
            height: 24,
            borderRadius: 12,
            border: `1px solid ${isLive ? "var(--ax-border, #e5e7eb)" : dotColor}`,
            background: isLive ? "var(--ax-bg-soft, #f9fafb)" : `${dotColor}1a`,
            color: isLive ? "var(--ax-text-secondary, #6b7280)" : dotColor,
            fontSize: 12,
            fontWeight: 600,
            cursor: "pointer",
            transition: "all 120ms ease",
          }}
        >
          <span
            style={{
              display: "inline-block",
              width: 6,
              height: 6,
              borderRadius: "50%",
              background: dotColor,
              boxShadow: `0 0 0 2px ${dotColor}33`,
              animation: "ax-pulse 1.8s ease-in-out infinite",
            }}
          />
          {isLive ? <Zap size={11} /> : <Clock size={11} />}
          {label}
        </button>
      </Tooltip>
      {pickerOpen && (
        <div
          style={{
            position: "absolute",
            top: "calc(100% + 6px)",
            right: 0,
            zIndex: 1000,
            background: "var(--ax-bg, #fff)",
            border: "1px solid var(--ax-border, #e5e7eb)",
            borderRadius: 8,
            padding: 12,
            boxShadow: "0 4px 16px rgba(0,0,0,0.08)",
          }}
        >
          <AsOfDatePicker
            onPick={(d) => {
              enterReplay(d);
              setPickerOpen(false);
              message.info(t("timeTravel.modeSwitch.enteredReplay", { date: d }));
            }}
            onCancel={() => setPickerOpen(false)}
          />
        </div>
      )}
      <Modal
        title={t("timeTravel.modeSwitch.confirmTitle")}
        open={confirmOpen || pendingLiveConfirm}
        onOk={onConfirmLive}
        onCancel={onCancelLive}
        okText={t("timeTravel.modeSwitch.confirmOk")}
        cancelText={t("timeTravel.modeSwitch.confirmCancel")}
      >
        <p>{t("timeTravel.modeSwitch.confirmBody")}</p>
        {asOfDate && (
          <p style={{ marginTop: 8, color: "var(--ax-text-tertiary, #6b7280)", fontSize: 12 }}>
            {t("timeTravel.modeSwitch.currentlyInReplay", { date: asOfDate })}
          </p>
        )}
      </Modal>
      <TimeAnchorTour />
    </div>
  );
}
