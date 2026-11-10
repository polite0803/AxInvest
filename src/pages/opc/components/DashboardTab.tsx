// SPDX-License-Identifier: AGPL-3.0-only

import { useIsDarkTheme } from "@/hooks/useIsDarkTheme";
import { cssVar } from "@/lib/cssVar";
import { ensureECharts } from "@/lib/echartsInstance";
import { invoke } from "@/lib/invoke";
import { invokePluginCommand } from "@/lib/pluginCommand";
import { resolvePluginForCommand } from "@/lib/pluginCommandResolve";
import type { LandingPageSummaryView } from "@/types/opc";
import {
  DollarOutlined,
  FileTextOutlined,
  ProjectOutlined,
  TeamOutlined,
  VideoCameraOutlined,
} from "@ant-design/icons";
import { Button, Card, Col, Empty, message, Row, Space, Spin, Statistic, Tag, Timeline } from "antd";
import * as echarts from "echarts";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

/**
 * 博客列表的取数命令（`plugin.opc` 的命名命令，与 manifest `commands[].name` 逐字一致）。
 *
 * 原先这条是宿主 Tauri 命令 `opc_list_blog_posts`，#36 第 4 步迁入 OPC 插件 ⇒ 宿主侧已删。
 * 本仪表盘只用它的 `length` 当 `blog_posts_count` ⇒ 不传分页时插件必须取尽整表，
 * 否则这个计数会随接缝的单页上限静默偏小（判据见 `plugin-opc` 的 `blog_posts`）。
 */
const LIST_BLOG_POSTS_COMMAND = "opc_list_blog_posts";
// 落地页列表（#36 第 D-4 刀：宿主同名 Tauri 命令迁入 OPC 插件，与 ContentMediaTab /
// SitesTab 同通道）。本仪表盘只用它的 `length` 当 `landing_pages_count` ⇒ 不传分页时
// 插件必须取尽整表，否则这个计数会随接缝的单页上限静默偏小。
const LIST_LANDING_PAGES_COMMAND = "opc_list_landing_pages";
// 内容资产 / 发布计划的只读取数命令（`plugin.opc` 命名命令；宿主同名 Tauri 命令本批仍在，
// 前端已改走插件通道，与 ContentMediaTab 同通道）。
const LIST_CONTENT_ASSETS_COMMAND = "opc_list_content_assets";
const LIST_PUBLISH_SCHEDULES_COMMAND = "opc_list_publish_schedules";

/**
 * 取一个插件只读命令的行数组。
 *
 * 插件未装 / 未启用（`resolvePluginForCommand` 回 `null`）与命令执行失败同处置：落空数组
 * ——「一条只读查询坏了不拖垮整张图」是本面板既有的 `.catch(() => [])` 语义。
 */
async function fetchPluginRows<T>(command: string): Promise<T[]> {
  const pluginId = await resolvePluginForCommand(command);
  if (!pluginId) {
    return [];
  }
  const raw = await invoke<string>("plugin_execute_command", {
    pluginId,
    name: command,
    input: {},
  });
  return JSON.parse(raw) as T[];
}

/**
 * OPC 看板汇总（#36：宿主命令 `opc_get_dashboard_summary` 已迁入 OPC 插件，改走
 * `invokePluginCommand` 通道）。字段一律 camelCase —— 与插件命令出参契约逐字一致
 * （Rust 侧 `#[serde(rename_all = "camelCase")]`，禁区 13）。
 */
interface DashboardSummary {
  totalRevenue: number;
  totalInvoices: number;
  activeProjects: number;
  totalCustomers: number;
  recentKpis: Array<{
    id: string;
    domainPackId: string;
    name: string;
    value: number;
    unit: string;
    period: string;
    recordedAt: number;
  }>;
  revenueTrend?: Array<{
    id: string;
    amount: number;
    currency: string;
    category: string;
    description: string;
    recordedAt: number;
  }>;
}

interface ContentMediaKpis {
  content_assets_count: number;
  blog_posts_count: number;
  landing_pages_count: number;
  publish_schedules_pending: number;
  publish_schedules_published: number;
}

interface ContentAsset {
  id: string;
  title: string;
  contentType: string;
  status: string;
}

