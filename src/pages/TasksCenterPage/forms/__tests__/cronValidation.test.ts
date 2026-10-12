// SPDX-License-Identifier: AGPL-3.0-only

import { describe, expect, it } from "vitest";
import { validateCronSyntax } from "../../CronScheduleBuilder";

/**
 * TR-7.2：cron 校验行为与设置页旧定时任务面板一致（纯逻辑平移，无 UI 依赖）。
 * 规则：必填、严格 5 段、词法可反解析、最小间隔 2 分钟（分钟字段覆盖 0-59 全集即拒绝）。
 */
describe("validateCronSyntax（迁移自设置页旧定时任务面板）", () => {
  it("空表达式 → 必填错误键", () => {
    expect(validateCronSyntax("")).toBe("settings.scheduler.cronRequired");
    expect(validateCronSyntax("   ")).toBe("settings.scheduler.cronRequired");
  });

  it("字段数不是 5 → 格式错误键", () => {
    expect(validateCronSyntax("0 9 * *")).toBe("settings.scheduler.cronInvalid");
    expect(validateCronSyntax("0 9 * * * *")).toBe("settings.scheduler.cronInvalid");
  });

  it("扩展词法（? / L / W / #）不可解析 → 格式错误键", () => {
    expect(validateCronSyntax("? 9 * * *")).toBe("settings.scheduler.cronInvalid");
    expect(validateCronSyntax("0 9 L * *")).toBe("settings.scheduler.cronInvalid");
    expect(validateCronSyntax("0 9 * * 1#2")).toBe("settings.scheduler.cronInvalid");
  });

  it("每分钟执行（分钟字段覆盖 0-59 全集）→ 过于频繁错误键", () => {
    expect(validateCronSyntax("* * * * *")).toBe("settings.scheduler.cronTooFrequent");
    expect(validateCronSyntax("*/1 * * * *")).toBe("settings.scheduler.cronTooFrequent");
    expect(validateCronSyntax("0-59/1 * * * *")).toBe("settings.scheduler.cronTooFrequent");
  });

  it("合法表达式返回 null：固定时点 / 工作日 / 2 分钟网格 / 间隔小时", () => {
    expect(validateCronSyntax("0 9 * * *")).toBeNull();
    expect(validateCronSyntax("30 15 * * 1-5")).toBeNull();
    expect(validateCronSyntax("*/2 * * * *")).toBeNull();
    expect(validateCronSyntax("0 */2 * * *")).toBeNull();
    expect(validateCronSyntax("0 21 * * 1-5")).toBeNull();
  });

  it("首尾空白不影响校验", () => {
    expect(validateCronSyntax("  0 9 * * *  ")).toBeNull();
  });
});
