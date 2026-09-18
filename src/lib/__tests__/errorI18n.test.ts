// SPDX-License-Identifier: AGPL-3.0-only

import { beforeAll, describe, expect, it, vi } from "vitest";

import i18n from "@/i18n";
import {
  getBackendErrorCategory,
  parseBackendError,
  showBackendError,
  translateBackendError,
  translateFailureText,
} from "../errorI18n";

// 使用 zh-CN 源语言（同步 bundle），确保翻译命中真实 locale 数据。
beforeAll(async () => {
  await i18n.changeLanguage("zh-CN");
});

describe("parseBackendError", () => {
  it("解析带合法 code 的对象（Tauri 直接序列化 ErrorResponse）", () => {
    const parsed = parseBackendError({
      code: "CONVERSATION_NOT_FOUND",
      category: "unrecoverable",
      detail: "id=42",
    });
    expect(parsed.code).toBe("CONVERSATION_NOT_FOUND");
    expect(parsed.category).toBe("unrecoverable");
    expect(parsed.detail).toBe("id=42");
  });

  it("解析 message 为 JSON 字符串的 Error", () => {
    const err = new Error(JSON.stringify({ code: "TOOL_NOT_FOUND", category: "validation" }));
    const parsed = parseBackendError(err);
    expect(parsed.code).toBe("TOOL_NOT_FOUND");
    expect(parsed.category).toBe("validation");
  });

  it("解析纯 JSON 字符串", () => {
    const parsed = parseBackendError('{"code":"COMMON_INTERNAL","detail":"boom"}');
    expect(parsed.code).toBe("COMMON_INTERNAL");
    expect(parsed.detail).toBe("boom");
  });

  it("纯字符串无 code 时仅返回 raw", () => {
    const parsed = parseBackendError("something went wrong");
    expect(parsed.code).toBeUndefined();
    expect(parsed.raw).toBe("something went wrong");
  });

  it("非法 code 格式不被识别（小写/单段）", () => {
    expect(parseBackendError({ code: "notACode" }).code).toBeUndefined();
    expect(parseBackendError({ code: "LOWER" }).code).toBeUndefined();
  });

  it("非法 category 被丢弃", () => {
    const parsed = parseBackendError({ code: "TOOL_NOT_FOUND", category: "bogus" });
    expect(parsed.code).toBe("TOOL_NOT_FOUND");
    expect(parsed.category).toBeUndefined();
  });

  it("null / undefined 安全处理", () => {
    expect(parseBackendError(null).raw).toBe("");
    expect(parseBackendError(undefined).raw).toBe("");
  });
});

describe("translateFailureText", () => {
  it("有码 ⇒ 用本地化译文（自由文本里的中文不再漏到界面）", () => {
    expect(translateFailureText("EXECUTION_CANCELLED: 节点执行已取消", "STOCK_WORKFLOW_STEP_CANCELLED"))
      .toBe("分析节点因分析被取消而中止");
    expect(translateFailureText("TIMEOUT: 节点执行超时", "STOCK_WORKFLOW_TIMEOUT"))
      .toBe("分析超时，请稍后重试");
  });

  it("无码（null = 后端明示无失败 / undefined = 旧载荷）⇒ 原文，零回归", () => {
    const raw = "EXECUTION_CANCELLED: 节点执行已取消";
    expect(translateFailureText(raw, null)).toBe(raw);
    expect(translateFailureText(raw, undefined)).toBe(raw);
  });

  it("非法码格式 ⇒ 原文（不被当成码查表）", () => {
    expect(translateFailureText("boom", "not_a_code")).toBe("boom");
    expect(translateFailureText("boom", "LOWER")).toBe("boom");
  });

  it("合法码但 locale 未收录 ⇒ 回退原文，而非对象的 JSON 串", () => {
    // 钉死 translateBackendError 的**对象入参陷阱**：无 detail 时它会回退 parsed.raw，
    // 而对象入参的 raw 是 JSON.stringify ⇒ 界面会显示 {"code":"TOTALLY_UNKNOWN_CODE"}。
    const text = translateFailureText("some real failure", "TOTALLY_UNKNOWN_CODE");
    expect(text).toBe("some real failure");
    expect(text).not.toContain("{");
  });

  it("原文为空 ⇒ 空串（绝不产生 JSON 串 / 不编造文案）", () => {
    expect(translateFailureText("", "STOCK_WORKFLOW_STEP_FAILED")).toBe("");
    expect(translateFailureText(null, "STOCK_WORKFLOW_STEP_FAILED")).toBe("");
    expect(translateFailureText(undefined, "STOCK_WORKFLOW_STEP_FAILED")).toBe("");
  });
});

