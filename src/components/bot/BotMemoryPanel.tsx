// SPDX-License-Identifier: AGPL-3.0-only

/**
 * 记忆查看面板（P1 只读）：MEMORY.md 索引正文 + 预算条 + 主题清单。
 *
 * 数据源：`read_bot_memory`（`indexContent` / `lineCount` / `byteCount` / `budget` / `topics`）。
 * 预算比例取「行数占比」与「字节占比」的较大者 —— 任一项超限，注入时都会被截断。
 */

import { useBotStore } from "@/stores/feature/botStore";
import type { BotMemory } from "@/types";
import { Alert, Card, List, Progress, Spin, theme, Typography } from "antd";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

const { Text } = Typography;

export interface BotMemoryPanelProps {
  botId: string;
}

/** 预算占用百分比（0-100+）；预算值非正时按 0 处理，避免除零。 */
export function memoryBudgetPercent(memory: BotMemory): number {
  const lineRatio = memory.budget.maxLines > 0 ? memory.lineCount / memory.budget.maxLines : 0;
  const byteRatio = memory.budget.maxBytes > 0 ? memory.byteCount / memory.budget.maxBytes : 0;
  return Math.max(lineRatio, byteRatio) * 100;
}

export function BotMemoryPanel({ botId }: BotMemoryPanelProps) {
  const { t } = useTranslation();
  const { token } = theme.useToken();
  const readMemory = useBotStore((s) => s.readMemory);
  const [memory, setMemory] = useState<BotMemory | null>(null);
  const [loading, setLoading] = useState(true);
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    let alive = true;
    setLoading(true);
    setFailed(false);
    setMemory(null);
    readMemory(botId)
      .then((value) => {
        if (alive) {
          setMemory(value);
          setLoading(false);
        }
      })
      .catch(() => {
        if (alive) {
          setFailed(true);
          setLoading(false);
        }
      });
    return () => {
      alive = false;
    };
  }, [botId, readMemory]);

  const hasContent = Boolean(memory && (memory.indexContent.trim() || memory.topics.length > 0));
  const percent = memory ? memoryBudgetPercent(memory) : 0;

  return (
    <Card size="small" title={t("bots.memoryTitle")} data-testid="bot-memory-panel">
      {loading
        ? <Spin size="small" />
        : failed
        ? <Alert type="error" showIcon message={t("bots.memoryLoadFailed")} />
        : !hasContent
        ? <Text type="secondary">{t("bots.memoryEmpty")}</Text>
        : (
          <div style={{ display: "flex", flexDirection: "column", gap: 10 }}>
            <div style={{ display: "flex", flexDirection: "column", gap: 2 }}>
              <Progress
                percent={Math.min(100, Math.round(percent))}
                status={percent > 100 ? "exception" : "normal"}
                size="small"
                showInfo={false}
              />
              <Text type="secondary" style={{ fontSize: 12 }}>
                {t("bots.memoryBudget", {
                  lines: memory?.lineCount ?? 0,
                  maxLines: memory?.budget.maxLines ?? 0,
                  bytes: memory?.byteCount ?? 0,
                  maxBytes: memory?.budget.maxBytes ?? 0,
                })}
              </Text>
              <Text type="secondary" style={{ fontSize: 12 }}>
                {t("bots.memoryBudgetHint")}
              </Text>
            </div>

            {memory?.indexContent.trim()
              ? (
                <pre
                  style={{
                    margin: 0,
                    padding: 8,
                    borderRadius: 6,
                    background: token.colorFillQuaternary,
                    color: token.colorText,
                    fontFamily: "monospace",
                    fontSize: 12,
                    whiteSpace: "pre-wrap",
                    wordBreak: "break-word",
                    maxHeight: 260,
                    overflow: "auto",
                  }}
                >
              {memory.indexContent}
                </pre>
              )
              : null}

            {memory && memory.topics.length > 0
              ? (
                <List
                  size="small"
                  dataSource={memory.topics}
                  renderItem={(topic) => (
                    <List.Item style={{ padding: "4px 0" }}>
                      <div style={{ display: "flex", flexDirection: "column", minWidth: 0 }}>
                        <Text ellipsis>{topic.title || topic.relativePath}</Text>
                        <Text type="secondary" style={{ fontSize: 12 }} ellipsis>
                          {topic.relativePath}
                        </Text>
                      </div>
                    </List.Item>
                  )}
                />
              )
              : null}
          </div>
        )}
    </Card>
  );
}
