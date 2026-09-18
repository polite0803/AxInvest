// i18n-exempt: 业务逻辑/格式化/日志字符串，非 UI 展示文本
// SPDX-License-Identifier: AGPL-3.0-only

/**
 * ChatPanel — 群聊智能路由面板。
 *
 * ## 两个数据源，不要混淆
 *
 * - `messagesByConversation[fleetId::group]`：**持久化**消息历史（数据库），
 *   对话的真源。刷新后仍在，也正是 agent 决策所依据的**群聊会话**上下文。
 * - `dispatchEvents`：本次操作的**过程回放**（routing / process / token / held…），
 *   切换 fleet 或下次发送即清空。
 *
 * ⚠ 本面板**只读群聊会话**：DM 走 `DirectMessagePanel`，其消息落在
 * `dm:<slug>` 会话里，两边的时间线由后端按 `conversation_id` 隔离。
 *
 * 流程：用户输入 → `store.dispatch` → 后端落库 + 路由 + 执行 →
 * `finally` 从库重读 ⇒ 界面显示的永远是库里的真源。
 */

import { useOfficeStore, useStockAnalysisStore } from "@/stores";
import type { DispatchEvent, FleetMessage } from "@/types";
import { CONVERSATION_GROUP, conversationKey } from "@/types";
import { Button, Input, Space, Spin, Tag, theme, Tooltip, Typography } from "antd";
import { Send } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

const { Text } = Typography;

/** 稳定的空数组：`?? []` 每次渲染都会造新引用，会让 zustand 选择器永远判定「变了」 */
const NO_MESSAGES: FleetMessage[] = [];

export function ChatPanel({ fleetId }: { fleetId: string }) {
  const { t } = useTranslation();
  const { token } = theme.useToken();
  const dispatch = useOfficeStore((s) => s.dispatch);
  const events = useOfficeStore((s) => s.dispatchEvents);
  const messages = useOfficeStore(
    (s) => s.messagesByConversation[conversationKey(fleetId, CONVERSATION_GROUP)] ?? NO_MESSAGES,
  );
  const loadMessages = useOfficeStore((s) => s.loadMessages);
  const loading = useOfficeStore((s) => s.loading);
  const clearEvents = useOfficeStore((s) => s.clearDispatchEvents);

  // 注入股票业务上下文：用户在 stock-analysis 页选中的当前股票
  const stockCode = useStockAnalysisStore((s) => s.stockCode);
  const stockName = useStockAnalysisStore((s) => s.stockName);
  const quote = useStockAnalysisStore((s) => s.quote);

  const [input, setInput] = useState("");
  const [sending, setSending] = useState(false);
  const listRef = useRef<HTMLDivElement>(null);

  // 切换 fleet 时清空事件流（过程回放），并加载**群聊会话**的持久化历史
  useEffect(() => {
    clearEvents();
    void loadMessages(fleetId, CONVERSATION_GROUP);
  }, [fleetId, clearEvents, loadMessages]);

  // 历史或事件流更新时滚动到底部
  useEffect(() => {
    if (listRef.current) {
      listRef.current.scrollTop = listRef.current.scrollHeight;
    }
  }, [events, messages]);

  // 构造业务上下文后缀：若当前选中有股票，自动追加到 user_message 末尾
  // 让 dispatcher LLM 能感知"用户正在看哪只股票"
  const buildBusinessContextSuffix = (): string => {
    if (!stockCode) {
      return "";
    }
    const parts: string[] = [`[当前股票上下文] 代码=${stockCode}`];
    if (stockName) {
      parts.push(`名称=${stockName}`);
    }
    if (quote) {
      parts.push(`现价=${quote.price}`);
      parts.push(`涨跌幅=${quote.changePct}%`);
      if (quote.pe != null) {
        parts.push(`PE=${quote.pe}`);
      }
      if (quote.pb != null) {
        parts.push(`PB=${quote.pb}`);
      }
    }
    return `\n${parts.join(" ")}`;
  };

  const handleSend = async () => {
    const msg = input.trim();
    if (!msg || sending) {
      return;
    }
    const ctxSuffix = buildBusinessContextSuffix();
    // ctxSuffix 自身以 "\n" 开头，故此处不再补换行（原写法会造出空行）
    const finalMsg = ctxSuffix ? `${msg}${ctxSuffix}` : msg;
    setSending(true);
    setInput("");
    try {
      await dispatch({ fleetId, userMessage: finalMsg });
    } finally {
      setSending(false);
    }
  };

  // 显示当前注入的股票上下文提示
  const hasContext = Boolean(stockCode);
  const isEmpty = messages.length === 0 && events.length === 0;

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%", gap: 8 }}>
      {/* 消息历史（真源）+ 本次事件流 */}
      <div
        ref={listRef}
        style={{
          flex: 1,
          overflow: "auto",
          padding: 8,
          background: token.colorBgLayout,
          borderRadius: 6,
          border: `1px solid ${token.colorBorderSecondary}`,
          minHeight: 200,
        }}
      >
        {isEmpty
          ? (
            <div style={{ textAlign: "center", color: token.colorTextQuaternary, fontSize: 12, padding: 24 }}>
              {t("office.chat.emptyHint")}
            </div>
          )
          : (
            <>
              <MessageList messages={messages} />
              <EventList events={events} />
            </>
          )}
      </div>

      {/* 股票上下文提示条 */}
      {hasContext && (
        <Tooltip title={t("office.chat.contextInjectedHint")}>
          <div
            style={{
              display: "flex",
              alignItems: "center",
              gap: 6,
              padding: "4px 8px",
              background: token.colorPrimaryBg,
              borderRadius: 4,
              fontSize: 11,
              color: token.colorPrimary,
            }}
          >
            <Tag color="blue" style={{ margin: 0, fontSize: 10 }}>
              {stockCode}
            </Tag>
            <span>
              {stockName}
              {quote ? ` · ${quote.price} (${quote.changePct}%)` : ""}
            </span>
          </div>
        </Tooltip>
      )}

      {/* 输入栏 */}
      <Space.Compact style={{ width: "100%" }}>
        <Input.TextArea
          value={input}
          onChange={(e) => setInput(e.target.value)}
          placeholder={t("office.chat.inputPlaceholder")}
          autoSize={{ minRows: 1, maxRows: 4 }}
          disabled={sending}
          onPressEnter={(e) => {
            if (!e.shiftKey) {
              e.preventDefault();
              void handleSend();
            }
          }}
        />
        <Button
          type="primary"
          icon={<Send size={14} />}
          loading={sending}
          onClick={handleSend}
          disabled={!input.trim()}
        >
          {t("office.chat.send")}
        </Button>
      </Space.Compact>

      {/* 状态栏 */}
      <div style={{ display: "flex", justifyContent: "space-between", fontSize: 11, color: token.colorTextTertiary }}>
        <Text type="secondary" style={{ fontSize: 11 }}>
          {t("office.chat.routingHint")}
        </Text>
        {loading && <Spin size="small" />}
      </div>
    </div>
  );
}

