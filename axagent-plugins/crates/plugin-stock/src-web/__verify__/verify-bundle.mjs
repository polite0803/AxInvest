// B2 动态验证：在 jsdom 里加载构建出的 bundle.js，mock 宿主 postMessage。
// 两个场景各跑一遍 JSDOM：
//   ①有数据：list/weights 回执带真实行 ⇒ 断言三条级联命令的入参取自真实回执
//     （generatedAt/period/strategyId 非空且逐字等于源值），并断言全帧无空串入参；
//   ②无数据：list=[]、weights={} ⇒ 断言三条级联命令零帧发出、渲染「未发调用」，
//     与「本环境不提供该数据」（真实空回执）两种文案不得混淆。
// 公共断言：入口发且仅发 1 帧 skill:ready；10 条命令全部经 plugin_execute_command 通道；
// 有数据场景渲染满 10 个 section；bundle 含 beforeunload+dispose 卸载钩子（源码级）。
// 本脚本不进插件产物（位于 src-web/__verify__/，不在 web/ 树内）。
import { JSDOM } from "jsdom";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const bundlePath = path.resolve(__dirname, "../../web/pages/bundle.js");
const bundle = fs.readFileSync(bundlePath, "utf8");

let failures = 0;
function check(cond, msg) {
  if (cond) {
    console.log("  PASS " + msg);
  } else {
    failures += 1;
    console.log("  FAIL " + msg);
  }
}

/**
 * 跑一个场景。receiptFor(frame) 返回宿主 plugin_execute_command 的回执
 * （JSON 文本字符串，与 plugin.rs 的 Result<String,String> 同形）。
 */
async function runScenario(name, receiptFor) {
  console.log(`\n=== 场景：${name} ===`);
  const dom = new JSDOM(
    '<!doctype html><html><body><div id="root"></div></body></html>',
    { runScripts: "outside-only", url: "http://localhost" },
  );
  const { window } = dom;

  // 捕获沙箱 -> 宿主的帧。jsdom 里 window.parent === window，覆盖 postMessage 后
  // bundle 发出的帧不会自动投递成 message 事件（真实浏览器会），故在这里手动模拟宿主：
  // 收到 rpc:request 就按 receiptFor 回投成功帧，source 设为宿主文档身份（= window.parent）。
  const posted = [];
  const hostWindow = window.parent; // 客户端的期望来源 = window.parent，回帧 source 必须同一对象
  window.parent.postMessage = (frame, targetOrigin) => {
    posted.push({ frame, targetOrigin });
    if (frame && frame.type === "rpc:request") {
      const response = {
        type: "rpc:response",
        callId: frame.callId,
        result: receiptFor(frame.args?.args?.name ?? ""),
      };
      setTimeout(() => {
        const event = new window.MessageEvent("message", { data: response });
        // source 是原型上的只读 getter，Object.assign 覆盖不了，必须 defineProperty。
        Object.defineProperty(event, "source", { value: hostWindow });
        window.dispatchEvent(event);
      }, 0);
    }
  };

  // 加载 bundle（顶层创建客户端 + announceReady + 触发 main）。
  window.eval(bundle);

  // 等待所有异步 RPC 结算（级联三条依赖第一轮回执，链路两轮 setTimeout 0）。
  await new Promise((resolve) => setTimeout(resolve, 300));

  return { posted, root: window.document.getElementById("root") };
}

const READY_TEXT = "本环境不提供该数据";
const SKIP_TEXT = "未发调用";
const DETAIL_TS = "2026-11-17T02:00:00Z";

function assertCommon({ posted, root }, expectedNames) {
  check(
    posted.filter((p) => p.frame && p.frame.type === "skill:ready").length === 1,
    "顶层发出且仅发出 1 帧 skill:ready",
  );
  check(
    posted
      .filter((p) => p.frame && p.frame.type === "skill:ready")
      .every((p) => p.targetOrigin === "*"),
    "ready 帧 targetOrigin 恒为 '*'（沙箱 opaque origin 唯一可行值）",
  );
  const reqFrames = posted.filter((p) => p.frame && p.frame.type === "rpc:request");
  const names = reqFrames.map((p) => p.frame.args?.args?.name);
  for (const name of expectedNames) {
    check(names.includes(name), `发出的命令包含 ${name}`);
  }
  check(
    reqFrames.every((p) => p.frame.args?.command === "plugin_execute_command"),
    "全部 rpc:request 的 envelope.command 都是 plugin_execute_command（走对通道）",
  );
  check(
    root.querySelectorAll("section").length === 10,
    `渲染 10 个 section（实际 ${root.querySelectorAll("section").length}）`,
  );
  return reqFrames;
}

