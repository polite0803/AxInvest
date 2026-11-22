// SPDX-License-Identifier: AGPL-3.0-only

/**
 * SOUL 面板：查看 + 编辑（PLAN T4 前端落点：调 `read_bot_soul` / `write_bot_soul`）。
 *
 * 数据源：`read_bot_soul` 返回 DB 正文 + hash + 与镜像文件的漂移标记。
 * 漂移（drift=true）表示镜像文件被外部改过 —— P1 **只提示**，不自动合并；
 * 用户保存时以当前编辑内容为准（`write_bot_soul` 写 DB 并原子重写镜像，漂移随之消除）。
 */

import { useBotStore } from "@/stores/feature/botStore";
import type { BotSoul } from "@/types";
import { Alert, Button, Card, Input, Space, Spin, theme, Typography } from "antd";
import { Pencil } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

const { Text } = Typography;
const { TextArea } = Input;

export interface BotSoulPanelProps {
  botId: string;
}

export function BotSoulPanel({ botId }: BotSoulPanelProps) {
  const { t } = useTranslation();
  const { token } = theme.useToken();
  const readSoul = useBotStore((s) => s.readSoul);
  const writeSoul = useBotStore((s) => s.writeSoul);

  const [soul, setSoul] = useState<BotSoul | null>(null);
  const [loading, setLoading] = useState(true);
  const [failed, setFailed] = useState(false);
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState("");
  const [saving, setSaving] = useState(false);
  const [saveFailed, setSaveFailed] = useState(false);

  useEffect(() => {
    let alive = true;
    setLoading(true);
    setFailed(false);
    setEditing(false);
    setSaveFailed(false);
    setSoul(null);
    readSoul(botId)
      .then((value) => {
        if (alive) {
          setSoul(value);
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
  }, [botId, readSoul]);

  const startEdit = () => {
    setDraft(soul?.content ?? "");
    setSaveFailed(false);
    setEditing(true);
  };

  const handleSave = async () => {
    setSaving(true);
    setSaveFailed(false);
    try {
      const written = await writeSoul(botId, draft);
      setSoul(written);
      setEditing(false);
    } catch {
      setSaveFailed(true);
    } finally {
      setSaving(false);
    }
  };

  const hasContent = Boolean(soul && soul.content.trim());

  return (
    <Card
      size="small"
      title={t("bots.soulTitle")}
      data-testid="bot-soul-panel"
      extra={!loading && !failed && !editing && soul
        ? (
          <Button type="link" size="small" icon={<Pencil size={14} />} onClick={startEdit} data-testid="bot-soul-edit">
            {t("common.edit")}
          </Button>
        )
        : null}
    >
      {loading
        ? <Spin size="small" />
        : failed
        ? <Alert type="error" showIcon message={t("bots.soulLoadFailed")} />
        : editing
        ? (
          <Space orientation="vertical" style={{ width: "100%" }} size={8}>
            <TextArea
              value={draft}
              onChange={(e) => setDraft(e.target.value)}
              placeholder={t("bots.soulPlaceholder")}
              autoSize={{ minRows: 6, maxRows: 16 }}
              data-testid="bot-soul-editor"
            />
            {saveFailed ? <Alert type="error" showIcon message={t("bots.saveFailed")} /> : null}
            <Space>
              <Button type="primary" loading={saving} onClick={() => void handleSave()} data-testid="bot-soul-save">
                {t("bots.save")}
              </Button>
              <Button onClick={() => setEditing(false)}>{t("common.cancel")}</Button>
            </Space>
          </Space>
        )
        : (
          <Space orientation="vertical" style={{ width: "100%" }} size={8}>
            {soul?.drift
              ? <Alert type="warning" showIcon message={t("bots.soulDrift")} data-testid="bot-soul-drift" />
              : null}
            {hasContent
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
                    maxHeight: 320,
                    overflow: "auto",
                  }}
                >
                  {soul?.content}
                </pre>
              )
              : <Text type="secondary">{t("bots.soulEmpty")}</Text>}
          </Space>
        )}
    </Card>
  );
}
