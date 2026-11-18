// SPDX-License-Identifier: AGPL-3.0-only
//
// sales_growth 插件页面入口：所有数据访问都走插件 SDK 的 executeCommand。

import { createAxAgentPluginClient } from "../vendor/sdk/pluginClientBridge";

const client = createAxAgentPluginClient();
client.announceReady();

interface LandingPageRow {
  id?: unknown;
  title?: unknown;
  slug?: unknown;
  published?: unknown;
  createdAt?: unknown;
}

interface ContactRow {
  id?: unknown;
  name?: unknown;
  email?: unknown;
  source?: unknown;
  read?: unknown;
  createdAt?: unknown;
}

const customerState = document.querySelector<HTMLElement>("#customer-state");
const landingState = document.querySelector<HTMLElement>("#landing-state");
const contactsState = document.querySelector<HTMLElement>("#contact-state");

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

function parseRows<T extends object>(raw: unknown): T[] {
  const value = parseValue(raw);
  if (!Array.isArray(value)) { return []; }
  return value.filter((row): row is T => row !== null && typeof row === "object");
}

function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function createdAt(value: unknown): string {
  return typeof value === "number" ? new Date(value * 1000).toLocaleString() : "-";
}

function setState(element: HTMLElement | null, value: string, isError = false): void {
  if (!element) { return; }
  element.className = isError ? "error" : "state";
  element.textContent = value;
}

async function loadLandingPages(): Promise<void> {
  const table = document.querySelector<HTMLTableElement>("#landing-pages");
  const tbody = table?.querySelector("tbody");
  if (!table || !tbody) { return; }
  setState(landingState, "加载中…");
  table.hidden = true;
  tbody.replaceChildren();
  try {
    const rows = parseRows<LandingPageRow>(
      await client.executeCommand({ name: "opc_list_landing_pages", input: {} }),
    );
    if (rows.length === 0) {
      setState(landingState, "暂无落地页");
      return;
    }
    for (const row of rows) {
      const tr = document.createElement("tr");
      for (
        const value of [
          text(row.title),
          text(row.slug),
          row.published === true ? "已发布" : "草稿",
          createdAt(row.createdAt),
        ]
      ) {
        const td = document.createElement("td");
        td.textContent = value;
        tr.append(td);
      }
      tbody.append(tr);
    }
    setState(landingState, `共 ${rows.length} 条落地页`);
    table.hidden = false;
  } catch (error) {
    setState(landingState, `加载落地页失败：${errorText(error)}`, true);
  }
}

async function loadContacts(): Promise<void> {
  const table = document.querySelector<HTMLTableElement>("#contacts");
  const tbody = table?.querySelector("tbody");
  if (!table || !tbody) { return; }
  setState(contactsState, "加载中…");
  table.hidden = true;
  tbody.replaceChildren();
  try {
    const rows = parseRows<ContactRow>(
      await client.executeCommand({ name: "opc_list_contacts", input: {} }),
    );
    if (rows.length === 0) {
      setState(contactsState, "暂无联系表单提交");
      return;
    }
    for (const row of rows) {
      const tr = document.createElement("tr");
      for (
        const value of [
          text(row.name),
          text(row.email),
          text(row.source),
          row.read === true ? "已读" : "未读",
          createdAt(row.createdAt),
        ]
      ) {
        const td = document.createElement("td");
        td.textContent = value;
        tr.append(td);
      }
      tbody.append(tr);
    }
    setState(contactsState, `共 ${rows.length} 条联系表单`);
    table.hidden = false;
  } catch (error) {
    setState(contactsState, `加载联系表单失败：${errorText(error)}`, true);
  }
}

async function createCustomer(form: HTMLFormElement): Promise<void> {
  const name = document.querySelector<HTMLInputElement>("#customer-name")?.value.trim() ?? "";
  const email = document.querySelector<HTMLInputElement>("#customer-email")?.value.trim() ?? "";
  const company = document.querySelector<HTMLInputElement>("#customer-company")?.value.trim() ?? "";
  if (!name || !email) {
    setState(customerState, "请填写姓名与邮箱", true);
    return;
  }
  setState(customerState, "创建中…");
  try {
    await client.executeCommand({
      name: "opc_create_customer",
      input: { name, email, company: company || undefined },
    });
    setState(customerState, "客户创建成功");
    form.reset();
  } catch (error) {
    setState(customerState, `创建客户失败：${errorText(error)}`, true);
  }
}

document.querySelector<HTMLFormElement>("#customer-form")?.addEventListener("submit", (event) => {
  event.preventDefault();
  const form = event.currentTarget;
  if (form instanceof HTMLFormElement) {
    void createCustomer(form);
  }
});

void loadLandingPages();
void loadContacts();
