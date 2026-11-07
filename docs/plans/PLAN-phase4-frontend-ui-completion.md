> 来源：导入自上游 AxAgent（基准 99c9b96f，2026-09-07） ｜ 导入日期：2026-11-04

# Phase 4: 前端完善

> 依赖：Phase 1 + Phase 2 + Phase 3
> 目标：在 Settings 页面中添加消息渠道配置入口，激活前端组件，实现完整的配置和管理界面。

---

## 1. 现状分析

### 1.1 死代码组件

| 组件 | 状态 | 说明 |
|------|------|------|
| [GatewayConfigPanel.tsx](../src/components/settings/GatewayConfigPanel.tsx) | 🔴 死代码 | 仅实现 Telegram/Discord，未导出 |
| [PlatformStatusCard.tsx](../src/components/settings/PlatformStatusCard.tsx) | 🔴 死代码 | 仅展示 3 平台，未导出 |

### 1.2 Settings 页面缺失入口

- [SettingsPage.tsx](../src/App.tsx) 的 `SECTION_COMPONENTS` 无 `platform` 或 `messageChannel` 入口
- [SettingsSidebar.tsx](../src/components/settings/SettingsSidebar.tsx) 的 `SECTION_KEYS` 无对应项
- [types/index.ts](../src/types/index.ts) 的 `SettingsSection` 类型无对应值

### 1.3 platformStore 不完整

- 仅覆盖 3 个平台（Telegram, Discord, API Server）
- 缺少 Slack, WhatsApp, WeChat, Feishu, QQ, DingTalk

---

## 2. 修改方案

### 2.1 扩展 SettingsSection 类型

**文件**: `src/types/index.ts`

```typescript
export type SettingsSection =
  | "providers"
  | "defaultModel"
  | "conversationSettings"
  | "general"
  | "display"
  | "proxy"
  | "shortcuts"
  | "data"
  | "storage"
  | "scheduler"
  | "backup"
  | "about"
  | "searchProviders"
  | "localTools"
  | "mcpServers"
  | "workflow"
  | "tools"
  | "userProfile"
  | "skillsHub"
  | "dashboardPlugins"
  | "webhooks"
  | "messageChannels";  // ← 新增
```

### 2.2 创建 MessageChannelSettings 组件

**新建文件**: `src/components/settings/MessageChannelSettings.tsx`

整体布局：左侧平台列表 + 右侧配置面板。

```tsx
import { usePlatformStore } from "@/stores";
import { ALL_PLATFORMS } from "@/types/platform";
import { Tabs, message as antMessage } from "antd";
import { PlatformStatusCard } from "./PlatformStatusCard";
import { GatewayConfigPanel } from "./GatewayConfigPanel";

export function MessageChannelSettings() {
  const reconcile = usePlatformStore((s) => s.reconcile);

  const items = [
    {
      key: "config",
      label: "平台配置",
      children: <GatewayConfigPanel />,
    },
    {
      key: "status",
      label: "连接状态",
      children: <PlatformStatusCard />,
    },
  ];

  return (
    <div className="p-6 pb-12">
      <div className="flex items-center justify-between mb-4">
        <h2 style={{ fontSize: 18, fontWeight: 600 }}>消息渠道</h2>
        <Button onClick={async () => {
          const report = await reconcile();
          if (report.started.length > 0)
            antMessage.success(`已启动: ${report.started.join(", ")}`);
          if (report.stopped.length > 0)
            antMessage.info(`已停止: ${report.stopped.join(", ")}`);
          if (report.errors.length > 0)
            antMessage.error(`错误: ${report.errors.map(e => e[0]).join(", ")}`);
        }}>
          应用配置
        </Button>
      </div>
      <Tabs items={items} />
    </div>
  );
}
```

### 2.3 重写 GatewayConfigPanel — 覆盖全部 8 平台

**重写文件**: `src/components/settings/GatewayConfigPanel.tsx`

使用 `ALL_PLATFORMS` 辅助常量 + 统一的表单渲染逻辑：

