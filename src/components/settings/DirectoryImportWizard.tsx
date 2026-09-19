// SPDX-License-Identifier: AGPL-3.0-only
//
// 目录导入三步向导：
//   步骤 1 配置（目录 + 递归 + 扩展名白名单 + ignore 模式 + 冲突策略 + 目标知识库）
//   步骤 2 预扫描预览（scan_knowledge_directory：文件清单 / 统计 / 与 KB 重叠标记）
//   步骤 3 执行导入（create：create_source + import_knowledge_directory；update：sync_project_knowledge_sources）
import { EmbeddingModelSelect } from "@/components/shared/EmbeddingModelSelect";
import { useEmbeddingProviderLabel } from "@/components/shared/ModelSelect";
import { invoke } from "@/lib/invoke";
import { useLlmWikiStore, useSourceStore } from "@/stores";
import type { UnifiedSource } from "@/stores/feature/sourceStore";
import type {
  ConflictPolicy,
  DirectoryScanFile,
  DirectoryScanResult,
  ImportDirectoryError,
  ImportDirectoryResult,
  SyncDirectoryResult,
} from "@/types";
import {
  App as AntdApp,
  Button,
  Form,
  Input,
  Modal,
  Radio,
  Select,
  Space,
  Spin,
  Steps,
  Switch,
  Table,
  Tag,
  Typography,
} from "antd";
import type { ColumnsType } from "antd/es/table";
import { FolderOpen, FolderPlus, RefreshCw, Rocket } from "lucide-react";
import { useCallback, useState } from "react";
import { useTranslation } from "react-i18next";

const { Text } = Typography;

/** i18n t 函数的宽松签名（支持插值参数，兼容 react-i18next TFunction）。 */
type TWithParams = (
  key: string,
  params?: Record<string, string | number>,
) => string;

/** 按错误码翻译；无码或未命中时回退显示原文。 */
function translateErrorCode(
  t: TWithParams,
  code?: string | null,
  fallback?: string,
): string {
  if (code) {
    const key = `error.${code}`;
    const translated = t(key);
    if (translated !== key) {
      return translated;
    }
  }
  return fallback ?? code ?? "";
}

/** 字节数人类可读格式化（KB / MB / GB）。 */
function formatBytes(bytes: number): string {
  if (bytes < 1024) { return `${bytes} B`; }
  const units = ["KB", "MB", "GB"];
  let v = bytes;
  let i = -1;
  do {
    v /= 1024;
    i += 1;
  } while (v >= 1024 && i < units.length - 1);
  return `${v.toFixed(v >= 100 ? 0 : 1)} ${units[i]}`;
}

/** 统一窗口标题下区分 create / update 模式的结果结构。 */
type ImportOutcome = { label: string; detail: string };

function summarizeOutcome(
  t: TWithParams,
  mode: "create" | "update",
  result: ImportDirectoryResult | SyncDirectoryResult,
): ImportOutcome {
  if (mode === "create") {
    const r = result as ImportDirectoryResult;
    return {
      label: t("sourceManager.importProjectModal.resultSummary", {
        imported: r.importedCount,
        skipped: r.skippedCount,
        errors: r.errorCount,
      }),
      detail: r.errorCount > 0
        ? `${t("sourceManager.importProjectModal.resultErrors", { count: r.errorCount })}`
        : t("sourceManager.importProjectModal.resultOk"),
    };
  }
  const r = result as SyncDirectoryResult;
  return {
    label: t("sourceManager.syncSuccess", {
      added: r.addedCount,
      updated: r.updatedCount,
      deleted: r.deletedCount,
      skipped: r.skippedCount,
    }),
    detail: r.errorCount > 0
      ? `${t("sourceManager.importProjectModal.resultErrors", { count: r.errorCount })}`
      : t("sourceManager.importProjectModal.resultOk"),
  };
}

