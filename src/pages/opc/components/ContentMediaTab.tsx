// SPDX-License-Identifier: AGPL-3.0-only

import { invoke } from "@/lib/invoke";
import { resolvePluginForCommand } from "@/lib/pluginCommandResolve";
import { DeleteOutlined, EditOutlined, PlusOutlined, ScheduleOutlined, SearchOutlined } from "@ant-design/icons";
import {
  Button,
  Card,
  Col,
  DatePicker,
  Form,
  Input,
  message,
  Modal,
  Row,
  Select,
  Space,
  Steps,
  Table,
  Tabs,
  Tag,
} from "antd";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

// 本面板的八条命令（`plugin.opc` 命名命令，与 manifest `commands[].name` 逐字一致）。
// 宿主同名 Tauri 命令本批仍在（另一批负责删除与登记表收口），前端统一改走插件通道。
// ⚠ 宿主 `opc_delete_publish_schedule` 的插件侧等价是 `opc_cancel_publish_schedule`：
// 名字不同，语义同 —— 取消发布计划 = 硬删该行（接缝 `cancel_publish_schedule` 走
// `delete_by_id`，不是状态位改写）。
const LIST_CONTENT_ASSETS_COMMAND = "opc_list_content_assets";
const CREATE_CONTENT_ASSET_COMMAND = "opc_create_content_asset";
const UPDATE_CONTENT_ASSET_COMMAND = "opc_update_content_asset";
const DELETE_CONTENT_ASSET_COMMAND = "opc_delete_content_asset";
const GET_CONTENT_ASSET_COMMAND = "opc_get_content_asset";
const LIST_PUBLISH_SCHEDULES_COMMAND = "opc_list_publish_schedules";
const CREATE_PUBLISH_SCHEDULE_COMMAND = "opc_create_publish_schedule";
const CANCEL_PUBLISH_SCHEDULE_COMMAND = "opc_cancel_publish_schedule";
const PROCESS_DUE_SCHEDULES_COMMAND = "opc_process_due_schedules";

// 插件命令通道执行（与 InvoicesTab / CustomersTab / SitesTab 同形）：按清单解析启用插件 ⇒
// 宿主 `plugin_execute_command` 派发，出参是命令回执的 JSON 文本。插件未装 / 未启用抛错，
// 与取数失败同一路径处置，不新造错误码。
async function execPluginCommand(name: string, input: Record<string, unknown>): Promise<string> {
  const pluginId = await resolvePluginForCommand(name);
  if (!pluginId) {
    throw new Error(`no enabled plugin provides command \`${name}\``);
  }
  return invoke<string>("plugin_execute_command", { pluginId, name, input });
}

// 行类型 = 接缝视图 `ContentAssetSummaryView` 的已读子集（出参线格式 = 视图 camelCase 键，
// 插件出口不改写，禁区 13）。`createdAt` 是 epoch 秒 ⇒ 渲染要 `* 1000`。
// ⚠ `body` / `updatedAt` **不过线**（摘要视图列裁剪）：搜索只能按标题，编辑弹窗无法回填正文
// ⇒ 未改动正文时**不发** `body`（见 `handleUpdate`），免得把库里正文覆盖成空串。
interface ContentAsset {
  id: string;
  title: string;
  contentType: string;
  status: string;
  tags: string[];
  createdAt: number;
}

// 行类型 = 接缝视图 `PublishScheduleSummaryView` 的已读子集。
interface PublishSchedule {
  id: string;
  contentRefType: string;
  contentRefId: string;
  status: string;
  scheduledAt: number;
  publishedAt: number | null;
}

const CONTENT_TYPES = ["article", "video", "image", "social_post", "newsletter"] as const;

const STATUS_COLORS: Record<string, string> = {
  draft: "default",
  published: "green",
  pending: "orange",
  failed: "red",
};

export function ContentMediaTab() {
  const { t } = useTranslation();
  const [subTab, setSubTab] = useState("assets");
  return (
    <Tabs
      activeKey={subTab}
      onChange={setSubTab}
      size="small"
      items={[
        { key: "assets", label: t("opc.contentMedia.tabAssets"), children: <ContentAssetsPanel /> },
        { key: "schedules", label: t("opc.contentMedia.tabSchedules"), children: <PublishSchedulesPanel /> },
        { key: "wizard", label: t("opc.contentMedia.tabWizard"), children: <ContentCreationWizard /> },
      ]}
    />
  );
}

