// SPDX-License-Identifier: AGPL-3.0-only

import { invoke } from "@/lib/invoke";
import { resolvePluginForCommand } from "@/lib/pluginCommandResolve";
import { DeleteOutlined, EditOutlined, PlusOutlined } from "@ant-design/icons";
import {
  Button,
  Card,
  Col,
  Form,
  Input,
  InputNumber,
  message,
  Modal,
  Popconfirm,
  Row,
  Segmented,
  Select,
  Space,
  Table,
  Tag,
} from "antd";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import {
  CUST_STATUS_COLOR_MAP,
  CUST_TYPE_COLOR_MAP,
  getCustomerStatusKey,
  getCustomerTypeKey,
  getSourceKey,
} from "../utils/constants";
import { CustomerMap } from "./CustomerMap";

// 本面板的取数与写面四条命令（#32 批 A 补齐 update/delete 后四条全是插件命令）。
// 出参线格式 = 接缝视图 `CustomerSummaryView` 的 camelCase 键（禁区 13，插件出口不改写）。
const LIST_CUSTOMERS_COMMAND = "opc_list_customers";
const CREATE_CUSTOMER_COMMAND = "opc_create_customer";
const UPDATE_CUSTOMER_COMMAND = "opc_update_customer";
const DELETE_CUSTOMER_COMMAND = "opc_delete_customer";

// 写命令的插件通道执行（先例 SitesTab）：按清单解析启用插件 ⇒ 宿主
// `plugin_execute_command` 派发。插件未装 / 未启用抛错，与取数失败同一路径处置。
async function execPluginCommand(name: string, input: Record<string, unknown>): Promise<string> {
  const pluginId = await resolvePluginForCommand(name);
  if (!pluginId) {
    throw new Error(`no enabled plugin provides command \`${name}\``);
  }
  return invoke<string>("plugin_execute_command", { pluginId, name, input });
}

// 插件命令 `opc_list_customers` 的行类型 = 接缝视图 `CustomerSummaryView` 的本面板已读子集
// （camelCase，禁区 13）。`createdAt` 是 epoch 秒。dev mock 下 `plugin_execute_command` 返回
// 空串 ⇒ 取数失败提示（mock 不模拟插件子进程，同 SitesTab 博客面板的既有形态）。
interface CustomerRow {
  id: string;
  name: string;
  email: string;
  phone: string | null;
  company: string | null;
  customerType: string;
  country: string | null;
  region: string | null;
  city: string | null;
  address: string | null;
  latitude: number | null;
  longitude: number | null;
  source: string | null;
  notes: string;
  status: string;
  totalRevenue: number;
  invoiceCount: number;
}