/** 持久化消息历史（来自数据库，按 seq 升序） */
function MessageList({ messages }: { messages: FleetMessage[] }) {
  const { t } = useTranslation();
  const { token } = theme.useToken();

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 6, marginBottom: 8 }}>
      {messages.map((m) => {
        const isHuman = m.authorKind === "human";
        return (
          <div
            key={m.id}
            style={{
              fontSize: 12,
              padding: "6px 8px",
              background: isHuman ? token.colorFillQuaternary : token.colorBgContainer,
              borderRadius: 4,
              border: `1px solid ${token.colorBorderSecondary}`,
            }}
          >
            <div
              style={{
                fontWeight: 600,
                color: isHuman ? token.colorTextSecondary : token.colorPrimary,
                marginBottom: 2,
                fontSize: 11,
              }}
            >
              {isHuman
                ? t("office.chat.speakerMe")
                : (m.authorDisplayName || m.authorSlug || "agent")}
            </div>
            <div style={{ color: token.colorText, whiteSpace: "pre-wrap" }}>{m.content}</div>
          </div>
        );
      })}
    </div>
  );
}

function EventList({ events }: { events: DispatchEvent[] }) {
  const { t } = useTranslation();
  const { token } = theme.useToken();

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
      {events.map((e, i) => {
        switch (e.type) {
          case "routing":
            return (
              <div key={i} style={{ fontSize: 11, color: token.colorTextTertiary }}>
                <Tag color="blue" style={{ fontSize: 10 }}>
                  {t("office.chat.tagRouting")}
                </Tag>
                <span>
                  {t("office.chat.routingMessage", { slug: e.agentSlug })}
                </span>
              </div>
            );
          case "agent_message":
            return (
              <div
                key={i}
                style={{
                  fontSize: 12,
                  padding: "6px 8px",
                  background: token.colorBgContainer,
                  borderRadius: 4,
                  border: `1px solid ${token.colorBorderSecondary}`,
                }}
              >
                <div style={{ fontWeight: 600, color: token.colorPrimary, marginBottom: 2, fontSize: 11 }}>
                  {e.agentSlug}
                </div>
                <div style={{ color: token.colorText, whiteSpace: "pre-wrap" }}>
                  {e.content}
                </div>
              </div>
            );
          case "process":
            return (
              <div key={i} style={{ fontSize: 11, color: token.colorTextQuaternary }}>
                <Tag color="purple" style={{ fontSize: 10 }}>
                  {t("office.chat.tagProcess")}
                </Tag>
                <span>{e.status}</span>
              </div>
            );
          case "token_usage":
            return (
              <div key={i} style={{ fontSize: 10, color: token.colorTextQuaternary }}>
                {t("office.chat.tokenUsage", {
                  slug: e.agentSlug,
                  input: e.inputTokens,
                  output: e.outputTokens,
                })}
              </div>
            );
          case "held":
            // 协调门暂扣：明确告诉用户「不是失败，是会话变了」以及下一步怎么做。
            // 文案带条数 + 房间水位（maxSeq）—— 条数说明「有什么新东西」，
            // 水位说明「你落后多远」（会话比基线推进到第几条）。
            // heldMessages 已由 store 合并进时间线（展示即已见），故此处只需提示。
            return (
              <div
                key={i}
                style={{
                  fontSize: 11,
                  color: token.colorWarning,
                  padding: "6px 8px",
                  background: token.colorWarningBg,
                  borderRadius: 4,
                }}
              >
                {t("office.chat.heldNotice", { count: e.heldMessages.length, seq: e.maxSeq })}
              </div>
            );
          case "complete":
            return (
              <div key={i} style={{ fontSize: 11, color: "#52c41a", fontWeight: 500 }}>
                ✓ {t("office.chat.completed")}
              </div>
            );
          case "error":
            return (
              <div key={i} style={{ fontSize: 11, color: "#ff4d4f" }}>
                ✗ {e.message}
              </div>
            );
          default:
            return null;
        }
      })}
    </div>
  );
}
