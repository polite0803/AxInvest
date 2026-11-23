// SPDX-License-Identifier: AGPL-3.0-only

/**
 * P3 多层委派链视图（T6'''）。按 rootId 过滤，depth 1..3 分组。
 *
 * 复用 P2-b Assignment 列表导出的状态色 / i18n 键 / 终态判定与 cancel testid 约定，
 * 但不改动 P2-b 源码；链视图只加 rootId/depth 投影与链级错误码展示。
 */

import { isTerminalStatus, statusBadgeColor, statusI18nKey } from "@/components/bot/BotAssignmentsPanel";
import { delegationRootIds, selectDelegationChain, workflowErrorI18nKey } from "@/lib/botWorkflowValidation";
import type { BotAssignment } from "@/types";
import { Alert, Badge, Button, Card, Empty, Popconfirm, Select, Space, Tag, Typography } from "antd";
import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";

const { Text } = Typography;

const CHAIN_ERROR_CODES = ["BOT_DELEGATE_CYCLE", "BOT_DELEGATE_MAX_DEPTH"] as const;

export interface BotDelegationChainPanelProps {
  assignments: BotAssignment[];
  /** 指定根 id；为空时展示可选根列表，默认选第一个。 */
  rootId?: string | null;
  /** 链级错误码（cycle/maxDepth）；单条 assignment 的错误由列表项自身展示。 */
  errorCode?: string | null;
  busy?: Record<string, boolean>;
  onCancel?: (assignment: BotAssignment) => void;
}

export function BotDelegationChainPanel({
  assignments,
  rootId,
  errorCode,
  busy = {},
  onCancel,
}: BotDelegationChainPanelProps) {
  const { t } = useTranslation();
  const roots = useMemo(() => delegationRootIds(assignments), [assignments]);
  const [selectedRoot, setSelectedRoot] = useState(rootId ?? roots[0] ?? "");

  useEffect(() => {
    if (rootId) {
      setSelectedRoot(rootId);
    } else if (!roots.includes(selectedRoot)) {
      setSelectedRoot(roots[0] ?? "");
    }
  }, [rootId, roots, selectedRoot]);

  const chain = useMemo(
    () => (selectedRoot ? selectDelegationChain(assignments, selectedRoot) : []),
    [assignments, selectedRoot],
  );

  const chainError = CHAIN_ERROR_CODES.find((code) => code === errorCode);
  const errorKey = chainError ? workflowErrorI18nKey(chainError) : null;

  return (
    <Card size="small" title={t("bot.assignment.chain")} data-testid="bot-delegation-chain-panel">
      <Space direction="vertical" style={{ width: "100%" }} size={8}>
        <Select
          value={selectedRoot || undefined}
          style={{ width: "100%" }}
          data-testid="bot-delegation-root-select"
          placeholder={t("bot.assignment.chain")}
          options={roots.map((id) => ({ value: id, label: id }))}
          onChange={(value) => setSelectedRoot(value)}
        />
        {errorKey
          ? <Alert type="error" showIcon message={t(errorKey)} data-testid="bot-delegation-chain-error" />
          : null}
        {chain.length === 0
          ? <Empty data-testid="bot-delegation-chain-empty" />
          : (
            <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
              {[1, 2, 3].map((depth) => {
                const group = chain.filter((assignment) => assignment.depth === depth);
                if (group.length === 0) {
                  return null;
                }
                return (
                  <div key={depth} data-testid="bot-delegation-depth-group" data-depth={depth}>
                    <Tag
                      data-testid="bot-delegation-depth-badge"
                      data-depth={depth}
                      style={{ marginBottom: 4 }}
                    >
                      {t("bot.assignment.depth")} {depth}
                    </Tag>
                    <div style={{ display: "flex", flexDirection: "column", gap: 4 }}>
                      {group.map((assignment) => {
                        const itemBusy = busy[assignment.id] === true;
                        return (
                          <div
                            key={assignment.id}
                            data-testid="bot-assignment-item"
                            data-status={assignment.status}
                            data-depth={assignment.depth}
                            data-root-id={assignment.rootId}
                            style={{
                              display: "flex",
                              flexDirection: "column",
                              gap: 4,
                              padding: "8px 10px",
                              borderRadius: 8,
                              border: "1px solid rgba(128,128,128,0.25)",
                            }}
                          >
                            <div style={{ display: "flex", alignItems: "center", gap: 8, minWidth: 0 }}>
                              <Badge
                                status={statusBadgeColor(assignment.status)}
                                text={
                                  <Text strong style={{ fontSize: 13 }}>
                                    {t(statusI18nKey(assignment.status))}
                                  </Text>
                                }
                              />
                              <Text type="secondary" ellipsis style={{ fontSize: 12, flex: 1 }}>
                                {`${t("bot.assignment.assignee")}: ${assignment.assigneeName}`}
                              </Text>
                              {!isTerminalStatus(assignment.status) && onCancel
                                ? (
                                  <Popconfirm
                                    title={t("bot.assignment.cancel")}
                                    okText={t("bot.assignment.cancel")}
                                    cancelText={t("common.cancel")}
                                    okButtonProps={{ danger: true, loading: itemBusy }}
                                    onConfirm={() => onCancel(assignment)}
                                  >
                                    <Button
                                      size="small"
                                      danger
                                      disabled={itemBusy}
                                      data-testid="bot-assignment-cancel"
                                    >
                                      {t("bot.assignment.cancel")}
                                    </Button>
                                  </Popconfirm>
                                )
                                : null}
                            </div>
                            <Text ellipsis={{ tooltip: assignment.taskText }} style={{ fontSize: 12 }}>
                              {assignment.taskText}
                            </Text>
                            {assignment.resultText
                              ? (
                                <Text
                                  type="secondary"
                                  ellipsis={{ tooltip: assignment.resultText }}
                                  style={{ fontSize: 12 }}
                                >
                                  {assignment.resultText}
                                </Text>
                              )
                              : null}
                            {assignment.errorDetail
                              ? (
                                <Text
                                  type="danger"
                                  ellipsis={{ tooltip: assignment.errorDetail }}
                                  style={{ fontSize: 12 }}
                                >
                                  {assignment.errorDetail}
                                </Text>
                              )
                              : null}
                          </div>
                        );
                      })}
                    </div>
                  </div>
                );
              })}
            </div>
          )}
        {chain.length > 0
          ? (
            <Text type="secondary" style={{ fontSize: 12 }} data-testid="bot-delegation-chain-order">
              {chain.map((assignment) => assignment.id).join(" > ")}
            </Text>
          )
          : null}
      </Space>
    </Card>
  );
}
