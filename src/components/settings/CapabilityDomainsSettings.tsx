// SPDX-License-Identifier: AGPL-3.0-only
// ! 设置页「能力域」面板 —— `PLAN-domain-single-source.md` §9.3 P2-⑤
//
// 面板职责：让用户改**覆盖层**（`capability_domain_overrides`）——
// 逐域启用/停用 + 追加别名。**不增删域**（域 id 集合由后端枚举持有）。
//
// ## 三条不可越界的约束（每条都在后端有对应判据）
//
// 1. **行来源只能是后端**：列表整体来自 `list_capability_domain_registry`，
//    本组件**不手抄任何 id 集合**。手抄的失败方式是静默的 —— 后端新增一个域后
//    界面少一行，而 `tsc` 拦不住（字符串数组漏一个元素仍是合法数组）。
// 2. **显示名派生**：直接用 DTO 的 `labelKey`（后端 `DomainNode::label_key()`），
//    不在组件里拼 `capabilityDomain.xxx` —— 两侧拼法不一致时界面显示裸 key，
//    而两侧各自都是合法代码（该契约由 `scripts/check-domain-single-source.mjs` 硬拦）。
// 3. **例外域不在这里维护**：`general` / `system` 不可停用，判据是 DTO 的
//    `toggleable`（后端 `is_toggleable` 派生）+ `toggleBlockReason`（原因码 → i18n）。
//    界面不复刻一份「哪些域例外」的表，也不自己写中文理由。
//
// ## 为什么每个动作各发一次命令（没有「保存全部」）
//
// 后端按域提供 `update_capability_domain`（局部更新 + **落库后回读**）。
// 逐域保存让「界面显示的状态 = 落库后回读的状态」这条不变量在每次操作后
// 重新成立一次；批量保存则需要界面表达「部分失败」（哪些域成功、哪些没有），
// 而后端返回的条目本就是逐域的 —— 那是自找的状态空间。
//
// ## 本面板**怎么**影响行为（三处消费端，均已在后端接线）
//
//   enabled      → ① L1 分类器 prompt 候选清单（`l1_classifier_domain_list`）
//                  ② L1 路由闸门（`DomainRouterImpl::route` 跳过停用域）
//                  ③ 能力过滤闸门（`check_domain_enabled`）
//   extraAliases → 用户输入 / LLM 输出的域解析（`resolve_enabled_domain`）
//
// 三者都是**后端运行时**读覆盖层，故保存后**无需重启**即生效。

import { showBackendError } from "@/lib/errorI18n";
import { invoke } from "@/lib/invoke";
import { message } from "@/lib/toast";
import type { CapabilityDomainEntry } from "@/types";
import { Alert, Button, Card, Input, Space, Spin, Switch, Tag, Tooltip, Typography } from "antd";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

const { Paragraph, Text, Title } = Typography;

/** 追加别名的编辑草稿 → 字符串数组（后端还会 trim / 去重 / 逐条校验）。 */
function parseAliasDraft(raw: string): string[] {
  return raw
    .split(/[,，]/)
    .map((s) => s.trim())
    .filter((s) => s.length > 0);
}

