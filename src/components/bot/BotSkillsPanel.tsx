// SPDX-License-Identifier: AGPL-3.0-only

/**
 * Bot「技能」页签（P5-c1 C1-4）。
 *
 * 数据源：`list_bot_skills` / `set_bot_skill_enabled` / `import_bot_skill_preset`。
 * 展示 `source(global/bot)`、`enabled`、`shadowedGlobal`、`missing`；预设导入按整包校验，
 * 白名单外条目跳过并回报（ADR-P5-5/6）。
 */

import { backendErrorText } from "@/lib/errorI18n";
import { useBotStore } from "@/stores/feature/botStore";
import type { BotSkill, BotSkillPreset, BotSkillPresetResult } from "@/types";
import { App, Button, Input, List, Space, Switch, Tag, Typography } from "antd";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

const { Text, Paragraph } = Typography;

const DEFAULT_PRESET_TEMPLATE = JSON.stringify(
  {
    version: "bot-skill-preset.v1",
    name: "my-preset",
    description: "",
    skills: [
      {
        name: "example-skill",
        description: "Example skill",
        content: "# example-skill\n\nDescribe what this skill does.\n",
      },
    ],
  },
  null,
  2,
);

export interface BotSkillsPanelProps {
  botId: string;
}

/** locale 用单花括号 `{count}`（I18N-KEYS-P5 §4 由前端格式化），react-i18next 默认不认，手动替换。 */
function formatCount(text: string, count: number): string {
  return text.replaceAll("{count}", String(count));
}

function toResultText(result: BotSkillPresetResult, t: (key: string) => string): string {
  const parts = [formatCount(t("bot.skills.presetImported"), result.imported.length)];
  if (result.skipped.length > 0) {
    parts.push(formatCount(t("bot.skills.presetSkipped"), result.skipped.length));
  }
  return parts.join("；");
}

