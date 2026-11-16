// B2 动态验证：在 jsdom 里加载构建出的 bundle.js，mock 宿主 postMessage。
// 断言：①入口发了 ready 帧；②10 条 stock_* 命令各经 plugin_execute_command 发一次；
// ③browser-mock 风格空数据下渲染出「本环境不提供该数据」；④卸载钩子存在（源码级）。
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

const dom = new JSDOM(
  '<!doctype html><html><body><div id="root"></div></body></html>',
  { runScripts: "outside-only", url: "http://localhost" },
);
const { window } = dom;

// 捕获沙箱 -> 宿主的帧。jsdom 里 window.parent === window，覆盖 postMessage 后
// bundle 发出的帧不会自动投递成 message 事件（真实浏览器会），故在这里手动模拟宿主：
// 收到 rpc:request 就回投一个成功帧到 window，source 设为宿主文档身份（= window.parent）。
const posted = [];
const hostWindow = window.parent; // 客户端的期望来源 = window.parent，回帧 source 必须同一对象
window.parent.postMessage = (frame, targetOrigin) => {
  posted.push({ frame, targetOrigin });
  if (frame && frame.type === "rpc:request") {
    // 宿主 plugin_execute_command 的返回是 JSON 文本字符串；browser-mock 十条皆空 ⇒ "null"/"[]"/"{}"
    const response = { type: "rpc:response", callId: frame.callId, result: JSON.stringify(null) };
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

// 等待所有异步 RPC 结算（10 条，每条一次 setTimeout 0 回帧）。
await new Promise((resolve) => setTimeout(resolve, 200));

console.log("=== ① ready 帧 ===");
const readyFrames = posted.filter((p) => p.frame && p.frame.type === "skill:ready");
check(readyFrames.length === 1, `顶层发出且仅发出 1 帧 skill:ready（实际 ${readyFrames.length}）`);
check(
  readyFrames.every((p) => p.targetOrigin === "*"),
  "ready 帧 targetOrigin 恒为 '*'（沙箱 opaque origin 唯一可行值）",
);

console.log("=== ② 10 条命令经 plugin_execute_command 各发一次 ===");
const reqFrames = posted.filter((p) => p.frame && p.frame.type === "rpc:request");
const names = reqFrames.map((p) => p.frame.args?.args?.name);
const expected = [
  "stock_reflection_stats",
  "stock_reco_ic_stats",
  "stock_get_reco_strategy_weights",
  "stock_get_market_status",
  "stock_get_vendor_fallback_log",
  "stock_list_reco_history",
  "stock_get_reco_detail",
  "stock_get_reco_signal_history",
  "stock_get_cached_recommendation",
  "stock_list_stock_tools",
];
check(reqFrames.length === 10, `共发出 10 帧 rpc:request（实际 ${reqFrames.length}）`);
check(
  reqFrames.every((p) => p.frame.args?.command === "plugin_execute_command"),
  "10 帧的 envelope.command 都是 plugin_execute_command（走对通道，未用白名单外直接 invoke）",
);
for (const name of expected) {
  check(names.filter((n) => n === name).length === 1, `${name} 恰好发一次`);
}

console.log("=== ③ 空数据渲染显式文案 ===");
const root = window.document.getElementById("root");
const sections = root.querySelectorAll("section");
check(sections.length === 10, `渲染 10 个 section（实际 ${sections.length}）`);
const emptyText = "本环境不提供该数据";
const emptyCount =
  Array.from(root.querySelectorAll("pre")).filter((pre) => pre.textContent?.includes(emptyText)).length;
check(emptyCount === 10, `10 个 <pre> 都显示「${emptyText}」（实际 ${emptyCount}）`);

console.log("=== ④ 卸载钩子 ===");
check(bundle.includes("beforeunload") && bundle.includes("dispose"), "bundle 含 beforeunload 卸载钩子并调 dispose()");

console.log(`\n结果：${failures === 0 ? "全部通过" : failures + " 项失败"}`);
process.exit(failures === 0 ? 0 : 1);
