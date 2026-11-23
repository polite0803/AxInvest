// SPDX-License-Identifier: AGPL-3.0-only

/**
 * Bot 列表（侧栏左列）—— 纯展示组件：数据与回调全部经 props 注入，
 * 不读任何 store（组合由 `pages/BotsPage` 负责）。
 *
 * P2-b：按部门分组展示（PLAN-P2B §5 ①）。分组投影是纯函数（导出供单测直断）：
 * - 有部门归属的 Bot 落入对应部门组（组头 = 部门名 + Chief 标注）；
 * - 无归属的 Bot 落入「未分组」组（Q4：不丢、可见）；
 * - 部门是常设组织归属，与「AI 办公室」（fleet，office.* 命名空间）无文案交集
 *   （ADR-P2B-1）。
 */

import type { Bot, BotDepartment } from "@/types";
import { Badge, Button, Dropdown, theme, Typography } from "antd";
import { Bot as BotIcon, FolderCog, MoreHorizontal, Plus } from "lucide-react";
import type { KeyboardEvent } from "react";
import { useTranslation } from "react-i18next";

const { Text } = Typography;

export interface BotListProps {
  bots: Bot[];
  activeBotId: string | null;
  /** P2-a：botId → 未读通知数（>0 时在列表项上渲染角标；页面层经 props 注入） */
  unreadByBot?: Map<string, number>;
  /** P2-b：部门清单（分组数据源；空 = 无分组退化） */
  departments?: BotDepartment[];
  onSelect: (botId: string) => void;
  onCreate: () => void;
  /** P2-b：部门管理入口（新建部门） */
  onManageDepartments?: () => void;
  /** P2-b：编辑单个部门（部门组头上的菜单） */
  onEditDepartment?: (department: BotDepartment) => void;
}

/** botId → 所属部门（单部门口径 Q2）。导出供单测直断。 */
export function departmentOfBot(departments: BotDepartment[], botId: string): BotDepartment | null {
  return departments.find((d) => d.botIds.includes(botId)) ?? null;
}

/** 分组投影：部门组列表（组序 = 部门 createdAt 序）+ 未组剩余。导出供单测直断。 */
export function projectDepartmentGroups(
  bots: Bot[],
  departments: BotDepartment[],
): { department: BotDepartment | null; bots: Bot[] }[] {
  const groups: { department: BotDepartment | null; bots: Bot[] }[] = departments.map((d) => ({
    department: d,
    bots: bots.filter((b) => d.botIds.includes(b.id)),
  }));
  const groupedIds = new Set(departments.flatMap((d) => d.botIds));
  const rest = bots.filter((b) => !groupedIds.has(b.id));
  if (rest.length > 0 || departments.length === 0) {
    groups.push({ department: null, bots: rest });
  }
  return groups;
}

export function BotList({
  bots,
  activeBotId,
  unreadByBot,
  departments = [],
  onSelect,
  onCreate,
  onManageDepartments,
  onEditDepartment,
}: BotListProps) {
  const { t } = useTranslation();
  const { token } = theme.useToken();

  const handleKeyDown = (event: KeyboardEvent<HTMLDivElement>, botId: string) => {
    if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      onSelect(botId);
    }
  };

  const groups = projectDepartmentGroups(bots, departments);

  const renderBot = (bot: Bot) => {
    const active = bot.id === activeBotId;
    return (
      <div
        key={bot.id}
        role="button"
        tabIndex={0}
        data-testid="bot-list-item"
        data-bot-id={bot.id}
        aria-pressed={active}
        onClick={() => onSelect(bot.id)}
        onKeyDown={(e) => handleKeyDown(e, bot.id)}
        style={{
          display: "flex",
          alignItems: "center",
          gap: 8,
          padding: "8px 10px",
          borderRadius: 8,
          cursor: "pointer",
          background: active ? token.colorFillSecondary : "transparent",
          border: `1px solid ${active ? token.colorBorderSecondary : "transparent"}`,
        }}
      >
        <Badge
          count={unreadByBot?.get(bot.id) ?? 0}
          size="small"
          offset={[-4, 4]}
          title={t("bots.unreadBadge", { count: unreadByBot?.get(bot.id) ?? 0 })}
        >
          <span style={{ display: "inline-flex", width: 20, justifyContent: "center", fontSize: 16 }}>
            {bot.icon ? bot.icon : <BotIcon size={16} />}
          </span>
        </Badge>
        <span style={{ display: "flex", flexDirection: "column", minWidth: 0, flex: 1 }}>
          <Text ellipsis style={{ fontWeight: active ? 600 : 400 }}>
            {bot.name}
          </Text>
          {bot.title
            ? (
              <Text type="secondary" ellipsis style={{ fontSize: 12 }}>
                {bot.title}
              </Text>
            )
            : null}
        </span>
      </div>
    );
  };

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 8, minHeight: 0 }}>
      <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", padding: "0 4px" }}>
        <Text strong>{t("bots.title")}</Text>
        <div style={{ display: "flex", alignItems: "center", gap: 2 }}>
          {onManageDepartments
            ? (
              <Button
                type="text"
                size="small"
                icon={<FolderCog size={15} />}
                title={t("bot.department.title")}
                onClick={onManageDepartments}
                data-testid="bot-department-manage"
              />
            )
            : null}
          <Button type="text" size="small" icon={<Plus size={16} />} onClick={onCreate} data-testid="bot-create-btn">
            {t("bots.newBot")}
          </Button>
        </div>
      </div>
      <div style={{ display: "flex", flexDirection: "column", gap: 8, overflowY: "auto" }}>
        {groups.map((group) => {
          const dept = group.department;
          return (
            <div key={dept?.id ?? "__ungrouped__"} style={{ display: "flex", flexDirection: "column", gap: 4 }}>
              <div
                style={{
                  display: "flex",
                  alignItems: "center",
                  gap: 6,
                  padding: "2px 6px",
                }}
              >
                <Text type="secondary" ellipsis style={{ fontSize: 12, flex: 1 }}>
                  {dept ? dept.name : t("bot.department.untitled")}
                </Text>
                {dept && onEditDepartment
                  ? (
                    <Dropdown
                      trigger={["click"]}
                      menu={{
                        items: [{ key: "edit", label: t("bot.department.edit") }],
                        onClick: ({ key }) => {
                          if (key === "edit" && dept) {
                            onEditDepartment(dept);
                          }
                        },
                      }}
                    >
                      <Button
                        type="text"
                        size="small"
                        icon={<MoreHorizontal size={14} />}
                        data-testid={`bot-department-menu-${dept.id}`}
                      />
                    </Dropdown>
                  )
                  : null}
              </div>
              {group.bots.map(renderBot)}
              {dept && group.bots.length === 0
                ? (
                  <Text type="secondary" style={{ fontSize: 12, padding: "0 10px" }}>
                    {t("bot.department.chiefEmpty")}
                  </Text>
                )
                : null}
            </div>
          );
        })}
      </div>
    </div>
  );
}
