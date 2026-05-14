import { ANALYST_NAMES } from "@/types";
import { Card, Typography } from "antd";

interface Props {
  expertId: string;
  report: string;
}

export function AnalystReportCard({ expertId, report }: Props) {
  const name = ANALYST_NAMES[expertId] || expertId;

  return (
    <Card size="small" title={name}>
      <Typography.Paragraph ellipsis={{ rows: 5, expandable: true }}>
        {report}
      </Typography.Paragraph>
    </Card>
  );
}