function ContentAssetsPanel() {
  const { t } = useTranslation();
  const [assets, setAssets] = useState<ContentAsset[]>([]);
  const [loading, setLoading] = useState(true);
  const [createOpen, setCreateOpen] = useState(false);
  const [editOpen, setEditOpen] = useState(false);
  const [editingAsset, setEditingAsset] = useState<ContentAsset | null>(null);
  const [searchText, setSearchText] = useState("");
  // 编辑弹窗的正文框是否被用户动过：正文不在列表视图里，没动过就不能回传（否则覆盖成空串）。
  const [bodyDirty, setBodyDirty] = useState(false);
  // 编辑序号：异步回读全文（见 openEdit）晚到时用它判「弹窗是否已切到别的行/已关闭」。
  const editSeqRef = useRef(0);
  const [form] = Form.useForm();

  const load = () => {
    setLoading(true);
    (async () => {
      try {
        const raw = await execPluginCommand(LIST_CONTENT_ASSETS_COMMAND, {});
        setAssets(JSON.parse(raw) as ContentAsset[]);
      } catch (e) {
        message.error(t("opc.common.loadFailed", { error: String(e) }));
        setAssets([]);
      } finally {
        setLoading(false);
      }
    })();
  };

  useEffect(() => {
    load();
  }, []);

  const handleCreate = async (values: Record<string, unknown>) => {
    try {
      // 出站 input 键 = 清单 `inputSchema` 的 camelCase（禁区 13）；表单内部字段名不变。
      // ⚠ 不传 `status`：接缝 `CreateContentAssetRequest` 没有该字段，新建恒为 draft
      // （发布走下面的 update / schedule，裁定见 PLAN §128.18）；`body` 是必填串 ⇒ 缺省补空串。
      await execPluginCommand(CREATE_CONTENT_ASSET_COMMAND, {
        title: values.title,
        contentType: values.content_type,
        body: values.body ?? "",
        tags: values.tags ? String(values.tags).split(",").map((s) => s.trim()).filter(Boolean) : [],
      });
      message.success(t("opc.contentMedia.assetCreated"));
      setCreateOpen(false);
      form.resetFields();
      load();
    } catch (e) {
      message.error(t("opc.common.createFailed", { error: String(e) }));
    }
  };

  const handleUpdate = async (values: Record<string, unknown>) => {
    if (!editingAsset) { return; }
    try {
      const payload: Record<string, unknown> = {
        id: editingAsset.id,
        title: values.title,
        contentType: values.content_type,
        status: values.status,
        tags: values.tags ? String(values.tags).split(",").map((s) => s.trim()).filter(Boolean) : [],
      };
      // 缺键 = 该列不动（接缝 `UpdateContentAssetInput` 五字段全 `Option`）。正文没回填，
      // 只有用户动过才发，否则「改标题」会顺手把整篇正文清空。
      if (bodyDirty) {
        payload.body = values.body ?? "";
      }
      await execPluginCommand(UPDATE_CONTENT_ASSET_COMMAND, payload);
      message.success(t("opc.contentMedia.assetUpdated"));
      setEditOpen(false);
      setEditingAsset(null);
      setBodyDirty(false);
      editSeqRef.current += 1;
      form.resetFields();
      load();
    } catch (e) {
      message.error(t("opc.common.updateFailed", { error: String(e) }));
    }
  };

  const handleDelete = async (id: string) => {
    try {
      await execPluginCommand(DELETE_CONTENT_ASSET_COMMAND, { id });
      message.success(t("opc.contentMedia.assetDeleted"));
      load();
    } catch (e) {
      message.error(t("opc.common.deleteFailed", { error: String(e) }));
    }
  };

  const openEdit = (asset: ContentAsset) => {
    const seq = ++editSeqRef.current;
    setEditingAsset(asset);
    setBodyDirty(false);
    form.setFieldsValue({
      title: asset.title,
      content_type: asset.contentType,
      tags: asset.tags.join(","),
      status: asset.status,
    });
    setEditOpen(true);
    // 正文不在列表摘要视图里（接缝裁剪，不过线）⇒ 编辑必须回读全文：插件命令
    // `opc_get_content_asset`（宿主同名 Tauri 命令本批仍在，另一批负责删除与登记表收口，
    // 前端统一走插件通道）返回完整 `ContentAsset`（含 body）。⚠ 域 DTO 无 `serde(rename_all)`
    // ⇒ 出参线格式是 snake_case 键（`content_type` / `created_at` / …）；本读点只取 `id` /
    // `body` 两个单词键，snake/camel 同形，消费端无需改名。
    // 取失败不阻塞编辑 —— 正文框保持空且 `bodyDirty=false`，用户不动就不覆盖原文；
    // 序号比对防止慢响应把值回填到已切行/已关闭的弹窗。
    void (async () => {
      try {
        const full = JSON.parse(
          await execPluginCommand(GET_CONTENT_ASSET_COMMAND, { id: asset.id }),
        ) as { id: string; body?: string | null };
        if (editSeqRef.current === seq) {
          form.setFieldsValue({ body: full.body ?? "" });
        }
      } catch {
        // 保持空正文 + bodyDirty 守卫。
      }
    })();
  };

  // 正文不过线（摘要视图裁剪）⇒ 关键词只匹配标题。
  const filteredAssets = searchText
    ? assets.filter((a) => a.title.toLowerCase().includes(searchText.toLowerCase()))
    : assets;

  const columns = [
    { title: t("opc.contentMedia.columnTitle"), dataIndex: "title", key: "title", width: 200 },
    {
      title: t("opc.contentMedia.columnType"),
      dataIndex: "contentType",
      key: "contentType",
      width: 100,
      render: (type: string) => t(`opc.contentMedia.type_${type}`),
    },
    {
      title: t("opc.contentMedia.columnStatus"),
      dataIndex: "status",
      key: "status",
      width: 90,
      render: (status: string) => (
        <Tag color={STATUS_COLORS[status] || "default"}>{t(`opc.contentMedia.status_${status}`)}</Tag>
      ),
    },
    {
      title: t("opc.contentMedia.columnTags"),
      key: "tags",
      width: 200,
      render: (_: unknown, r: ContentAsset) => r.tags.map((tag) => <Tag key={tag}>{tag}</Tag>),
    },
    {
      title: t("opc.contentMedia.columnCreated"),
      key: "createdAt",
      width: 120,
      render: (_: unknown, r: ContentAsset) => new Date(r.createdAt * 1000).toLocaleDateString(),
    },
    {
      title: t("opc.common.actions"),
      key: "actions",
      width: 120,
      render: (_: unknown, r: ContentAsset) => (
        <Space size="small">
          <Button size="small" icon={<EditOutlined />} onClick={() => openEdit(r)}>
            {t("opc.common.update")}
          </Button>
          <Button size="small" danger icon={<DeleteOutlined />} onClick={() => handleDelete(r.id)}>
            {t("opc.common.actions")}
          </Button>
        </Space>
      ),
    },
  ];

  return (
    <Card
      extra={
        <Button
          type="primary"
          size="small"
          icon={<PlusOutlined />}
          onClick={() => {
            form.resetFields();
            form.setFieldsValue({ status: "draft", content_type: "article" });
            setCreateOpen(true);
          }}
        >
          {t("opc.contentMedia.newAsset")}
        </Button>
      }
    >
      <Row gutter={12} style={{ marginBottom: 12 }} align="middle">
        <Col xs={24} sm={12} md={8}>
          <Input
            placeholder={t("opc.common.search")}
            prefix={<SearchOutlined />}
            allowClear
            value={searchText}
            onChange={(e) => setSearchText(e.target.value)}
          />
        </Col>
      </Row>
      <Table
        dataSource={filteredAssets}
        columns={columns}
        rowKey="id"
        loading={loading}
        size="small"
        pagination={{ pageSize: 20 }}
        scroll={{ x: 900 }}
      />
      <Modal
        title={t("opc.contentMedia.assetModalTitle")}
        open={createOpen}
        onOk={() => form.submit()}
        onCancel={() => {
          setCreateOpen(false);
          form.resetFields();
        }}
        okText={t("opc.common.create")}
        cancelText={t("opc.common.cancel")}
        width={560}
      >
        <Form form={form} layout="vertical" onFinish={handleCreate}>
          <Form.Item name="title" label={t("opc.contentMedia.columnTitle")} rules={[{ required: true }]}>
            <Input />
          </Form.Item>
          <Form.Item name="content_type" label={t("opc.contentMedia.columnType")} rules={[{ required: true }]}>
            <Select
              options={CONTENT_TYPES.map((c) => ({ value: c, label: t(`opc.contentMedia.type_${c}`) }))}
            />
          </Form.Item>
          <Form.Item name="body" label={t("opc.contentMedia.columnBody")}>
            <Input.TextArea rows={4} />
          </Form.Item>
          <Form.Item name="tags" label={t("opc.contentMedia.columnTags")}>
            <Input placeholder={t("opc.contentMedia.tagsPlaceholder")} />
          </Form.Item>
          <Form.Item name="status" label={t("opc.contentMedia.columnStatus")}>
            <Select
              options={[
                { value: "draft", label: t("opc.contentMedia.status_draft") },
                { value: "published", label: t("opc.contentMedia.status_published") },
              ]}
            />
          </Form.Item>
        </Form>
      </Modal>
      <Modal
        title={t("opc.contentMedia.assetEditTitle")}
        open={editOpen}
        onOk={() => form.submit()}
        onCancel={() => {
          setEditOpen(false);
          setEditingAsset(null);
          setBodyDirty(false);
          editSeqRef.current += 1;
          form.resetFields();
        }}
        okText={t("opc.common.update")}
        cancelText={t("opc.common.cancel")}
        width={560}
      >
        <Form form={form} layout="vertical" onFinish={handleUpdate}>
          <Form.Item name="title" label={t("opc.contentMedia.columnTitle")} rules={[{ required: true }]}>
            <Input />
          </Form.Item>
          <Form.Item name="content_type" label={t("opc.contentMedia.columnType")} rules={[{ required: true }]}>
            <Select
              options={CONTENT_TYPES.map((c) => ({ value: c, label: t(`opc.contentMedia.type_${c}`) }))}
            />
          </Form.Item>
          <Form.Item name="body" label={t("opc.contentMedia.columnBody")}>
            {/* 正文不在列表视图里（不过线）⇒ 弹窗为空是正常态；用户动过才回传（bodyDirty）。 */}
            <Input.TextArea rows={4} onChange={() => setBodyDirty(true)} />
          </Form.Item>
          <Form.Item name="tags" label={t("opc.contentMedia.columnTags")}>
            <Input placeholder={t("opc.contentMedia.tagsPlaceholder")} />
          </Form.Item>
          <Form.Item name="status" label={t("opc.contentMedia.columnStatus")}>
            <Select
              options={[
                { value: "draft", label: t("opc.contentMedia.status_draft") },
                { value: "published", label: t("opc.contentMedia.status_published") },
              ]}
            />
          </Form.Item>
        </Form>
      </Modal>
    </Card>
  );
}

