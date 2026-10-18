#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-only
//
// `npm run diag` —— 应用卡顿时的一键体检。
//
// 前提：应用以 `npm run dev:app` 启动（该脚本会挂 `--remote-debugging-port=9333`）。
// 通过 CDP 连进**真实运行中**的 WebView2，量五件事，各自对应一类不同的病因：
//
//   1. 主线程调度吞吐 —— 区分「主线程被节流/占满」与「其实空闲」
//   2. rAF 帧率        —— 区分「合成器/GPU 不出帧」与「渲染正常」
//   3. 长动画帧         —— 区分「有卡帧（含 style/layout 相位）」与「无卡帧」
//   4. 导航/tab 切换     —— 端到端体验值（含真实 invoke 后端调用）
//   5. 渲染进程内存      —— 长期运行后是否膨胀到 GB 级
//
// 用法:
//   npm run diag                 # 全套
//   npm run diag -- --quick      # 只测 1/2/3（不点导航，不打扰当前页面）
//
// ⚠ 只读：不修改应用状态、不写库、不点非导航元素。

import { chromium } from "playwright";

const EP = process.env.AXAGENT_CDP || "http://127.0.0.1:9333";
const QUICK = process.argv.includes("--quick");

const line = (s) => console.log(s);
const pad = (s, n) => String(s).padEnd(n);

