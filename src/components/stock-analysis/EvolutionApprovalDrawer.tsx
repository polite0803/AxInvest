/**
 * P4 受控自动优化：进化计划审批抽屉（计划-审批两段式的「审批」段）。
 *
 * 双入口复用：
 * - EvolutionDriftPanel：待审批卡 + accepted 历史「查看」
 * - TasksCenter/OverviewTab：健康卡「待审批进化」
 *
 * 数据走 stockAnalysisStore（pendingEvolutionPlans）；选中行由父组件持有，
 * 抽屉顶部待办列表可切换。接受 = 后端重放执行并回写；拒绝必填原因；
 * accepted 行可一键回滚到上一已接受版本。
 */

import {
  AuditOutlined,
  CheckCircleOutlined,
  CloseCircleOutlined,
  HistoryOutlined,
  RollbackOutlined,
} from "@ant-design/icons";
import {
  App,
  Badge,
  Button,
  Collapse,
  Descriptions,
  Drawer,
  Empty,
  Input,
  List,
  Modal,
  Popconfirm,
  Space,
  Spin,
  Tag,
  Tooltip,
  Typography,
} from "antd";
import { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";

import { type EvolutionHistoryEntry, useStockAnalysisStore } from "@/stores/feature/stockAnalysisStore";
import {
  evolutionStatusMeta,
  formatEvolutionTime,
  parseEvolutionResult,
  parseTrigger,
  parseTriggerSamples,
} from "./evolution-ui-utils";

const { Text, Paragraph } = Typography;

export interface EvolutionApprovalDrawerProps {
  open: boolean;
  /** 当前选中行（pending 行可审批；accepted/rejected 行只读查看 + accepted 可回滚） */
  selected: EvolutionHistoryEntry | null;
  onSelect: (row: EvolutionHistoryEntry) => void;
  onClose: () => void;
  /** 审批/拒绝/回滚成功后回调：父组件刷新历史与 dashboard */
  onChanged?: () => void;
}

/** 触发样本分组（有效 + blocked）的单行展示 */
function SampleGroup({
  ids,
  blocked,
  label,
}: {
  ids: string[];
  blocked: boolean;
  label: string;
}) {
  const { t } = useTranslation();
  if (ids.length === 0) {
    return null;
  }
  return (
    <div style={{ marginBottom: 8 }}>
      <Space size={6} wrap>
        <Tag color={blocked ? "red" : "blue"}>
          {blocked ? "⛔ " : ""}
          {label}
        </Tag>
        <Text type="secondary" style={{ fontSize: 12 }}>
          {ids.length}
        </Text>
      </Space>
      <Collapse
        size="small"
        ghost
        items={[
          {
            key: "ids",
            label: t("stockAnalysis.evolutionApproval.samples.expand"),
            children: (
              <Space size={4} wrap>
                {ids.map((id) => (
                  <Tag key={id} color={blocked ? "red" : "default"} style={{ fontSize: 11 }}>
                    {id}
                  </Tag>
                ))}
              </Space>
            ),
          },
        ]}
      />
    </div>
  );
}

export function EvolutionApprovalDrawer({
  open,
  selected,
  onSelect,
  onClose,
  onChanged,
}: EvolutionApprovalDrawerProps) {
  const { t } = useTranslation();
  const { message } = App.useApp();
  const pendingPlans = useStockAnalysisStore((s) => s.pendingEvolutionPlans);
  const pendingLoading = useStockAnalysisStore((s) => s.pendingEvolutionLoading);
  const fetchPending = useStockAnalysisStore((s) => s.fetchPendingEvolutionPlans);
  const approvePlan = useStockAnalysisStore((s) => s.approveEvolutionPlan);
  const rejectPlan = useStockAnalysisStore((s) => s.rejectEvolutionPlan);
  const rollbackPlan = useStockAnalysisStore((s) => s.rollbackEvolution);

  const [submitting, setSubmitting] = useState(false);
  const [rejectOpen, setRejectOpen] = useState(false);
  const [rejectReason, setRejectReason] = useState("");

  useEffect(() => {
    if (open) {
      void fetchPending();
    }
  }, [open, fetchPending]);

  // 打开抽屉且父组件未指定行时，默认选中第一条待办
  useEffect(() => {
    if (open && !selected && pendingPlans && pendingPlans.length > 0) {
      onSelect(pendingPlans[0]);
    }
  }, [open, selected, pendingPlans, onSelect]);

  const samples = useMemo(
    () => parseTriggerSamples(selected?.triggerSampleJson),
    [selected?.triggerSampleJson],
  );
  const evolutionResult = useMemo(
    () => parseEvolutionResult(selected?.parameterResultJson),
    [selected?.parameterResultJson],
  );

  const isPending = selected?.status === "pending_approval";
  const isAccepted = selected?.status === "accepted";
  const canRollback = isAccepted && !!selected?.rollbackRef;
  const hasBlocked = samples
    ? samples.blockedReflections.length
        + samples.blockedValidations.length
        + samples.blockedMoverEvents.length
      > 0
    : false;

  const refreshAfterChange = useCallback(async () => {
    await fetchPending();
    onChanged?.();
  }, [fetchPending, onChanged]);

  const handleApprove = useCallback(async () => {
    if (!selected) {
      return;
    }
    setSubmitting(true);
    try {
      await approvePlan(selected.id);
      message.success(t("stockAnalysis.evolutionApproval.approveSuccess"));
      await refreshAfterChange();
    } catch (e) {
      message.error(
        t("stockAnalysis.evolutionApproval.actionFailed", { msg: String(e) }),
      );
    } finally {
      setSubmitting(false);
    }
  }, [selected, approvePlan, message, t, refreshAfterChange]);

  const handleRejectConfirm = useCallback(async () => {
    if (!selected) {
      return;
    }
    const reason = rejectReason.trim();
    if (!reason) {
      message.warning(t("stockAnalysis.evolutionApproval.rejectReasonRequired"));
      return;
    }
    setSubmitting(true);
    try {
      await rejectPlan(selected.id, reason.slice(0, 500));
      message.success(t("stockAnalysis.evolutionApproval.rejectSuccess"));
      setRejectOpen(false);
      setRejectReason("");
      await refreshAfterChange();
    } catch (e) {
      message.error(
        t("stockAnalysis.evolutionApproval.actionFailed", { msg: String(e) }),
      );
    } finally {
      setSubmitting(false);
    }
  }, [selected, rejectReason, rejectPlan, message, t, refreshAfterChange]);

  const handleRollback = useCallback(async () => {
    if (!selected) {
      return;
    }
    setSubmitting(true);
    try {
      await rollbackPlan(selected.id);
      message.success(t("stockAnalysis.evolutionApproval.rollbackSuccess"));
      await refreshAfterChange();
    } catch (e) {
      message.error(
        t("stockAnalysis.evolutionApproval.actionFailed", { msg: String(e) }),
      );
    } finally {
      setSubmitting(false);
    }
  }, [selected, rollbackPlan, message, t, refreshAfterChange]);

  const statusTag = selected
    ? (
      <Tag color={evolutionStatusMeta(selected.status).color}>
        {t(evolutionStatusMeta(selected.status).textKey)}
      </Tag>
    )
    : null;

  return (
    <Drawer
      title={
        <Space>
          <AuditOutlined />
          {t("stockAnalysis.evolutionApproval.drawerTitle")}
          <Badge
            count={pendingPlans?.length ?? 0}
            showZero
            style={{ backgroundColor: (pendingPlans?.length ?? 0) > 0 ? "#fa8c16" : "#bfbfbf" }}
          />
        </Space>
      }
      open={open}
      onClose={onClose}
      width={580}
      destroyOnHidden
    >
      {/* 待审批列表（双入口共用，点击切换详情） */}
      <div style={{ marginBottom: 16 }}>
        <Text strong>
          {t("stockAnalysis.evolutionApproval.pendingTitle", {
            count: pendingPlans?.length ?? 0,
          })}
        </Text>
        <Spin spinning={pendingLoading} size="small">
          {!pendingPlans || pendingPlans.length === 0
            ? (
              <Empty
                image={Empty.PRESENTED_IMAGE_SIMPLE}
                description={t("stockAnalysis.evolutionApproval.emptyPending")}
                style={{ margin: "8px 0" }}
              />
            )
            : (
              <List
                size="small"
                bordered
                dataSource={pendingPlans}
                style={{ marginTop: 8 }}
                renderItem={(item) => (
                  <List.Item
                    onClick={() => onSelect(item)}
                    style={{
                      cursor: "pointer",
                      padding: "6px 12px",
                      backgroundColor: selected?.id === item.id ? "var(--ant-color-primary-bg, #e6f4ff)" : undefined,
                    }}
                  >
                    <Space size={6} wrap>
                      <Tag color="orange">
                        {t("stockAnalysis.evolutionApproval.status.pendingApproval")}
                      </Tag>
                      <Tag>{item.evolutionType}</Tag>
                      <Text type="secondary" style={{ fontSize: 12 }}>
                        {formatEvolutionTime(item.createdAt)}
                      </Text>
                      <Text ellipsis style={{ maxWidth: 260, fontSize: 12 }}>
                        {parseTrigger(item.trigger, t)}
                      </Text>
                    </Space>
                  </List.Item>
                )}
              />
            )}
        </Spin>
      </div>

      {!selected ? <Empty description={t("stockAnalysis.evolutionApproval.selectHint")} /> : (
        <div>
          {/* 详情头 */}
          <Descriptions column={1} size="small" colon={false}>
            <Descriptions.Item label={t("stockAnalysis.evolutionApproval.meta.status")}>
              <Space size={6} wrap>
                {statusTag}
                <Tag>{selected.evolutionType}</Tag>
              </Space>
            </Descriptions.Item>
            <Descriptions.Item label={t("stockAnalysis.evolutionApproval.meta.createdAt")}>
              {formatEvolutionTime(selected.createdAt)}
            </Descriptions.Item>
            <Descriptions.Item label={t("stockAnalysis.evolutionApproval.meta.trigger")}>
              {parseTrigger(selected.trigger, t) || "—"}
            </Descriptions.Item>
            <Descriptions.Item label={t("stockAnalysis.evolutionApproval.meta.summary")}>
              {selected.improvementSummary || "—"}
            </Descriptions.Item>
          </Descriptions>

          {/* 参数 diff（accepted 行：回测最优 vs 默认） */}
          {evolutionResult && (
            <div style={{ marginTop: 16 }}>
              <Text strong>{t("stockAnalysis.evolutionApproval.diffTitle")}</Text>
              <Descriptions column={1} size="small" bordered style={{ marginTop: 8 }}>
                <Descriptions.Item
                  label={t("stockAnalysis.evolutionApproval.param.ewmaAlpha")}
                >
                  <span>
                    <Text delete type="secondary">
                      {evolutionResult.defaultConfig.ewmaAlpha}
                    </Text>
                    {" → "}
                    <Text strong>{evolutionResult.bestConfig.ewmaAlpha}</Text>
                  </span>
                </Descriptions.Item>
                <Descriptions.Item
                  label={t("stockAnalysis.evolutionApproval.param.lookbackDays")}
                >
                  <span>
                    <Text delete type="secondary">
                      {evolutionResult.defaultConfig.lookbackDays}
                    </Text>
                    {" → "}
                    <Text strong>{evolutionResult.bestConfig.lookbackDays}</Text>
                  </span>
                </Descriptions.Item>
                <Descriptions.Item
                  label={t("stockAnalysis.evolutionApproval.param.sampleSaturation")}
                >
                  <span>
                    <Text delete type="secondary">
                      {evolutionResult.defaultConfig.sampleSaturation}
                    </Text>
                    {" → "}
                    <Text strong>{evolutionResult.bestConfig.sampleSaturation}</Text>
                  </span>
                </Descriptions.Item>
              </Descriptions>
            </div>
          )}

          {/* 触发样本快照 */}
          {samples && (
            <div style={{ marginTop: 16 }}>
              <Text strong>{t("stockAnalysis.evolutionApproval.samplesTitle")}</Text>
              <div style={{ marginTop: 8 }}>
                <SampleGroup
                  ids={samples.reflections}
                  blocked={false}
                  label={t("stockAnalysis.evolutionApproval.samples.reflections")}
                />
                <SampleGroup
                  ids={samples.validations}
                  blocked={false}
                  label={t("stockAnalysis.evolutionApproval.samples.validations")}
                />
                <SampleGroup
                  ids={samples.moverEvents}
                  blocked={false}
                  label={t("stockAnalysis.evolutionApproval.samples.moverEvents")}
                />
                <SampleGroup
                  ids={samples.blockedReflections}
                  blocked
                  label={t("stockAnalysis.evolutionApproval.samples.blockedReflections")}
                />
                <SampleGroup
                  ids={samples.blockedValidations}
                  blocked
                  label={t("stockAnalysis.evolutionApproval.samples.blockedValidations")}
                />
                <SampleGroup
                  ids={samples.blockedMoverEvents}
                  blocked
                  label={t("stockAnalysis.evolutionApproval.samples.blockedMoverEvents")}
                />
              </div>
              {hasBlocked && (
                <Paragraph type="warning" style={{ fontSize: 12, marginTop: 4 }}>
                  {t("stockAnalysis.evolutionApproval.samples.blockedHint")}
                </Paragraph>
              )}
            </div>
          )}

          {/* 审批留痕 */}
          {(selected.reviewer || selected.reviewedAt || selected.rejectReason) && (
            <Descriptions column={1} size="small" style={{ marginTop: 16 }}>
              <Descriptions.Item label={t("stockAnalysis.evolutionApproval.meta.reviewer")}>
                {selected.reviewer ?? "—"}
              </Descriptions.Item>
              <Descriptions.Item label={t("stockAnalysis.evolutionApproval.meta.reviewedAt")}>
                {formatEvolutionTime(selected.reviewedAt)}
              </Descriptions.Item>
              {selected.rejectReason && (
                <Descriptions.Item
                  label={t("stockAnalysis.evolutionApproval.meta.rejectReason")}
                >
                  <Text type="danger">{selected.rejectReason}</Text>
                </Descriptions.Item>
              )}
              {selected.rollbackRef && (
                <Descriptions.Item
                  label={t("stockAnalysis.evolutionApproval.meta.rollbackRef")}
                >
                  <Text code>{selected.rollbackRef}</Text>
                </Descriptions.Item>
              )}
            </Descriptions>
          )}

          {/* 操作区 */}
          <div
            style={{ marginTop: 20, paddingTop: 12, borderTop: "1px solid var(--ant-color-border-secondary, #f0f0f0)" }}
          >
            <Space wrap>
              {isPending && (
                <>
                  <Popconfirm
                    title={t("stockAnalysis.evolutionApproval.approveConfirm")}
                    okText={t("stockAnalysis.evolutionApproval.approve")}
                    cancelText={t("common.cancel")}
                    okButtonProps={{ loading: submitting }}
                    onConfirm={() => void handleApprove()}
                  >
                    <Button type="primary" icon={<CheckCircleOutlined />} loading={submitting}>
                      {t("stockAnalysis.evolutionApproval.approve")}
                    </Button>
                  </Popconfirm>
                  <Button
                    danger
                    icon={<CloseCircleOutlined />}
                    disabled={submitting}
                    onClick={() => {
                      setRejectReason("");
                      setRejectOpen(true);
                    }}
                  >
                    {t("stockAnalysis.evolutionApproval.reject")}
                  </Button>
                </>
              )}
              {isAccepted
                && (canRollback
                  ? (
                    <Popconfirm
                      title={t("stockAnalysis.evolutionApproval.rollbackConfirm")}
                      okText={t("stockAnalysis.evolutionApproval.rollback")}
                      cancelText={t("common.cancel")}
                      okButtonProps={{ danger: true, loading: submitting }}
                      onConfirm={() => void handleRollback()}
                    >
                      <Button danger icon={<RollbackOutlined />} loading={submitting}>
                        {t("stockAnalysis.evolutionApproval.rollback")}
                      </Button>
                    </Popconfirm>
                  )
                  : (
                    <Tooltip title={t("stockAnalysis.evolutionApproval.rollbackUnavailable")}>
                      <Button danger icon={<RollbackOutlined />} disabled>
                        {t("stockAnalysis.evolutionApproval.rollback")}
                      </Button>
                    </Tooltip>
                  ))}
              {!isPending && !isAccepted && (
                <Text type="secondary" style={{ fontSize: 12 }}>
                  <HistoryOutlined /> {t("stockAnalysis.evolutionApproval.readonlyHint")}
                </Text>
              )}
            </Space>
          </div>
        </div>
      )}

      {/* 拒绝原因 Modal（必填） */}
      <Modal
        title={t("stockAnalysis.evolutionApproval.rejectTitle")}
        open={rejectOpen}
        onCancel={() => setRejectOpen(false)}
        confirmLoading={submitting}
        okText={t("stockAnalysis.evolutionApproval.rejectConfirmOk")}
        cancelText={t("common.cancel")}
        okButtonProps={{ danger: true }}
        onOk={() => void handleRejectConfirm()}
      >
        <Input.TextArea
          rows={4}
          maxLength={500}
          showCount
          value={rejectReason}
          onChange={(e) => setRejectReason(e.target.value)}
          placeholder={t("stockAnalysis.evolutionApproval.rejectReasonPlaceholder")}
        />
      </Modal>
    </Drawer>
  );
}