interface PublishSchedule {
  id: string;
  status: string;
}

interface BlogPost {
  id: string;
  published: boolean;
}

const formatTs = (ts: number): string => {
  const d = new Date(ts * 1000);
  return `${d.getMonth() + 1}/${d.getDate()}`;
};

export function DashboardTab() {
  const { t } = useTranslation();
  const isDark = useIsDarkTheme();
  const [loading, setLoading] = useState(true);
  const [summary, setSummary] = useState<DashboardSummary | null>(null);
  const [cmKpis, setCmKpis] = useState<ContentMediaKpis | null>(null);
  const [cmLoading, setCmLoading] = useState(true);

  const revenueChartRef = useRef<HTMLDivElement>(null);
  const revenueChartInstance = useRef<echarts.ECharts | null>(null);
  const contentTypeChartRef = useRef<HTMLDivElement>(null);
  const contentTypeChartInstance = useRef<echarts.ECharts | null>(null);
  const scheduleStatusChartRef = useRef<HTMLDivElement>(null);
  const scheduleStatusChartInstance = useRef<echarts.ECharts | null>(null);

  useEffect(() => {
    (async () => {
      try {
        const data = await invokePluginCommand<DashboardSummary>("opc_get_dashboard_summary", {});
        setSummary(data);
      } catch (e) {
        message.error(t("opc.common.loadFailed", { error: String(e) }));
        setSummary(null);
      } finally {
        setLoading(false);
      }
    })();
  }, []);

  // 依赖含 isDark：ECharts 主题只在 init 生效，档位翻转必须重跑一次建图
  // （代价是这四个只读查询再来一遍，本地 SQLite 读，换主题的频次极低）
  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const [assets, schedules, posts, pages] = await Promise.all([
          // 四条只读插件命令（#36 系列迁移）同处置：失败 / 插件没装都落空数组，
          // 与本行既存的 `.catch(() => [])` 语义一致（一条只读查询坏了不拖垮整张图）。
          fetchPluginRows<ContentAsset>(LIST_CONTENT_ASSETS_COMMAND).catch(() => []),
          fetchPluginRows<PublishSchedule>(LIST_PUBLISH_SCHEDULES_COMMAND).catch(() => []),
          fetchPluginRows<BlogPost>(LIST_BLOG_POSTS_COMMAND).catch(() => []),
          fetchPluginRows<LandingPageSummaryView>(LIST_LANDING_PAGES_COMMAND).catch(() => []),
        ]);
        if (cancelled) { return; }
        const typeCounts: Record<string, number> = {};
        for (const a of assets) {
          typeCounts[a.contentType] = (typeCounts[a.contentType] || 0) + 1;
        }
        const pending = schedules.filter((s) => s.status === "pending").length;
        const published = schedules.filter((s) => s.status === "published").length;

        setCmKpis({
          content_assets_count: assets.length,
          blog_posts_count: posts.length,
          landing_pages_count: pages.length,
          publish_schedules_pending: pending,
          publish_schedules_published: published,
        });

        if (contentTypeChartRef.current) {
          contentTypeChartInstance.current = ensureECharts(contentTypeChartRef.current, isDark);
          contentTypeChartInstance.current.setOption({
            tooltip: { trigger: "item" },
            series: [
              {
                type: "pie",
                radius: ["40%", "70%"],
                data: Object.entries(typeCounts).map(([type, count]) => ({
                  name: type,
                  value: count,
                })),
                label: { show: false },
              },
            ],
          });
        }

        if (scheduleStatusChartRef.current) {
          scheduleStatusChartInstance.current = ensureECharts(scheduleStatusChartRef.current, isDark);
          scheduleStatusChartInstance.current.setOption({
            tooltip: { trigger: "item" },
            series: [
              {
                type: "pie",
                radius: ["40%", "70%"],
                data: [
                  { name: t("opc.contentMedia.schedule_pending"), value: pending },
                  { name: t("opc.contentMedia.schedule_published"), value: published },
                  {
                    name: t("opc.contentMedia.schedule_failed"),
                    value: schedules.filter((s) => s.status === "failed").length,
                  },
                ].filter((d) => d.value > 0),
                label: { show: false },
              },
            ],
          });
        }
      } catch (e) {
        if (!cancelled) {
          message.error(t("opc.common.loadFailed", { error: String(e) }));
        }
      } finally {
        if (!cancelled) { setCmLoading(false); }
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [t, isDark]);

  useEffect(() => {
    if (!summary) { return; }

    if (revenueChartRef.current && summary.revenueTrend) {
      revenueChartInstance.current = ensureECharts(revenueChartRef.current, isDark);
      revenueChartInstance.current.setOption({
        tooltip: { trigger: "axis" },
        grid: { left: 50, right: 20, top: 20, bottom: 30 },
        xAxis: {
          type: "category",
          data: summary.revenueTrend.map((item) => formatTs(item.recordedAt)),
          axisLabel: { color: cssVar("--muted") },
        },
        yAxis: {
          type: "value",
          axisLabel: { color: cssVar("--muted"), formatter: "¥{value}" },
        },
        series: [
          {
            name: t("opc.dashboard.totalRevenue"),
            type: "line",
            data: summary.revenueTrend.map((item) => item.amount),
            smooth: true,
            areaStyle: { opacity: 0.3 },
            lineStyle: { color: cssVar("--green") },
            itemStyle: { color: cssVar("--green") },
          },
        ],
      });
    }

    const handleResize = () => {
      revenueChartInstance.current?.resize();
      contentTypeChartInstance.current?.resize();
      scheduleStatusChartInstance.current?.resize();
    };
    window.addEventListener("resize", handleResize);
    return () => window.removeEventListener("resize", handleResize);
  }, [summary, t, isDark]);

  useEffect(() => {
    return () => {
      revenueChartInstance.current?.dispose();
      contentTypeChartInstance.current?.dispose();
      scheduleStatusChartInstance.current?.dispose();
    };
  }, []);

  if (loading) {
    return <Spin size="large" style={{ display: "block", margin: "80px auto" }} />;
  }
  if (!summary) {
    return <Empty description={t("opc.dashboard.loadFailed")} />;
  }

  return (
    <div className="space-y-4">
      {/* 核心 KPI 卡片 */}
      <Row gutter={[16, 16]}>
        <Col xs={24} sm={12} md={6}>
          <Card size="small" className="h-full">
            <Statistic
              title={t("opc.dashboard.totalRevenue")}
              value={summary.totalRevenue}
              prefix="¥"
              precision={2}
              valueStyle={{ color: "var(--green)" }}
            />
          </Card>
        </Col>
        <Col xs={24} sm={12} md={6}>
          <Card size="small" className="h-full">
            <Statistic
              title={t("opc.dashboard.totalInvoices")}
              value={summary.totalInvoices}
              prefix={<FileTextOutlined />}
            />
          </Card>
        </Col>
        <Col xs={24} sm={12} md={6}>
          <Card size="small" className="h-full">
            <Statistic
              title={t("opc.dashboard.activeCustomers")}
              value={summary.totalCustomers}
              prefix={<TeamOutlined />}
            />
          </Card>
        </Col>
        <Col xs={24} sm={12} md={6}>
          <Card size="small" className="h-full">
            <Statistic
              title={t("opc.dashboard.activeProjects")}
              value={summary.activeProjects}
              prefix={<ProjectOutlined />}
            />
          </Card>
        </Col>
      </Row>

      {/* 收入趋势图 */}
      {summary.revenueTrend && (
        <Row gutter={[16, 16]}>
          <Col xs={24}>
            <Card title={t("opc.dashboard.revenueTrend")} size="small">
              <div ref={revenueChartRef} style={{ height: 240 }} />
            </Card>
          </Col>
        </Row>
      )}

      {/* 内容媒体 KPI */}
      <Row gutter={[16, 16]}>
        <Col xs={24}>
          <Card
            title={
              <Space>
                <VideoCameraOutlined />
                {t("opc.contentMedia.dashboardTitle")}
              </Space>
            }
            size="small"
          >
            {cmLoading ? <Spin /> : cmKpis
              ? (
                <>
                  <Row gutter={[12, 12]} style={{ marginBottom: 16 }}>
                    <Col xs={12} sm={8} md={4}>
                      <Card size="small">
                        <Statistic
                          title={t("opc.contentMedia.kpiAssets")}
                          value={cmKpis.content_assets_count}
                          valueStyle={{ color: "var(--accent)" }}
                        />
                      </Card>
                    </Col>
                    <Col xs={12} sm={8} md={4}>
                      <Card size="small">
                        <Statistic
                          title={t("opc.contentMedia.kpiBlogPosts")}
                          value={cmKpis.blog_posts_count}
                          valueStyle={{ color: "var(--purple)" }}
                        />
                      </Card>
                    </Col>
                    <Col xs={12} sm={8} md={4}>
                      <Card size="small">
                        <Statistic
                          title={t("opc.contentMedia.kpiLandingPages")}
                          value={cmKpis.landing_pages_count}
                          valueStyle={{ color: "var(--teal)" }}
                        />
                      </Card>
                    </Col>
                    <Col xs={12} sm={8} md={4}>
                      <Card size="small">
                        <Statistic
                          title={t("opc.contentMedia.kpiSchedulesPending")}
                          value={cmKpis.publish_schedules_pending}
                          valueStyle={{ color: "var(--amber)" }}
                        />
                      </Card>
                    </Col>
                    <Col xs={12} sm={8} md={4}>
                      <Card size="small">
                        <Statistic
                          title={t("opc.contentMedia.kpiSchedulesPublished")}
                          value={cmKpis.publish_schedules_published}
                          valueStyle={{ color: "var(--green)" }}
                        />
                      </Card>
                    </Col>
                  </Row>
                  <Row gutter={[16, 16]}>
                    <Col xs={24} md={12}>
                      <Card size="small" title={t("opc.contentMedia.chartContentType")}>
                        <div ref={contentTypeChartRef} style={{ height: 200 }} />
                      </Card>
                    </Col>
                    <Col xs={24} md={12}>
                      <Card size="small" title={t("opc.contentMedia.chartScheduleStatus")}>
                        <div ref={scheduleStatusChartRef} style={{ height: 200 }} />
                      </Card>
                    </Col>
                  </Row>
                </>
              )
              : <Empty description={t("opc.contentMedia.noData")} />}
          </Card>
        </Col>
      </Row>

      {/* KPI 时间线 + 快捷操作 */}
      <Row gutter={16}>
        <Col xs={24} lg={12}>
          <Card title={t("opc.dashboard.kpiTitle")} size="small">
            {summary.recentKpis.length === 0 ? <Empty description={t("opc.dashboard.noKpi")} /> : (
              <Timeline
                items={summary.recentKpis.slice(0, 5).map((kpi) => ({
                  color: "blue",
                  children: (
                    <>
                      <strong>{kpi.name}</strong>: {kpi.value} {kpi.unit} <Tag>{kpi.period}</Tag>
                    </>
                  ),
                }))}
              />
            )}
          </Card>
        </Col>
        <Col xs={24} lg={12}>
          <Card title={t("opc.dashboard.quickActionsTitle")} size="small">
            <Space direction="vertical" style={{ width: "100%" }}>
              <Button
                type="primary"
                block
                icon={<DollarOutlined />}
                onClick={() => window.dispatchEvent(new CustomEvent("opc-switch-tab", { detail: "invoices" }))}
              >
                {t("opc.dashboard.manageInvoices")}
              </Button>
              <Button
                block
                icon={<TeamOutlined />}
                onClick={() => window.dispatchEvent(new CustomEvent("opc-switch-tab", { detail: "customers" }))}
              >
                {t("opc.dashboard.manageCustomers")}
              </Button>
              <Button
                block
                icon={<ProjectOutlined />}
                onClick={() => window.dispatchEvent(new CustomEvent("opc-switch-tab", { detail: "projects" }))}
              >
                {t("opc.dashboard.manageProjects")}
              </Button>
              <Button
                block
                icon={<VideoCameraOutlined />}
                onClick={() => window.dispatchEvent(new CustomEvent("opc-switch-tab", { detail: "content_media" }))}
              >
                {t("opc.contentMedia.manageContent")}
              </Button>
            </Space>
          </Card>
        </Col>
      </Row>
    </div>
  );
}
