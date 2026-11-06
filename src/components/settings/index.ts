// SPDX-License-Identifier: AGPL-3.0-only

export { AboutPage } from "./AboutPage";
export { AcpSettings } from "./AcpSettings";
export { AdvancedSettings } from "./AdvancedSettings";
export { CloudWorkspaceSelector as CloudWorkspaceSettings } from "./CloudWorkspaceSelector";
export { DashboardPluginsSettings } from "./DashboardPluginsSettings";
export { DataManager } from "./DataManager";
export { DisplaySettings } from "./DisplaySettings";
export { GatewayConfigPanel } from "./GatewayConfigPanel";
export { GeneralSettings } from "./GeneralSettings";
export { LocalToolSettings } from "./LocalToolSettings";
export { McpServerSettings } from "./McpServerSettings";
export { MemorySettings } from "./MemorySettings";
export { MessageChannelSettings } from "./MessageChannelSettings";
export { PlatformStatusCard } from "./PlatformStatusCard";
export { ProviderList } from "./ProviderList";
export { ProviderSettings } from "./ProviderSettings";
export { ProxySettings } from "./ProxySettings";
export { SearchProviderSettings } from "./SearchProviderSettings";
export { SettingsGroup } from "./SettingsGroup";
export { SettingsPanel } from "./SettingsPanel";
export { SettingsSelect } from "./SettingsSelect";
export { SettingsSidebar } from "./SettingsSidebar";
export { ShortcutSettings } from "./ShortcutSettings";
export { ToolManager } from "./ToolManager";
export { WebhookSettings } from "./WebhookSettings";

// 2026-11-07（任务 #193，组件中立性）：此处原有把 `PluginMarketplace`（chat 特性目录的组件）
// 再导出一遍的写法，让 settings 目录反向依赖 chat 特性目录（门禁违规）。该 re-export 全仓零消费方
// （`src/pages/SettingsPage.tsx` 一直直连 `@/components/chat/PluginMarketplace`），故直接删除，
// 不要再加回来 —— 需要时从 `@/components/chat/PluginMarketplace` 直取。
// ⚠ 本注释刻意不写 import/export 语句形态：中立性门禁按 `from "…"` 字面量扫描、不剥注释。
export { BackupCenter } from "./BackupCenter";
export { CacheConfigPanel } from "./CacheConfigPanel";
export { DynamicPagesSettings } from "./DynamicPagesSettings";
export { ImageGenSettings } from "./ImageGenSettings";
export { StorageSpaceManager } from "./StorageSpaceManager";
export { ThemeManager } from "./ThemeManager";
export { UserProfileSettings } from "./UserProfileSettings";
export { WorkflowSettings } from "./WorkflowSettings";