describe("translateBackendError", () => {
  it("已知码翻译为 zh-CN 文本", () => {
    expect(translateBackendError({ code: "CONVERSATION_NOT_FOUND" })).toBe("会话未找到");
    expect(translateBackendError({ code: "TOOL_NOT_FOUND" })).toBe("工具未找到");
  });

  it("未知码回退 detail", () => {
    expect(translateBackendError({ code: "TOTALLY_UNKNOWN_CODE", detail: "fallback detail" }))
      .toBe("fallback detail");
  });

  it("【行为记录】合法码 + locale 未收录 + detail 空 ⇒ 退化成 JSON 串（translateFailureText 的空值短路正为规避它）", () => {
    // 尖角的**精确条件**（三者和集，缺一不成立）：码合法 ⇢ locale 未收录 ⇢ detail 空/缺失。
    // 码被收录时 L156-158 直接返回译文、不看 detail（且 `{ code, detail: "" }` 实测 = "工具未找到"）；
    // 未收录时落到 L162 `parsed.detail || parsed.raw`，而对象入参的 raw 是 JSON.stringify(对象)。
    // 若此断言将来失败 ⇒ 翻译层已修好该尖角，届时可重估 `translateFailureText` 的空值短路是否还需要。
    const text = translateBackendError({ code: "TOTALLY_UNKNOWN_CODE", detail: "" });
    expect(text).toContain('"code"');
    // 负向对照：同码但 detail 非空 ⇒ 正常回退原文，不产生 JSON 串
    expect(translateBackendError({ code: "TOTALLY_UNKNOWN_CODE", detail: "boom" })).toBe("boom");
  });

  it("未知码无 detail 时回退原始文本", () => {
    expect(translateBackendError("plain error text")).toBe("plain error text");
  });

  it("单花括号占位符被手动替换", () => {
    // AGENT_STATUS_STEER_APPLIED -> "已应用 {count} 条引导指令"
    const text = translateBackendError({
      code: "AGENT_STATUS_STEER_APPLIED",
      params: { count: "3" },
    });
    expect(text).toBe("已应用 3 条引导指令");
  });

  it("Error 对象的 JSON message 也能翻译", () => {
    const err = new Error(JSON.stringify({ code: "CONVERSATION_NOT_FOUND" }));
    expect(translateBackendError(err)).toBe("会话未找到");
  });
});

describe("getBackendErrorCategory", () => {
  it("提取合法分类", () => {
    expect(getBackendErrorCategory({ code: "TOOL_NOT_FOUND", category: "retryable" }))
      .toBe("retryable");
  });

  it("无分类返回 undefined", () => {
    expect(getBackendErrorCategory("plain")).toBeUndefined();
  });
});

describe("showBackendError", () => {
  it("retryable 分类走 warning", () => {
    const message = { error: vi.fn(), warning: vi.fn() };
    const text = showBackendError(message, { code: "TOOL_NOT_FOUND", category: "retryable" });
    expect(message.warning).toHaveBeenCalledWith("工具未找到", undefined);
    expect(message.error).not.toHaveBeenCalled();
    expect(text).toBe("工具未找到");
  });

  it("非 retryable 分类走 error", () => {
    const message = { error: vi.fn(), warning: vi.fn() };
    showBackendError(message, { code: "CONVERSATION_NOT_FOUND", category: "unrecoverable" });
    expect(message.error).toHaveBeenCalledWith("会话未找到", undefined);
    expect(message.warning).not.toHaveBeenCalled();
  });

  it("纯字符串错误走 error 并原样展示", () => {
    const message = { error: vi.fn(), warning: vi.fn() };
    showBackendError(message, "raw failure");
    expect(message.error).toHaveBeenCalledWith("raw failure", undefined);
  });
});
