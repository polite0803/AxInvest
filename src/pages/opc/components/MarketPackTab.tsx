// SPDX-License-Identifier: AGPL-3.0-only

import { invoke, isTauri } from "@/lib/invoke";
import { Button, Card, Col, message, Row, Space, Switch, Tag, Typography } from "antd";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import type { MarketPack } from "../utils/constants";

const { Text } = Typography;

export function MarketPackTab() {
  const { t } = useTranslation();
  const [packs, setPacks] = useState<MarketPack[]>([]);
  const [loading, setLoading] = useState(false);

  const refresh = useCallback(async () => {
    setLoading(true);
    try {
      const data = await invoke<MarketPack[]>("opc_market_list");
      setPacks(data);
    } catch (e) {
      message.error(t("opc.common.loadFailed", { error: String(e) }));
      setPacks([]);
    } finally {
      setLoading(false);
    }
  }, []);

  const handleExport = useCallback(
    async (p: MarketPack) => {
      if (!isTauri()) {
        message.info(t("opc.market.exportDesktopOnly"));
        return;
      }
      const { save } = await import("@tauri-apps/plugin-dialog");
      const filePath = await save({
        defaultPath: `${p.id}.opcip`,
        filters: [{ name: "Capability Pack", extensions: ["opcip"] }],
      });
      if (!filePath) {
        return;
      }
      try {
        const outPath = await invoke<string>("opc_export_capability_pack", {
          id: p.id,
          outDir: filePath.replace(/[^/\\]+$/, ""),
        });
        message.success(`${t("opc.market.exportSuccess", { name: p.name })} —— ${outPath}`);
      } catch (e) {
        message.error(t("opc.market.exportFailed", { error: String(e) }));
      }
    },
    [t],
  );

  // 导入能力包：弹文件选择对话框挑 .opcip 归档，交给后端解包安装/升级
  const handleImport = useCallback(
    async (packName?: string) => {
      if (!isTauri()) {
        message.info(t("opc.market.exportDesktopOnly"));
        return;
      }
      const { open } = await import("@tauri-apps/plugin-dialog");
      const chosen = await open({
        filters: [{ name: "Capability Pack", extensions: ["opcip"] }],
        multiple: false,
      });
      if (!chosen) {
        return;
      }
      const filePath = Array.isArray(chosen) ? chosen[0] : chosen;
      try {
        const auditMsg = await invoke<string>("opc_import_capability_pack", {
          archivePath: filePath,
        });
        message.success(
          packName
            ? `${t("opc.market.importSuccess", { name: packName })} —— ${auditMsg}`
            : auditMsg,
        );
        refresh();
      } catch (e) {
        message.error(t("opc.market.importFailed", { error: String(e) }));
      }
    },
    [t],
  );

  useEffect(() => {
    refresh();
  }, [refresh]);

  return (
    <div>
      <Space style={{ marginBottom: 12 }}>
        <Button size="small" type="primary" onClick={refresh} loading={loading}>
          {t("opc.market.refresh")}
        </Button>
        <Button size="small" onClick={() => handleImport()}>
          {t("opc.market.import")}
        </Button>
        <Text type="secondary">{t("opc.market.subtitle")}</Text>
      </Space>
      <Row gutter={[12, 12]}>
        {packs.map((p) => (
          <Col key={p.id} xs={24} sm={12} md={8}>
            <Card
              size="small"
              title={
                <Space>
                  <span>{p.icon}</span>
                  {p.name}
                  <Tag color={p.installed ? "green" : "blue"}>
                    {p.installed ? t("opc.market.installed") : t("opc.market.notInstalled")}
                  </Tag>
                </Space>
              }
            >
              <div style={{ fontSize: 12, color: "#888" }}>
                <div>ID: {p.id}</div>
                <div>{t("opc.market.version", { version: p.version })}</div>
                <div>
                  {t("opc.market.domain", {
                    domain: p.domain ?? t("opc.market.domainUndefined"),
                  })}
                </div>
                <div>
                  {t("opc.market.capabilityCount", {
                    count: p.capabilityCount ?? 0,
                  })}
                </div>
                {p.capabilities && p.capabilities.length > 0 && (
                  <div style={{ marginTop: 6 }}>
                    <Text type="secondary" style={{ fontSize: 12 }}>
                      {t("opc.market.capabilityListTitle")}
                    </Text>
                    <Space size={[4, 4]} wrap style={{ marginTop: 4 }}>
                      {p.capabilities.map((c) => (
                        <Tag key={c} style={{ fontSize: 11 }}>
                          {c.replace(/^(tool:|workflow:|skill:|agent:|mcp:)/, "")}
                        </Tag>
                      ))}
                    </Space>
                  </div>
                )}
              </div>
              <Space style={{ marginTop: 8 }} wrap>
                <Button
                  size="small"
                  type={p.installed ? "default" : "primary"}
                  onClick={() => handleImport(p.name)}
                >
                  {t("opc.market.import")}
                </Button>
                {p.installed && (
                  <Button size="small" onClick={() => handleExport(p)}>
                    {t("opc.market.export")}
                  </Button>
                )}
                <Space size={4}>
                  <Switch
                    size="small"
                    checked={p.enabled}
                    onChange={async (checked) => {
                      try {
                        await invoke("opc_set_capability_pack_enabled", { packId: p.id, enabled: checked });
                        message.success(t("opc.market.toggleSuccess", { name: p.name }));
                        refresh();
                      } catch (e) {
                        message.error(t("opc.market.toggleFailed", { error: String(e) }));
                        refresh();
                      }
                    }}
                  />
                  <Text style={{ fontSize: 12 }}>{t("opc.market.toggle")}</Text>
                </Space>
              </Space>
            </Card>
          </Col>
        ))}
      </Row>
    </div>
  );
}
