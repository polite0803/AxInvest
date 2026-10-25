// SPDX-License-Identifier: AGPL-3.0-only

import fs from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";

/**
 * 托盘两个设置的**上下级**锁（2026-10-25 接通轮）。
 *
 * 为什么单独钉：`trayEnabled`（托盘这个承载物在不在）与 `minimizeToTray`（关窗口时藏进托盘）
 * 是两个字段，但后者依赖前者存在。接通前托盘恒创建，所以这层依赖没人写过；接通后若允许
 * 「托盘关掉 + 最小化到托盘开着」，关窗口即 `hide()` 且没有恢复入口（Windows 上隐藏后任务栏
 * 也没有）—— 用户会把整个应用「弄丢」。这类「新接通一个开关，同时把它下面的行为开关变成陷阱」
 * 的后果，`tsc` 与后端门禁一条都查不出来。
 *
 * 静态源文判据（本仓既有写法，见 `llmParamDefaults.test.ts`）：不渲染组件，只钉三处形态。
 */
function readSource(...segments: string[]) {
  return fs.readFileSync(path.resolve(process.cwd(), ...segments), "utf8");
}

describe("托盘开关与最小化到托盘的上下级", () => {
  const ui = readSource("src/components/settings/GeneralSettings.tsx");

  it("关掉托盘的同一批把 minimizeToTray 打回 false", () => {
    expect(ui).toContain("{ trayEnabled: checked, minimizeToTray: false }");
    // 关托盘还要把「关窗口即隐藏」这条运行时状态也撤掉，否则内存态仍是 true
    expect(ui).toContain('invoke("set_close_to_tray", { enabled: false })');
  });

  it("托盘关着时，最小化到托盘的开关必须不可用", () => {
    expect(ui).toContain("disabled={!(settings.trayEnabled ?? true)}");
  });

  it("托盘开关有消费方，且不另开第二条启动通道", () => {
    expect(ui).toContain('invoke("set_tray_enabled", { enabled: checked })');
    // 唯一权威路径是 set_tray_enabled；不得借 apply_startup_settings 再塞一份托盘参数
    expect(ui).not.toMatch(/apply_startup_settings[^)]{0,160}tray/i);
  });
});
