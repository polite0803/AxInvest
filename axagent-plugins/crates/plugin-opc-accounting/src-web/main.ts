// SPDX-License-Identifier: AGPL-3.0-only
//
// accounting 插件页面入口：所有数据访问都走插件 SDK 的 executeCommand，
// 不出现 invoke("opc_*")（vendored 客户端白名单也没有那条通道）。

import { createAxAgentPluginClient } from "../vendor/sdk/pluginClientBridge";

const client = createAxAgentPluginClient();
client.announceReady();

const listState = document.querySelector<HTMLElement>("#list-state");
const table = document.querySelector<HTMLTableElement>("#invoices");
const tbody = table?.querySelector("tbody");
const createState = document.querySelector<HTMLElement>("#create-state");
const customerInput = document.querySelector<HTMLInputElement>("#customer-id");
const descInput = document.querySelector<HTMLInputElement>("#item-desc");

interface InvoiceRow {
  id?: unknown;
  invoiceNumber?: unknown;
  customerId?: unknown;
  status?: unknown;
  total?: unknown;
}

function text(value: unknown): string {
  return value === null || value === undefined ? "-" : String(value);
}

function parseValue(raw: unknown): unknown {
  if (typeof raw !== "string") { return raw; }
  try {
    return JSON.parse(raw) as unknown;
  } catch {
    return raw;
  }
}

function parseRows(raw: unknown): InvoiceRow[] {
  const value = parseValue(raw);
  if (!Array.isArray(value)) { return []; }
  return value.filter((row): row is InvoiceRow => row !== null && typeof row === "object");
}

function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

async function reloadInvoices(): Promise<void> {
  if (!listState || !table || !tbody) { return; }
  listState.textContent = "加载中…";
  listState.className = "state";
  table.hidden = true;
  tbody.replaceChildren();
  try {
    const raw = await client.executeCommand({ name: "opc_list_invoices", input: {} });
    const rows = parseRows(raw);
    if (rows.length === 0) {
      listState.textContent = "暂无发票";
      return;
    }
    for (const row of rows) {
      const tr = document.createElement("tr");
      for (
        const value of [
          text(row.invoiceNumber ?? row.id),
          text(row.customerId),
          text(row.status),
          text(row.total),
        ]
      ) {
        const td = document.createElement("td");
        td.textContent = value;
        tr.append(td);
      }
      const action = document.createElement("td");
      const button = document.createElement("button");
      button.type = "button";
      button.textContent = "标记已付";
      button.addEventListener("click", () => void transition(String(row.id ?? "")));
      action.append(button);
      tr.append(action);
      tbody.append(tr);
    }
    listState.textContent = `共 ${rows.length} 条`;
    table.hidden = false;
  } catch (error) {
    listState.className = "error";
    listState.textContent = `加载失败：${errorText(error)}`;
  }
}

async function createInvoice(): Promise<void> {
  if (!createState) { return; }
  const customerId = customerInput?.value.trim() ?? "";
  const description = descInput?.value.trim() ?? "";
  if (!customerId || !description) {
    createState.className = "error";
    createState.textContent = "请填客户 ID 与项目描述";
    return;
  }
  createState.className = "state";
  createState.textContent = "创建中…";
  try {
    await client.executeCommand({
      name: "opc_create_invoice",
      input: {
        customerId,
        lineItems: [{ description, quantity: 1, unitPrice: 0, taxRate: 0 }],
      },
    });
    createState.textContent = "创建成功";
    descInput && (descInput.value = "");
    await reloadInvoices();
  } catch (error) {
    createState.className = "error";
    createState.textContent = `创建失败：${errorText(error)}`;
  }
}

async function transition(id: string): Promise<void> {
  if (!listState || !id) { return; }
  listState.className = "state";
  listState.textContent = "状态变更中…";
  try {
    const raw = await client.executeCommand({
      name: "opc_transition_invoice",
      input: { id, status: "paid" },
    });
    const value = parseValue(raw);
    listState.textContent = typeof value === "object" && value !== null && "value" in value
      ? `状态变更回执：${text((value as Record<string, unknown>).value)}`
      : "状态变更完成";
    await reloadInvoices();
  } catch (error) {
    listState.className = "error";
    listState.textContent = `状态变更失败：${errorText(error)}`;
  }
}

document.querySelector("#reload")?.addEventListener("click", () => void reloadInvoices());
document.querySelector("#create")?.addEventListener("click", () => void createInvoice());
void reloadInvoices();
