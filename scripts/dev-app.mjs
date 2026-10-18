#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-only
//
// `npm run dev:app` —— 本机开发启动入口，补上 WebView2 的环境参数。
//
// 为什么需要它（2026-10-18 实测，见 .workbuddy/memory/2026-10-18.md）：
//   本机渲染进程沙箱不可用（火绒 HipsDaemon 在跑）。沙箱被拦截时，渲染进程主线程
//   约每 1 秒才轮到一次任务 —— 每个 `await invoke()` 的回调都要排 1 秒，串三条就是
//   3.3~3.5 秒，界面上表现为「点导航假死」。同一台机器上，`--no-sandbox` 下
//   MessageChannel 自泵环从 5 次/5s 恢复到 178 万次/5s，`Slow call` 归零。
//
//   ⚠ `--no-sandbox` 会关闭渲染进程沙箱，**只允许用于本机开发**，不要带进发布包。
//   根治办法：把 axagent.exe / msedgewebview2.exe / 项目目录加入火绒信任区，
//   恢复沙箱后即可删掉本脚本，直接 `npm run tauri dev`。
//
// 已有 `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS` 时以外部值为准，本脚本不覆盖。

import { spawn } from "node:child_process";

const KEY = "WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS";
// `--remote-debugging-port=9333`：让 `npm run diag` 能在应用**运行中**连进 WebView2
// 做无侵入体检（主线程吞吐 / 帧率 / 长帧 / 导航耗时 / JS 堆）。
// 端口仅监听本机回环，且只在开发启动路径出现，不进发布包。
// ⚠ 9222 不要用：本机 LenovoVantage 常驻占用该端口。
const fallback = "--no-sandbox --remote-debugging-port=9333";

if (!process.env[KEY]) {
  process.env[KEY] = fallback;
  console.warn(
    `[dev-app] 已设置 ${KEY}="${fallback}"（本机 WebView2 渲染进程沙箱不可用的绕行方案）。\n`
      + "[dev-app] 这是开发期兜底，发布包不含此参数；把相关程序加入火绒信任区后可删除本脚本。",
  );
} else {
  console.log(`[dev-app] 沿用外部 ${KEY}="${process.env[KEY]}"`);
}

const child = spawn("npx", ["tauri", "dev"], {
  stdio: "inherit",
  shell: true,
  env: process.env,
});

child.on("exit", (code) => process.exit(code ?? 1));
child.on("error", (err) => {
  console.error(`[dev-app] 启动失败: ${err.message}`);
  process.exit(1);
});
