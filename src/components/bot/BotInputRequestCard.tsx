import { backendErrorText } from "@/lib/errorI18n";
import { useBotStore } from "@/stores/feature/botStore";
import type { BotInputRequest } from "@/types";
import { App, Button, Card, Input, Space, Tag, Typography } from "antd";
import { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";

const { Text } = Typography;

function fmtLeft(ms: number): string {
  const total = Math.max(0, Math.ceil(ms / 1000));
  const m = Math.floor(total / 60);
  const s = total % 60;
  return `${m}:${String(s).padStart(2, "0")}`;
}

export function BotInputRequestCard({ request }: { request: BotInputRequest }) {
  const { t } = useTranslation();
  const { message } = App.useApp();
  const answerInputRequest = useBotStore((s) => s.answerInputRequest);
  const cancelInputRequest = useBotStore((s) => s.cancelInputRequest);
  const [answer, setAnswer] = useState(request.answer ?? "");
  const [selected, setSelected] = useState<string | null>(request.answer ?? null);
  const [submitting, setSubmitting] = useState(false);
  const [now, setNow] = useState(Date.now());

  useEffect(() => {
    if (request.status !== "pending") { return; }
    const timer = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, [request.status]);

  useEffect(() => {
    if (request.answer) {
      setAnswer(request.answer);
      setSelected(request.answer);
    }
  }, [request.answer]);

  const pending = request.status === "pending";
  const expired = request.status === "expired";
  const cancelled = request.status === "cancelled";
  const answered = request.status === "answered";
  const trimmed = (selected ?? answer).trim();
  const canSubmit = pending && trimmed.length > 0 && !submitting;
  const remaining = Math.max(0, request.expiresAt - now);

  const statusText = useMemo(() => {
    if (answered) { return t("bot.input.answered"); }
    if (expired) { return t("bot.input.expired"); }
    if (cancelled) { return t("bot.input.cancelled"); }
    return t("bot.input.pendingBadge");
  }, [answered, cancelled, expired, t]);

  const onSubmit = useCallback(async () => {
    if (!canSubmit) { return; }
    setSubmitting(true);
    try {
      await answerInputRequest(request.id, trimmed);
    } catch (e) {
      message.error(backendErrorText(e) || t("bot.input.answerFailed"));
    } finally {
      setSubmitting(false);
    }
  }, [answerInputRequest, canSubmit, request.id, t, trimmed]);

  const onCancel = useCallback(async () => {
    if (!pending || submitting) { return; }
    setSubmitting(true);
    try {
      await cancelInputRequest(request.id);
    } catch (e) {
      message.error(backendErrorText(e));
    } finally {
      setSubmitting(false);
    }
  }, [cancelInputRequest, pending, request.id, submitting]);

  return (
    <Card size="small" data-testid="bot-input-request-card" style={{ marginTop: 8 }}>
      <Space direction="vertical" style={{ width: "100%" }} size={8}>
        <Space style={{ width: "100%", justifyContent: "space-between" }}>
          <Text strong>{t("bot.input.pendingTitle")}</Text>
          <Space size={8}>
            {pending ? <Text type="secondary">{t("bot.input.expiresIn", { time: fmtLeft(remaining) })}</Text> : null}
            <Tag
              data-testid="bot-input-request-status"
              color={answered ? "green" : expired ? "red" : cancelled ? "default" : "gold"}
            >
              {statusText}
            </Tag>
          </Space>
        </Space>

        <div>
          <Text type="secondary">{t("bot.input.question")}</Text>
          <div data-testid="bot-input-request-question" style={{ whiteSpace: "pre-wrap" }}>{request.question}</div>
        </div>

        {request.options.length > 0
          ? (
            <Space wrap data-testid="bot-input-request-options">
              {request.options.map((opt) => (
                <Button
                  key={opt}
                  size="small"
                  type={selected === opt ? "primary" : "default"}
                  disabled={!pending || submitting}
                  onClick={() => {
                    setSelected(opt);
                    setAnswer(opt);
                  }}
                >
                  {opt}
                </Button>
              ))}
            </Space>
          )
          : null}

        <Input.TextArea
          data-testid="bot-input-request-answer"
          value={answer}
          disabled={!pending || submitting}
          placeholder={t("bot.input.placeholder")}
          autoSize={{ minRows: 2, maxRows: 5 }}
          onChange={(e) => {
            setAnswer(e.target.value);
            setSelected(null);
          }}
        />

        {expired ? <Text type="secondary">{t("bot.input.expiredHint")}</Text> : null}

        <Space>
          <Button
            type="primary"
            data-testid="bot-input-request-submit"
            disabled={!canSubmit}
            loading={submitting && pending}
            onClick={() => {
              void onSubmit();
            }}
          >
            {t("bot.input.submit")}
          </Button>
          {pending
            ? (
              <Button
                data-testid="bot-input-request-cancel"
                disabled={submitting}
                onClick={() => {
                  void onCancel();
                }}
              >
                {t("bot.input.cancel")}
              </Button>
            )
            : null}
        </Space>
      </Space>
    </Card>
  );
}
