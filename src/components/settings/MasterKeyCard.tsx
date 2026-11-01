// SPDX-License-Identifier: AGPL-3.0-only

// 主密钥（master.key）导出 / 导入卡片 —— 跨设备共享数据库的钥匙搬运入口。
//
// 场景：多机共享同一 PostgreSQL 时，落库密文由「写入方」的 master.key 加密，
// 消费方读不出来（v2 密文头指纹不匹配 ⇒ `KEY_FROM_OTHER_DEVICE`）。本卡片提供
// 「加密方导出 → 消费方导入」的通道；后端命令见 `master_key.rs`（绝不静默生成、
// 先验证后落盘、旧物必备份三条契约都在那边，前端只负责把后果讲清楚）。
//
// ⚠ 导入成功后**必须重启应用**：运行中的进程把旧钥匙载在内存里（启动时载入一次），
// 重启前保存的密钥类数据会以旧钥匙加密落库，重启后反而读不出来 —— 这是本卡片
// 最重要的警示，导入成功后以持久 Alert 呈现（不用 toast，不能一闪而过）。

import { backendErrorText, parseBackendError, translateBackendError } from "@/lib/errorI18n";
import { invoke, logIpcError } from "@/lib/invoke";
import type { MasterKeyExport, MasterKeyImportReport, MasterKeyInfo } from "@/types";
import { Alert, App, Button, Card, Input, Modal, Space, Typography } from "antd";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

