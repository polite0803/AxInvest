import { render, screen } from "@testing-library/react";
import type { ReactNode } from "react";
import { describe, expect, it, vi } from "vitest";

vi.mock("react-i18next", () => ({
  // 返回 key 本身，方便断言命中了哪个 key（含插值参数）
  useTranslation: () => ({
    t: (key: string, opts?: Record<string, unknown>) => opts ? `${key}|${JSON.stringify(opts)}` : key,
  }),
  initReactI18next: { type: "3rdParty", init: () => {} },
}));

vi.mock("@/lib/invoke", () => ({
  invoke: vi.fn().mockResolvedValue(undefined),
  listen: vi.fn().mockResolvedValue(() => {}),
  isTauri: () => false,
}));

// antd Tooltip 的 title 只在 hover 时才进 DOM，无法直接断言。
// 替换成把 title 落到 `data-tooltip` 的透气实现，断言才有确定性。
vi.mock("@/components/layout/Tooltip", () => ({
  Tooltip: ({ title, children }: { title?: ReactNode; children?: ReactNode }) => (
    <span data-tooltip={typeof title === "string" ? title : ""}>{children}</span>
  ),
}));

import type { Plan, PlanStatus } from "@/types";
import { PlanCard } from "../PlanCard";

function makePlan(over: Partial<Plan> = {}): Plan {
  return {
    id: "p1",
    conversationId: "c1",
    userMessageId: "m1",
    title: "T",
    steps: [],
    status: "reviewing",
    executionAuthorized: false,
    isActive: true,
    createdAt: 0,
    updatedAt: 0,
    ...over,
  };
}

function renderPlan(plan: Plan) {
  return render(<PlanCard plan={plan} conversationId="c1" />);
}

describe("PlanCard —— 执行授权位展示（迁移 v225）", () => {
  it("已授权时渲染「已授权」标签，并在 tooltip 中带上来源与时间", () => {
    renderPlan(
      makePlan({
        status: "executing",
        executionAuthorized: true,
        authorizedAt: 1_700_000_000_000,
        authorizedBy: "user",
      }),
    );
    expect(screen.getByText("plan.authorization.authorized")).toBeTruthy();
    // 未授权标签不应出现
    expect(screen.queryByText("plan.authorization.unauthorized")).toBeNull();
    // tooltip 文案里的来源被解析成具体白名单标签（而非原始 value）
    expect(document.body.innerHTML).toContain("plan.authorization.source.user");
    // 授权时间也被带进 tooltip（使用本地化时间串，非原始毫秒）
    expect(document.body.innerHTML).not.toContain("1700000000000");
  });

  it("draft / reviewing 未授权时不渲染任何授权标签（这是正常态，不该告警）", () => {
    renderPlan(makePlan({ status: "reviewing", executionAuthorized: false }));
    expect(screen.queryByText("plan.authorization.unauthorized")).toBeNull();
    expect(screen.queryByText("plan.authorization.authorized")).toBeNull();
  });

  it("approved 但未授权时不告警（批准与授权是两个独立步骤）", () => {
    renderPlan(makePlan({ status: "approved", executionAuthorized: false }));
    expect(screen.queryByText("plan.authorization.unauthorized")).toBeNull();
  });

  it("cancelled 未授权时不告警（撤权是正常语义）", () => {
    renderPlan(makePlan({ status: "cancelled", executionAuthorized: false }));
    expect(screen.queryByText("plan.authorization.unauthorized")).toBeNull();
  });

  it.each(["executing", "completed", "partial"] as PlanStatus[])(
    "状态已是 %s 却无授权位 ⇒ 渲染漂移告警（这些状态不可能在未授权时出现）",
    (status) => {
      renderPlan(makePlan({ status, executionAuthorized: false }));
      expect(screen.getByText("plan.authorization.unauthorized")).toBeTruthy();
    },
  );

  it("authorizedBy 落在白名单外时归到 unknown，不直接回显原始值", () => {
    renderPlan(
      makePlan({
        status: "executing",
        executionAuthorized: true,
        authorizedBy: "model",
      }),
    );
    expect(screen.getByText("plan.authorization.authorized")).toBeTruthy();
    expect(document.body.innerHTML).toContain("plan.authorization.source.unknown");
    expect(document.body.innerHTML).not.toContain("plan.authorization.source.model");
  });
});