async function main() {
  let browser;
  try {
    browser = await chromium.connectOverCDP(EP);
  } catch (e) {
    line(`❌ 连不上 CDP ${EP}`);
    line("   请用 `npm run dev:app` 启动应用（该脚本会带 --remote-debugging-port=9333）。");
    line(`   ${String(e).slice(0, 160)}`);
    process.exit(1);
  }

  const ctx = browser.contexts()[0];
  let page = ctx?.pages().find((p) => !p.url().startsWith("devtools://"));
  if (!page) {
    line("❌ 没有找到应用页面");
    await browser.close().catch(() => {});
    process.exit(1);
  }

  line(`已连接: ${page.url()}\n`);

  // ── 1. 主线程调度吞吐 ──
  const pump = await page.evaluate(() => new Promise((resolve) => {
    let n = 0;
    const ch = new MessageChannel();
    ch.port1.onmessage = () => { n++; ch.port2.postMessage(0); };
    ch.port2.postMessage(0);
    setTimeout(() => resolve(n), 4000);
  }));
  // ⚠ 量具局限：MessageChannel 自泵对「CDP 已附着 + Performance 域已启用」很敏感，
  //   同一台机器上干净态实测可达 124 万/5s，附着后降到 13 万/4s，而帧率与点击响应
  //   完全不变（2026-10-18 实测）。因此**只把 <5 万的量级当作节流信号**，
  //   3 位数万级不构成结论 —— 判慢要看 2)/3)/5)。
  const pumpVerdict = pump > 50000 ? "正常（不作为判据）" : "严重节流";
  line(`1) 主线程自泵 4s = ${pump.toLocaleString()} 次  → ${pumpVerdict}`);
  line(`   （仅 ≲5万 有意义：说明主线程被压到 ~1s 才轮到一次任务；万级受 CDP 采样影响，勿据此判慢）`);

  // ── 2. rAF 帧率 ──
  const raf = await page.evaluate(() => new Promise((resolve) => {
    const ts = [];
    const t0 = performance.now();
    const tick = (t) => {
      ts.push(t);
      if (performance.now() - t0 < 3000) requestAnimationFrame(tick);
      else {
        const gaps = [];
        for (let i = 1; i < ts.length; i++) gaps.push(ts[i] - ts[i - 1]);
        gaps.sort((a, b) => a - b);
        resolve({
          frames: ts.length,
          fps: +(ts.length / ((performance.now() - t0) / 1000)).toFixed(1),
          p50: +(gaps[Math.floor(gaps.length * 0.5)] ?? 0).toFixed(1),
          p95: +(gaps[Math.floor(gaps.length * 0.95)] ?? 0).toFixed(1),
        });
      }
    };
    requestAnimationFrame(tick);
  }));
  line(`2) rAF 3s = ${raf.frames} 帧 / ${raf.fps} fps（帧间隔 p50=${raf.p50}ms p95=${raf.p95}ms）→ ${raf.fps > 40 ? "健康" : "掉帧严重"}`);

  // ── 3. 长动画帧 ──
  const loaf = await page.evaluate(() => new Promise((resolve) => {
    const out = [];
    let ob;
    try {
      ob = new PerformanceObserver((l) => {
        for (const e of l.getEntries()) {
          out.push({
            d: Math.round(e.duration),
            styleLayout: Math.round((e.styleAndLayoutStart || 0) - (e.renderStart || 0)),
            blocking: Math.round(e.blockingDuration || 0),
          });
        }
      });
      ob.observe({ type: "long-animation-frame" });
    } catch { return resolve(null); }
    setTimeout(() => { try { ob.disconnect(); } catch {} resolve(out); }, 5000);
  }));
  if (!loaf) line("3) long-animation-frame 不支持（WebView2 版本过旧）");
  else {
    const tot = loaf.reduce((s, e) => s + e.d, 0);
    line(`3) 长动画帧 5s = ${loaf.length} 条，合计 ${tot}ms${loaf.length ? "  明细: " + JSON.stringify(loaf.slice(0, 5)) : ""} → ${loaf.length === 0 ? "无卡帧" : "有卡帧"}`);
  }

  // ── 4. 渲染进程内存 ──
  try {
    const cdp = await ctx.newCDPSession(page);
    await cdp.send("Performance.enable");
    const { metrics } = await cdp.send("Performance.getMetrics");
    const g = (n) => metrics.find((x) => x.name === n)?.value ?? -1;
    const heapMB = +(g("JSHeapUsedSize") / 1048576).toFixed(1);
    const nodes = g("Nodes");
    const listeners = g("JSEventListeners");
    line(`4) JS 堆 ${heapMB} MB ｜ DOM 节点 ${nodes} ｜ 事件监听器 ${listeners} → ${heapMB > 600 ? "偏大（关注泄漏）" : "正常"}`);
    await cdp.detach().catch(() => {});
  } catch (e) {
    line(`4) 内存读取失败: ${String(e).slice(0, 100)}`);
  }

  if (QUICK) {
    await browser.close().catch(() => {});
    return;
  }

  // ── 5. 导航切换 ──
  line("\n5) 导航切换（真实后端）");
  const items = await page.$$eval("button.nav-item, button.nav-item-expanded", (els) =>
    els.map((e, i) => ({ i, label: (e.getAttribute("aria-label") || e.textContent || "").trim().slice(0, 18) }))
  ).catch(() => []);

  line("   " + pad("页面", 20) + pad("路由ms", 10) + pad("内容ms", 10));
  const slow = [];
  for (const it of items.slice(0, 8)) {
    const before = await page.evaluate(() => ({
      p: location.pathname,
      h: document.querySelector("main")?.innerHTML.length ?? document.body.innerHTML.length,
    }));
    const t = Date.now();
    await page.locator("button.nav-item, button.nav-item-expanded").nth(it.i).click({ force: true, timeout: 8000 }).catch(() => {});

    let routeMs = -1;
    try {
      await page.waitForFunction((p) => location.pathname !== p, before.p, { timeout: 5000, polling: 30 });
      routeMs = Date.now() - t;
    } catch { /* 同页或无路由变化 */ }

    let contentMs = -1;
    try {
      await page.waitForFunction((h) => (document.querySelector("main")?.innerHTML.length ?? document.body.innerHTML.length) !== h, before.h, { timeout: 5000, polling: 30 });
      contentMs = Date.now() - t;
    } catch { /* 内容长度未变 */ }

    line("   " + pad(it.label, 20) + pad(routeMs, 10) + pad(contentMs, 10));
    if (routeMs > 500 || contentMs > 500) slow.push({ ...it, routeMs, contentMs });
    await page.waitForTimeout(300);
  }

  line("\n=== 结论 ===");
  if (pump < 50000) line("⚠ 主线程被节流 —— 优先查安全软件/沙箱（火绒 HIPS），不是前端代码。");
  else if (raf.fps < 30) line("⚠ 合成器不出帧 —— 查 GPU 进程是否反复崩溃（--disable-gpu-sandbox / 驱动）。");
  else if (slow.length) line(`⚠ 这些页面切换 >500ms: ${slow.map((s) => `${s.label}(${Math.max(s.routeMs, s.contentMs)}ms)`).join(", ")}`);
  else line("✅ 各维度均正常。若手感仍慢，慢在应用进程之外（窗口合成/输入法/远程桌面等），需另取进程级快照。");

  await browser.close().catch(() => {});
}

main().catch(async (e) => {
  console.error("体检失败:", e);
  process.exit(1);
});
