import { readFileSync } from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";

/**
 * 「工作流列表要看得见模板代」的门（用户 2026-10-09 原话：
 * 「我的工作量列表中没有显示工作流版本，这不符合版本的频繁更新需求」）。
 *
 * 这条不是美化：本仓种子模板按代频繁更新（实测库里停在 129 时代码已到 143，六代落差全靠人肉查库
 * 才发现），而列表里只显示名称/标签/描述 ⇒ 同一张图的两代在界面上**完全不可分**，
 * 「这条结论是哪代图给的」就只能靠猜。
 *
 * 断言按**四层**各锁一处（缺一层就是一条静默失效面）：
 *   ① 后端 DTO 有该字段（删掉它 ⇒ 前端拿到 undefined，`v` + undefined 显示成 `vundefined`）；
 *   ② 前端响应类型有该字段（TS 侧没声明就编译不过，但**声明成可选**会让它静默缺席 ⇒ 断言必填）；
 *   ③④ 业务模板页与系统模板页两个列表都真的把它渲染出来（只加类型不加控件＝界面上不存在）。
 */
// __tests__ → workflow → components → src → 仓库根
const ROOT = path.resolve(import.meta.dirname, "..", "..", "..", "..");

const read = (rel: string) => readFileSync(path.join(ROOT, rel), "utf8");

describe("工作流模板列表的代（version）呈现", () => {
  it("后端 DTO 带 version: i32", () => {
    const rs = read("src-tauri/crates/harness/src/workflow_types.rs");
    const start = rs.indexOf("pub struct WorkflowTemplateResponse");
    expect(start, "应存在 WorkflowTemplateResponse").toBeGreaterThan(-1);
    expect(rs.slice(start, start + 1200)).toMatch(/pub version: i32,/);
  });

  it("前端响应类型把 version 声明成必填（不得可选）", () => {
    const ts = read("src/components/workflow/types/workflow.types.ts");
    const start = ts.indexOf("export interface WorkflowTemplateResponse");
    expect(start, "应存在 WorkflowTemplateResponse").toBeGreaterThan(-1);
    const body = ts.slice(start, start + 900);
    expect(body).toContain("  version: number;\n");
    expect(body).not.toContain("version?:");
  });

  it("业务模板页与系统模板页都把 v<代> 渲染在标题行", () => {
    for (
      const rel of [
        "src/components/workflow/Templates/TemplateList.tsx",
        "src/components/workflow/Templates/SystemTemplateList.tsx",
      ]
    ) {
      const src = read(rel);
      expect(src, `${rel} 应渲染模板代`).toContain("{`v${template.version}`}");
    }
  });

  it("负控：把控件摘掉后上一条断言真的会红（不是恒真）", () => {
    const src = read("src/components/workflow/Templates/TemplateList.tsx")
      .replace("{`v${template.version}`}", "{template.name}");
    expect(src).not.toContain("{`v${template.version}`}");
  });
});
