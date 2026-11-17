// SPDX-License-Identifier: AGPL-3.0-only

import { Tooltip } from "@/components/layout/Tooltip";
import { showBackendError } from "@/lib/errorI18n";
import { invoke } from "@/lib/invoke";
import { message } from "@/lib/toast";
import type { UiActionKind, UiNode, UiNodeId, UiScrollIntent, UiTreeQuery, UiTreeSnapshot } from "@/types";
import { Button, Card, Input, Space, Switch, Tree, Typography } from "antd";
import { ListTree, Search } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

interface TreeDataNode {
  key: string;
  title: string;
  node: UiNode;
  children?: TreeDataNode[];
}

const keyOf = (id: UiNodeId) => `${id.backend}:${id.rawId}:${id.sessionGen}`;

/** 扁平节点表 + 根 id → antd Tree 数据（按平台树顺序，缺失父节点的节点提升为根）。 */
function buildTreeData(snapshot: UiTreeSnapshot): TreeDataNode[] {
  const byKey = new Map(snapshot.nodes.map((n) => [keyOf(n.id), n]));
  const rootKeys = new Set(snapshot.roots.map(keyOf));
  const convert = (node: UiNode): TreeDataNode => ({
    key: keyOf(node.id),
    title: `${node.role}: ${node.name || node.id.rawId}`,
    node,
    children: node.children.map(convert),
  });
  const roots = snapshot.roots.flatMap((id) => {
    const node = byKey.get(keyOf(id));
    return node ? [convert(node)] : [];
  });
  if (roots.length > 0) {
    return roots;
  }
  // 平台未提供 roots 时，用无父节点的节点作为根。
  const childKeys = new Set(
    snapshot.nodes.flatMap((n) => n.children.map((child) => keyOf(child.id))),
  );
  return snapshot.nodes
    .filter((n) => !childKeys.has(keyOf(n.id)) || rootKeys.has(keyOf(n.id)))
    .map(convert);
}