export function BotSkillsPanel({ botId }: BotSkillsPanelProps) {
  const { t } = useTranslation();
  const { message } = App.useApp();
  const listBotSkills = useBotStore((s) => s.listBotSkills);
  const setBotSkillEnabled = useBotStore((s) => s.setBotSkillEnabled);
  const importBotSkillPreset = useBotStore((s) => s.importBotSkillPreset);
  const saving = useBotStore((s) => s.botSkillsSaving);

  const [skills, setSkills] = useState<BotSkill[]>([]);
  const [loading, setLoading] = useState(true);
  const [errorText, setErrorText] = useState<string | null>(null);
  const [presetText, setPresetText] = useState(DEFAULT_PRESET_TEMPLATE);
  const [presetResult, setPresetResult] = useState<BotSkillPresetResult | null>(null);
  const [importError, setImportError] = useState<string | null>(null);

  const reload = useCallback(async () => {
    const next = await listBotSkills(botId);
    setSkills(next);
  }, [botId, listBotSkills]);

  useEffect(() => {
    let alive = true;
    setLoading(true);
    setErrorText(null);
    void reload()
      .catch((e) => {
        if (alive) {
          setErrorText(backendErrorText(e));
        }
      })
      .finally(() => {
        if (alive) {
          setLoading(false);
        }
      });
    return () => {
      alive = false;
    };
  }, [reload]);

  const onToggle = useCallback(async (skill: BotSkill, enabled: boolean) => {
    setErrorText(null);
    try {
      const row = await setBotSkillEnabled(botId, skill.name, enabled);
      setSkills((prev) => prev.map((item) => (item.name === row.name ? row : item)));
    } catch (e) {
      setErrorText(backendErrorText(e));
    }
  }, [botId, setBotSkillEnabled]);

  const onImport = useCallback(async () => {
    setImportError(null);
    setPresetResult(null);
    let parsed: BotSkillPreset;
    try {
      parsed = JSON.parse(presetText) as BotSkillPreset;
    } catch (e) {
      const detail = e instanceof Error ? e.message : String(e);
      setImportError(`${t("bot.skills.presetInvalid")}: ${detail}`);
      return;
    }
    try {
      const result = await importBotSkillPreset(botId, parsed);
      setPresetResult(result);
      message.success(toResultText(result, t));
      await reload();
    } catch (e) {
      setImportError(backendErrorText(e));
    }
  }, [botId, importBotSkillPreset, message, presetText, reload, t]);

  return (
    <div data-testid="bot-skills-panel" style={{ display: "flex", flexDirection: "column", gap: 12 }}>
      <div>
        <Text strong>{t("bot.skills.title")}</Text>
        <div>
          <Text type="secondary" style={{ fontSize: 12 }}>{t("bot.skills.subtitle")}</Text>
        </div>
      </div>

      {errorText ? <Text type="danger" data-testid="bot-skills-error">{errorText}</Text> : null}

      {loading && skills.length === 0
        ? <List size="small" loading />
        : skills.length === 0
        ? <Text type="secondary" data-testid="bot-skills-empty">{t("bot.skills.empty")}</Text>
        : (
          <List
            size="small"
            dataSource={skills}
            renderItem={(skill) => (
              <List.Item
                data-testid={`bot-skill-row-${skill.name}`}
                actions={[
                  <Switch
                    key="enabled"
                    size="small"
                    checked={skill.enabled}
                    disabled={saving}
                    onChange={(checked) => {
                      void onToggle(skill, checked);
                    }}
                    aria-label={`${t("bot.skills.enabled")}:${skill.name}`}
                    data-testid={`bot-skill-enabled-${skill.name}`}
                  />,
                ]}
              >
                <div style={{ display: "flex", flexDirection: "column", gap: 4, minWidth: 0, flex: 1 }}>
                  <Space size={4} wrap>
                    <Text strong ellipsis>{skill.name}</Text>
                    <Tag data-testid={`bot-skill-source-${skill.name}`}>
                      {skill.source === "global" ? t("bot.skills.globalSource") : t("bot.skills.botSource")}
                    </Tag>
                    {skill.shadowedGlobal
                      ? (
                        <Tag color="orange" data-testid={`bot-skill-shadowed-${skill.name}`}>
                          {t("bot.skills.shadowedGlobal")}
                        </Tag>
                      )
                      : null}
                    {skill.missing
                      ? (
                        <Tag color="red" data-testid={`bot-skill-missing-${skill.name}`}>
                          {t("bot.skills.missing")}
                        </Tag>
                      )
                      : null}
                    {!skill.enabled ? <Tag>{t("bot.skills.disabled")}</Tag> : null}
                  </Space>
                  {skill.description
                    ? <Text type="secondary" style={{ fontSize: 12 }} ellipsis>{skill.description}</Text>
                    : null}
                </div>
              </List.Item>
            )}
          />
        )}

      <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
        <Text strong>{t("bot.skills.importPreset")}</Text>
        <Input.TextArea
          rows={8}
          value={presetText}
          onChange={(e) => setPresetText(e.target.value)}
          data-testid="bot-skills-preset-input"
          spellCheck={false}
        />
        <Space>
          <Button
            type="primary"
            onClick={() => {
              void onImport();
            }}
            loading={saving}
            data-testid="bot-skills-import"
          >
            {t("bot.skills.importPreset")}
          </Button>
        </Space>
        {importError
          ? <Text type="danger" data-testid="bot-skills-import-error">{importError}</Text>
          : null}
        {presetResult
          ? (
            <div data-testid="bot-skills-import-result">
              <Text>{toResultText(presetResult, t)}</Text>
              {presetResult.skipped.length > 0
                ? (
                  <Paragraph type="secondary" style={{ margin: "4px 0 0", fontSize: 12 }}>
                    {presetResult.skipped.map((item) => `${item.name}: ${item.reason} ${item.detail}`).join(" | ")}
                  </Paragraph>
                )
                : null}
            </div>
          )
          : null}
      </div>
    </div>
  );
}
