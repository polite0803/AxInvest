// SPDX-License-Identifier: AGPL-3.0-only
// ! 能力域聚合入口页（DomainHubPage）
//
// 每个标准域（见 domainMeta.ts）的「域路径」路由渲染本页面，集中展示该域下
// 归入的内置导航项。域路径由 CAPABILITY_DOMAIN_META 定义（如 /finance、/automation）。

import { CAPABILITY_DOMAIN_META, domainLabelKey } from "@/lib/domainMeta";
import { navItemsByDomain } from "@/lib/navRegistry";
import { Button, Card, Empty, Result, Typography } from "antd";
import { useTranslation } from "react-i18next";
import { useNavigate, useParams } from "react-router-dom";

const { Title, Paragraph } = Typography;

/** 从路由参数解析当前标准域（2026-10-28：域聚合页并入 OPC，不再按 pathname 反查） */
function domainFromParam(domainId: string | undefined) {
  return CAPABILITY_DOMAIN_META.find((d) => d.id === domainId);
}

export function DomainHubPage() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const { domainId } = useParams<{ domainId: string }>();

  const domain = domainFromParam(domainId);
  if (!domain) {
    return (
      <Result
        status="404"
        title="404"
        subTitle={t("error.pageNotFound")}
        extra={
          <Button type="primary" onClick={() => navigate("/")}>
            {t("common.back")}
          </Button>
        }
      />
    );
  }

  const items = navItemsByDomain(domain.id);

  return (
    <div style={{ padding: 24, maxWidth: 1200, margin: "0 auto", width: "100%" }}>
      {/* 域头部 */}
      <div style={{ marginBottom: 32 }}>
        <div style={{ display: "flex", alignItems: "center", gap: 12 }}>
          <span
            style={{
              display: "inline-block",
              width: 14,
              height: 14,
              borderRadius: "50%",
              backgroundColor: domain.color,
            }}
          />
          <Title level={3} style={{ margin: 0 }}>
            {t(domainLabelKey(domain.id))}
          </Title>
        </div>
        <Paragraph type="secondary" style={{ marginTop: 8 }}>
          {t("domain.description")}
        </Paragraph>
      </div>

      {items.length > 0
        ? (
          <div
            style={{
              display: "grid",
              gridTemplateColumns: "repeat(auto-fill, minmax(220px, 1fr))",
              gap: 16,
              width: "100%",
            }}
          >
            {items.map((item) => (
              <Card
                key={item.key}
                hoverable
                onClick={() => navigate(item.path)}
                style={{
                  cursor: "pointer",
                  borderLeft: `4px solid ${domain.color}`,
                  transition: "all 0.2s",
                  width: "100%",
                }}
                bodyStyle={{ padding: 16 }}
              >
                <div style={{ display: "flex", alignItems: "center", gap: 12, minWidth: 0 }}>
                  <div
                    style={{
                      width: 40,
                      height: 40,
                      borderRadius: 8,
                      backgroundColor: `${domain.color}15`,
                      display: "flex",
                      alignItems: "center",
                      justifyContent: "center",
                      color: domain.color,
                      flexShrink: 0,
                    }}
                  >
                    {item.icon}
                  </div>
                  <Title
                    level={5}
                    style={{
                      margin: 0,
                      minWidth: 0,
                      overflow: "hidden",
                      textOverflow: "ellipsis",
                      whiteSpace: "nowrap",
                    }}
                  >
                    {item.isPlugin ? item.labelKey : t(item.labelKey)}
                  </Title>
                </div>
              </Card>
            ))}
          </div>
        )
        : <Empty description={t("domain.empty")} />}
    </div>
  );
}
