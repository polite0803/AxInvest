// SPDX-License-Identifier: AGPL-3.0-only

import { Tooltip } from "@/components/layout/Tooltip";
import { usePluginGallery } from "@/hooks/usePluginGallery";
import { isTauri } from "@/lib/invoke";
import {
  isPluginSha256,
  isReleasePluginSource,
  isRemotePluginSource,
  normalizePluginSource,
} from "@/lib/pluginSourceSecurity";
import { message } from "@/lib/toast";
import type { PluginManifestDto } from "@/types";
import { Badge, Button, Card, Descriptions, Input, Modal, Select, Space, Tag, Typography } from "antd";
import { Boxes, CheckCircle, Code2, Loader2, PackageSearch, RefreshCw, XCircle } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

const { Text, Title } = Typography;

export function PluginGallery() {
  const { t } = useTranslation();

  // 插件清单 + 能力注册表读面（hook 内部聚合 pluginStore 与 capabilityStore）
  const {
    plugins,
    loading,
    installing,
    validating,
    loadPlugins,
    validateSource,
    installPlugin,
    updatePlugin,
    enablePlugin,
    disablePlugin,
    uninstallPlugin,
    error,
    registry,
    registryLoading,
    loadRegistry,
  } = usePluginGallery();

  // 组件本地 UI 状态（不适合放入全局 Store 的交互状态）
  const [installInput, setInstallInput] = useState("");
  const [confirmManifest, setConfirmManifest] = useState<PluginManifestDto | null>(null);
  const [confirmSource, setConfirmSource] = useState("");
  const [confirmSha256, setConfirmSha256] = useState("");
  const [confirmPages, setConfirmPages] = useState<string[]>([]);
  const [sha256Input, setSha256Input] = useState("");
  const [updatingPluginId, setUpdatingPluginId] = useState<string | null>(null);
  const [updateSha256Input, setUpdateSha256Input] = useState("");
  const [updateValidated, setUpdateValidated] = useState(false);
  const [updateConfirmSha256, setUpdateConfirmSha256] = useState("");
  const [registryQuery, setRegistryQuery] = useState("");
  const [pluginQuery, setPluginQuery] = useState("");
  const [sourceType, setSourceType] = useState<"all" | "release" | "remote" | "local">("all");
  const [enabledFilter, setEnabledFilter] = useState<"all" | "enabled" | "disabled">("all");
  const [integrityFilter, setIntegrityFilter] = useState<
    "all" | "verified" | "mismatch" | "unpinned" | "unknown"
  >("all");

  const normalizedInstallSource = normalizePluginSource(installInput);
  const expectedSha256 = sha256Input.trim();
  const sha256Valid = isPluginSha256(expectedSha256);
  const sourceRemote = isRemotePluginSource(normalizedInstallSource);
  const releaseSource = isReleasePluginSource(normalizedInstallSource);
  const releaseInstalling = !!releaseSource && installing === normalizedInstallSource;
  const confirmSourceRemote = isRemotePluginSource(confirmSource);
  const confirmReleaseSource = isReleasePluginSource(confirmSource);
  const confirmSha256Valid = isPluginSha256(confirmSha256);
  const updateSource = updatingPluginId
    ? plugins.find((p) => p.id === updatingPluginId)?.source ?? ""
    : "";
  const updateSourceRemote = isRemotePluginSource(updateSource);
  const updateReleaseSource = isReleasePluginSource(updateSource);
  const updateSha256 = updateSha256Input.trim();
  const updateSha256Valid = isPluginSha256(updateSha256);
  const updateConfirmSha256Valid = isPluginSha256(updateConfirmSha256);
  const sourceTypeLabelKey = (source: string): string => {
    if (isReleasePluginSource(source)) {
      return "chat.plugins.marketplace.sourceTypeRelease";
    }
    return isRemotePluginSource(source)
      ? "chat.plugins.marketplace.sourceTypeRemote"
      : "chat.plugins.marketplace.sourceTypeLocal";
  };

  // 初始化加载插件列表 + 能力注册表
  useEffect(() => {
    loadPlugins();
    loadRegistry();
  }, [loadPlugins, loadRegistry]);

  const filteredRegistry = registry.filter(
    (r) =>
      r.id.toLowerCase().includes(registryQuery.toLowerCase())
      || (r.pluginId ?? "").toLowerCase().includes(registryQuery.toLowerCase()),
  );

  const query = pluginQuery.trim().toLowerCase();
  const filteredPlugins = plugins.filter((plugin) => {
    const matchesQuery = !query
      || [plugin.name, plugin.source, ...plugin.permissions, ...plugin.pages]
        .some((value) => value.toLowerCase().includes(query));
    const release = isReleasePluginSource(plugin.source);
    const remote = !release && isRemotePluginSource(plugin.source);
    const matchesSource = sourceType === "all"
      || (sourceType === "release" && release)
      || (sourceType === "remote" && remote)
      || (sourceType === "local" && !release && !remote);
    const matchesEnabled = enabledFilter === "all"
      || (enabledFilter === "enabled" && plugin.enabled)
      || (enabledFilter === "disabled" && !plugin.enabled);
    const matchesIntegrity = integrityFilter === "all" || plugin.integrityStatus === integrityFilter;
    return matchesQuery && matchesSource && matchesEnabled && matchesIntegrity;
  });

  const integrityLabelKey = (status: string): string => {
    switch (status) {
      case "verified":
        return "chat.plugins.marketplace.integrityVerified";
      case "mismatch":
        return "chat.plugins.marketplace.integrityMismatch";
      case "unpinned":
        return "chat.plugins.marketplace.integrityUnpinned";
      default:
        return "chat.plugins.marketplace.integrityUnknown";
    }
  };

  const handleRefresh = useCallback(() => {
    loadPlugins();
  }, [loadPlugins]);

  const handleSearchInstall = async () => {
    const source = normalizePluginSource(installInput);
    if (!source) {
      return;
    }
    if (sourceRemote && !releaseSource) {
      if (!expectedSha256) {
        return;
      }
      if (!sha256Valid) {
        return;
      }
    }

    const manifest = await validateSource(
      source,
      releaseSource ? undefined : expectedSha256 || undefined,
    );
    if (!manifest) {
      // validateSource 在 store 中已经处理了错误日志
      // 这里可以根据需要补充提示
      return;
    }

    if (releaseSource) {
      const result = await installPlugin(source, manifest.expectedSha256 || undefined);
      if (result) {
        message.success(
          t("chat.plugins.marketplace.installSuccess", {
            id: result.pluginId,
            version: result.version,
          }),
        );
        setInstallInput("");
        setSha256Input("");
      }
      return;
    }

    setConfirmManifest(manifest);
    setConfirmSource(source);
    // 冻结预览返回/清单声明的哈希；后续用户改输入框不会影响确认内容。
    setConfirmSha256(manifest.expectedSha256 ?? expectedSha256);
    setConfirmPages(manifest.pages ?? []);
  };

  const handleConfirmInstall = async () => {
    if (!confirmSource) {
      return;
    }
    // 按钮禁用只是交互层；处理函数自身也必须守住同一门禁，避免回车/程序化触发绕过。
    if (confirmSourceRemote && !confirmReleaseSource && !confirmSha256Valid) {
      return;
    }

    const result = await installPlugin(confirmSource, confirmSha256 || undefined);
    if (result) {
      message.success(
        t("chat.plugins.marketplace.installSuccess", {
          id: result.pluginId,
          version: result.version,
        }),
      );
      setInstallInput("");
      setConfirmSource("");
      setConfirmSha256("");
      setConfirmPages([]);
      setConfirmManifest(null);
      setSha256Input("");
    } else {
      // 失败后不回退到旧弹窗：确认态与哈希快照必须一起清掉，下一次安装重新校验。
      setConfirmManifest(null);
      setConfirmSource("");
      setConfirmSha256("");
      setConfirmPages([]);
    }
  };

  const handleToggle = async (pluginId: string, enable: boolean) => {
    if (enable) {
      const ok = await enablePlugin(pluginId);
      if (!ok) {
        message.error(t("chat.plugins.marketplace.toggleFailed"));
      }
    } else {
      const ok = await disablePlugin(pluginId);
      if (!ok) {
        message.error(t("chat.plugins.marketplace.toggleFailed"));
      }
    }
  };

  const handleUninstall = async (pluginId: string) => {
    const ok = await uninstallPlugin(pluginId);
    if (!ok) {
      message.error(t("chat.plugins.marketplace.uninstallFailed"));
    }
  };

  const closeUpdateDialog = () => {
    setUpdatingPluginId(null);
    setUpdateSha256Input("");
    setUpdateValidated(false);
    setUpdateConfirmSha256("");
  };

  const handleValidateUpdate = async () => {
    if (!updatingPluginId || !updateSource) {
      message.error(t("chat.plugins.marketplace.updateSourceMissing"));
      return;
    }
    if (updateSourceRemote && !updateReleaseSource && !updateSha256) {
      return;
    }
    if (!updateReleaseSource && updateSha256 && !updateSha256Valid) {
      return;
    }
    const manifest = await validateSource(
      updateSource,
      updateReleaseSource ? undefined : updateSha256 || undefined,
    );
    if (manifest) {
      setUpdateConfirmSha256(updateReleaseSource ? manifest.expectedSha256 ?? "" : updateSha256);
      setUpdateValidated(true);
    }
  };

  const handleConfirmUpdate = async () => {
    if (!updatingPluginId) {
      return;
    }
    // 浏览器模式没有真实安装根，插件更新必须走桌面应用，不能伪造更新结果。
    if (!isTauri()) {
      message.error(t("chat.plugins.marketplace.browserNotice"));
      closeUpdateDialog();
      return;
    }
    if (updateSourceRemote && !updateReleaseSource && !updateConfirmSha256Valid) {
      return;
    }
    const outcome = await updatePlugin(updatingPluginId, updateConfirmSha256 || undefined);
    if (outcome) {
      message.success(
        t("chat.plugins.marketplace.updateSuccess", {
          id: updatingPluginId,
          version: outcome.newVersion,
        }),
      );
    } else {
      message.error(t("chat.plugins.marketplace.updateFailed"));
    }
    closeUpdateDialog();
  };

  return (
    <>
      <Card size="small">
        <div className="flex items-center justify-between mb-3">
          <Space>
            <PackageSearch size={16} className="text-purple-500" />
            <Title level={5} className="mb-0">
              {t("chat.plugins.marketplace.title")}
            </Title>
            <Badge count={plugins.length} size="small" />
          </Space>
          <Button size="small" onClick={handleRefresh} loading={loading}>
            {t("chat.plugins.marketplace.refresh")}
          </Button>
        </div>

        <Input.Search
          size="small"
          placeholder={t("chat.plugins.marketplace.listSearchPlaceholder")}
          value={pluginQuery}
          onChange={(e) => setPluginQuery(e.target.value)}
          allowClear
          className="w-72 mb-3"
        />

        <Space size="small" wrap className="mb-3">
          <Select<"all" | "release" | "remote" | "local">
            size="small"
            aria-label={t("chat.plugins.marketplace.sourceTypeFilter")}
            value={sourceType}
            onChange={setSourceType}
            options={[
              { value: "all", label: t("chat.plugins.marketplace.sourceTypeAll") },
              { value: "release", label: t("chat.plugins.marketplace.sourceTypeRelease") },
              { value: "remote", label: t("chat.plugins.marketplace.sourceTypeRemote") },
              { value: "local", label: t("chat.plugins.marketplace.sourceTypeLocal") },
            ]}
            style={{ minWidth: 112 }}
            popupMatchSelectWidth={false}
          />
          <Select<"all" | "enabled" | "disabled">
            size="small"
            aria-label={t("chat.plugins.marketplace.enabledFilter")}
            value={enabledFilter}
            onChange={setEnabledFilter}
            options={[
              { value: "all", label: t("chat.plugins.marketplace.enabledAll") },
              { value: "enabled", label: t("chat.plugins.marketplace.enabledOnly") },
              { value: "disabled", label: t("chat.plugins.marketplace.disabledOnly") },
            ]}
            style={{ minWidth: 112 }}
            popupMatchSelectWidth={false}
          />
          <Select<"all" | "verified" | "mismatch" | "unpinned" | "unknown">
            size="small"
            aria-label={t("chat.plugins.marketplace.integrityFilter")}
            value={integrityFilter}
            onChange={setIntegrityFilter}
            options={[
              { value: "all", label: t("chat.plugins.marketplace.integrityAll") },
              { value: "verified", label: t("chat.plugins.marketplace.integrityVerified") },
              { value: "mismatch", label: t("chat.plugins.marketplace.integrityMismatch") },
              { value: "unpinned", label: t("chat.plugins.marketplace.integrityUnpinned") },
              { value: "unknown", label: t("chat.plugins.marketplace.integrityUnknown") },
            ]}
            style={{ minWidth: 132 }}
            popupMatchSelectWidth={false}
          />
        </Space>

        <div className="mb-3">
          <Space.Compact>
            <Input
              placeholder={releaseSource
                ? t("chat.plugins.marketplace.releasePlaceholder")
                : t("chat.plugins.marketplace.installPlaceholder")}
              value={installInput}
              onChange={(e) => setInstallInput(e.target.value)}
              onPressEnter={handleSearchInstall}
            />
            <Button
              type="primary"
              loading={validating || releaseInstalling}
              disabled={validating || releaseInstalling
                || (!normalizedInstallSource || (!sourceRemote && !releaseSource))
                || (sourceRemote && !releaseSource && !expectedSha256)
                || (!!expectedSha256 && !sha256Valid)}
              onClick={handleSearchInstall}
            >
              {t("chat.plugins.marketplace.install")}
            </Button>
          </Space.Compact>
          <Input
            className="mt-2 font-mono"
            placeholder={releaseSource
              ? t("chat.plugins.marketplace.sha256FromRelease")
              : t("chat.plugins.marketplace.sha256Placeholder")}
            value={sha256Input}
            onChange={(e) => setSha256Input(e.target.value)}
            status={expectedSha256 && !sha256Valid ? "error" : undefined}
          />
          <Text
            type={expectedSha256 && !sha256Valid
              ? "danger"
              : sourceRemote && !expectedSha256 && !releaseSource
              ? "warning"
              : "secondary"}
            className="text-xs block mt-1"
          >
            {releaseSource
              ? t("chat.plugins.marketplace.sha256FromRelease")
              : expectedSha256 && !sha256Valid
              ? t("chat.plugins.marketplace.sha256Invalid")
              : sourceRemote && !expectedSha256 && !releaseSource
              ? t("chat.plugins.marketplace.sha256Required")
              : t("chat.plugins.marketplace.sha256Hint")}
          </Text>
          {error && (
            <Text type="danger" className="text-xs block mt-1">
              {error}
            </Text>
          )}
          {releaseSource && (
            <Text type="secondary" className="text-xs block mt-1">
              {t("chat.plugins.marketplace.releaseHint")}
            </Text>
          )}
        </div>

        {loading && plugins.length === 0 && (
          <div className="flex items-center gap-2 py-4 text-sm text-zinc-500">
            <Loader2 size={14} className="animate-spin" />
            <span>{t("chat.plugins.marketplace.loading")}</span>
          </div>
        )}

        {!loading && filteredPlugins.length === 0 && (
          <Text type="secondary" className="text-xs block">
            {t("chat.plugins.marketplace.listEmpty")}
          </Text>
        )}

        {!isTauri() && (
          <Text type="secondary" className="text-xs block mt-2">
            {t("chat.plugins.marketplace.browserNotice")}
          </Text>
        )}

        <div className="space-y-2 max-h-96 overflow-auto">
          {filteredPlugins.map((plugin) => (
            <Card key={plugin.id} size="small" className="plugin-card">
              <div className="flex items-start justify-between">
                <div className="flex-1">
                  <div className="flex items-center gap-2">
                    <Code2 size={14} className="text-purple-500" />
                    <Text strong className="text-sm">
                      {plugin.name}
                    </Text>
                    <Tag color="purple" className="text-xs">
                      {plugin.version}
                    </Tag>
                    {plugin.enabled && <CheckCircle size={12} className="text-green-500" />}
                    {!plugin.enabled && (
                      <Tag color="default" className="text-xs">
                        {t("chat.plugins.marketplace.disabledBadge")}
                      </Tag>
                    )}
                  </div>
                  <Text type="secondary" className="text-xs block mt-1">
                    {plugin.description}
                  </Text>
                  <Text type="secondary" className="text-xs block mt-1 break-all">
                    {t("chat.plugins.marketplace.source")}: {plugin.source} · {t(sourceTypeLabelKey(plugin.source))}
                  </Text>
                  <Space size="small" className="mt-1">
                    <Tag color="geekblue" className="text-xs">
                      {plugin.kind}
                    </Tag>
                    <Tag
                      color={plugin.integrityStatus === "verified"
                        ? "green"
                        : plugin.integrityStatus === "mismatch"
                        ? "red"
                        : "default"}
                      className="text-xs"
                    >
                      {t("chat.plugins.marketplace.integrity")}: {t(integrityLabelKey(plugin.integrityStatus))}
                    </Tag>
                    {plugin.pages.length > 0 && (
                      <Tag color="blue" className="text-xs">
                        {t("chat.plugins.marketplace.pages")}: {plugin.pages.join(", ")}
                      </Tag>
                    )}
                    {(plugin.mcpServers.length > 0
                      || plugin.skills.length > 0) && (
                      <Text type="secondary" className="text-xs">
                        MCP:{plugin.mcpServers.length} Skills:
                        {plugin.skills.length}
                      </Text>
                    )}
                  </Space>
                </div>

                <div className="flex items-center gap-1">
                  <Button
                    size="small"
                    icon={<RefreshCw size={12} />}
                    onClick={() => setUpdatingPluginId(plugin.id)}
                  >
                    {t("chat.plugins.marketplace.update")}
                  </Button>
                  <Button
                    size="small"
                    type={plugin.enabled ? "default" : "primary"}
                    onClick={() => handleToggle(plugin.id, !plugin.enabled)}
                  >
                    {plugin.enabled
                      ? t("chat.plugins.marketplace.disable")
                      : t("chat.plugins.marketplace.enable")}
                  </Button>
                  <Button
                    size="small"
                    danger
                    icon={<XCircle size={12} />}
                    loading={installing === plugin.id}
                    onClick={() => handleUninstall(plugin.id)}
                  />
                </div>
              </div>

              {plugin.tools.length > 0 && (
                <div className="flex gap-2 mt-2 flex-wrap">
                  {plugin.tools.slice(0, 5).map((tool, _i) => (
                    <Tag key={tool} color="cyan" className="text-xs">
                      {tool}
                    </Tag>
                  ))}
                  {plugin.tools.length > 5 && (
                    <Text type="secondary" className="text-xs">
                      +{plugin.tools.length - 5}
                    </Text>
                  )}
                </div>
              )}

              {plugin.permissions.length > 0 && (
                <div className="flex gap-2 mt-2 flex-wrap">
                  {plugin.permissions.map((permission) => (
                    <Tag key={permission} color="volcano" className="text-xs">
                      {permission}
                    </Tag>
                  ))}
                </div>
              )}

              {/* `eventStats === null` = 没挂订阅；`{0,0}` = 挂着但没来一条 —— 两种状态不许合并显示。 */}
              <div className="mt-2">
                {plugin.eventStats
                  ? (
                    <Tooltip
                      title={t("chat.plugins.marketplace.eventStatsDetail", {
                        serialize: plugin.eventStats.droppedSerialize,
                        invoke: plugin.eventStats.droppedInvoke,
                        unavailable: plugin.eventStats.droppedUnavailable,
                        wait: plugin.eventStats.droppedWait,
                      })}
                    >
                      <span className="text-xs text-text-quaternary cursor-help">
                        {t("chat.plugins.marketplace.eventStats", {
                          delivered: plugin.eventStats.delivered,
                          dropped: plugin.eventStats.droppedTotal,
                        })}
                      </span>
                    </Tooltip>
                  )
                  : (
                    <Text type="secondary" className="text-xs">
                      {t("chat.plugins.marketplace.eventStatsNone")}
                    </Text>
                  )}
              </div>
            </Card>
          ))}
        </div>
      </Card>

      <Card size="small" className="mt-3">
        <div className="flex items-center justify-between mb-3 gap-2">
          <Space>
            <Boxes size={16} className="text-teal-500" />
            <Title level={5} className="mb-0">
              {t("chat.plugins.marketplace.registryTitle")}
            </Title>
            <Badge count={registry.length} size="small" />
          </Space>
          <Input.Search
            size="small"
            placeholder={t("chat.plugins.marketplace.registrySearchPlaceholder")}
            value={registryQuery}
            onChange={(e) => setRegistryQuery(e.target.value)}
            allowClear
            className="w-56"
          />
        </div>

        {registryLoading && registry.length === 0 && (
          <div className="flex items-center gap-2 py-4 text-sm text-zinc-500">
            <Loader2 size={14} className="animate-spin" />
            <span>{t("chat.plugins.marketplace.registryLoading")}</span>
          </div>
        )}

        {!registryLoading && registry.length === 0 && (
          <Text type="secondary" className="text-xs">
            {t("chat.plugins.marketplace.registryEmpty")}
          </Text>
        )}

        <div className="space-y-2 max-h-96 overflow-auto">
          {filteredRegistry.map((r) => (
            <div
              key={`${r.pluginId ?? "builtin"}:${r.id}`}
              className="flex items-start justify-between gap-2 border border-zinc-200 rounded-md px-2 py-1.5"
            >
              <div className="min-w-0">
                <div className="flex items-center gap-2">
                  <Text strong className="text-xs" ellipsis>
                    {r.id}
                  </Text>
                  {r.implemented
                    ? (
                      <Tag
                        color={r.origin === "builtin" ? "green" : "purple"}
                        className="text-xs shrink-0"
                      >
                        {r.origin === "builtin"
                          ? t("chat.plugins.marketplace.registryBuiltin")
                          : t("chat.plugins.marketplace.registryExternal")}
                      </Tag>
                    )
                    : (
                      <Tag color="orange" className="text-xs shrink-0">
                        {t("chat.plugins.marketplace.registryDeclared")}
                      </Tag>
                    )}
                </div>
                <Text type="secondary" className="text-xs block truncate">
                  {r.contract}
                </Text>
              </div>
              <div className="flex items-center gap-2 shrink-0">
                {r.pluginId && (
                  <Tag color="geekblue" className="text-xs">
                    {t("chat.plugins.marketplace.registryPlugin")} · {r.pluginId}
                  </Tag>
                )}
                <Tag className="text-xs">
                  {t("chat.plugins.marketplace.registryVersion")}: {r.version}
                </Tag>
              </div>
            </div>
          ))}
        </div>
      </Card>

      <Modal
        title={t("chat.plugins.marketplace.installTitle", {
          name: confirmManifest?.name ?? "",
        })}
        open={!!confirmManifest}
        onOk={handleConfirmInstall}
        onCancel={() => {
          setConfirmManifest(null);
          setConfirmSource("");
          setConfirmSha256("");
          setConfirmPages([]);
          setUpdateConfirmSha256("");
        }}
        okButtonProps={{
          disabled: confirmSourceRemote && !confirmReleaseSource && !confirmSha256Valid,
        }}
        okText={t("chat.plugins.marketplace.confirmInstall")}
        cancelText={t("chat.plugins.marketplace.cancel")}
        width={560}
      >
        {confirmManifest && (
          <Descriptions column={1} size="small" bordered>
            <Descriptions.Item label={t("chat.plugins.marketplace.version")}>
              {confirmManifest.version}
            </Descriptions.Item>
            <Descriptions.Item label={t("chat.plugins.marketplace.installSource")}>
              <Text className="break-all">
                {confirmSource} · {t(sourceTypeLabelKey(confirmSource))}
              </Text>
            </Descriptions.Item>
            <Descriptions.Item label={t("chat.plugins.marketplace.sha256Label")}>
              {confirmReleaseSource
                ? (
                  <Text className="font-mono break-all">
                    {confirmSha256 || t("chat.plugins.marketplace.sha256FromRelease")}
                  </Text>
                )
                : confirmSha256
                ? <Text className="font-mono break-all">{confirmSha256}</Text>
                : confirmSourceRemote
                ? <Text type="danger">{t("chat.plugins.marketplace.sha256Required")}</Text>
                : t("chat.plugins.marketplace.none")}
            </Descriptions.Item>
            <Descriptions.Item
              label={t("chat.plugins.marketplace.description")}
            >
              {confirmManifest.description}
            </Descriptions.Item>
            <Descriptions.Item
              label={t("chat.plugins.marketplace.permissions")}
            >
              {confirmManifest.permissions.length > 0
                ? confirmManifest.permissions.join(", ")
                : t("chat.plugins.marketplace.none")}
            </Descriptions.Item>
            <Descriptions.Item label={t("chat.plugins.marketplace.pages")}>
              {confirmPages.length > 0
                ? confirmPages.join(", ")
                : t("chat.plugins.marketplace.none")}
            </Descriptions.Item>
            <Descriptions.Item label={t("chat.plugins.marketplace.mcpServers")}>
              {confirmManifest.mcpServers.length > 0
                ? confirmManifest.mcpServers
                  .map((s) => `${s.name} (${s.command})`)
                  .join(", ")
                : t("chat.plugins.marketplace.none")}
            </Descriptions.Item>
            <Descriptions.Item label={t("chat.plugins.marketplace.skills")}>
              {confirmManifest.skills.length > 0
                ? confirmManifest.skills.map((s) => s.name).join(", ")
                : t("chat.plugins.marketplace.none")}
            </Descriptions.Item>
            <Descriptions.Item label={t("chat.plugins.marketplace.tools")}>
              {confirmManifest.tools.length > 0
                ? confirmManifest.tools.map((tool) => tool.name).join(", ")
                : t("chat.plugins.marketplace.none")}
            </Descriptions.Item>
          </Descriptions>
        )}
      </Modal>

      <Modal
        title={t("chat.plugins.marketplace.updateTitle", {
          name: plugins.find((p) => p.id === updatingPluginId)?.name ?? "",
        })}
        open={!!updatingPluginId}
        onCancel={closeUpdateDialog}
        onOk={updateValidated ? handleConfirmUpdate : handleValidateUpdate}
        okText={updateValidated
          ? t("chat.plugins.marketplace.confirmUpdate")
          : t("chat.plugins.marketplace.validate")}
        cancelText={t("chat.plugins.marketplace.cancel")}
        okButtonProps={{
          loading: validating,
          disabled: updateValidated
            ? updateSourceRemote && !updateReleaseSource && !updateConfirmSha256Valid
            : (updateSourceRemote && !updateReleaseSource && !updateSha256)
              || (!updateReleaseSource && !!updateSha256 && !updateSha256Valid),
        }}
        width={560}
      >
        <Descriptions column={1} size="small" bordered>
          <Descriptions.Item label={t("chat.plugins.marketplace.installSource")}>
            <Text className="break-all">
              {updateSource} · {t(sourceTypeLabelKey(updateSource))}
            </Text>
          </Descriptions.Item>
          <Descriptions.Item label={t("chat.plugins.marketplace.sha256Label")}>
            {updateReleaseSource
              ? updateValidated
                ? (
                  <Text className="font-mono break-all">
                    {updateConfirmSha256 || t("chat.plugins.marketplace.sha256FromRelease")}
                  </Text>
                )
                : (
                  <Input
                    className="font-mono"
                    placeholder={t("chat.plugins.marketplace.sha256FromRelease")}
                    value=""
                    disabled
                  />
                )
              : updateValidated
              ? <Text className="font-mono break-all">{updateConfirmSha256}</Text>
              : (
                <Input
                  className="font-mono"
                  placeholder={updateSourceRemote
                    ? t("chat.plugins.marketplace.sha256Required")
                    : t("chat.plugins.marketplace.sha256Placeholder")}
                  value={updateSha256Input}
                  onChange={(e) => setUpdateSha256Input(e.target.value)}
                  status={updateSha256 && !updateSha256Valid ? "error" : undefined}
                />
              )}
          </Descriptions.Item>
        </Descriptions>
        <Text
          type={updateReleaseSource
            ? "secondary"
            : updateSha256 && !updateSha256Valid
            ? "danger"
            : updateSourceRemote && !updateSha256
            ? "warning"
            : "secondary"}
          className="text-xs block mt-2"
        >
          {updateReleaseSource
            ? t("chat.plugins.marketplace.sha256FromRelease")
            : updateSha256 && !updateSha256Valid
            ? t("chat.plugins.marketplace.sha256Invalid")
            : updateSourceRemote && !updateSha256
            ? t("chat.plugins.marketplace.sha256Required")
            : t("chat.plugins.marketplace.sha256Hint")}
        </Text>
        {error && (
          <Text type="danger" className="text-xs block mt-1">
            {error}
          </Text>
        )}
      </Modal>
    </>
  );
}
