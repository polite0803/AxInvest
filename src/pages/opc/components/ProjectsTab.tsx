// SPDX-License-Identifier: AGPL-3.0-only

import { invoke } from "@/lib/invoke";
import { resolvePluginForCommand } from "@/lib/pluginCommandResolve";
import { DeleteOutlined, EditOutlined, FlagOutlined, PlusOutlined } from "@ant-design/icons";
import {
  Button,
  Card,
  Col,
  DatePicker,
  Descriptions,
  Divider,
  Empty,
  Form,
  Input,
  InputNumber,
  message,
  Modal,
  Popconfirm,
  Row,
  Select,
  Space,
  Spin,
  Table,
  Tag,
  Timeline,
  Typography,
} from "antd";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import { getProjectStatusKey, PROJ_STATUS_COLOR_MAP } from "../utils/constants";

// 本面板的八条命令（#32 批 B 补齐项目详情 / 更新 / 删除 / 里程碑完成后**全部**走插件通道，
// 前端不再 invoke 任何宿主 `opc_*` 项目命令）。
const LIST_PROJECTS_COMMAND = "opc_list_projects";
const GET_PROJECT_COMMAND = "opc_get_project";
const CREATE_PROJECT_COMMAND = "opc_create_project";
const UPDATE_PROJECT_COMMAND = "opc_update_project";
const DELETE_PROJECT_COMMAND = "opc_delete_project";
const ADD_MILESTONE_COMMAND = "opc_add_milestone";
const COMPLETE_MILESTONE_COMMAND = "opc_complete_milestone";
const LIST_CUSTOMERS_COMMAND = "opc_list_customers";

// 插件命令的统一执行通道（先例 SitesTab / CustomersTab / InvoicesTab）：按清单解析启用插件
// ⇒ 宿主 `plugin_execute_command` 派发。插件未装 / 未启用抛错，与取数失败同一路径处置，
// 不新造错误码。dev mock 下 `plugin_execute_command` 返回空串 ⇒ 取数失败提示。
async function execPluginCommand(name: string, input: Record<string, unknown>): Promise<string> {
  const pluginId = await resolvePluginForCommand(name);
  if (!pluginId) {
    throw new Error(`no enabled plugin provides command \`${name}\``);
  }
  return invoke<string>("plugin_execute_command", { pluginId, name, input });
}

// 插件命令 `opc_list_customers` 的下拉选项类型 = 接缝视图 `CustomerSummaryView` 的本面板
// 已读子集（出参线格式 = 视图 camelCase 键，插件出口不改写，禁区 13）。
interface CustomerOption {
  id: string;
  name: string;
  email: string;
}

// 项目行 = 接缝 `ProjectSummaryView`（列表 op 出参，camelCase，禁区 13）。里程碑只有两个
// 计数 —— 明细在详情命令 `opc_get_project` 的 `ProjectDetailView` 里（列表不扩列）。
interface ProjectRow {
  id: string;
  customerId: string | null;
  title: string;
  status: string;
  budget: number | null;
  currency: string;
  deadline: number | null;
  milestoneTotal: number;
  milestoneCompleted: number;
  createdAt: number;
}

// 一条里程碑明细 = 接缝 `MilestoneView`（时间同为 epoch 秒）。
interface MilestoneRow {
  id: string;
  title: string;
  description: string;
  dueAt: number | null;
  completedAt: number | null;
  status: string;
}

// 详情命令出参 = 接缝 `ProjectDetailView`：摘要**平铺**（`#[serde(flatten)]`）+ 详情三列，
// 故本类型直接继承行类型，不为详情另造一套同义字段。
interface ProjectDetail extends ProjectRow {
  description: string;
  notes: string;
  milestones: MilestoneRow[];
}

const { Text } = Typography;

