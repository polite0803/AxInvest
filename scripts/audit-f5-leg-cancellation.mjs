// 300642 透景生命（2026-09-22 运行，template_version=76）反事实精算
//
// 目的：把「三条估值腿互相抵消 ⇒ f5 ≈ 0 ⇒ 结论 = 先验」的定性判断，量化到最终决策字段。
//
// 数据来源（全部来自 stock_analyses / decision_json 实测，非推测）：
//   · 12 因子 σ/w —— decision_json.evidence.factors
//   · prior=0.5 / totals —— decision_json.evidence + computation_logs
//   · DCF: low=24.76 mid=40.53 high=71.89 upsidePct=+92.8（t-valuation.dcf）
//   · graham: intrinsicValue=3.35 upsidePct=−84.0（t-valuation.graham）
//   · PE 分位: current=277.2 currentPercentile=99.9557（t-valuation-band.metricPe）
//   · risk_bias = −0.08（posteriorRaw 51.4 → posterior 43.4）
//   · action 阶梯 —— decision_json.effective_params

const F = {
  f1: { name: "trend", s: 0.3, w: 0.15 },
  f2: { name: "consensus", s: -0.3, w: 0.25 },
  f3: { name: "catalyst", s: 0.335, w: 0.2 },
  f4: { name: "risk", s: -0.328, w: 0.15 },
  f5: { name: "valuation", s: 0.038, w: 0.165 },
  f6: { name: "data_quality", s: 0.0, w: 0.15 },
  f7: { name: "trade_signal", s: 0.0, w: 0.1 },
  f9: { name: "money_flow", s: -0.381, w: 0.09 },
  f10: { name: "chip", s: -0.076, w: 0.08 },
  f11: { name: "pace", s: 0.29, w: 0.052 },
  f12: { name: "momentum", s: 0.9, w: 0.075 },
  f13: { name: "bottleneck", s: 0.0, w: 0.0 },
};

const PRIOR = 0.5;
const MAXW_DEFAULT = 1.49; // rhai: 0.15+0.25+0.20+0.15+0.15+0.15+0.10+0.08+0.08+0.08+0.10
const RISK_BIAS = -0.08; // 高风险档

// portfolio_formula::compute_evidence_scale 逐字复刻
function evidenceScale(totalW, maxW) {
  if (totalW < 0.3 || maxW <= 0.0) return 0.2;
  return 0.1 + 0.45 * Math.sqrt(Math.min(totalW / maxW, 1.0));
}
// rhai: fn pm_saturate(r, k) = r / (|r| + k)
const saturate = (r, k) => r / (Math.abs(r) + k);

// action 阶梯（effective_posterior 落档）
const T = { buy: 0.63, increase: 0.53, hold: 0.48, watch: 0.38, reduce: 0.3 };
function action(eff) {
  if (eff >= T.buy) return "买入";
  if (eff >= T.increase) return "增持";
  if (eff >= T.hold) return "持有";
  if (eff >= T.watch) return "观望";
  if (eff >= T.reduce) return "减持";
  return "卖出";
}

// f5 三腿分解（V79 口径，权重 0.56/0.24/0.20）
const DCF_UP = 92.8, GRAHAM_UP = -84.0, PE_PCT = 99.955713020372;
const FSCORE_MULT = 1.3; // 复现 DB f5=0.038 时反解所需的乘子
const MOAT_MULT = 1.0;
const dcfSig = saturate(DCF_UP, 40.0);
const ghSig = saturate(GRAHAM_UP, 40.0);
const bandSig = Math.max(-1, Math.min(1, (50.0 - PE_PCT) / 50.0));

function fuseF5({ dcf = true, graham = true, band = true, fscore = FSCORE_MULT, moat = MOAT_MULT }) {
  const wsum = (dcf ? 0.56 : 0) + (graham ? 0.24 : 0) + (band ? 0.2 : 0);
  if (wsum <= 0) return 0;
  const raw = (dcf ? dcfSig * 0.56 : 0) + (graham ? ghSig * 0.24 : 0) + (band ? bandSig * 0.2 : 0);
  return Math.max(-1, Math.min(1, (raw / wsum) * fscore * moat));
}

function run(label, opts = {}) {
  const factors = { ...F };
  if (opts.f5sig !== undefined) factors.f5 = { ...factors.f5, s: opts.f5sig };
  let list = Object.values(factors);
  if (opts.dropZero) list = list.filter((f) => !(f.w > 0 && f.s === 0));
  const totalW = list.reduce((a, f) => a + f.w, 0);
  const weighted = list.reduce((a, f) => a + f.w * f.s, 0);
  const avg = totalW > 0 ? weighted / totalW : 0;
  const scale = evidenceScale(totalW, MAXW_DEFAULT);
  const post = Math.max(0, Math.min(1, PRIOR + avg * scale));
  const eff = Math.max(0, Math.min(1, post + RISK_BIAS));
  return {
    label,
    f5: factors.f5.s.toFixed(4),
    avg: avg.toFixed(4),
    post: post.toFixed(4),
    eff: eff.toFixed(4),
    act: action(eff),
  };
}