export function CustomersTab() {
  const { t } = useTranslation();
  const [customers, setCustomers] = useState<CustomerRow[]>([]);
  const [loading, setLoading] = useState(true);
  const [modalOpen, setModalOpen] = useState(false);
  const [editing, setEditing] = useState<CustomerRow | null>(null);
  const [filterStatus, setFilterStatus] = useState<string>("");
  const [filterSource, setFilterSource] = useState<string>("");
  const [filterType, setFilterType] = useState<string>("");
  const [viewMode, setViewMode] = useState<"list" | "map">("list");
  const [selectedRowKeys, setSelectedRowKeys] = useState<string[]>([]);
  const [form] = Form.useForm();

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const raw = await execPluginCommand(LIST_CUSTOMERS_COMMAND, {});
      setCustomers(JSON.parse(raw) as CustomerRow[]);
    } catch (e) {
      message.error(t("opc.common.loadFailed", { error: String(e) }));
      setCustomers([]);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    load();
  }, [load]);

  const handleSave = async (values: Record<string, unknown>) => {
    try {
      // 出站 input 键 = 清单 `inputSchema` 的 camelCase（禁区 13）；`line_items` 等表单内部
      // 字段名不适用本表单 —— 这里直接平铺。可选字段的 undefined（用户没填）经
      // JSON.stringify 丢键 = 接缝 DTO 外层 `None` = 该列不动；antd Input 清空后是 ""
      // （转发 = 写空串，清空生效 —— 旧宿主链路 `|| null` 会让清空变成「不动」，行为修正）；
      // 经纬度 InputNumber 清空后是 undefined ⇒ 不动（接缝线格式上「清空」不可达，与
      // 宿主同名旧命令一致）。`source` / `tags` 无更新位（域包能力面外），不转发。
      const payload: Record<string, unknown> = {
        name: values.name as string,
        email: values.email as string,
        customerType: values.customer_type as string,
        phone: values.phone as string | undefined,
        company: values.company as string | undefined,
        country: values.country as string | undefined,
        region: values.region as string | undefined,
        city: values.city as string | undefined,
        address: values.address as string | undefined,
        latitude: values.latitude as number | undefined,
        longitude: values.longitude as number | undefined,
        notes: values.notes as string | undefined,
      };
      if (editing) {
        await execPluginCommand(UPDATE_CUSTOMER_COMMAND, { id: editing.id, ...payload });
        message.success(t("opc.customer.updated"));
      } else {
        await execPluginCommand(CREATE_CUSTOMER_COMMAND, payload);
        message.success(t("opc.customer.created"));
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
      await execPluginCommand(DELETE_CUSTOMER_COMMAND, { id });
      message.success(t("opc.customer.deleted"));
      load();
    } catch (e) {
      message.error(t("opc.common.deleteFailed", { error: String(e) }));
    }
  };

  const filteredCustomers = customers.filter((c) => {
    if (filterStatus && c.status !== filterStatus) { return false; }
    if (filterSource && c.source !== filterSource) { return false; }
    if (filterType && c.customerType !== filterType) { return false; }
    return true;
  });

  const handleBatchDelete = async () => {
    try {
      await Promise.all(
        selectedRowKeys.map((id) => execPluginCommand(DELETE_CUSTOMER_COMMAND, { id })),
      );
      message.success(t("opc.common.batchDeleteSuccess"));
      setSelectedRowKeys([]);
      load();
    } catch (e) {
      message.error(t("opc.common.deleteFailed", { error: String(e) }));
    }
  };

  const columns = [
    { title: t("opc.customer.columnName"), dataIndex: "name", key: "name" },
    { title: t("opc.customer.columnEmail"), dataIndex: "email", key: "email" },
    {
      title: t("opc.customer.columnCompany"),
      dataIndex: "company",
      key: "company",
      render: (v: string | null) => v || "-",
    },
    {
      title: t("opc.customer.columnType"),
      key: "customerType",
      render: (_: unknown, r: CustomerRow) => {
        const color = CUST_TYPE_COLOR_MAP[r.customerType] || "default";
        return <Tag color={color}>{t(getCustomerTypeKey(r.customerType))}</Tag>;
      },
    },
    {
      title: t("opc.customer.columnLocation"),
      key: "location",
      render: (_: unknown, r: CustomerRow) => {
        const parts = [r.country, r.region, r.city].filter(Boolean);
        return parts.length ? parts.join(" / ") : "-";
      },
    },
    {
      title: t("opc.customer.columnStatus"),
      key: "status",
      render: (_: unknown, r: CustomerRow) => {
        const color = CUST_STATUS_COLOR_MAP[r.status] || "default";
        return <Tag color={color}>{t(getCustomerStatusKey(r.status))}</Tag>;
      },
    },
    {
      title: t("opc.customer.columnSource"),
      key: "source",
      render: (_: unknown, r: CustomerRow) => (r.source ? t(getSourceKey(r.source)) : "-"),
    },
    {
      title: t("opc.customer.columnRevenue"),
      key: "revenue",
      render: (_: unknown, r: CustomerRow) => `¥${r.totalRevenue.toFixed(2)}`,
      sorter: (a: CustomerRow, b: CustomerRow) => a.totalRevenue - b.totalRevenue,
    },
    { title: t("opc.customer.columnInvoiceCount"), dataIndex: "invoiceCount", key: "count", width: 80 },
    {
      title: t("opc.common.actions"),
      key: "actions",
      width: 120,
      render: (_: unknown, r: CustomerRow) => (
        <Space size="small">
          <Button
            size="small"
            icon={<EditOutlined />}
            onClick={() => {
              setEditing(r);
              // 表单内部字段名保留 snake_case（Modal 表单 name 与 :401 既有声明一致），
              // 值读取从行数据取 camelCase 接缝键。
              form.setFieldsValue({
                name: r.name,
                email: r.email,
                phone: r.phone,
                company: r.company,
                customer_type: r.customerType,
                country: r.country,
                region: r.region,
                city: r.city,
                address: r.address,
                latitude: r.latitude,
                longitude: r.longitude,
                source: r.source,
                notes: r.notes,
              });
              setModalOpen(true);
            }}
          />
          <Popconfirm title={t("opc.customer.confirmDelete")} onConfirm={() => handleDelete(r.id)}>
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
              placeholder={t("opc.customer.filterStatus")}
              allowClear
              style={{ width: "100%" }}
              value={filterStatus || undefined}
              onChange={(val) => setFilterStatus(val || "")}
              options={[
                { value: "lead", label: t("opc.customerStatus.lead") },
                { value: "prospect", label: t("opc.customerStatus.prospect") },
                { value: "active", label: t("opc.customerStatus.active") },
                { value: "inactive", label: t("opc.customerStatus.inactive") },
                { value: "churned", label: t("opc.customerStatus.churned") },
              ]}
            />
          </Col>
          <Col xs={24} sm={8} md={6}>
            <Select
              placeholder={t("opc.customer.filterSource")}
              allowClear
              style={{ width: "100%" }}
              value={filterSource || undefined}
              onChange={(val) => setFilterSource(val || "")}
              options={[
                { value: "referral", label: t("opc.source.referral") },
                { value: "website", label: t("opc.source.website") },
                { value: "social_media", label: t("opc.source.social_media") },
                { value: "marketplace", label: t("opc.source.marketplace") },
                { value: "direct", label: t("opc.source.direct") },
              ]}
            />
          </Col>
          <Col xs={24} sm={8} md={6}>
            <Select
              placeholder={t("opc.customer.filterType")}
              allowClear
              style={{ width: "100%" }}
              value={filterType || undefined}
              onChange={(val) => setFilterType(val || "")}
              options={[
                { value: "consumer", label: t("opc.customerType.consumer") },
                { value: "business", label: t("opc.customerType.business") },
                { value: "unknown", label: t("opc.customerType.unknown") },
              ]}
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
                  setFilterSource("");
                  setFilterType("");
                  setSelectedRowKeys([]);
                }}
              >
                {t("opc.common.reset")}
              </Button>
            </Space>
          </Col>
          <Col style={{ marginLeft: "auto" }}>
            <Segmented
              value={viewMode}
              onChange={(v) => setViewMode(v as "list" | "map")}
              options={[
                { value: "list", label: t("opc.customerMap.list") },
                { value: "map", label: t("opc.customerMap.map") },
              ]}
            />
          </Col>
        </Row>
      </Card>

      {viewMode === "map"
        ? (
          <Card size="small" title={t("opc.customerMap.title")} style={{ marginBottom: 12 }}>
            <CustomerMap customers={filteredCustomers} />
          </Card>
        )
        : (
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
                {t("opc.customer.newCustomer")}
              </Button>
            }
          >
            <Table
              dataSource={filteredCustomers}
              columns={columns}
              rowKey="id"
              loading={loading}
              size="small"
              pagination={{ pageSize: 20 }}
              rowSelection={{
                selectedRowKeys,
                onChange: (keys) => setSelectedRowKeys(keys.map(String)),
              }}
              locale={{
                emptyText: loading ? t("opc.common.loading") : t("opc.customer.emptyTip"),
              }}
            />
          </Card>
        )}

      <Modal
        title={editing ? t("opc.customer.editTitle") : t("opc.customer.newCustomer")}
        open={modalOpen}
        onOk={() => form.submit()}
        onCancel={() => {
          setModalOpen(false);
          setEditing(null);
          form.resetFields();
        }}
        okText={editing ? t("opc.common.update") : t("opc.common.create")}
        cancelText={t("opc.common.cancel")}
      >
        <Form form={form} layout="vertical" onFinish={handleSave}>
          <Form.Item
            name="name"
            label={t("opc.customer.nameLabel")}
            rules={[{ required: true, message: t("opc.customer.nameRequired") }]}
          >
            <Input />
          </Form.Item>
          <Form.Item
            name="customer_type"
            label={t("opc.customer.typeLabel")}
            rules={[{ required: true, message: t("opc.customer.typeRequired") }]}
          >
            <Select
              placeholder={t("opc.customer.typePlaceholder")}
              options={[
                { value: "consumer", label: t("opc.customerType.consumer") },
                { value: "business", label: t("opc.customerType.business") },
              ]}
            />
          </Form.Item>
          <Form.Item
            name="email"
            label={t("opc.customer.emailLabel")}
            rules={[{ required: true, type: "email", message: t("opc.customer.emailRequired") }]}
          >
            <Input />
          </Form.Item>
          <Form.Item name="phone" label={t("opc.customer.phoneLabel")}>
            <Input />
          </Form.Item>
          <Form.Item name="company" label={t("opc.customer.companyLabel")}>
            <Input />
          </Form.Item>
          <Form.Item name="country" label={t("opc.customer.countryLabel")}>
            <Input placeholder={t("opc.customer.countryPlaceholder")} />
          </Form.Item>
          <Form.Item name="region" label={t("opc.customer.regionLabel")}>
            <Input placeholder={t("opc.customer.regionPlaceholder")} />
          </Form.Item>
          <Form.Item name="city" label={t("opc.customer.cityLabel")}>
            <Input placeholder={t("opc.customer.cityPlaceholder")} />
          </Form.Item>
          <Form.Item name="address" label={t("opc.customer.addressLabel")}>
            <Input placeholder={t("opc.customer.addressPlaceholder")} />
          </Form.Item>
          <Row gutter={8}>
            <Col span={12}>
              <Form.Item name="latitude" label={t("opc.customer.latitudeLabel")}>
                <InputNumber style={{ width: "100%" }} placeholder={t("opc.customer.latitudePlaceholder")} />
              </Form.Item>
            </Col>
            <Col span={12}>
              <Form.Item name="longitude" label={t("opc.customer.longitudeLabel")}>
                <InputNumber style={{ width: "100%" }} placeholder={t("opc.customer.longitudePlaceholder")} />
              </Form.Item>
            </Col>
          </Row>
          <Form.Item name="source" label={t("opc.customer.sourceLabel")}>
            <Select
              allowClear
              placeholder={t("opc.customer.sourcePlaceholder")}
              options={[
                { value: "referral", label: t("opc.source.referral") },
                { value: "website", label: t("opc.source.website") },
                { value: "social_media", label: t("opc.source.social_media") },
                { value: "marketplace", label: t("opc.source.marketplace") },
                { value: "direct", label: t("opc.source.direct") },
              ]}
            />
          </Form.Item>
          <Form.Item name="notes" label={t("opc.common.notes")}>
            <Input.TextArea rows={3} />
          </Form.Item>
        </Form>
      </Modal>
    </>
  );
}