export function CapabilityDomainsSettings() {
  const { t } = useTranslation();
  // `null` = 尚未加载成功。刻意不用 `[]`：空数组会被渲染成「一个域都没有」，
  // 而真实原因是「还没取到 / 取失败」—— 两者在界面上必须有区别。
  const [entries, setEntries] = useState<CapabilityDomainEntry[] | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [pendingId, setPendingId] = useState<string | null>(null);
  /** 追加别名的编辑草稿；key 存在 = 该行处于编辑态（未落库） */
  const [aliasDrafts, setAliasDrafts] = useState<Record<string, string>>({});

  const load = useCallback(async () => {
    try {
      const list = await invoke<CapabilityDomainEntry[]>("list_capability_domain_registry");
      setEntries(list);
      setLoadError(null);
    } catch (e) {
      setLoadError(
        showBackendError(message, e, { context: "list_capability_domain_registry" }),
      );
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  /**
   * 提交一次局部更新。
   *
   * 成功时用**后端回读的条目**替换本行，而不是把发出去的意图写进状态：
   * 回读能同时暴露「写入被静默改写」与「落库的行在回读路径上解析失败」两类问题，
   * 而回显意图会把它们伪装成成功。
   */
  const applyUpdate = useCallback(
    async (domain: string, patch: { enabled?: boolean; extraAliases?: string[] }) => {
      setPendingId(domain);
      try {
        const updated = await invoke<CapabilityDomainEntry>("update_capability_domain", {
          request: { domain, ...patch },
        });
        setEntries((prev) => (prev ? prev.map((e) => (e.id === updated.id ? updated : e)) : prev));
        // 保存成功 ⇒ 丢弃该行草稿，输入框回到**落库值**（后端已做 trim / 去重）
        setAliasDrafts((prev) => {
          if (!(domain in prev)) {
            return prev;
          }
          const next = { ...prev };
          delete next[domain];
          return next;
        });
      } catch (e) {
        // 失败时**保留草稿**：用户改的内容不该因为一次可修复的报错而丢失
        showBackendError(message, e, { context: `update_capability_domain(${domain})` });
      } finally {
        setPendingId(null);
      }
    },
    [],
  );

  const blockReasonText = (entry: CapabilityDomainEntry): string =>
    t(`settings.capabilityDomains.blockReason.${entry.toggleBlockReason ?? "unknown"}`, {
      defaultValue: t("settings.capabilityDomains.blockReason.unknown"),
    });

  if (entries === null) {
    return loadError
      ? (
        <div style={{ padding: 24, maxWidth: 960 }} data-testid="capability-domains-panel">
          <Alert
            type="error"
            showIcon
            message={t("settings.capabilityDomains.loadFailed")}
            description={loadError}
            action={
              <Button size="small" onClick={() => void load()}>
                {t("settings.capabilityDomains.loadRetry")}
              </Button>
            }
          />
        </div>
      )
      : (
        <div style={{ display: "flex", justifyContent: "center", padding: 48 }}>
          <Spin />
        </div>
      );
  }

  const enabledCount = entries.filter((e) => e.enabled).length;
  const overriddenCount = entries.filter((e) => e.hasOverride).length;

  return (
    <div style={{ padding: 24, maxWidth: 960 }} data-testid="capability-domains-panel">
      <Title level={4} style={{ marginTop: 0 }}>
        {t("settings.capabilityDomains.title")}
      </Title>
      <Paragraph type="secondary">{t("settings.capabilityDomains.description")}</Paragraph>

      <Alert
        type="info"
        showIcon
        style={{ marginBottom: 16 }}
        message={t("settings.capabilityDomains.consumersTitle")}
        description={
          <ul style={{ margin: 0, paddingLeft: 18 }}>
            <li>{t("settings.capabilityDomains.consumerClassifier")}</li>
            <li>{t("settings.capabilityDomains.consumerRouter")}</li>
            <li>{t("settings.capabilityDomains.consumerFilter")}</li>
            <li>{t("settings.capabilityDomains.consumerAlias")}</li>
          </ul>
        }
      />

      <Space size="small" wrap style={{ marginBottom: 12 }}>
        <Tag>{t("settings.capabilityDomains.statTotal", { count: entries.length })}</Tag>
        <Tag color="green">
          {t("settings.capabilityDomains.statEnabled", { count: enabledCount })}
        </Tag>
        <Tag color="orange">
          {t("settings.capabilityDomains.statOverridden", { count: overriddenCount })}
        </Tag>
      </Space>

      {entries.map((entry) => {
        const pending = pendingId === entry.id;
        const storedDraft = entry.extraAliases.join(", ");
        const draft = aliasDrafts[entry.id] ?? storedDraft;
        const dirty = entry.id in aliasDrafts && aliasDrafts[entry.id] !== storedDraft;

        return (
          <Card
            key={entry.id}
            size="small"
            style={{ marginBottom: 12 }}
            title={
              <Space size="small" wrap>
                <Text strong>{t(entry.labelKey)}</Text>
                <Tag>{entry.id}</Tag>
                {entry.isSystem && <Tag>{t("settings.capabilityDomains.tagSystem")}</Tag>}
                {entry.hasOverride && <Tag color="orange">{t("settings.capabilityDomains.tagOverridden")}</Tag>}
                {!entry.enabled && <Tag color="red">{t("settings.capabilityDomains.tagDisabled")}</Tag>}
              </Space>
            }
            extra={
              // Tooltip 包一层 span：disabled 的 Switch 不派发鼠标事件，
              // 直接包 Tooltip 时「鼠标悬停看原因」在最需要它的场景（不可停用）失效。


                <Tooltip title={entry.toggleable ? undefined : blockReasonText(entry)}>
                  <span style={{ display: "inline-block" }}>
                    <Switch
                      checked={entry.enabled}
                      disabled={!entry.toggleable || pending}
                      loading={pending}
                      checkedChildren={t("settings.capabilityDomains.switchOn")}
                      unCheckedChildren={t("settings.capabilityDomains.switchOff")}
                      onChange={(checked) => void applyUpdate(entry.id, { enabled: checked })}
                    />
                  </span>
                </Tooltip>

            }
          >
            <div style={{ marginBottom: 8 }}>
              <Text type="secondary" style={{ marginRight: 8 }}>
                {t("settings.capabilityDomains.builtinAliases")}
              </Text>
              {entry.builtinAliases.length === 0
                ? <Text type="secondary">{t("settings.capabilityDomains.noAliases")}</Text>
                : entry.builtinAliases.map((a) => <Tag key={a}>{a}</Tag>)}
            </div>

            <Text type="secondary" style={{ display: "block", marginBottom: 4 }}>
              {t("settings.capabilityDomains.extraAliases")}
            </Text>
            <Space.Compact style={{ width: "100%" }}>
              <Input
                value={draft}
                disabled={pending}
                placeholder={t("settings.capabilityDomains.extraAliasesPlaceholder")}
                onChange={(e) => setAliasDrafts((prev) => ({ ...prev, [entry.id]: e.target.value }))}
              />
              <Button
                type="primary"
                disabled={!dirty || pending}
                loading={pending}
                onClick={() => void applyUpdate(entry.id, { extraAliases: parseAliasDraft(draft) })}
              >
                {t("settings.capabilityDomains.saveAliases")}
              </Button>
            </Space.Compact>

            <div style={{ marginTop: 8 }}>
              <Text type="secondary" style={{ marginRight: 8 }}>
                {t("settings.capabilityDomains.effectiveAliases")}
              </Text>
              <Text>{entry.effectiveAliases.join(", ") || t("settings.capabilityDomains.noAliases")}</Text>
            </div>

            <div style={{ marginTop: 8 }}>
              <Tooltip title={t("settings.capabilityDomains.restoreDefaultHint")}>
                <Button
                  size="small"
                  disabled={!entry.hasOverride || pending}
                  onClick={() => void applyUpdate(entry.id, { enabled: true, extraAliases: [] })}
                >
                  {t("settings.capabilityDomains.restoreDefault")}
                </Button>
              </Tooltip>
              {entry.navPath && (
                <Text type="secondary" style={{ marginLeft: 12 }}>
                  {t("settings.capabilityDomains.navPath")}
                  {": "}
                  {entry.navPath}
                </Text>
              )}
            </div>

            {!entry.toggleable && (
              <Paragraph type="secondary" style={{ marginTop: 8, marginBottom: 0 }}>
                {blockReasonText(entry)}
              </Paragraph>
            )}
          </Card>
        );
      })}
    </div>
  );
}
