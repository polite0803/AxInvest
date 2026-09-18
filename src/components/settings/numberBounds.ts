/**
 * 数值型变量的滑杆量程与步长 —— 按默认值量级推断。
 *
 * 为什么不能写死 `0~100` + `step=1`（也不能写死 `0~2`）：
 * 策略参数里大量是 0~1.5 量级的比值/乘数（`trend_high_20_threshold = 0.97`、
 * `cap_kline_vol_ratio_min = 1.5`）。在 0~100 的滑杆上它们全部挤在最左端零点几像素内，
 * 再叠加 `step=1`，**拖动一下就从 0 跳到 1**；反过来若一律给 0~2，`min_confidence = 50`
 * 这类 0~100 口径的参数就废了。所以按默认值的**量级**分层，而不是按名字或描述猜。
 *
 * 量程规则（按默认值判断，不用当前值，避免拖动时量程抖动）：
 * - 负值 → 跨 0 对称量程（0 刻度滑杆无法表达负区间）
 * - 默认值 >100 → 上限抬到现值 1.5 倍（否则滑杆一拖就把参数往下改）
 * - 默认值 <1 → 0~2 窄量程 + 0.001 细步长（比例 / 权重 / 比值阈值）
 * - 名字属「计数类」→ 0~100 + step 1
 * - 默认值 1~3 → 0~10 + 0.001（入场/止损/目标乘数、标准差倍数）
 * - 其余 → 0~100 + 0.1（评分权重 / RSI 阈值 / 置信度 / DCF 贴现率）
 *
 * 滑杆外的兜底：调用方右侧的 `InputNumber` 不应设 `min`/`max`，任何被量程挡住的精确值
 * 都能直接键入，因此这里的启发式只影响「拖动手感」，不会锁死参数。
 *
 * 两条不变量（改动本函数后必须重新校验，脚本见 `output/tmp-verify-bounds.py`）：
 *   I1  `min <= 默认值 <= max` —— 上限低于现值会让滑杆一拖就把参数往下改
 *   I2  `(默认值 - min) / step` 为整数 —— 现值在步长网格上，否则显示与写回不一致
 */
import type { Variable } from "@/components/workflow/types";

export function inferNumberBounds(v: Variable): { min: number; max: number; step: number } {
  // 温度类参数需要 0.1 步进，用变量名判断更可靠
  if (v.name === "agent_temperature") { return { min: 0, max: 2, step: 0.1 }; }

  const raw = Number(v.value ?? 0);
  if (!Number.isFinite(raw)) { return { min: 0, max: 100, step: 1 }; }

  // ① 负值 → 跨 0 对称量程。0 刻度滑杆无法表达负区间（如 cap_kline_mom_5_min = -0.02、
  //    earnings_th_strong_neg = -20）。
  if (raw < 0) {
    const span = Math.max(0.1, Math.abs(raw) * 2);
    return { min: -span, max: span, step: span / 100 };
  }

  // ② 默认值本身已超出 0~100（kline_limit=120、agent_timeout_secs=600、
  //    agent_max_tokens=32768、budget_high_max=100000）。上限必须 ≥ 现值，否则滑杆一拖
  //    就把参数往下改 —— 比「调不动」更糟。step 取 1：32768 这类 2 的幂对齐值不在
  //    10/100 网格上。
  if (raw > 100) {
    return { min: 0, max: Math.ceil((raw * 1.5) / 10) * 10, step: 1 };
  }

  // ③ 默认值 < 1 的比例 / 权重 / 比值阈值（新增参数的主体：0.97 / 0.3 / 0.003 …）
  //    → 0~2 窄量程 + 0.001 细步长。注意这里**不能**用「描述里有没有 %」判别：
  //    cost_pct = 0.003（=0.3%）也带 %，但它的量级必须走 0.001 步长。
  if (raw < 1) { return { min: 0, max: 2, step: 0.001 }; }

  // ④ 计数类（天数 / 周期 / 条数 / 轮数 / 秒数 / 重试次数 / 触碰次数 …）→ 0~100 + step 1
  const COUNT_LIKE =
    /(_days|_period|_limit|_len|_count|_rounds|_depth|_tokens|_secs|_retry|_window|_concurrent|_positions|_touches|_max)$/;
  if (COUNT_LIKE.test(v.name)) { return { min: 0, max: 100, step: 1 }; }

  // ⑤ 1~3 的乘数（入场 ×1.03 / 止损 ×0.93 / 目标 ×1.3 / 布林 ×2 标准差）→ 0~10 + 0.001
  //    （0.001 是为兼容 *_ultra_short_entry_* = 1.005 / 0.998 这类三位小数）
  if (raw <= 3) { return { min: 0, max: 10, step: 0.001 }; }

  // ⑥ 其余（评分权重 30、RSI 阈值 70、置信度 50、F-Score 7、DCF 贴现率 8.5 …）
  //    → 0~100 + 0.1。用 0.1 而非 1，是为了让 value_dcf_discount_rate = 8.5
  //    这类带小数的百分比落在步长网格上。
  return { min: 0, max: 100, step: 0.1 };
}