// ── 场景一：库里有真实荐股记录 + 权重台账 ──
const withData = await runScenario("有数据（list 1 行 mid + weights {trend_mid,capital_long}）", (name) => {
  if (name === "stock_list_reco_history") {
    return JSON.stringify([
      { generatedAt: DETAIL_TS, period: "mid", stockCount: 3, styles: "trend", createdAt: DETAIL_TS },
    ]);
  }
  if (name === "stock_get_reco_strategy_weights") {
    return JSON.stringify({ trend_mid: 0.8, capital_long: 1.0 });
  }
  return JSON.stringify(null);
});

{
  const reqFrames = assertCommon(withData, [
    "stock_reflection_stats",
    "stock_reco_ic_stats",
    "stock_get_reco_strategy_weights",
    "stock_get_market_status",
    "stock_get_vendor_fallback_log",
    "stock_list_reco_history",
    "stock_list_stock_tools",
    "stock_get_reco_detail",
    "stock_get_cached_recommendation",
    "stock_get_reco_signal_history",
  ]);
  const frameOf = (name) => reqFrames.find((p) => p.frame.args?.args?.name === name)?.frame;

  console.log("  -- 级联入参取自真实回执 --");
  const detailInput = frameOf("stock_get_reco_detail")?.args?.args?.input;
  check(
    detailInput?.generatedAt === DETAIL_TS,
    `stock_get_reco_detail 的 generatedAt 取自 list 首行（实际 ${JSON.stringify(detailInput)}）`,
  );
  const cachedInput = frameOf("stock_get_cached_recommendation")?.args?.args?.input;
  check(
    cachedInput?.period === "mid",
    `stock_get_cached_recommendation 的 period 取自 list 首行档位（实际 ${JSON.stringify(cachedInput)}）`,
  );
  const signalInput = frameOf("stock_get_reco_signal_history")?.args?.args?.input;
  check(
    signalInput?.strategyId === "trend",
    `stock_get_reco_signal_history 的 strategyId 切自 weights 键 trend_mid（实际 ${JSON.stringify(signalInput)}）`,
  );

  console.log("  -- 全帧无空串入参（红灯回归断言）--");
  const emptyInputFrames = reqFrames.filter((p) => {
    const input = p.frame.args?.args?.input;
    if (input === undefined || input === null) { return false; }
    return Object.values(input).some((v) => v === "");
  });
  check(emptyInputFrames.length === 0, `没有任何一帧携带空串入参（实际 ${emptyInputFrames.length} 帧）`);

  const preTexts = Array.from(withData.root.querySelectorAll("pre")).map((pre) => pre.textContent ?? "");
  check(!preTexts.some((t) => t.includes(SKIP_TEXT)), "有数据场景不出现「未发调用」");
}

// ── 场景二：库里无记录、权重台账为空 ──
const noData = await runScenario("无数据（list=[]、weights={}）", (name) => {
  if (name === "stock_list_reco_history") {
    return JSON.stringify([]);
  }
  if (name === "stock_get_reco_strategy_weights") {
    return JSON.stringify({});
  }
  return JSON.stringify(null);
});

{
  const reqFrames = assertCommon(noData, [
    "stock_reflection_stats",
    "stock_reco_ic_stats",
    "stock_get_reco_strategy_weights",
    "stock_get_market_status",
    "stock_get_vendor_fallback_log",
    "stock_list_reco_history",
    "stock_list_stock_tools",
  ]);
  const names = reqFrames.map((p) => p.frame.args?.args?.name);
  for (const name of ["stock_get_reco_detail", "stock_get_cached_recommendation", "stock_get_reco_signal_history"]) {
    check(!names.includes(name), `无数据场景不发出 ${name}（零帧）`);
  }

  const preTexts = Array.from(noData.root.querySelectorAll("pre")).map((pre) => pre.textContent ?? "");
  const skipCount = preTexts.filter((t) => t.includes(SKIP_TEXT)).length;
  check(skipCount === 3, `恰好 3 个 section 显示「${SKIP_TEXT}」+原因（实际 ${skipCount}）`);
  const emptyCount = preTexts.filter((t) => t.includes(READY_TEXT)).length;
  check(emptyCount === 7, `7 条无参命令显示「${READY_TEXT}」（实际 ${emptyCount}）`);
}

console.log("\n=== ④ 卸载钩子 ===");
check(bundle.includes("beforeunload") && bundle.includes("dispose"), "bundle 含 beforeunload 卸载钩子并调 dispose()");

console.log(`\n结果：${failures === 0 ? "全部通过" : failures + " 项失败"}`);
process.exit(failures === 0 ? 0 : 1);
