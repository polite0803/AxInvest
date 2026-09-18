// SPDX-License-Identifier: AGPL-3.0-only

import type { Variable } from "@/components/workflow/types";
import { isSecretOf, toDbVariable, varTypeOf } from "@/lib/workflowVariables";
import { describe, expect, it } from "vitest";

// i18n-exempt: 测试内断言值，模拟 DB 变量对象字段名，非 UI 文案。
describe("workflowVariables 字段名兼容层", () => {
  // 背景：DB 里的变量对象是 snake_case（后端 `harness::workflow_types::Variable`
  // 没有 rename_all="camelCase"），前端 `Variable` 类型是 camelCase。
  // 任何直接读 `v.varType` 的地方都会恒 undefined → 控件静默退化成纯文本框；
  // 任何直接写 camelCase 对象回模板的地方都会因缺 `var_type` 反序列化失败。
  // 本模块是唯一正确读法。

  /** 构造一个「后端 DB 原样返回」的变量对象（snake_case 键） */
  const dbVar = (name: string, varType: string, isSecret: boolean): Variable =>
    ({ name, var_type: varType, value: 1, description: "d", is_secret: isSecret }) as unknown as Variable;

  /** 构造一个「前端本地默认值」的变量对象（camelCase 键） */
  const feVar = (name: string, varType: string, isSecret: boolean): Variable => ({
    name,
    varType,
    value: 1,
    description: "d",
    isSecret,
  });

  describe("varTypeOf", () => {
    it("读得到 DB 来源的 snake_case（var_type）", () => {
      expect(varTypeOf(dbVar("a", "number", false))).toBe("number");
      expect(varTypeOf(dbVar("b", "boolean", false))).toBe("boolean");
    });

    it("读得到前端 camelCase（varType）", () => {
      expect(varTypeOf(feVar("a", "enum", false))).toBe("enum");
    });

    it("两者都缺时返回空串（调用方 switch 落 default 分支）", () => {
      expect(varTypeOf({ name: "x", value: 1 } as unknown as Variable)).toBe("");
    });

    it("camelCase 优先于 snake_case（同一对象同时存在时）", () => {
      const both = {
        name: "x",
        varType: "number",
        var_type: "boolean",
        value: 1,
        isSecret: false,
      } as unknown as Variable;
      expect(varTypeOf(both)).toBe("number");
    });
  });

  describe("isSecretOf", () => {
    it("读得到 snake_case 的 true", () => {
      expect(isSecretOf(dbVar("k", "string", true))).toBe(true);
    });

    it("显式 false 不被误判为 false 之外的默认值", () => {
      // 用 `v.isSecret || raw.is_secret || false` 这类写法会把显式 false 与
      // 「字段缺失」混为一谈；此处锁死 ?? 语义。
      expect(isSecretOf(dbVar("k", "string", false))).toBe(false);
      expect(isSecretOf(feVar("k", "string", false))).toBe(false);
    });

    it("字段全缺时默认 false", () => {
      expect(isSecretOf({ name: "x", value: 1 } as unknown as Variable)).toBe(false);
    });
  });

  describe("toDbVariable", () => {
    it("把 camelCase 对象转成 snake_case 键（保证后端 serde 可解析）", () => {
      const out = toDbVariable(feVar("t", "number", true)) as unknown as Record<string, unknown>;
      expect(out.var_type).toBe("number");
      expect(out.is_secret).toBe(true);
      expect(out.name).toBe("t");
      expect(out).toHaveProperty("value");
      expect(out).toHaveProperty("description");
    });

    it("输入本身就是 snake_case 时无损往返", () => {
      const src = dbVar("t", "boolean", true);
      const out = toDbVariable(src) as unknown as Record<string, unknown>;
      expect(out.var_type).toBe("boolean");
      expect(out.is_secret).toBe(true);
    });

    it("输出仅含后端 Variable 的 5 个字段（不多写未知键）", () => {
      const out = toDbVariable(feVar("t", "string", false)) as unknown as Record<string, unknown>;
      expect(Object.keys(out).sort()).toEqual(
        ["description", "is_secret", "name", "value", "var_type"].sort(),
      );
    });
  });
});