```tsx
import { usePlatformStore } from "@/stores";
import { ALL_PLATFORMS, type PlatformConfig } from "@/types/platform";
import { Card, Input, Switch, Typography } from "antd";

const { Text } = Typography;

type PlatformFieldDef = {
  key: keyof PlatformConfig;
  label: string;
  type: "switch" | "password" | "text" | "number";
  placeholder?: string;
};

const PLATFORM_FIELDS: Record<string, PlatformFieldDef[]> = {
  telegram: [
    { key: "telegram_enabled", label: "启用", type: "switch" },
    { key: "telegram_bot_token", label: "Bot Token", type: "password",
      placeholder: "从 @BotFather 获取" },
    { key: "telegram_webhook_url", label: "Webhook URL (可选)", type: "text" },
    { key: "telegram_webhook_secret", label: "Webhook Secret (可选)", type: "password" },
  ],
  discord: [
    { key: "discord_enabled", label: "启用", type: "switch" },
    { key: "discord_bot_token", label: "Bot Token", type: "password",
      placeholder: "从 Discord Developer Portal 获取" },
    { key: "discord_webhook_url", label: "Webhook URL (可选)", type: "text" },
  ],
  slack: [
    { key: "slack_enabled", label: "启用", type: "switch" },
    { key: "slack_bot_token", label: "Bot Token", type: "password" },
    { key: "slack_signing_secret", label: "Signing Secret", type: "password" },
  ],
  whatsapp: [
    { key: "whatsapp_enabled", label: "启用", type: "switch" },
    { key: "whatsapp_phone_number_id", label: "Phone Number ID", type: "text" },
    { key: "whatsapp_access_token", label: "Access Token", type: "password" },
  ],
  wechat: [
    { key: "wechat_enabled", label: "启用", type: "switch" },
    { key: "wechat_app_id", label: "App ID", type: "text" },
    { key: "wechat_app_secret", label: "App Secret", type: "password" },
  ],
  feishu: [
    { key: "feishu_enabled", label: "启用", type: "switch" },
    { key: "feishu_app_id", label: "App ID", type: "text" },
    { key: "feishu_app_secret", label: "App Secret", type: "password" },
  ],
  qq: [
    { key: "qq_enabled", label: "启用", type: "switch" },
    { key: "qq_bot_app_id", label: "App ID", type: "text" },
    { key: "qq_bot_token", label: "Token", type: "password" },
  ],
  dingtalk: [
    { key: "dingtalk_enabled", label: "启用", type: "switch" },
    { key: "dingtalk_app_key", label: "App Key", type: "text" },
    { key: "dingtalk_app_secret", label: "App Secret", type: "password" },
  ],
};

export function GatewayConfigPanel() {
  const config = usePlatformStore((s) => s.config);
  const saveConfig = usePlatformStore((s) => s.saveConfig);

  const handleChange = (key: keyof PlatformConfig, value: unknown) => {
    saveConfig({ [key]: value });
  };

  return (
    <div className="flex flex-col gap-3">
      {ALL_PLATFORMS.map((platform) => {
        if (platform.name === "api_server") return null; // API Server 单独处理
        const fields = PLATFORM_FIELDS[platform.name];
        if (!fields) return null;

        const enabled = config[platform.enabledKey] as boolean;

        return (
          <Card key={platform.name} size="small" title={`${platform.icon} ${platform.label}`}>
            {fields.map((field) => {
              if (field.type === "switch") {
                return (
                  <div key={field.key} className="flex items-center justify-between py-1">
                    <span>{field.label}</span>
                    <Switch
                      checked={enabled}
                      onChange={(v) => handleChange(field.key, v)}
                    />
                  </div>
                );
              }
              if (!enabled) return null;
              return (
                <div key={field.key} className="mt-3">
                  <Text type="secondary">{field.label}</Text>
                  {field.type === "password" ? (
                    <Input.Password
                      value={(config[field.key] as string) ?? ""}
                      onChange={(e) => handleChange(field.key, e.target.value)}
                      placeholder={field.placeholder}
                    />
                  ) : (
                    <Input
                      value={(config[field.key] as string) ?? ""}
                      onChange={(e) => handleChange(field.key, e.target.value)}
                      placeholder={field.placeholder}
                    />
                  )}
                </div>
              );
            })}
          </Card>
        );
      })}

      {/* 通用设置 */}
      <Card size="small" title="通用设置">
        <div className="flex items-center justify-between py-1">
          <span>启用 API Server</span>
          <Switch
            checked={config.api_server_enabled}
            onChange={(v) => handleChange("api_server_enabled", v)}
          />
        </div>
        {config.api_server_enabled && (
          <div className="mt-3">
            <Text type="secondary">端口</Text>
            <Input
              type="number"
              value={config.api_server_port ?? 8080}
              onChange={(e) =>
                handleChange("api_server_port", Number.parseInt(e.target.value, 10) || 8080)
              }
            />
          </div>
        )}
        <div className="flex items-center justify-between py-1 mt-2">
          <span>自动同步消息</span>
          <Switch
            checked={config.auto_sync_messages}
            onChange={(v) => handleChange("auto_sync_messages", v)}
          />
        </div>
        <div className="mt-3">
          <Text type="secondary">单会话最大历史消息数</Text>
          <Input
            type="number"
            value={config.max_history_per_session}
            onChange={(e) =>
              handleChange("max_history_per_session", Number.parseInt(e.target.value, 10) || 100)
            }
          />
        </div>
      </Card>
    </div>
  );
}
```

### 2.4 更新 PlatformStatusCard

**重写文件**: `src/components/settings/PlatformStatusCard.tsx`

