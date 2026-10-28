// SPDX-License-Identifier: AGPL-3.0-only

import { useIsDarkTheme } from "@/hooks/useIsDarkTheme";
import { hueColor } from "@/theme/categoricalColors";
import type { NodeProps } from "@xyflow/react";
import { Tag } from "antd";
import React, { memo } from "react";
import { useTranslation } from "react-i18next";
import type { ContainerNodeData } from "./ContainerNode";
import { ContainerNode } from "./ContainerNode";
import { nodeIconFor } from "./nodeIcons";

interface AggregatorNodeData extends ContainerNodeData {
  aggregationType?: string;
  inputCount?: number;
}

const AggregatorNodeComponent: React.FC<NodeProps> = ({ data: _data, selected }) => {
  const data = _data as unknown as AggregatorNodeData; // SAFE: ReactFlow NodeProps.data is untyped; runtime data matches expected component data
  const { t } = useTranslation();
  const isDark = useIsDarkTheme();
  const accent = hueColor("orange", isDark);

  return (
    <ContainerNode
      data={data}
      selected={selected}
      icon={nodeIconFor("aggregator")}
      childLabel={t("workflow.aggregatorNode.inputs")}
      extraTags={
        <>
          {data.aggregationType && (
            <Tag
              style={{
                margin: 0,
                fontSize: 9,
                padding: "0 4px",
                background: `${accent}20`,
                border: `1px solid ${accent}50`,
                color: accent,
              }}
            >
              {data.aggregationType.toUpperCase()}
            </Tag>
          )}
        </>
      }
    />
  );
};

export const AggregatorNode = memo(AggregatorNodeComponent);