export function ProjectsTab() {
  const { t } = useTranslation();
  const [projects, setProjects] = useState<ProjectRow[]>([]);
  const [customers, setCustomers] = useState<CustomerOption[]>([]);
  const [loading, setLoading] = useState(true);
  const [modalOpen, setModalOpen] = useState(false);
  const [editing, setEditing] = useState<ProjectRow | null>(null);
  const [detailOpen, setDetailOpen] = useState(false);
  const [detailLoading, setDetailLoading] = useState(false);
  const [detailProject, setDetailProject] = useState<ProjectDetail | null>(null);
  const [milestoneOpen, setMilestoneOpen] = useState(false);
  const [filterStatus, setFilterStatus] = useState<string>("");
  const [filterCustomer, setFilterCustomer] = useState<string>("");
  const [selectedRowKeys, setSelectedRowKeys] = useState<string[]>([]);
  const [milestoneForm] = Form.useForm();
  const [form] = Form.useForm();

  // 详情取数：列表行只有摘要列，描述 / 备注 / 里程碑明细必须追一次详情命令
  // （详情弹层与「编辑」回填共用这一条）。
  const fetchDetail = useCallback(async (id: string): Promise<ProjectDetail> => {
    const raw = await execPluginCommand(GET_PROJECT_COMMAND, { id });
    return JSON.parse(raw) as ProjectDetail;
  }, []);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const [rawProjects, rawCust] = await Promise.all([
        execPluginCommand(LIST_PROJECTS_COMMAND, {}),
        execPluginCommand(LIST_CUSTOMERS_COMMAND, {}),
      ]);
      setProjects(JSON.parse(rawProjects) as ProjectRow[]);
      setCustomers(JSON.parse(rawCust) as CustomerOption[]);
    } catch (e) {
      message.error(t("opc.common.loadFailed", { error: String(e) }));
      setProjects([]);
      setCustomers([]);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    load();
  }, [load]);

  const openDetail = async (row: ProjectRow) => {
    setDetailOpen(true);
    setDetailLoading(true);
    try {
      setDetailProject(await fetchDetail(row.id));
    } catch (e) {
      setDetailProject(null);
      message.error(t("opc.common.loadFailed", { error: String(e) }));
    } finally {
      setDetailLoading(false);
    }
  };

  const openEdit = async (row: ProjectRow) => {
    try {
      const detail = await fetchDetail(row.id);
      setEditing(row);
      // Form 内部字段名保持旧称（`customer_id` / `due_at` 等），只有出站键转 camelCase。
      form.setFieldsValue({
        title: detail.title,
        description: detail.description,
        customer_id: detail.customerId,
        budget: detail.budget,
        deadline: detail.deadline ? new Date(detail.deadline * 1000) : null,
        notes: detail.notes,
      });
      setModalOpen(true);
    } catch (e) {
      message.error(t("opc.common.loadFailed", { error: String(e) }));
    }
  };

  const handleSave = async (values: Record<string, unknown>) => {
    try {
      const title = values.title as string;
      const description = (values.description as string) || "";
      const notes = (values.notes as string) || "";
      const budget = typeof values.budget === "number" ? values.budget : undefined;
      const deadline = values.deadline
        ? Math.floor(new Date(values.deadline as string).getTime() / 1000)
        : undefined;
      if (editing) {
        // 出站键 camelCase（禁区 13）；缺键 = 该列不动。⚠ `customerId` 刻意不转发：域包
        // `UpdateProjectInput` 没有客户更新位（转发等于宣称改了，实际静默不生效）。
        await execPluginCommand(UPDATE_PROJECT_COMMAND, {
          id: editing.id,
          title,
          description,
          budget,
          deadline,
          notes,
        });
        message.success(t("opc.project.updated"));
      } else {
        await execPluginCommand(CREATE_PROJECT_COMMAND, {
          customerId: (values.customer_id as string) || undefined,
          title,
          description,
          budget,
          currency: "CNY",
          deadline,
          notes,
        });
        message.success(t("opc.project.created"));
      }
      setModalOpen(false);
      setEditing(null);
      form.resetFields();
      load();
    } catch (e) {
      message.error(t("opc.common.opFailed", { error: String(e) }));
    }
  };

  const handleDelete = async (id: string) => {
    try {
      await execPluginCommand(DELETE_PROJECT_COMMAND, { id });
      message.success(t("opc.project.deleted"));
      load();
    } catch (e) {
      message.error(t("opc.common.deleteFailed", { error: String(e) }));
    }
  };

  const handleAddMilestone = async (values: Record<string, unknown>) => {
    if (!detailProject) { return; }
    try {
      // 里程碑 id / 初始状态由宿主 provider 生成（`Pending`）—— 前端不再自造 id。
      await execPluginCommand(ADD_MILESTONE_COMMAND, {
        projectId: detailProject.id,
        title: values.title as string,
        description: (values.description as string) || "",
        dueAt: values.due_at
          ? Math.floor(new Date(values.due_at as string).getTime() / 1000)
          : undefined,
      });
      message.success(t("opc.project.milestoneAdded"));
      setMilestoneOpen(false);
      milestoneForm.resetFields();
      setDetailProject(await fetchDetail(detailProject.id));
      load();
    } catch (e) {
      message.error(t("opc.project.milestoneAddFailed", { error: String(e) }));
    }
  };

  const handleCompleteMilestone = async (milestoneId: string) => {
    if (!detailProject) { return; }
    try {
      // 出参就是完成后的详情视图 ⇒ 详情弹层原地刷新，不必再追一次 `opc_get_project`。
      const raw = await execPluginCommand(COMPLETE_MILESTONE_COMMAND, {
        projectId: detailProject.id,
        milestoneId,
      });
      message.success(t("opc.project.milestoneCompleted"));
      setDetailProject(JSON.parse(raw) as ProjectDetail);
      load();
    } catch (e) {
      message.error(t("opc.common.opFailed", { error: String(e) }));
    }
  };

  const filteredProjects = projects.filter((p) => {
    if (filterStatus && p.status !== filterStatus) { return false; }
    if (filterCustomer && p.customerId !== filterCustomer) { return false; }
    return true;
  });

  const handleBatchDelete = async () => {
    try {
      await Promise.all(
        selectedRowKeys.map((id) => execPluginCommand(DELETE_PROJECT_COMMAND, { id })),
      );
      message.success(t("opc.common.batchDeleteSuccess"));
      setSelectedRowKeys([]);
      load();
    } catch (e) {
      message.error(t("opc.common.deleteFailed", { error: String(e) }));
    }
  };

  const columns = [
    { title: t("opc.project.columnTitle"), dataIndex: "title", key: "title" },
    {
      title: t("opc.project.columnStatus"),
      key: "status",
      render: (_: unknown, r: ProjectRow) => {
        const color = PROJ_STATUS_COLOR_MAP[r.status] || "default";
        return <Tag color={color}>{t(getProjectStatusKey(r.status))}</Tag>;
      },
    },
    {
      title: t("opc.project.columnMilestones"),
      key: "milestones",
      render: (_: unknown, r: ProjectRow) => r.milestoneTotal > 0 ? `${r.milestoneCompleted}/${r.milestoneTotal}` : "-",
    },
    {
      title: t("opc.project.columnBudget"),
      key: "budget",
      render: (_: unknown, r: ProjectRow) => (r.budget ? `¥${r.budget.toFixed(2)}` : "-"),
      sorter: (a: ProjectRow, b: ProjectRow) => (a.budget || 0) - (b.budget || 0),
    },
    {
      title: t("opc.project.columnDeadline"),
      key: "deadline",
      render: (_: unknown, r: ProjectRow) => r.deadline ? new Date(r.deadline * 1000).toLocaleDateString() : "-",
    },
    {
      title: t("opc.common.actions"),
      key: "actions",
      width: 160,
      render: (_: unknown, r: ProjectRow) => (
        <Space size="small">
          <Button size="small" icon={<FlagOutlined />} onClick={() => openDetail(r)}>
            {t("opc.project.details")}
          </Button>
          <Button size="small" icon={<EditOutlined />} onClick={() => openEdit(r)} />
          <Popconfirm title={t("opc.project.confirmDelete")} onConfirm={() => handleDelete(r.id)}>
            <Button size="small" danger icon={<DeleteOutlined />} />
          </Popconfirm>
        </Space>
      ),
    },
  ];

  return (
    <>
      {/* 筛选工具栏 */}
      <Card size="small" style={{ marginBottom: 12 }}>
        <Row gutter={12} align="middle">
          <Col xs={24} sm={8} md={6}>
            <Select
              placeholder={t("opc.project.filterStatus")}
              allowClear
              style={{ width: "100%" }}
              value={filterStatus || undefined}
              onChange={(val) => setFilterStatus(val || "")}
              options={[
                { value: "planning", label: t("opc.projectStatus.planning") },
                { value: "active", label: t("opc.projectStatus.active") },
                { value: "paused", label: t("opc.projectStatus.paused") },
                { value: "completed", label: t("opc.projectStatus.completed") },
                { value: "cancelled", label: t("opc.projectStatus.cancelled") },
              ]}
            />
          </Col>
          <Col xs={24} sm={8} md={6}>
            <Select
              placeholder={t("opc.project.filterCustomer")}
              allowClear
              style={{ width: "100%" }}
              value={filterCustomer || undefined}
              onChange={(val) => setFilterCustomer(val || "")}
              optionFilterProp="label"
              options={customers.map((c) => ({ value: c.id, label: `${c.name} (${c.email})` }))}
            />
          </Col>
          <Col>
            <Space>
              {selectedRowKeys.length > 0 && (
                <Button danger size="small" onClick={handleBatchDelete}>
                  {t("opc.common.batchDelete", { count: selectedRowKeys.length })}
                </Button>
              )}
              <Button
                size="small"
                onClick={() => {
                  setFilterStatus("");
                  setFilterCustomer("");
                  setSelectedRowKeys([]);
                }}
              >
                {t("opc.common.reset")}
              </Button>
            </Space>
          </Col>
        </Row>
      </Card>

      <Card
        extra={
          <Button
            type="primary"
            size="small"
            icon={<PlusOutlined />}
            onClick={() => {
              setEditing(null);
              form.resetFields();
              setModalOpen(true);
            }}
          >
            {t("opc.project.newProject")}
          </Button>
        }
      >
        <Table
          dataSource={filteredProjects}
          columns={columns}
          rowKey="id"
          loading={loading}
          size="small"
          pagination={{ pageSize: 20 }}
          rowSelection={{
            selectedRowKeys,
            onChange: (keys) => setSelectedRowKeys(keys.map(String)),
          }}
        />
      </Card>

      {/* 新建/编辑项目 Modal */}
      <Modal
        title={editing ? t("opc.project.editTitle") : t("opc.project.newProject")}
        open={modalOpen}
        onOk={() => form.submit()}
        onCancel={() => {
          setModalOpen(false);
          setEditing(null);
          form.resetFields();
        }}
        width={560}
        okText={editing ? t("opc.common.update") : t("opc.common.create")}
        cancelText={t("opc.common.cancel")}
      >
        <Form form={form} layout="vertical" onFinish={handleSave}>
          <Form.Item
            name="title"
            label={t("opc.project.titleLabel")}
            rules={[{ required: true, message: t("opc.project.titleRequired") }]}
          >
            <Input />
          </Form.Item>
          <Form.Item name="description" label={t("opc.common.description")}>
            <Input.TextArea rows={3} />
          </Form.Item>
          <Form.Item name="customer_id" label={t("opc.project.customerLabel")}>
            <Select
              allowClear
              placeholder={t("opc.project.customerPlaceholder")}
              optionFilterProp="label"
              options={customers.map((c) => ({ value: c.id, label: `${c.name} (${c.email})` }))}
            />
          </Form.Item>
          <Form.Item name="budget" label={t("opc.project.budgetLabel")}>
            <InputNumber min={0} precision={2} prefix="¥" style={{ width: "100%" }} />
          </Form.Item>
          <Form.Item name="deadline" label={t("opc.common.dueDate")}>
            <DatePicker style={{ width: "100%" }} />
          </Form.Item>
          <Form.Item name="notes" label={t("opc.common.notes")}>
            <Input.TextArea rows={2} />
          </Form.Item>
        </Form>
      </Modal>

      {/* 项目详情 Modal（描述 / 备注 / 里程碑明细走 `opc_get_project`，打开时现取） */}
      <Modal
        title={detailProject?.title}
        open={detailOpen}
        onCancel={() => {
          setDetailOpen(false);
          setDetailProject(null);
        }}
        footer={null}
        width={520}
      >
        {detailLoading ? <Spin /> : detailProject && (
          <div>
            <Descriptions size="small" column={1}>
              <Descriptions.Item label={t("opc.project.columnStatus")}>
                <Tag color={PROJ_STATUS_COLOR_MAP[detailProject.status] || "default"}>
                  {t(getProjectStatusKey(detailProject.status))}
                </Tag>
              </Descriptions.Item>
              <Descriptions.Item label={t("opc.common.description")}>
                {detailProject.description || "-"}
              </Descriptions.Item>
              <Descriptions.Item label={t("opc.project.budgetLabel")}>
                {detailProject.budget ? `¥${detailProject.budget.toFixed(2)}` : "-"}
              </Descriptions.Item>
              <Descriptions.Item label={t("opc.common.dueDate")}>
                {detailProject.deadline
                  ? new Date(detailProject.deadline * 1000).toLocaleDateString()
                  : "-"}
              </Descriptions.Item>
            </Descriptions>
            <Divider />
            <div
              style={{
                display: "flex",
                justifyContent: "space-between",
                alignItems: "center",
                marginBottom: 12,
              }}
            >
              <Text strong>
                {t("opc.project.milestonesTitle", { count: detailProject.milestones.length })}
              </Text>
              <Button
                size="small"
                icon={<PlusOutlined />}
                onClick={() => {
                  milestoneForm.resetFields();
                  setMilestoneOpen(true);
                }}
              >
                {t("opc.project.addMilestone")}
              </Button>
            </div>
            {detailProject.milestones.length === 0
              ? <Empty description={t("opc.project.noMilestones")} />
              : (
                <Timeline
                  items={detailProject.milestones.map((m) => ({
                    color: m.status === "completed"
                      ? "green"
                      : m.status === "in_progress"
                      ? "blue"
                      : "gray",
                    children: (
                      <div>
                        <div style={{ display: "flex", justifyContent: "space-between" }}>
                          <Text strong>{m.title}</Text>
                          {m.status !== "completed" && (
                            <Button
                              size="small"
                              type="link"
                              onClick={() => handleCompleteMilestone(m.id)}
                            >
                              {t("opc.project.complete")}
                            </Button>
                          )}
                        </div>
                        <div>
                          <Text type="secondary">{m.description}</Text>
                        </div>
                        <div>
                          <Text type="secondary" style={{ fontSize: 12 }}>
                            {m.dueAt
                              ? t("opc.project.dueBy", {
                                date: new Date(m.dueAt * 1000).toLocaleDateString(),
                              })
                              : ""}
                          </Text>
                        </div>
                      </div>
                    ),
                  }))}
                />
              )}
          </div>
        )}
      </Modal>

      {/* 添加里程碑 Modal */}
      <Modal
        title={t("opc.project.milestoneModalTitle")}
        open={milestoneOpen}
        onOk={() => milestoneForm.submit()}
        onCancel={() => {
          setMilestoneOpen(false);
          milestoneForm.resetFields();
        }}
        okText={t("opc.project.milestoneOkAdd")}
        cancelText={t("opc.common.cancel")}
      >
        <Form form={milestoneForm} layout="vertical" onFinish={handleAddMilestone}>
          <Form.Item
            name="title"
            label={t("opc.project.milestoneTitleLabel")}
            rules={[{ required: true, message: t("opc.project.milestoneTitleRequired") }]}
          >
            <Input />
          </Form.Item>
          <Form.Item name="description" label={t("opc.common.description")}>
            <Input.TextArea rows={2} />
          </Form.Item>
          <Form.Item name="due_at" label={t("opc.common.dueDate")}>
            <DatePicker style={{ width: "100%" }} />
          </Form.Item>
        </Form>
      </Modal>
    </>
  );
}