function PublishSchedulesPanel() {
  const { t } = useTranslation();
  const [schedules, setSchedules] = useState<PublishSchedule[]>([]);
  const [loading, setLoading] = useState(true);
  const [createOpen, setCreateOpen] = useState(false);
  const [assets, setAssets] = useState<ContentAsset[]>([]);
  const [form] = Form.useForm();

  const load = () => {
    setLoading(true);
    (async () => {
      try {
        const raw = await execPluginCommand(LIST_PUBLISH_SCHEDULES_COMMAND, {});
        setSchedules(JSON.parse(raw) as PublishSchedule[]);
      } catch (e) {
        message.error(t("opc.common.loadFailed", { error: String(e) }));
        setSchedules([]);
      } finally {
        setLoading(false);
      }
    })();
  };

  const loadAssets = async () => {
    try {
      const raw = await execPluginCommand(LIST_CONTENT_ASSETS_COMMAND, {});
      setAssets(JSON.parse(raw) as ContentAsset[]);
    } catch (e) {
      message.error(t("opc.common.loadFailed", { error: String(e) }));
      setAssets([]);
    }
  };

  useEffect(() => {
    load();
    loadAssets();
  }, []);

  const handleCreate = async (values: Record<string, unknown>) => {
    try {
      // `contentRefId` 是**被发布内容的主键**（内容资产 id），不是发布计划自己的 id。
      await execPluginCommand(CREATE_PUBLISH_SCHEDULE_COMMAND, {
        contentRefType: "content_asset",
        contentRefId: values.content_ref_id,
        scheduledAt: Math.floor((values.scheduled_at as Date).getTime() / 1000),
      });
      message.success(t("opc.contentMedia.scheduleCreated"));
      setCreateOpen(false);
      form.resetFields();
      load();
    } catch (e) {
      message.error(t("opc.common.createFailed", { error: String(e) }));
    }
  };

  // 宿主 `opc_delete_publish_schedule` 的插件侧等价是 `opc_cancel_publish_schedule`（硬删）。
  const handleCancel = async (id: string) => {
    try {
      await execPluginCommand(CANCEL_PUBLISH_SCHEDULE_COMMAND, { id });
      message.success(t("opc.contentMedia.scheduleCancelled"));
      load();
    } catch (e) {
      message.error(t("opc.common.opFailed", { error: String(e) }));
    }
  };

  const handleProcessDue = async () => {
    try {
      await execPluginCommand(PROCESS_DUE_SCHEDULES_COMMAND, {});
      message.success(t("opc.contentMedia.processDueSuccess"));
      load();
    } catch (e) {
      message.error(t("opc.common.opFailed", { error: String(e) }));
    }
  };

  const assetOptions = assets.map((a) => ({
    value: a.id,
    label: `${a.title} (${a.contentType})`,
  }));

  const columns = [
    {
      title: t("opc.contentMedia.columnContentRef"),
      dataIndex: "contentRefId",
      key: "contentRefId",
      render: (id: string) => {
        const asset = assets.find((a) => a.id === id);
        return asset ? asset.title : id;
      },
    },
    {
      title: t("opc.contentMedia.columnRefType"),
      dataIndex: "contentRefType",
      key: "contentRefType",
      width: 120,
    },
    {
      title: t("opc.contentMedia.columnScheduledAt"),
      dataIndex: "scheduledAt",
      key: "scheduledAt",
      width: 160,
      render: (ts: number) => new Date(ts * 1000).toLocaleString(),
    },
    {
      title: t("opc.contentMedia.columnStatus"),
      dataIndex: "status",
      key: "status",
      width: 100,
      render: (status: string) => (
        <Tag color={STATUS_COLORS[status] || "default"}>{t(`opc.contentMedia.schedule_${status}`)}</Tag>
      ),
    },
    {
      title: t("opc.common.actions"),
      key: "actions",
      width: 100,
      render: (_: unknown, r: PublishSchedule) =>
        r.status === "pending" && (
          <Button size="small" danger onClick={() => handleCancel(r.id)}>
            {t("opc.contentMedia.cancelSchedule")}
          </Button>
        ),
    },
  ];

  return (
    <Card
      extra={
        <Space>
          <Button icon={<ScheduleOutlined />} onClick={handleProcessDue}>
            {t("opc.contentMedia.processDue")}
          </Button>
          <Button
            type="primary"
            size="small"
            icon={<PlusOutlined />}
            onClick={() => {
              form.resetFields();
              setCreateOpen(true);
            }}
          >
            {t("opc.contentMedia.newSchedule")}
          </Button>
        </Space>
      }
    >
      <Table
        dataSource={schedules}
        columns={columns}
        rowKey="id"
        loading={loading}
        size="small"
        pagination={{ pageSize: 20 }}
      />
      <Modal
        title={t("opc.contentMedia.scheduleModalTitle")}
        open={createOpen}
        onOk={() => form.submit()}
        onCancel={() => {
          setCreateOpen(false);
          form.resetFields();
        }}
        okText={t("opc.common.create")}
        cancelText={t("opc.common.cancel")}
      >
        <Form form={form} layout="vertical" onFinish={handleCreate}>
          <Form.Item
            name="content_ref_id"
            label={t("opc.contentMedia.selectContent")}
            rules={[{ required: true }]}
          >
            <Select
              showSearch
              placeholder={t("opc.contentMedia.selectContentPlaceholder")}
              options={assetOptions}
              filterOption={(input, option) => String(option?.label ?? "").toLowerCase().includes(input.toLowerCase())}
            />
          </Form.Item>
          <Form.Item
            name="scheduled_at"
            label={t("opc.contentMedia.scheduledAt")}
            rules={[{ required: true }]}
          >
            <DatePicker showTime style={{ width: "100%" }} />
          </Form.Item>
        </Form>
      </Modal>
    </Card>
  );
}

