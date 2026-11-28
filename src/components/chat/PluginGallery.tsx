// SPDX-License-Identifier: AGPL-3.0-only

import { Tooltip } from "@/components/layout/Tooltip";
import { usePluginGallery } from "@/hooks/usePluginGallery";
import { isTauri } from "@/lib/invoke";
import { isReleasePluginSource, normalizePluginSource } from "@/lib/pluginSourceSecurity";
import { message } from "@/lib/toast";
import type { PluginSourceValidationDto } from "@/types";
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
  const [confirmManifest, setConfirmManifest] = useState<PluginSourceValidationDto | null>(null);
  const [confirmSource, setConfirmSource] = useState("");
  const [confirmPages, setConfirmPages] = useState<string[]>([]);
  const [updatingPluginId, setUpdatingPluginId] = useState<string | null>(null);
  const [updateValidated, setUpdateValidated] = useState(false);
  const [updateManifest, setUpdateManifest] = useState<PluginSourceValidationDto | null>(null);
  const [registryQuery, setRegistryQuery] = useState("");
  const [pluginQuery, setPluginQuery] = useState("");
  const [sourceType, setSourceType] = useState<"all" | "release">("all");
  const [enabledFilter, setEnabledFilter] = useState<"all" | "enabled" | "disabled">("all");
  const [integrityFilter, setIntegrityFilter] = useState<
    "all" | "verified" | "mismatch" | "unpinned" | "unknown"
  >("all");

  const normalizedInstallSource = normalizePluginSource(installInput);
  const releaseSource = isReleasePluginSource(normalizedInstallSource);
  const releaseInstalling = !!releaseSource && installing === normalizedInstallSource;
  const updateSource = updatingPluginId
    ? plugins.find((p) => p.id === updatingPluginId)?.source ?? ""
    : "";
  const updateReleaseSource = isReleasePluginSource(updateSource);

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
    const matchesSource = sourceType === "all"
      || (sourceType === "release" && isReleasePluginSource(plugin.source));
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
    if (!releaseSource || !source) {
      return;
    }
    // 用户唯一输入是远程发布地址；哈希与完整性由后端从 release.json 自行校验。
    const manifest = await validateSource(source);
    if (!manifest) {
      return;
    }
    setConfirmManifest(manifest);
    setConfirmSource(source);
    setConfirmPages(manifest.pages ?? []);
  };

  const handleConfirmInstall = async () => {
    if (!confirmSource || !confirmManifest) {
      return;
    }
    const result = await installPlugin(confirmSource, confirmManifest.previewToken);
    if (result) {
      message.success(
        t("chat.plugins.marketplace.installSuccess", {
          id: result.pluginId,
          version: result.version,
        }),
      );
      setInstallInput("");
      setConfirmSource("");
      setConfirmPages([]);
      setConfirmManifest(null);
    } else {
      // 失败后清掉确认态，下一次安装必须重新预览，不能复用旧 token。
      setConfirmManifest(null);
      setConfirmSource("");
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
    setUpdateValidated(false);
  };

  const handleValidateUpdate = async () => {
    if (!updatingPluginId || !updateSource || !updateReleaseSource) {
      message.error(t("chat.plugins.marketplace.updateSourceMissing"));
      return;
    }
    const manifest = await validateSource(updateSource);
    if (manifest) {
      setUpdateManifest(manifest);
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
    if (!updateManifest) {
      closeUpdateDialog();
      return;
    }
    const outcome = await updatePlugin(updatingPluginId, updateManifest.previewToken);
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
          <Select<"all" | "release">
            size="small"
            aria-label={t("chat.plugins.marketplace.sourceTypeFilter")}
            value={sourceType}
            onChange={setSourceType}
            options={[
              { value: "all", label: t("chat.plugins.marketplace.sourceTypeAll") },
              { value: "release", label: t("chat.plugins.marketplace.sourceTypeRelease") },
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
              placeholder={t("chat.plugins.marketplace.releasePlaceholder")}
              value={installInput}
              onChange={(e) => setInstallInput(e.target.value)}
              onPressEnter={handleSearchInstall}
            />
            <Button
              type="primary"
              loading={validating || releaseInstalling}
              disabled={validating || releaseInstalling || !releaseSource}
              onClick={handleSearchInstall}
            >
              {t("chat.plugins.marketplace.install")}
            </Button>
          </Space.Compact>
          {error && (
            <Text type="danger" className="text-xs block mt-1">
              {error}
            </Text>
          )}
          <Text type="secondary" className="text-xs block mt-1">
            {t("chat.plugins.marketplace.releaseHint")}
          </Text>
          <Text type="secondary" className="text-xs block mt-1">
            {t("chat.plugins.marketplace.remoteOnlyHint")}
          </Text>
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
                    {t("chat.plugins.marketplace.source")}: {plugin.source}
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
          setConfirmPages([]);
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
              <Text className="break-all">{confirmSource}</Text>
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
            <Descriptions.Item label={t("chat.plugins.marketplace.installAfterEnable")}>
              {t("chat.plugins.marketplace.installAfterEnableHint")}
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
        onOk={handleValidateUpdate}
        okText={t("chat.plugins.marketplace.validate")}
        cancelText={t("chat.plugins.marketplace.cancel")}
        okButtonProps={{
          loading: validating,
          disabled: !updateReleaseSource,
        }}
        footer={
          <>
            <Button onClick={closeUpdateDialog}>
              {t("chat.plugins.marketplace.cancel")}
            </Button>
            <Button type="primary" loading={validating} disabled={!updateReleaseSource} onClick={handleValidateUpdate}>
              {t("chat.plugins.marketplace.validate")}
            </Button>
            <Button
              type="primary"
              disabled={!updateValidated}
              onClick={handleConfirmUpdate}
            >
              {t("chat.plugins.marketplace.confirmUpdate")}
            </Button>
          </>
        }
        width={560}
      >
        <Descriptions column={1} size="small" bordered>
          <Descriptions.Item label={t("chat.plugins.marketplace.installSource")}>
            <Text className="break-all">{updateSource}</Text>
          </Descriptions.Item>
        </Descriptions>
        <Text type="secondary" className="text-xs block mt-2">
          {t("chat.plugins.marketplace.releaseHint")}
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