export function ComputerControlPanel() {
  const { t } = useTranslation();
  const mountedRef = useRef(true);
  useEffect(
    () => () => {
      mountedRef.current = false;
    },
    [],
  );
  const [autoMode, setAutoMode] = useState(false);
  const [snapshot, setSnapshot] = useState<UiTreeSnapshot | null>(null);
  const [loading, setLoading] = useState(false);
  const [appFilter, setAppFilter] = useState("");
  const [nameFilter, setNameFilter] = useState("");
  const [selectedKey, setSelectedKey] = useState<string | null>(null);
  const [valueInput, setValueInput] = useState("");

  const selectedNode = useMemo(
    () => snapshot?.nodes.find((n) => keyOf(n.id) === selectedKey) ?? null,
    [snapshot, selectedKey],
  );
  const treeData = useMemo(
    () => (snapshot ? buildTreeData(snapshot) : []),
    [snapshot],
  );

  const refreshTree = async () => {
    setLoading(true);
    try {
      const query: UiTreeQuery = {
        ...(appFilter.trim() ? { application: appFilter.trim() } : {}),
        ...(nameFilter.trim() ? { nameContains: nameFilter.trim() } : {}),
      };
      const result = await invoke<UiTreeSnapshot>("find_ui_elements", { query });
      if (mountedRef.current) {
        setSnapshot(result);
        setSelectedKey(null);
      }
    } catch (e) {
      showBackendError(message, e);
    } finally {
      if (mountedRef.current) {
        setLoading(false);
      }
    }
  };

  const runSemantic = async (
    label: string,
    invokeName: string,
    args: Record<string, unknown>,
  ) => {
    if (!selectedNode) {
      return;
    }
    try {
      await invoke(invokeName, { id: selectedNode.id, ...args });
      message.success(t("computerControl.actionDone", { action: label }));
    } catch (e) {
      showBackendError(message, e);
    }
  };

  const handlePerform = (
    action: UiActionKind,
    label: string,
    scrollIntent?: UiScrollIntent,
  ) => {
    void runSemantic(label, "perform_ui_element_action", {
      action,
      ...(scrollIntent ? { scrollIntent } : {}),
    });
  };

  const has = (action: UiActionKind) => selectedNode?.supportedActions.includes(action) ?? false;

  const handlePressKey = async (key: string, modifiers: string[] = []) => {
    try {
      await invoke("press_key", { key, modifiers });
      message.success(t("computerControl.keyPressed", { key }));
    } catch (e) {
      showBackendError(message, e);
    }
  };

  return (
    <div
      style={{ padding: 16, display: "flex", flexDirection: "column", gap: 12 }}
    >
      <Space wrap>
        <Input
          aria-label={t("computerControl.appFilter")}
          placeholder={t("computerControl.appFilter")}
          value={appFilter}
          onChange={(e) => setAppFilter(e.target.value)}
          style={{ width: 160 }}
        />
        <Input
          aria-label={t("computerControl.nameFilter")}
          placeholder={t("computerControl.nameFilter")}
          value={nameFilter}
          onChange={(e) => setNameFilter(e.target.value)}
          onPressEnter={() => void refreshTree()}
          style={{ width: 160 }}
        />
        <Button
          icon={<Search size={14} />}
          onClick={() => void refreshTree()}
          loading={loading}
        >
          {t("computerControl.refreshTree")}
        </Button>
        <TooltipAutoMode
          checked={autoMode}
          onChange={setAutoMode}
          label={t("computerControl.auto")}
          manual={t("computerControl.manual")}
          hint={t("computerControl.autoModeTooltip")}
        />
      </Space>

      <Card
        size="small"
        title={t("computerControl.treeTitle", {
          count: snapshot?.nodes.length ?? 0,
        })}
      >
        {treeData.length === 0
          ? (
            <Typography.Text type="secondary">
              {t("computerControl.noNodes")}
            </Typography.Text>
          )
          : (
            <div style={{ maxHeight: 260, overflow: "auto" }}>
              <Tree
                treeData={treeData}
                selectedKeys={selectedKey ? [selectedKey] : []}
                onSelect={(keys) => setSelectedKey(typeof keys[0] === "string" ? keys[0] : null)}
                defaultExpandAll
              />
            </div>
          )}
      </Card>

      {selectedNode && (
        <Card size="small" title={t("computerControl.nodeActions")}>
          <Space orientation="vertical" style={{ width: "100%" }}>
            <Typography.Text type="secondary" style={{ fontSize: 12 }}>
              {selectedNode.role}: {selectedNode.name || selectedNode.id.rawId}
              {selectedNode.value && ` — ${selectedNode.value}`}
            </Typography.Text>
            <Space wrap>
              {has("invoke") && (
                <Button
                  size="small"
                  type="primary"
                  icon={<ListTree size={14} />}
                  onClick={() => void runSemantic(t("computerControl.invoke"), "invoke_ui_element", {})}
                >
                  {t("computerControl.invoke")}
                </Button>
              )}
              {has("focus") && (
                <Button
                  size="small"
                  onClick={() => void runSemantic(t("computerControl.focus"), "focus_ui_element", {})}
                >
                  {t("computerControl.focus")}
                </Button>
              )}
              {has("setValue") && (
                <Space.Compact>
                  <Input
                    aria-label={t("computerControl.setValuePlaceholder")}
                    placeholder={t("computerControl.setValuePlaceholder")}
                    value={valueInput}
                    onChange={(e) => setValueInput(e.target.value)}
                    style={{ width: 180 }}
                  />
                  <Button
                    size="small"
                    onClick={() =>
                      void runSemantic(t("computerControl.setValue"), "set_ui_element_value", {
                        value: valueInput,
                      })}
                  >
                    {t("computerControl.setValue")}
                  </Button>
                </Space.Compact>
              )}
              {has("toggle") && (
                <Button
                  size="small"
                  onClick={() => handlePerform("toggle", t("computerControl.toggle"))}
                >
                  {t("computerControl.toggle")}
                </Button>
              )}
              {has("select") && (
                <Button
                  size="small"
                  onClick={() => handlePerform("select", t("computerControl.select"))}
                >
                  {t("computerControl.select")}
                </Button>
              )}
              {has("expand") && (
                <Button
                  size="small"
                  onClick={() => handlePerform("expand", t("computerControl.expand"))}
                >
                  {t("computerControl.expand")}
                </Button>
              )}
              {has("collapse") && (
                <Button
                  size="small"
                  onClick={() => handlePerform("collapse", t("computerControl.collapse"))}
                >
                  {t("computerControl.collapse")}
                </Button>
              )}
              {has("scrollIntoView") && (
                <Button
                  size="small"
                  onClick={() => handlePerform("scrollIntoView", t("computerControl.scrollIntoView"))}
                >
                  {t("computerControl.scrollIntoView")}
                </Button>
              )}
              {has("scroll") && (
                <Space>
                  <Button
                    size="small"
                    onClick={() => handlePerform("scroll", t("computerControl.scrollUp"), "up")}
                  >
                    {t("computerControl.scrollUp")}
                  </Button>
                  <Button
                    size="small"
                    onClick={() => handlePerform("scroll", t("computerControl.scrollDown"), "down")}
                  >
                    {t("computerControl.scrollDown")}
                  </Button>
                </Space>
              )}
            </Space>
          </Space>
        </Card>
      )}

      <Card size="small" title={t("computerControl.shortcuts")}>
        <Space wrap>
          <Button size="small" onClick={() => void handlePressKey("Enter")}>
            Enter
          </Button>
          <Button size="small" onClick={() => void handlePressKey("Tab")}>
            Tab
          </Button>
          <Button size="small" onClick={() => void handlePressKey("Escape")}>
            Esc
          </Button>
          <Button size="small" onClick={() => void handlePressKey("a", ["ctrl"])}>
            Ctrl+A
          </Button>
          <Button size="small" onClick={() => void handlePressKey("c", ["ctrl"])}>
            Ctrl+C
          </Button>
          <Button size="small" onClick={() => void handlePressKey("v", ["ctrl"])}>
            Ctrl+V
          </Button>
          <Button size="small" onClick={() => void handlePressKey("z", ["ctrl"])}>
            Ctrl+Z
          </Button>
        </Space>
      </Card>
    </div>
  );
}

/** Tooltip 包裹的 Switch 必须 forwardRef 透传，避免浮层定位异常。 */
function TooltipAutoMode({
  checked,
  onChange,
  label,
  manual,
  hint,
}: {
  checked: boolean;
  onChange: (v: boolean) => void;
  label: string;
  manual: string;
  hint: string;
}) {
  return (
    <Tooltip title={hint}>
      <Switch
        checked={checked}
        onChange={onChange}
        checkedChildren={label}
        unCheckedChildren={manual}
      />
    </Tooltip>
  );
}