```tsx
import { usePlatformStore } from "@/stores";
import { ALL_PLATFORMS } from "@/types/platform";
import { Card, Tag, Typography } from "antd";
import { CheckCircle, Loader2 } from "lucide-react";
import { useEffect } from "react";

const { Text } = Typography;

export function PlatformStatusCard() {
  const statuses = usePlatformStore((s) => s.statuses);
  const loadStatuses = usePlatformStore((s) => s.loadStatuses);

  useEffect(() => {
    loadStatuses();
    const interval = setInterval(loadStatuses, 30000);
    return () => clearInterval(interval);
  }, [loadStatuses]);

  const metaMap = new Map(ALL_PLATFORMS.map(p => [p.name, p]));

  return (
    <div className="flex flex-col gap-3">
      {statuses.map((s) => {
        const meta = metaMap.get(s.name);
        return (
          <Card key={s.name} size="small"
            title={`${meta?.icon ?? "?"} ${meta?.label ?? s.name}`}>
            <div className="flex flex-col gap-2">
              <div className="flex items-center justify-between">
                <Text type="secondary">状态</Text>
                {!s.enabled ? (
                  <Tag color="default">未启用</Tag>
                ) : s.connected ? (
                  <Tag icon={<CheckCircle size={14} />} color="success">已连接</Tag>
                ) : (
                  <Tag icon={<Loader2 size={14} className="animate-spin" />} color="processing">
                    连接中
                  </Tag>
                )}
              </div>
              {s.last_activity && (
                <div className="flex items-center justify-between">
                  <Text type="secondary">最近活动</Text>
                  <Text>{new Date(s.last_activity).toLocaleString()}</Text>
                </div>
              )}
              {s.enabled && (
                <div className="flex items-center justify-between">
                  <Text type="secondary">活跃会话</Text>
                  <Text>{s.active_sessions}</Text>
                </div>
              )}
            </div>
          </Card>
        );
      })}
    </div>
  );
}
```

### 2.5 注册到 SettingsPage

**文件**: `src/pages/SettingsPage.tsx`

```tsx
import { MessageChannelSettings } from "@/components/settings/MessageChannelSettings";
// ... 在 SECTION_COMPONENTS 中添加：
const SECTION_COMPONENTS: Record<SettingsSection, React.ComponentType<any>> = {
  // ... 现有项
  messageChannels: MessageChannelSettings,  // ← 新增
};
```

### 2.6 添加到 SettingsSidebar

**文件**: `src/components/settings/SettingsSidebar.tsx`

在 `MENU_ICONS` 和 `SECTION_KEYS` 中添加：

```tsx
const MENU_ICONS: Partial<Record<SettingsSection, React.ReactNode>> = {
  // ... 现有
  messageChannels: <Send size={16} color={SETTINGS_ICON_COLORS.Send} />,
};

const SECTION_KEYS: SettingsSection[] = [
  "general",
  "display",
  "providers",
  // ...
  "messageChannels",  // ← 在合适位置插入
  // ...
  "about",
];
```

### 2.7 扩展前端 Store

**文件**: `src/stores/feature/platformStore.ts`

添加 `reconcile` 方法：

```typescript
reconcile: async () => {
  try {
    const report = await invoke<PlatformReconcileReport>("reconcile_platforms");
    await get().loadStatuses();
    return report;
  } catch (e) {
    set({ error: String(e) });
    throw e;
  }
},
```

### 2.8 从 settings/index.ts 导出新组件

**文件**: `src/components/settings/index.ts`

```typescript
export { MessageChannelSettings } from "./MessageChannelSettings";
export { GatewayConfigPanel } from "./GatewayConfigPanel";
export { PlatformStatusCard } from "./PlatformStatusCard";
```

### 2.9 添加 i18n 翻译键

需要在各语言 locale 文件中添加：

```json
{
  "settings": {
    "messageChannels": "消息渠道"
  }
}
```

---

## 3. 实施步骤汇总

| # | 操作 | 文件 | 类型 |
|---|------|------|------|
| 1 | 扩展 SettingsSection 类型 | `src/types/index.ts` | 修改 |
| 2 | 创建 MessageChannelSettings 组件 | `src/components/settings/MessageChannelSettings.tsx` | **新建** |
| 3 | 重写 GatewayConfigPanel（8平台） | `src/components/settings/GatewayConfigPanel.tsx` | 修改 |
| 4 | 更新 PlatformStatusCard | `src/components/settings/PlatformStatusCard.tsx` | 修改 |
| 5 | 注册到 SettingsPage | `src/pages/SettingsPage.tsx` | 修改 |
| 6 | 添加到 SettingsSidebar | `src/components/settings/SettingsSidebar.tsx` | 修改 |
| 7 | 扩展前端 Store | `src/stores/feature/platformStore.ts` | 修改 |
| 8 | 导出新组件 | `src/components/settings/index.ts` | 修改 |
| 9 | 添加 i18n 翻译键 | locale 文件 | 修改 |

---

## 4. 风险评估

| 风险 | 缓解措施 |
|------|---------|
| `SettingsSection` 类型修改导致大量 TS 报错 | 仅在末尾新增 `messageChannels`，不影响已有分支 |
| GatewayConfigPanel 重写过于复杂 | 使用 `PLATFORM_FIELDS` 声明式配置减少重复代码 |
| `reconcile_platforms` 命令依赖 Phase 2 实现 | Phase 4 实施前确认 Phase 2 已完成 |

---

## 5. 不做的事项

- ❌ Dashboard 首页的消息渠道卡片（后续可添加）
- ❌ 各平台的 OAuth 认证流程（仅支持 Token 配置）
