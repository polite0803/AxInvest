// SPDX-License-Identifier: AGPL-3.0-only

import { invoke } from "@/lib/invoke";
import { resolvePluginForCommand } from "@/lib/pluginCommandResolve";
import type { ContactSummaryView, LandingPageSummaryView } from "@/types/opc";
import { PlusOutlined, SearchOutlined } from "@ant-design/icons";
import { Button, Card, Col, Form, Input, message, Modal, Row, Space, Table, Tabs, Tag } from "antd";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

/**
 * 博客列表的取数命令（`plugin.opc` 的命名命令，与 manifest `commands[].name` 逐字一致）。
 *
 * 原先这条是宿主 Tauri 命令 `opc_list_blog_posts`，#36 第 4 步迁入 OPC 插件 ⇒ 宿主侧已删。
 * 出参线格式 = 接缝视图 `BlogPostView` 的 camelCase 键，插件出口不再做键名改写（禁区 13）。
 */
const LIST_BLOG_POSTS_COMMAND = "opc_list_blog_posts";
// 博客写面三条（#36 blog 刀）：宿主同名 Tauri 命令已删，插件命令是唯一通道。
// 入参全部平铺（插件清单 `inputSchema` 口径），出参的枚举串语义见各 handle。
const PUBLISH_BLOG_POST_COMMAND = "opc_publish_blog_post";
const DELETE_BLOG_POST_COMMAND = "opc_delete_blog_post";
const CREATE_BLOG_POST_COMMAND = "opc_create_blog_post";

// 落地页 + 联系人六条（#36 第 D-4 刀）：宿主同名 Tauri 命令迁入 OPC 插件，插件命令
// 是唯一通道。入参平铺（与博客命令同口径），出参 = `@/types/opc` 的接缝视图
// `LandingPageSummaryView` / `ContactSummaryView`（camelCase，禁区 13）。
const LIST_LANDING_PAGES_COMMAND = "opc_list_landing_pages";
const CREATE_LANDING_PAGE_COMMAND = "opc_create_landing_page";
const PUBLISH_LANDING_PAGE_COMMAND = "opc_publish_landing_page";
const DELETE_LANDING_PAGE_COMMAND = "opc_delete_landing_page";
const LIST_CONTACTS_COMMAND = "opc_list_contacts";
const MARK_CONTACT_READ_COMMAND = "opc_mark_contact_read";

// 写命令的插件通道执行（与上面 load() 的取数同形）：按清单解析启用插件 ⇒ 宿主
// `plugin_execute_command` 派发。插件未装 / 未启用抛错，与取数失败同一路径处置，
// 不新造错误码（新增 UI 文案要齐 11 语言）。
async function execPluginCommand(name: string, input: Record<string, unknown>): Promise<string> {
  const pluginId = await resolvePluginForCommand(name);
  if (!pluginId) {
    throw new Error(`no enabled plugin provides command \`${name}\``);
  }
  return invoke<string>("plugin_execute_command", { pluginId, name, input });
}

// 插件命令 `opc_list_blog_posts` 的行类型 = 接缝视图 `BlogPostView` 的本面板已读子集
// （帧里其实有 10 个 camelCase 键，含 `publishedAt` / `updatedAt`，这里只声明消费到的那些）。
// 时间口径与 `@/types/opc` 的 `LandingPageSummaryView.createdAt` / `ContactSummaryView.createdAt`
// **是同一个单位**：epoch 秒（这几列由宿主 `now_ts()` 写入，`now_ms()` 才是毫秒）⇒ 渲染要 `* 1000`。
interface BlogPost {
  id: string;
  title: string;
  slug: string;
  excerpt: string;
  tags: string[];
  published: boolean;
  viewCount: number;
  createdAt: number;
}