function DirectoryImportWizard({
  open,
  initialMode,
  onClose,
}: {
  open: boolean;
  initialMode: "create" | "update";
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const { message: messageApi } = AntdApp.useApp();
  const { fetchSources } = useSourceStore();
  const allSources = useSourceStore((s) => s.sources);
  const wikis = useLlmWikiStore((s) => s.wikis);
  const loadWikis = useLlmWikiStore((s) => s.loadWikis);
  const knowledgeSources = allSources.filter(
    (s) => s.containerType === "knowledge" && s.name,
  );
  const formatProviderLabel = useEmbeddingProviderLabel();

  const [form] = Form.useForm();
  const mode: "create" | "update" = Form.useWatch("mode", form) ?? initialMode;
  const wikiCopyEnabled = Form.useWatch("wikiCopy", form) ?? false;

  const [step, setStep] = useState(0);
  const [dirPath, setDirPath] = useState("");
  const [scanning, setScanning] = useState(false);
  const [importing, setImporting] = useState(false);
  const [scanResult, setScanResult] = useState<DirectoryScanResult | null>(null);
  const [importResult, setImportResult] = useState<
    ImportDirectoryResult | SyncDirectoryResult | null
  >(null);

  // 打开时重置向导状态
  const reset = useCallback(() => {
    setStep(0);
    setDirPath("");
    setScanResult(null);
    setImportResult(null);
    form.setFieldsValue({
      mode: initialMode,
      sourceName: "",
      targetBaseId: undefined,
      sourcePath: "",
      recursive: true,
      extensionsText: "",
      ignorePatternsText: "",
      conflict: "skip",
      embeddingProvider: undefined,
      wikiCopy: false,
      wikiVaultId: undefined,
    });
  }, [form, initialMode]);

  const handleOpenChange = useCallback(
    (next: boolean) => {
      if (next) {
        reset();
        // 预加载 Wiki 列表（wikiVault 下拉选项；失败静默，下拉显示空态）
        void loadWikis();
      } else {
        onClose();
      }
    },
    [onClose, reset, loadWikis],
  );

  const handleSelectDirectory = useCallback(async () => {
    try {
      const { open: openDialog } = await import("@tauri-apps/plugin-dialog");
      const selected = await openDialog({ directory: true, multiple: false });
      if (typeof selected === "string" && selected.length > 0) {
        setDirPath(selected);
        setScanResult(null);
      }
    } catch {
      // 用户取消或环境不支持
    }
  }, []);

  /** 解析扩展名白名单文本（逗号/空格分隔，去点、小写、去空）。 */
  const parseExtensions = useCallback((text?: string): string[] | undefined => {
    const exts = (text ?? "")
      .split(/[,，\s]+/)
      .map((e) => e.trim().replace(/^\./, "").toLowerCase())
      .filter((e) => e.length > 0);
    return exts.length > 0 ? exts : undefined;
  }, []);

  /** 解析 ignore 模式文本（每行一个 glob）。 */
  const parseIgnorePatterns = useCallback(
    (text?: string): string[] | undefined => {
      const patterns = (text ?? "")
        .split("\n")
        .map((p) => p.trim())
        .filter((p) => p.length > 0);
      return patterns.length > 0 ? patterns : undefined;
    },
    [],
  );

  /** 步骤 1 → 2：预扫描目录（参数与导入完全一致，保证「所见即所得」）。 */
  const handleScan = useCallback(async () => {
    const values = await form.validateFields();
    if (!dirPath) {
      messageApi.error(t("sourceManager.importProjectModal.directoryRequired"));
      return;
    }
    const baseId = mode === "update"
      ? (values.targetBaseId as string)
      : undefined;
    setScanning(true);
    try {
      const result = await invoke<DirectoryScanResult>("scan_knowledge_directory", {
        baseId,
        directoryPath: dirPath,
        recursive: values.recursive,
        extensions: parseExtensions(values.extensionsText),
        ignorePatterns: parseIgnorePatterns(values.ignorePatternsText),
      });
      setScanResult(result);
      setStep(1);
    } catch (e) {
      messageApi.error(String(e));
    } finally {
      setScanning(false);
    }
  }, [dirPath, form, messageApi, mode, parseExtensions, parseIgnorePatterns, t]);

  /** 步骤 2 → 3：执行导入。 */
  const handleStartImport = useCallback(async () => {
    if (!scanResult) {
      return;
    }
    const values = await form.validateFields();
    setImporting(true);
    try {
      if (mode === "create") {
        const created = await invoke<UnifiedSource>("create_source", {
          input: {
            name: values.sourceName,
            sourceType: "knowledge",
            description: null,
            embeddingProvider: values.embeddingProvider ?? null,
          },
        });
        const result = await invoke<ImportDirectoryResult>(
          "import_knowledge_directory",
          {
            baseId: created.id,
            directoryPath: dirPath,
            recursive: values.recursive,
            extensions: parseExtensions(values.extensionsText),
            ignorePatterns: parseIgnorePatterns(values.ignorePatternsText),
            conflict: values.conflict as ConflictPolicy,
            generateMarkdown: values.wikiCopy,
            vaultId: values.wikiCopy ? values.wikiVaultId : undefined,
          },
        );
        if (!created.embeddingProvider) {
          messageApi.info(t("sourceManager.importNoEmbedding"));
        }
        setImportResult(result);
      } else {
        const result = await invoke<SyncDirectoryResult>(
          "sync_project_knowledge_sources",
          {
            baseId: values.targetBaseId,
            sourcePath: dirPath,
            recursive: values.recursive,
            ignorePatterns: parseIgnorePatterns(values.ignorePatternsText),
          },
        );
        setImportResult(result);
      }
      await fetchSources();
      setStep(2);
    } catch (e) {
      messageApi.error(String(e));
    } finally {
      setImporting(false);
    }
  }, [
    dirPath,
    fetchSources,
    form,
    messageApi,
    mode,
    parseExtensions,
    parseIgnorePatterns,
    scanResult,
    t,
  ]);

  const previewColumns: ColumnsType<DirectoryScanFile> = [
    {
      title: t("sourceManager.importProjectModal.previewFile"),
      dataIndex: "relPath",
      key: "relPath",
      ellipsis: true,
      render: (rel: string, record) => (
        <Space size={4}>
          {record.fromArchive && <FolderOpen size={12} />}
          <span>{rel}</span>
          {record.exists && (
            <Tag color="orange" style={{ marginInlineEnd: 0 }}>
              {t("sourceManager.importProjectModal.statusExisting")}
            </Tag>
          )}
        </Space>
      ),
    },
    {
      title: t("sourceManager.importProjectModal.previewType"),
      dataIndex: "extension",
      key: "extension",
      width: 90,
      render: (ext: string) => (ext ? `.${ext}` : "-"),
    },
    {
      title: t("sourceManager.importProjectModal.previewSize"),
      dataIndex: "sizeBytes",
      key: "sizeBytes",
      width: 100,
      render: (bytes: number) => formatBytes(bytes),
    },
  ];

  const summary = summarizeOutcome(t, mode, importResult!);

  return (
    <Modal
      title={t("sourceManager.importProjectModal.title")}
      open={open}
      onCancel={() => handleOpenChange(false)}
      width={720}
      footer={step === 0
        ? (
          <Space>
            <Button onClick={() => handleOpenChange(false)}>
              {t("common.cancel")}
            </Button>
            <Button
              type="primary"
              icon={<RefreshCw size={14} />}
              loading={scanning}
              onClick={handleScan}
            >
              {t("sourceManager.importProjectModal.scanPreview")}
            </Button>
          </Space>
        )
        : step === 1
        ? (
          <Space>
            <Button onClick={() => setStep(0)}>
              {t("sourceManager.importProjectModal.back")}
            </Button>
            <Button
              type="primary"
              icon={<Rocket size={14} />}
              loading={importing}
              onClick={handleStartImport}
            >
              {t("sourceManager.importProjectModal.startImport")}
            </Button>
          </Space>
        )
        : (
          <Space>
            <Button type="primary" onClick={() => handleOpenChange(false)}>
              {t("common.done")}
            </Button>
          </Space>
        )}
    >
      <Steps
        size="small"
        current={step}
        style={{ marginBottom: 16 }}
        items={[
          { title: t("sourceManager.importProjectModal.stepConfig") },
          { title: t("sourceManager.importProjectModal.stepPreview") },
          { title: t("sourceManager.importProjectModal.stepResult") },
        ]}
      />

      {step === 0 && (
        <Form form={form} layout="vertical">
          <Form.Item label={t("sourceManager.importProjectModal.directory")}>
            <Space.Compact style={{ width: "100%" }}>
              <Input
                placeholder={t("sourceManager.importProjectModal.directoryPlaceholder")}
                value={dirPath}
                readOnly
              />
              <Button
                onClick={handleSelectDirectory}
                icon={<FolderPlus size={12} />}
              >
                {t("sourceManager.importProjectModal.selectDirectory")}
              </Button>
            </Space.Compact>
          </Form.Item>
          <Form.Item
            name="mode"
            label={t("sourceManager.importProjectModal.mode")}
            rules={[{ required: true }]}
          >
            <Radio.Group>
              <Radio value="create">
                <Text strong>{t("sourceManager.importProjectModal.modeCreate")}</Text>
                <br />
                <Text type="secondary" style={{ fontSize: 12 }}>
                  {t("sourceManager.importProjectModal.modeCreateDesc")}
                </Text>
              </Radio>
              <Radio value="update" style={{ display: "block", marginLeft: 0, marginTop: 8 }}>
                <Text strong>{t("sourceManager.importProjectModal.modeUpdate")}</Text>
                <br />
                <Text type="secondary" style={{ fontSize: 12 }}>
                  {t("sourceManager.importProjectModal.modeUpdateDesc")}
                </Text>
              </Radio>
            </Radio.Group>
          </Form.Item>
          <Form.Item
            name={mode === "update" ? "targetBaseId" : "sourceName"}
            label={mode === "update"
              ? t("sourceManager.importProjectModal.existingSources")
              : t("sourceManager.importProjectModal.sourceName")}
            rules={[{
              required: true,
              message: mode === "update"
                ? t("sourceManager.importProjectModal.selectRequired")
                : t("sourceManager.importProjectModal.sourceNameRequired"),
            }]}
            extra={mode === "create" ? t("sourceManager.importProjectModal.sourceNameHint") : undefined}
          >
            {mode === "update"
              ? (
                <Select
                  placeholder={t("sourceManager.importProjectModal.selectExisting")}
                  options={knowledgeSources.map((s) => ({ label: s.name, value: s.id }))}
                />
              )
              : <Input placeholder={t("sourceManager.importProjectModal.sourceNamePlaceholder")} />}
          </Form.Item>
          {mode === "create" && (
            <Form.Item
              name="embeddingProvider"
              label={t("sourceManager.importProjectModal.embeddingModel")}
              extra={t("sourceManager.importProjectModal.embeddingModelHint")}
            >
              <EmbeddingModelSelect
                value={form.getFieldValue("embeddingProvider")}
                onChange={(val) => form.setFieldValue("embeddingProvider", val)}
                placeholder={t("sourceManager.importProjectModal.embeddingModelPlaceholder")}
                style={{ width: "100%" }}
              />
            </Form.Item>
          )}
          <Form.Item
            name="recursive"
            label={t("sourceManager.importProjectModal.recursive")}
            valuePropName="checked"
            extra={t("sourceManager.importProjectModal.recursiveHint")}
          >
            <Switch />
          </Form.Item>
          <Form.Item
            name="extensionsText"
            label={t("sourceManager.importProjectModal.extensions")}
            extra={t("sourceManager.importProjectModal.extensionsHint")}
          >
            <Input placeholder={t("sourceManager.importProjectModal.extensionsPlaceholder")} />
          </Form.Item>
          <Form.Item
            name="ignorePatternsText"
            label={t("sourceManager.importProjectModal.ignorePatterns")}
            extra={t("sourceManager.importProjectModal.ignorePatternsHint")}
          >
            <Input.TextArea
              rows={2}
              placeholder={t("sourceManager.importProjectModal.ignorePatternsPlaceholder")}
            />
          </Form.Item>
          {mode === "create" && (
            <Form.Item
              name="conflict"
              label={t("sourceManager.importProjectModal.conflict")}
              extra={t("sourceManager.importProjectModal.conflictHint")}
            >
              <Radio.Group>
                <Radio value="skip">
                  {t("sourceManager.importProjectModal.conflictSkip")}
                </Radio>
                <Radio value="overwrite">
                  {t("sourceManager.importProjectModal.conflictOverwrite")}
                </Radio>
              </Radio.Group>
            </Form.Item>
          )}
          {mode === "create" && (
            <>
              <Form.Item
                name="wikiCopy"
                label={t("sourceManager.importProjectModal.wikiCopy")}
                valuePropName="checked"
                extra={t("sourceManager.importProjectModal.wikiCopyHint")}
              >
                <Switch />
              </Form.Item>
              {wikiCopyEnabled && (
                <Form.Item
                  name="wikiVaultId"
                  label={t("sourceManager.importProjectModal.wikiVault")}
                  rules={[{
                    required: true,
                    message: t("sourceManager.importProjectModal.wikiVaultRequired"),
                  }]}
                >
                  <Select
                    placeholder={t("sourceManager.importProjectModal.wikiVaultPlaceholder")}
                    options={wikis
                      .filter((w) => w.rootPath.trim().length > 0)
                      .map((w) => ({ label: w.name, value: w.id }))}
                  />
                </Form.Item>
              )}
            </>
          )}
        </Form>
      )}

      {step === 1 && (
        <Space direction="vertical" style={{ width: "100%" }}>
          <Space wrap>
            <Tag color="blue">
              {t("sourceManager.importProjectModal.previewTotal", {
                total: scanResult?.totalCount ?? 0,
              })}
            </Tag>
            <Tag color="green">
              {t("sourceManager.importProjectModal.previewNew", {
                count: (scanResult?.totalCount ?? 0) - (scanResult?.existingCount ?? 0),
              })}
            </Tag>
            <Tag color="orange">
              {t("sourceManager.importProjectModal.previewExisting", {
                count: scanResult?.existingCount ?? 0,
              })}
            </Tag>
            <Tag>
              {t("sourceManager.importProjectModal.previewSkipped", {
                count: scanResult?.skippedCount ?? 0,
              })}
            </Tag>
            {scanResult?.embeddingProvider && (
              <Tag color="purple">
                {t("sourceManager.importProjectModal.previewEmbedding", {
                  provider: formatProviderLabel(scanResult.embeddingProvider),
                })}
              </Tag>
            )}
            <Button size="small" icon={<RefreshCw size={12} />} onClick={handleScan}>
              {t("sourceManager.importProjectModal.rescan")}
            </Button>
          </Space>
          <Table
            size="small"
            rowKey="path"
            columns={previewColumns}
            dataSource={scanResult?.files ?? []}
            pagination={{ pageSize: 8, hideOnSinglePage: true }}
            scroll={{ y: 260 }}
          />
          {mode === "update" && (
            <Text type="warning" style={{ fontSize: 12 }}>
              {t("sourceManager.importProjectModal.updateWarning")}
            </Text>
          )}
        </Space>
      )}

      {step === 2 && (
        <Space direction="vertical" style={{ width: "100%" }}>
          <Spin spinning={importing}>
            <Space direction="vertical" style={{ width: "100%" }}>
              <Text strong>{summary.label}</Text>
              <Text type="secondary">{summary.detail}</Text>
            </Space>
          </Spin>
          {importResult && importResult.errorCount > 0 && (
            <Table<ImportDirectoryError>
              size="small"
              rowKey={(r, i) => `${r.path}-${i}`}
              pagination={false}
              scroll={{ y: 180 }}
              columns={[
                {
                  title: t("sourceManager.importProjectModal.previewFile"),
                  dataIndex: "path",
                  ellipsis: true,
                },
                {
                  title: t("sourceManager.importProjectModal.previewError"),
                  dataIndex: "error",
                  width: 240,
                  render: (err, record) => (
                    <Text type="danger" style={{ fontSize: 12 }}>
                      {translateErrorCode(t, record.code, err)}
                    </Text>
                  ),
                },
              ]}
              dataSource={importResult.errors}
            />
          )}
        </Space>
      )}
    </Modal>
  );
}

export { DirectoryImportWizard };