const sig = (f5) => fuseF5(f5);

const rows = [
  run("[0] 现状（DB 存档）", {}),
  run("[1] f5=仅 DCF 腿（剔 graham + band）", { f5sig: sig({ graham: false, band: false }) }),
  run("[2] f5=DCF+graham（剔 band，两腿归一）", { f5sig: sig({ band: false }) }),
  run("[3] f5=三腿但 band 权重归 0 等价 [2]", { f5sig: sig({ band: false }) }),
  run("[4] f5=仅 DCF+band（剔 graham）", { f5sig: sig({ graham: false }) }),
  run("[5] f5 理论满格 +1.0", { f5sig: 1.0 }),
  run("[6] f5 中性 0（估值完全弃权）", { f5sig: 0.0 }),
  run("[7] 现状 + 剔除 f6/f7 两个零信号因子", { dropZero: true }),
  run("[8] 现状 + f5 仅 DCF + 剔 f6/f7", { f5sig: sig({ graham: false, band: false }), dropZero: true }),
  run("[9] 天花板：所有负信号归零", {}),
];

console.log("=== f5 三腿分解（DB 实测输入）===");
console.log(`DCF      upside=+${DCF_UP}%  σ_leg=${saturate(DCF_UP, 40).toFixed(4)}  w=0.56  → 贡献 ${(dcfSig * 0.56).toFixed(4)}`);
console.log(`graham   upside=${GRAHAM_UP}%  σ_leg=${ghSig.toFixed(4)}  w=0.24  → 贡献 ${(ghSig * 0.24).toFixed(4)}`);
console.log(`PE 分位  pct=${PE_PCT.toFixed(4)}  σ_leg=${bandSig.toFixed(4)}  w=0.20  → 贡献 ${(bandSig * 0.2).toFixed(4)}`);
console.log(`raw 合计 = ${(dcfSig * 0.56 + ghSig * 0.24 + bandSig * 0.2).toFixed(4)}  wsum=1.0`);
console.log(`×fscore(${FSCORE_MULT}) ×moat(${MOAT_MULT}) ⇒ f5 σ = ${sig({}).toFixed(4)}   [DB 实测 0.038]`);
console.log();

console.log("=== 反事实：f5 口径 → 最终决策 ===");
console.log("场景".padEnd(42) + "f5σ".padEnd(10) + "avgSig".padEnd(10) + "post".padEnd(9) + "eff".padEnd(9) + "action");
for (const r of rows) {
  console.log(
    r.label.padEnd(38) + r.f5.padEnd(10) + r.avg.padEnd(10) + r.post.padEnd(9) + r.eff.padEnd(9) + r.act,
  );
}

// 天花板：全部负信号 → 0，正信号保留
const posOnly = Object.values(F).map((f) => ({ ...f, s: f.s < 0 ? 0 : f.s }));
const tw = posOnly.reduce((a, f) => a + f.w, 0);
const ws = posOnly.reduce((a, f) => a + f.w * f.s, 0);
{
  const avg = ws / tw;
  const post = Math.max(0, Math.min(1, PRIOR + avg * evidenceScale(tw, MAXW_DEFAULT)));
  const eff = Math.max(0, Math.min(1, post + RISK_BIAS));
  console.log();
  console.log(`天花板（所有 σ<0 归零，f5 保持 0.038）: avgSig=${avg.toFixed(4)} post=${post.toFixed(4)} eff=${eff.toFixed(4)} → ${action(eff)}`);
}
{
  const p = Object.values(F).map((f) => ({ ...f, s: f.s < 0 ? 0 : f.s, ...(f.name === "valuation" ? { s: 1.0 } : {}) }));
  const tw2 = p.reduce((a, f) => a + f.w, 0);
  const ws2 = p.reduce((a, f) => a + f.w * f.s, 0);
  const avg = ws2 / tw2;
  const post = Math.max(0, Math.min(1, PRIOR + avg * evidenceScale(tw2, MAXW_DEFAULT)));
  const eff = Math.max(0, Math.min(1, post + RISK_BIAS));
  console.log(`天花板（负信号全归零 + f5 拉满 +1.0）: avgSig=${avg.toFixed(4)} post=${post.toFixed(4)} eff=${eff.toFixed(4)} → ${action(eff)}`);
}

// 买入档可达性
const needAvg = (T.buy - RISK_BIAS - PRIOR) / evidenceScale(1.462, MAXW_DEFAULT);
console.log();
console.log(`要达到「买入」需 avgSignal ≥ ${needAvg.toFixed(4)}（当前 0.0254，差 ${(needAvg / 0.0254).toFixed(1)} 倍）`);