export function SitesTab() {
  const { t } = useTranslation();
  const [subTab, setSubTab] = useState("landing");
  return (
    <Tabs
      activeKey={subTab}
      onChange={setSubTab}
      size="small"
      items={[
        { key: "landing", label: t("opc.site.tabLanding"), children: <LandingPagesPanel /> },
        { key: "blog", label: t("opc.site.tabBlog"), children: <BlogPostsPanel /> },
        { key: "contacts", label: t("opc.site.tabContacts"), children: <ContactsPanel /> },
      ]}
    />
  );
}

function LandingPagesPanel() {
  const { t } = useTranslation();
  const [pages, setPages] = useState<LandingPageSummaryView[]>([]);
  const [loading, setLoading] = useState(true);
  const [modalOpen, setModalOpen] = useState(false);
  const [searchText, setSearchText] = useState("");
  const [selectedRowKeys, setSelectedRowKeys] = useState<string[]>([]);
  const [form] = Form.useForm();

  const load = () => {
    setLoading(true);
    (async () => {
      try {
        // 取数走插件执行通道（#36 第 D-4 刀）：宿主同名命令已删 ⇒ 没有兜底第二条路。
        // 插件未装 / 未启用按「取数失败」处置，与 catch 里既有那条 `loadFailed` 同一路径。
        const raw = await execPluginCommand(LIST_LANDING_PAGES_COMMAND, {});
        setPages(JSON.parse(raw) as LandingPageSummaryView[]);
      } catch (e) {
        message.error(t("opc.common.loadFailed", { error: String(e) }));
        setPages([]);
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
      // 入参平铺（插件清单 inputSchema 口径，不再套宿主命令时代的 `{ input }` 外层）；
      // 表单字段原样透传，content 由面板固定传空串（正文不在本面板编辑）。
      // 出参是创建行的列裁剪视图（含 id 句柄），本处不消费，刷新列表即见。
      await execPluginCommand(CREATE_LANDING_PAGE_COMMAND, {
        title: values.title,
        slug: values.slug,
        description: (values.description as string) || "",
        content: "",
      });
      message.success(t("opc.site.landingCreated"));
      setModalOpen(false);
      form.resetFields();
      load();
    } catch (e) {
      message.error(t("opc.common.createFailed", { error: String(e) }));
    }
  };

  const handlePublish = async (id: string) => {
    try {
      // 出参是发布后的 `LandingPageSummaryView`，本处不消费（状态列随 load() 刷新即见）。
      await execPluginCommand(PUBLISH_LANDING_PAGE_COMMAND, { id });
      message.success(t("opc.site.published"));
      load();
    } catch (e) {
      message.error(t("opc.site.publishFailed", { error: String(e) }));
    }
  };

  const filteredPages = searchText
    ? pages.filter((p) =>
      p.title.toLowerCase().includes(searchText.toLowerCase())
      || p.slug.toLowerCase().includes(searchText.toLowerCase())
    )
    : pages;

  const handleBatchDelete = async () => {
    try {
      // 出参 null（行已删）⇒ 本处不消费，逐条 await 成功即计成功。
      await Promise.all(
        selectedRowKeys.map((id) => execPluginCommand(DELETE_LANDING_PAGE_COMMAND, { id })),
      );
      message.success(t("opc.common.batchDeleteSuccess"));
      setSelectedRowKeys([]);
      load();
    } catch (e) {
      message.error(t("opc.common.deleteFailed", { error: String(e) }));
    }
  };

  const columns = [
    { title: t("opc.site.columnTitle"), dataIndex: "title", key: "title" },
    { title: t("opc.site.columnSlug"), dataIndex: "slug", key: "slug" },
    {
      title: t("opc.site.columnStatus"),
      key: "status",
      render: (_: unknown, r: LandingPageSummaryView) =>
        r.published ? <Tag color="green">{t("opc.site.published")}</Tag> : <Tag>{t("opc.site.draftTag")}</Tag>,
    },
    {
      title: t("opc.site.columnCreated"),
      key: "created",
      render: (_: unknown, r: LandingPageSummaryView) => new Date(r.createdAt * 1000).toLocaleDateString(),
    },
    {
      title: t("opc.common.actions"),
      key: "actions",
      width: 100,
      render: (_: unknown, r: LandingPageSummaryView) =>
        !r.published && <Button size="small" onClick={() => handlePublish(r.id)}>{t("opc.site.publish")}</Button>,
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
            setModalOpen(true);
          }}
        >
          {t("opc.site.newLanding")}
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
        <Col>
          <Space>
            {selectedRowKeys.length > 0 && (
              <Button danger size="small" onClick={handleBatchDelete}>
                {t("opc.common.batchDelete", { count: selectedRowKeys.length })}
              </Button>
            )}
          </Space>
        </Col>
      </Row>
      <Table
        dataSource={filteredPages}
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
      <Modal
        title={t("opc.site.landingModalTitle")}
        open={modalOpen}
        onOk={() => form.submit()}
        onCancel={() => {
          setModalOpen(false);
          form.resetFields();
        }}
        okText={t("opc.common.create")}
        cancelText={t("opc.common.cancel")}
      >
        <Form form={form} layout="vertical" onFinish={handleCreate}>
          <Form.Item name="title" label={t("opc.site.titleLabel")} rules={[{ required: true }]}>
            <Input />
          </Form.Item>
          <Form.Item name="slug" label={t("opc.site.slugLabel")} rules={[{ required: true }]}>
            <Input placeholder={t("opc.site.landingSlugPlaceholder")} />
          </Form.Item>
          <Form.Item name="description" label={t("opc.common.description")}>
            <Input.TextArea rows={2} />
          </Form.Item>
        </Form>
      </Modal>
    </Card>
  );
}

function BlogPostsPanel() {
  const { t } = useTranslation();
  const [posts, setPosts] = useState<BlogPost[]>([]);
  const [loading, setLoading] = useState(true);
  const [modalOpen, setModalOpen] = useState(false);
  const [searchText, setSearchText] = useState("");
  const [selectedRowKeys, setSelectedRowKeys] = useState<string[]>([]);
  const [form] = Form.useForm();

  const load = () => {
    setLoading(true);
    (async () => {
      try {
        // 取数改走插件执行通道（#36 第 4 步）：宿主同名命令已删 ⇒ 没有兜底第二条路。
        // 插件未装 / 未启用按「取数失败」处置，与下面 catch 里既有那条 `loadFailed` 同一路径，
        // 不新造错误码（新增 UI 文案要齐 11 语言，本批不该引入）。
        const pluginId = await resolvePluginForCommand(LIST_BLOG_POSTS_COMMAND);
        if (!pluginId) {
          throw new Error(`no enabled plugin provides command \`${LIST_BLOG_POSTS_COMMAND}\``);
        }
        const raw = await invoke<string>("plugin_execute_command", {
          pluginId,
          name: LIST_BLOG_POSTS_COMMAND,
          input: {},
        });
        setPosts(JSON.parse(raw) as BlogPost[]);
      } catch (e) {
        message.error(t("opc.common.loadFailed", { error: String(e) }));
        setPosts([]);
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
      // 插件命令收平铺五字段（清单 inputSchema），不再套宿主命令时代的 `{ input }` 外层；
      // 出参是创建行的列裁剪视图（含 id 句柄），本处不消费，刷新列表即见。
      await execPluginCommand(CREATE_BLOG_POST_COMMAND, {
        title: values.title,
        slug: values.slug,
        excerpt: (values.excerpt as string) || "",
        content: "",
        tags: values.tags ? (values.tags as string).split(",").map((s) => s.trim()).filter(Boolean) : [],
      });
      message.success(t("opc.site.postCreated"));
      setModalOpen(false);
      form.resetFields();
      load();
    } catch (e) {
      message.error(t("opc.common.createFailed", { error: String(e) }));
    }
  };

  const handlePublish = async (id: string) => {
    try {
      // 两态枚举逐字透传（payload，不走错误码）：not_found 在旧宿主命令里是 Err，
      // 这里显式翻回失败提示，不让「行已不存在」伪装成发布成功。
      const status = JSON.parse(await execPluginCommand(PUBLISH_BLOG_POST_COMMAND, { id }));
      if (status !== "published") {
        throw new Error(String(status));
      }
      message.success(t("opc.site.published"));
      load();
    } catch (e) {
      message.error(t("opc.site.publishFailed", { error: String(e) }));
    }
  };

  const filteredPosts = searchText
    ? posts.filter((p) =>
      p.title.toLowerCase().includes(searchText.toLowerCase())
      || p.slug.toLowerCase().includes(searchText.toLowerCase())
    )
    : posts;

  const handleBatchDelete = async () => {
    try {
      // 三态出参（payload，不走错误码）：旧宿主命令对 not_found 回 Err；protected 是
      // 接缝层新增的契约（受保护 id 拒删）。任一非 removed 都显式翻成失败提示，
      // 不让「行已不存在 / 受保护」伪装成批量删除成功。
      const statuses = await Promise.all(
        selectedRowKeys.map(async (id) =>
          JSON.parse(await execPluginCommand(DELETE_BLOG_POST_COMMAND, { id })) as string
        ),
      );
      const blocked = statuses.filter((s) => s !== "removed");
      if (blocked.length > 0) {
        throw new Error(blocked.join(", "));
      }
      message.success(t("opc.common.batchDeleteSuccess"));
      setSelectedRowKeys([]);
      load();
    } catch (e) {
      message.error(t("opc.common.deleteFailed", { error: String(e) }));
    }
  };

  const columns = [
    { title: t("opc.site.columnTitle"), dataIndex: "title", key: "title" },
    { title: t("opc.site.columnSlug"), dataIndex: "slug", key: "slug" },
    {
      title: t("opc.site.columnStatus"),
      key: "status",
      render: (_: unknown, r: BlogPost) =>
        r.published ? <Tag color="green">{t("opc.site.published")}</Tag> : <Tag>{t("opc.site.draftTag")}</Tag>,
    },
    { title: t("opc.site.columnViews"), dataIndex: "viewCount", key: "views", width: 60 },
    {
      title: t("opc.site.columnTags"),
      key: "tags",
      render: (_: unknown, r: BlogPost) => r.tags.map((tag) => <Tag key={tag}>{tag}</Tag>),
    },
    {
      title: t("opc.common.actions"),
      key: "actions",
      width: 100,
      render: (_: unknown, r: BlogPost) =>
        !r.published && <Button size="small" onClick={() => handlePublish(r.id)}>{t("opc.site.publish")}</Button>,
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
            setModalOpen(true);
          }}
        >
          {t("opc.site.newPost")}
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
        <Col>
          <Space>
            {selectedRowKeys.length > 0 && (
              <Button danger size="small" onClick={handleBatchDelete}>
                {t("opc.common.batchDelete", { count: selectedRowKeys.length })}
              </Button>
            )}
          </Space>
        </Col>
      </Row>
      <Table
        dataSource={filteredPosts}
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
      <Modal
        title={t("opc.site.postModalTitle")}
        open={modalOpen}
        onOk={() => form.submit()}
        onCancel={() => {
          setModalOpen(false);
          form.resetFields();
        }}
        okText={t("opc.common.create")}
        cancelText={t("opc.common.cancel")}
      >
        <Form form={form} layout="vertical" onFinish={handleCreate}>
          <Form.Item name="title" label={t("opc.site.titleLabel")} rules={[{ required: true }]}>
            <Input />
          </Form.Item>
          <Form.Item name="slug" label={t("opc.site.slugLabel")} rules={[{ required: true }]}>
            <Input placeholder={t("opc.site.postSlugPlaceholder")} />
          </Form.Item>
          <Form.Item name="excerpt" label={t("opc.site.excerptLabel")}>
            <Input.TextArea rows={2} />
          </Form.Item>
          <Form.Item name="tags" label={t("opc.site.tagsLabel")}>
            <Input placeholder={t("opc.site.tagsPlaceholder")} />
          </Form.Item>
        </Form>
      </Modal>
    </Card>
  );
}

function ContactsPanel() {
  const { t } = useTranslation();
  const [contacts, setContacts] = useState<ContactSummaryView[]>([]);
  const [loading, setLoading] = useState(true);
  const [searchText, setSearchText] = useState("");
  const [selectedRowKeys, setSelectedRowKeys] = useState<string[]>([]);

  const load = () => {
    setLoading(true);
    (async () => {
      try {
        // 取数走插件执行通道（#36 第 D-4 刀），处置与落地页面板同路径。
        // 行类型 = 接缝 `ContactSummaryView`：无 message 字段（列表命令不回传正文）。
        const raw = await execPluginCommand(LIST_CONTACTS_COMMAND, {});
        setContacts(JSON.parse(raw) as ContactSummaryView[]);
      } catch (e) {
        message.error(t("opc.common.loadFailed", { error: String(e) }));
        setContacts([]);
      } finally {
        setLoading(false);
      }
    })();
  };
  useEffect(() => {
    load();
  }, []);

  const handleMarkRead = async (id: string) => {
    try {
      // 出参 null（已置位）⇒ 不消费，状态列随 load() 刷新即见。
      await execPluginCommand(MARK_CONTACT_READ_COMMAND, { id });
      load();
    } catch (e) {
      message.error(t("opc.common.opFailed", { error: String(e) }));
    }
  };

  const filteredContacts = searchText
    ? contacts.filter((c) =>
      c.name.toLowerCase().includes(searchText.toLowerCase())
      || c.email.toLowerCase().includes(searchText.toLowerCase())
    )
    : contacts;

  const handleBatchMarkRead = async () => {
    try {
      await Promise.all(
        selectedRowKeys.map((id) => execPluginCommand(MARK_CONTACT_READ_COMMAND, { id })),
      );
      message.success(t("opc.common.batchSuccess"));
      setSelectedRowKeys([]);
      load();
    } catch (e) {
      message.error(t("opc.common.opFailed", { error: String(e) }));
    }
  };

  const columns = [
    { title: t("opc.site.contactColumnName"), dataIndex: "name", key: "name" },
    { title: t("opc.site.contactColumnEmail"), dataIndex: "email", key: "email" },
    // message 列已删（#36 第 D-4 刀）：`ContactSummaryView` 契约无 message（列表命令
    // 不回传正文），保留列位会渲染一列恒空的「留言」假象。
    { title: t("opc.site.contactColumnSource"), dataIndex: "source", key: "source" },
    {
      title: t("opc.site.contactColumnStatus"),
      key: "status",
      render: (_: unknown, r: ContactSummaryView) =>
        r.read ? <Tag>{t("opc.site.readTag")}</Tag> : <Tag color="orange">{t("opc.site.unreadTag")}</Tag>,
    },
    {
      title: t("opc.site.contactColumnTime"),
      key: "created",
      render: (_: unknown, r: ContactSummaryView) => new Date(r.createdAt * 1000).toLocaleString(),
    },
    {
      title: t("opc.common.actions"),
      key: "actions",
      width: 80,
      render: (_: unknown, r: ContactSummaryView) =>
        !r.read && <Button size="small" onClick={() => handleMarkRead(r.id)}>{t("opc.site.markRead")}</Button>,
    },
  ];

  return (
    <Card>
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
        <Col>
          <Space>
            {selectedRowKeys.length > 0 && (
              <Button size="small" onClick={handleBatchMarkRead}>
                {t("opc.common.batchMarkRead", { count: selectedRowKeys.length })}
              </Button>
            )}
          </Space>
        </Col>
      </Row>
      <Table
        dataSource={filteredContacts}
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
  );
}
