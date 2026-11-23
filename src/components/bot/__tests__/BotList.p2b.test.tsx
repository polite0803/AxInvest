// SPDX-License-Identifier: AGPL-3.0-only

/**
 * BotList 分组投影回归锁（P2-b T5''，PLAN-P2B §5 ①）。
 *
 * 钉住的缺陷形态：「无部门 Bot 在分组渲染下消失」——Q4 的红线
 * （无归属 Bot 必须在「未分组」组可见），投影函数是纯函数直接断言。
 */

import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { BotList, departmentOfBot, projectDepartmentGroups } from "../BotList";
import i18n from "@/i18n";
import type { Bot, BotDepartment } from "@/types";

function makeBot(id: string, name: string): Bot {
  return {
    id,
    name,
    title: null,
    description: null,
    soul: "",
    soulHash: "",
    soulDrift: false,
    icon: "",
    color: null,
    section: "",
    modelId: null,
    providerId: null,
    effort: null,
    workdir: null,
    approvalMode: "edits",
    agentProfileId: null,
    isEnabled: true,
    sortOrder: 0,
    createdAt: 0,
    updatedAt: 0,
  };
}

function makeDepartment(id: string, name: string, botIds: string[], chiefBotId: string | null = null): BotDepartment {
  return {
    id,
    name,
    description: "",
    chiefBotId,
    botIds,
    createdAt: 0,
    updatedAt: 0,
  };
}

describe("projectDepartmentGroups", () => {
  it("有归属 Bot 落对应部门组；无归属落「未分组」组（Q4 不丢）", () => {
    const bots = [makeBot("a", "A"), makeBot("b", "B"), makeBot("c", "C")];
    const departments = [makeDepartment("d1", "投研部", ["a", "b"])];
    const groups = projectDepartmentGroups(bots, departments);
    expect(groups).toHaveLength(2);
    expect(groups[0].department?.id).toBe("d1");
    expect(groups[0].bots.map((b) => b.id)).toEqual(["a", "b"]);
    expect(groups[1].department).toBeNull();
    expect(groups[1].bots.map((b) => b.id)).toEqual(["c"]); // 未分组可见
  });

  it("空部门清单：全体落单组（无分组退化）", () => {
    const bots = [makeBot("a", "A")];
    const groups = projectDepartmentGroups(bots, []);
    expect(groups).toHaveLength(1);
    expect(groups[0].department).toBeNull();
    expect(groups[0].bots).toHaveLength(1);
  });

  it("departmentOfBot：单部门口径（Q2）", () => {
    const departments = [makeDepartment("d1", "甲", ["a"]), makeDepartment("d2", "乙", ["b"])];
    expect(departmentOfBot(departments, "a")?.id).toBe("d1");
    expect(departmentOfBot(departments, "b")?.id).toBe("d2");
    expect(departmentOfBot(departments, "zzz")).toBeNull();
  });
});

describe("BotList 渲染", () => {
  beforeEach(async () => {
    if (!i18n.isInitialized) {
      await i18n.init();
    }
    await i18n.changeLanguage("zh-CN");
    localStorage.clear();
  });

  it("部门组头渲染部门名，未分组组头渲染「未分组」；Bot 全部可见", async () => {
    const bots = [makeBot("a", "Alpha"), makeBot("b", "Beta")];
    const departments = [makeDepartment("d1", "投研部", ["a"])];
    render(
      <BotList
        bots={bots}
        activeBotId={null}
        departments={departments}
        onSelect={vi.fn()}
        onCreate={vi.fn()}
      />,
    );
    expect(await screen.findByText("投研部")).toBeTruthy();
    expect(screen.getByText("未分组")).toBeTruthy();
    expect(screen.getByText("Alpha")).toBeTruthy();
    expect(screen.getByText("Beta")).toBeTruthy();
  });

  it("部门管理入口与部门组头菜单触发编辑回调", async () => {
    const user = userEvent.setup();
    const onEdit = vi.fn();
    const bots = [makeBot("a", "Alpha")];
    const departments = [makeDepartment("d1", "投研部", ["a"])];
    render(
      <BotList
        bots={bots}
        activeBotId={null}
        departments={departments}
        onSelect={vi.fn()}
        onCreate={vi.fn()}
        onManageDepartments={vi.fn()}
        onEditDepartment={onEdit}
      />,
    );
    const menuBtn = await screen.findByTestId("bot-department-menu-d1");
    await user.click(menuBtn);
    await user.click(await screen.findByText("编辑部门"));
    expect(onEdit).toHaveBeenCalledWith(departments[0]);
  });
});