export function MasterKeyCard() {
  const { t } = useTranslation();
  const { message } = App.useApp();
  const [fingerprint, setFingerprint] = useState<string | null>(null);
  const [missing, setMissing] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [exporting, setExporting] = useState(false);
  const [exportData, setExportData] = useState<MasterKeyExport | null>(null);
  const [importOpen, setImportOpen] = useState(false);
  const [importHex, setImportHex] = useState("");
  const [importing, setImporting] = useState(false);
  const [report, setReport] = useState<MasterKeyImportReport | null>(null);

  // 指纹查询是「展示」不是「报错」：MASTER_KEY_NOT_FOUND 是预期状态（本机从未生成
  // 钥匙，或钥匙待导入），走引导文案而非 toast。其余失败才进 error 形态。
  const refreshInfo = useCallback(() => {
    setLoadError(null);
    invoke<MasterKeyInfo>("get_master_key_info")
      .then((info) => {
        setFingerprint(info.fingerprint);
        setMissing(false);
      })
      .catch((e) => {
        if (parseBackendError(e).code === "MASTER_KEY_NOT_FOUND") {
          setMissing(true);
          setFingerprint(null);
        } else {
          setLoadError(backendErrorText(e));
          logIpcError("get_master_key_info")(e);
        }
      });
  }, []);

  useEffect(() => {
    refreshInfo();
  }, [refreshInfo]);

  const handleExport = useCallback(() => {
    setExporting(true);
    invoke<MasterKeyExport>("export_master_key")
      .then((data) => setExportData(data))
      .catch((e) => {
        message.error(translateBackendError(e));
        logIpcError("export_master_key")(e);
      })
      .finally(() => setExporting(false));
  }, [message]);

  const handleImport = useCallback(async () => {
    const hex = importHex.trim();
    if (!hex) {
      message.warning(t("settings.database.masterKey.importEmpty"));
      return;
    }
    setImporting(true);
    try {
      const r = await invoke<MasterKeyImportReport>("import_master_key", { keyHex: hex });
      setReport(r);
      setImportOpen(false);
      setImportHex("");
      refreshInfo();
    } catch (e) {
      // 失败保持弹窗打开，用户可修正输入重试
      message.error(translateBackendError(e));
      logIpcError("import_master_key")(e);
    } finally {
      setImporting(false);
    }
  }, [importHex, message, refreshInfo, t]);

  const importedSameKey = report !== null && report.oldFingerprint === report.newFingerprint;

  return (
    <Card
      size="small"
      title={t("settings.database.masterKey.title")}
      style={{ marginTop: 16 }}
      extra={
        <Space>
          <Button size="small" loading={exporting} onClick={handleExport}>
            {t("settings.database.masterKey.exportButton")}
          </Button>
          <Button size="small" danger onClick={() => setImportOpen(true)}>
            {t("settings.database.masterKey.importButton")}
          </Button>
        </Space>
      }
    >
      {missing
        ? (
          <Alert
            type="warning"
            showIcon
            title={t("settings.database.masterKey.noKeyWarning")}
          />
        )
        : loadError
        ? (
          <Alert
            type="error"
            showIcon
            title={t("settings.database.masterKey.loadFailed", { error: loadError })}
          />
        )
        : (
          <Typography.Text type="secondary">
            {t("settings.database.masterKey.fingerprintLabel", { fingerprint: fingerprint ?? "" })}
          </Typography.Text>
        )}

      {report
        && (importedSameKey
          ? (
            <Alert
              type="info"
              showIcon
              style={{ marginTop: 8 }}
              title={t("settings.database.masterKey.importNoop", { fp: report.newFingerprint })}
            />
          )
          : (
            <Alert
              type="warning"
              showIcon
              style={{ marginTop: 8 }}
              title={t("settings.database.masterKey.importDoneTitle")}
              description={
                <>
                  <Typography.Text style={{ display: "block" }}>
                    {t("settings.database.masterKey.importDoneFingerprints", {
                      oldFp: report.oldFingerprint ?? "—",
                      newFp: report.newFingerprint,
                    })}
                  </Typography.Text>
                  {report.backupPath && (
                    <Typography.Text type="secondary" style={{ display: "block" }}>
                      {t("settings.database.masterKey.backupPathLabel", { path: report.backupPath })}
                    </Typography.Text>
                  )}
                  {report.dbConfigReencrypted && (
                    <Typography.Text type="secondary" style={{ display: "block" }}>
                      {t("settings.database.masterKey.reencrypted")}
                    </Typography.Text>
                  )}
                  <Typography.Text strong style={{ display: "block", marginTop: 4 }}>
                    {t("settings.database.masterKey.restartRequired")}
                  </Typography.Text>
                </>
              }
            />
          ))}

      <Modal
        open={exportData !== null}
        title={t("settings.database.masterKey.exportModalTitle")}
        onCancel={() => setExportData(null)}
        footer={null}
        width={560}
      >
        <Alert
          type="warning"
          showIcon
          style={{ marginBottom: 12 }}
          title={t("settings.database.masterKey.exportWarning")}
        />
        <Typography.Paragraph>
          {t("settings.database.masterKey.exportHexLabel", { fp: exportData?.fingerprint ?? "" })}
        </Typography.Paragraph>
        <Typography.Paragraph copyable={{ text: exportData?.keyHex ?? "" }} code style={{ wordBreak: "break-all" }}>
          {exportData?.keyHex}
        </Typography.Paragraph>
      </Modal>

      <Modal
        open={importOpen}
        title={t("settings.database.masterKey.importModalTitle")}
        onCancel={() => {
          if (!importing) {
            setImportOpen(false);
          }
        }}
        onOk={handleImport}
        okText={t("settings.database.masterKey.importConfirmOk")}
        okButtonProps={{ danger: true }}
        cancelText={t("settings.database.masterKey.cancel")}
        confirmLoading={importing}
        width={560}
      >
        <Alert
          type="warning"
          showIcon
          style={{ marginBottom: 12 }}
          title={t("settings.database.masterKey.importWarning")}
        />
        <Input.TextArea
          rows={3}
          value={importHex}
          onChange={(e) => setImportHex(e.target.value)}
          placeholder={t("settings.database.masterKey.importPlaceholder")}
        />
      </Modal>
    </Card>
  );
}
