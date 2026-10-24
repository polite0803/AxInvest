/**
 * 策略回测页签的分区标题：编号 + 标题 + 一句业务说明。
 *
 * 该页签由五段业务串成（数据基础 → 策略表现 → 信号追踪 → 漏检核查 → 趋势智选验证）。
 * 此前只是一串平铺的卡片、没有段落边界，入口与阅读顺序都不明显（2026-10-24 反馈）。
 */
export function SectionHeading({
  step,
  title,
  description,
}: {
  step: number;
  title: string;
  description: string;
}) {
  return (
    <div style={{ display: "flex", gap: 10, alignItems: "flex-start", marginTop: 4 }}>
      <span
        style={{
          flex: "0 0 auto",
          width: 20,
          height: 20,
          borderRadius: 10,
          background: "var(--sa-blue, #1677ff)",
          color: "#fff",
          fontSize: 12,
          fontWeight: 600,
          display: "inline-flex",
          alignItems: "center",
          justifyContent: "center",
          marginTop: 1,
        }}
        aria-hidden
      >
        {step}
      </span>
      <div>
        <div style={{ fontSize: 13, fontWeight: 600, lineHeight: 1.3 }}>{title}</div>
        <div style={{ fontSize: 11, color: "var(--color-text-tertiary)", marginTop: 2 }}>
          {description}
        </div>
      </div>
    </div>
  );
}