function ContentCreationWizard() {
  const { t } = useTranslation();
  const [current, setCurrent] = useState(0);
  const [wizardData, setWizardData] = useState({
    topic: "",
    angle: "",
    title: "",
    body: "",
    tags: "",
    meta_description: "",
    publish_type: "immediate" as "immediate" | "scheduled",
    scheduled_at: null as number | null,
    asset_id: "",
  });
  const [form] = Form.useForm();
  const [loading, setLoading] = useState(false);

  const steps = [
    { title: t("opc.contentMedia.wizard.step1Title"), desc: t("opc.contentMedia.wizard.step1Desc") },
    { title: t("opc.contentMedia.wizard.step2Title"), desc: t("opc.contentMedia.wizard.step2Desc") },
    { title: t("opc.contentMedia.wizard.step3Title"), desc: t("opc.contentMedia.wizard.step3Desc") },
    { title: t("opc.contentMedia.wizard.step4Title"), desc: t("opc.contentMedia.wizard.step4Desc") },
  ];

  const next = () => {
    form
      .validateFields()
      .then((values) => {
        setWizardData((prev) => ({ ...prev, ...values }));
        setCurrent((c) => c + 1);
      })
      .catch(() => {});
  };

  const prev = () => setCurrent((c) => c - 1);

  const handleFinish = async () => {
    setLoading(true);
    try {
      // #180-④：改前用 `wizardData.asset_id || crypto.randomUUID()` 当后端 id —— 但本分支**总是新建**，
      // 后端会自己 `gen_id()` 生成真 id ⇒ 排期落一个悬空 `content_ref_id`、直发 `opc_update_content_asset`
      // 打到不存在的行（回 NotFound）。改为直接取 create 的返回值。
      const created = JSON.parse(
        await execPluginCommand(CREATE_CONTENT_ASSET_COMMAND, {
          title: wizardData.title,
          contentType: "article",
          body: wizardData.body,
          tags: wizardData.tags ? wizardData.tags.split(",").map((s) => s.trim()).filter(Boolean) : [],
          // ⚠ 不传 status：接缝 `CreateContentAssetRequest` 没有该字段（清单 `inputSchema` 同样不收），
          // 新建恒为 draft；发布/排期走下面的 update / schedule（裁定见 PLAN §128.18）。
        }),
      ) as ContentAsset;
      const assetId = created.id;

      if (wizardData.publish_type === "scheduled" && wizardData.scheduled_at) {
        await execPluginCommand(CREATE_PUBLISH_SCHEDULE_COMMAND, {
          contentRefType: "content_asset",
          contentRefId: assetId,
          scheduledAt: wizardData.scheduled_at,
        });
        message.success(t("opc.contentMedia.wizard.scheduleCreated"));
      } else {
        await execPluginCommand(UPDATE_CONTENT_ASSET_COMMAND, {
          id: assetId,
          status: "published",
        });
        message.success(t("opc.contentMedia.wizard.publishedNow"));
      }

      message.success(t("opc.contentMedia.wizard.complete"));
      setCurrent(0);
      setWizardData({
        topic: "",
        angle: "",
        title: "",
        body: "",
        tags: "",
        meta_description: "",
        publish_type: "immediate",
        scheduled_at: null,
        asset_id: "",
      });
      form.resetFields();
    } catch (e) {
      message.error(t("opc.common.opFailed", { error: String(e) }));
    } finally {
      setLoading(false);
    }
  };

  return (
    <Card>
      <Steps current={current} items={steps} style={{ marginBottom: 32 }} />
      <div style={{ minHeight: 240 }}>
        {current === 0 && (
          <Form
            form={form}
            layout="vertical"
            initialValues={{ topic: wizardData.topic, angle: wizardData.angle }}
          >
            <Form.Item
              name="topic"
              label={t("opc.contentMedia.wizard.topicLabel")}
              rules={[{ required: true, message: t("opc.contentMedia.wizard.topicRequired") }]}
            >
              <Input placeholder={t("opc.contentMedia.wizard.topicPlaceholder")} />
            </Form.Item>
            <Form.Item name="angle" label={t("opc.contentMedia.wizard.angleLabel")}>
              <Input.TextArea rows={2} placeholder={t("opc.contentMedia.wizard.anglePlaceholder")} />
            </Form.Item>
          </Form>
        )}
        {current === 1 && (
          <Form
            form={form}
            layout="vertical"
            initialValues={{
              title: wizardData.title,
              body: wizardData.body,
              tags: wizardData.tags,
            }}
          >
            <Form.Item
              name="title"
              label={t("opc.contentMedia.wizard.titleLabel")}
              rules={[{ required: true, message: t("opc.contentMedia.wizard.titleRequired") }]}
            >
              <Input />
            </Form.Item>
            <Form.Item
              name="body"
              label={t("opc.contentMedia.wizard.bodyLabel")}
              rules={[{ required: true, message: t("opc.contentMedia.wizard.bodyRequired") }]}
            >
              <Input.TextArea rows={8} placeholder={t("opc.contentMedia.wizard.bodyPlaceholder")} />
            </Form.Item>
            <Form.Item name="tags" label={t("opc.contentMedia.columnTags")}>
              <Input placeholder={t("opc.contentMedia.tagsPlaceholder")} />
            </Form.Item>
          </Form>
        )}
        {current === 2 && (
          <Form
            form={form}
            layout="vertical"
            initialValues={{ meta_description: wizardData.meta_description }}
          >
            <Form.Item name="meta_description" label={t("opc.contentMedia.wizard.seoLabel")}>
              <Input.TextArea rows={3} placeholder={t("opc.contentMedia.wizard.seoPlaceholder")} />
            </Form.Item>
          </Form>
        )}
        {current === 3 && (
          <Form
            form={form}
            layout="vertical"
            initialValues={{
              publish_type: wizardData.publish_type,
              scheduled_at: wizardData.scheduled_at,
            }}
          >
            <Form.Item
              name="publish_type"
              label={t("opc.contentMedia.wizard.publishType")}
              rules={[{ required: true }]}
            >
              <Select
                options={[
                  { value: "immediate", label: t("opc.contentMedia.wizard.publishImmediate") },
                  { value: "scheduled", label: t("opc.contentMedia.wizard.publishScheduled") },
                ]}
              />
            </Form.Item>
            <Form.Item
              noStyle
              shouldUpdate={(prev, cur) => prev.publish_type !== cur.publish_type}
            >
              {({ getFieldValue }) =>
                getFieldValue("publish_type") === "scheduled"
                  ? (
                    <Form.Item
                      name="scheduled_at"
                      label={t("opc.contentMedia.wizard.scheduleTime")}
                      rules={[{ required: true, message: t("opc.contentMedia.wizard.scheduleRequired") }]}
                    >
                      <DatePicker showTime style={{ width: "100%" }} />
                    </Form.Item>
                  )
                  : null}
            </Form.Item>
          </Form>
        )}
      </div>
      <div style={{ textAlign: "right", marginTop: 24 }}>
        <Space>
          {current > 0 && <Button onClick={prev}>{t("opc.common.cancel")}</Button>}
          {current < steps.length - 1 && (
            <Button type="primary" onClick={next}>
              {t("opc.common.create")}
            </Button>
          )}
          {current === steps.length - 1 && (
            <Button type="primary" loading={loading} onClick={handleFinish}>
              {t("opc.contentMedia.wizard.finish")}
            </Button>
          )}
        </Space>
      </div>
    </Card>
  );
}
