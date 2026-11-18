// SPDX-License-Identifier: AGPL-3.0-only
//
// content_media 插件页面入口：所有数据访问都走插件 SDK 的 executeCommand。

import { createAxAgentPluginClient } from "../vendor/sdk/pluginClientBridge";

const client = createAxAgentPluginClient();
client.announceReady();

interface BlogRow {
  id?: unknown;
  title?: unknown;
  slug?: unknown;
  published?: unknown;
  viewCount?: unknown;
  updatedAt?: unknown;
}

interface ContentAssetRow {
  id?: unknown;
  title?: unknown;
  contentType?: unknown;
  status?: unknown;
  tags?: unknown;
  createdAt?: unknown;
}

interface PublishScheduleRow {
  id?: unknown;
  contentRefType?: unknown;
  contentRefId?: unknown;
  status?: unknown;
  scheduledAt?: unknown;
  publishedAt?: unknown;
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

function parseRows<T extends object>(raw: unknown): T[] {
  const value = parseValue(raw);
  if (!Array.isArray(value)) { return []; }
  return value.filter((row): row is T => row !== null && typeof row === "object");
}

function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function formattedTime(value: unknown): string {
  if (typeof value !== "number") { return "-"; }
  const millis = value < 10_000_000_000 ? value * 1000 : value;
  return new Date(millis).toLocaleString();
}

function setState(element: HTMLElement | null, value: string, isError = false): void {
  if (!element) { return; }
  element.className = isError ? "error" : "state";
  element.textContent = value;
}

function fillRows(
  tableSelector: string,
  emptyText: string,
  state: HTMLElement | null,
  rows: unknown[][],
): void {
  const table = document.querySelector<HTMLTableElement>(tableSelector);
  const tbody = table?.querySelector("tbody");
  if (!table || !tbody) { return; }
  tbody.replaceChildren();
  if (rows.length === 0) {
    table.hidden = true;
    setState(state, emptyText);
    return;
  }
  for (const cells of rows) {
    const tr = document.createElement("tr");
    for (const value of cells) {
      const td = document.createElement("td");
      if (value instanceof Node) {
        td.append(value);
      } else {
        td.textContent = text(value);
      }
      tr.append(td);
    }
    tbody.append(tr);
  }
  table.hidden = false;
}

function actionButton(label: string, handler: () => Promise<void>): HTMLButtonElement {
  const button = document.createElement("button");
  button.type = "button";
  button.textContent = label;
  button.addEventListener("click", () => void handler());
  return button;
}

const blogState = document.querySelector<HTMLElement>("#blog-list-state");
const assetState = document.querySelector<HTMLElement>("#asset-list-state");
const scheduleState = document.querySelector<HTMLElement>("#schedule-list-state");
const blogCreateState = document.querySelector<HTMLElement>("#blog-state");
const assetCreateState = document.querySelector<HTMLElement>("#asset-create-state");
const scheduleCreateState = document.querySelector<HTMLElement>("#schedule-create-state");

async function loadBlogs(): Promise<void> {
  setState(blogState, "加载中…");
  try {
    const rows = parseRows<BlogRow>(
      await client.executeCommand({ name: "opc_list_blog_posts", input: {} }),
    );
    fillRows(
      "#blog-posts",
      "暂无博客",
      blogState,
      rows.map((row) => [
        row.title,
        row.slug,
        row.published === true ? "已发布" : "草稿",
        row.viewCount,
        formattedTime(row.updatedAt),
      ]),
    );
    if (rows.length > 0) { setState(blogState, `共 ${rows.length} 篇博客`); }
  } catch (error) {
    setState(blogState, `加载博客失败：${errorText(error)}`, true);
  }
}

async function loadAssets(): Promise<void> {
  setState(assetState, "加载中…");
  try {
    const rows = parseRows<ContentAssetRow>(
      await client.executeCommand({ name: "opc_list_content_assets", input: {} }),
    );
    fillRows(
      "#content-assets",
      "暂无内容资产",
      assetState,
      rows.map((row) => {
        const id = String(row.id ?? "");
        return [
          row.title,
          row.contentType,
          row.status,
          Array.isArray(row.tags) ? row.tags.join(", ") : "-",
          formattedTime(row.createdAt),
          actionButton("删除", async () => {
            if (!id) { return; }
            setState(assetState, "删除中…");
            try {
              await client.executeCommand({ name: "opc_delete_content_asset", input: { id } });
              await loadAssets();
            } catch (error) {
              setState(assetState, `删除失败：${errorText(error)}`, true);
            }
          }),
        ];
      }),
    );
    if (rows.length > 0) { setState(assetState, `共 ${rows.length} 条内容资产`); }
  } catch (error) {
    setState(assetState, `加载内容资产失败：${errorText(error)}`, true);
  }
}

async function loadSchedules(): Promise<void> {
  setState(scheduleState, "加载中…");
  try {
    const rows = parseRows<PublishScheduleRow>(
      await client.executeCommand({ name: "opc_list_publish_schedules", input: {} }),
    );
    fillRows(
      "#publish-schedules",
      "暂无发布计划",
      scheduleState,
      rows.map((row) => {
        const id = String(row.id ?? "");
        return [
          `${text(row.contentRefType)} / ${text(row.contentRefId)}`,
          row.status,
          formattedTime(row.scheduledAt),
          formattedTime(row.publishedAt),
          actionButton("取消", async () => {
            if (!id) { return; }
            setState(scheduleState, "取消中…");
            try {
              await client.executeCommand({
                name: "opc_cancel_publish_schedule",
                input: { id },
              });
              await loadSchedules();
            } catch (error) {
              setState(scheduleState, `取消失败：${errorText(error)}`, true);
            }
          }),
        ];
      }),
    );
    if (rows.length > 0) { setState(scheduleState, `共 ${rows.length} 条发布计划`); }
  } catch (error) {
    setState(scheduleState, `加载发布计划失败：${errorText(error)}`, true);
  }
}

async function createBlog(form: HTMLFormElement): Promise<void> {
  const title = document.querySelector<HTMLInputElement>("#blog-title")?.value.trim() ?? "";
  const slug = document.querySelector<HTMLInputElement>("#blog-slug")?.value.trim() ?? "";
  const excerpt = document.querySelector<HTMLInputElement>("#blog-excerpt")?.value.trim() ?? "";
  const content = document.querySelector<HTMLTextAreaElement>("#blog-content")?.value ?? "";
  if (!title || !slug || !excerpt || !content.trim()) {
    setState(blogCreateState, "请填写标题、路径、摘要与正文", true);
    return;
  }
  setState(blogCreateState, "创建中…");
  try {
    await client.executeCommand({
      name: "opc_create_blog_post",
      input: { title, slug, excerpt, content, tags: [] },
    });
    setState(blogCreateState, "博客创建成功");
    form.reset();
    await loadBlogs();
  } catch (error) {
    setState(blogCreateState, `创建博客失败：${errorText(error)}`, true);
  }
}

async function createAsset(form: HTMLFormElement): Promise<void> {
  const title = document.querySelector<HTMLInputElement>("#asset-title")?.value.trim() ?? "";
  const contentType = document.querySelector<HTMLInputElement>("#asset-type")?.value.trim() ?? "";
  const body = document.querySelector<HTMLTextAreaElement>("#asset-body")?.value ?? "";
  if (!title || !contentType || !body.trim()) {
    setState(assetCreateState, "请填写标题、类型与正文", true);
    return;
  }
  setState(assetCreateState, "创建中…");
  try {
    await client.executeCommand({
      name: "opc_create_content_asset",
      input: { title, contentType, body, tags: [] },
    });
    setState(assetCreateState, "内容资产创建成功");
    form.reset();
    await loadAssets();
  } catch (error) {
    setState(assetCreateState, `创建内容资产失败：${errorText(error)}`, true);
  }
}

async function createSchedule(form: HTMLFormElement): Promise<void> {
  const contentRefType = document.querySelector<HTMLInputElement>("#schedule-type")?.value.trim() ?? "";
  const contentRefId = document.querySelector<HTMLInputElement>("#schedule-id")?.value.trim() ?? "";
  const rawTime = document.querySelector<HTMLInputElement>("#schedule-time")?.value ?? "";
  if (!contentRefType || !contentRefId || !rawTime) {
    setState(scheduleCreateState, "请填写关联类型、关联 ID 与计划时间", true);
    return;
  }
  const millis = new Date(rawTime).getTime();
  if (!Number.isFinite(millis)) {
    setState(scheduleCreateState, "计划时间格式无效", true);
    return;
  }
  setState(scheduleCreateState, "创建中…");
  try {
    await client.executeCommand({
      name: "opc_create_publish_schedule",
      input: {
        contentRefType,
        contentRefId,
        scheduledAt: Math.floor(millis / 1000),
      },
    });
    setState(scheduleCreateState, "发布计划创建成功");
    form.reset();
    await loadSchedules();
  } catch (error) {
    setState(scheduleCreateState, `创建发布计划失败：${errorText(error)}`, true);
  }
}

document.querySelector<HTMLFormElement>("#blog-form")?.addEventListener("submit", (event) => {
  event.preventDefault();
  const form = event.currentTarget;
  if (form instanceof HTMLFormElement) { void createBlog(form); }
});
document.querySelector<HTMLFormElement>("#asset-form")?.addEventListener("submit", (event) => {
  event.preventDefault();
  const form = event.currentTarget;
  if (form instanceof HTMLFormElement) { void createAsset(form); }
});
document.querySelector<HTMLFormElement>("#schedule-form")?.addEventListener("submit", (event) => {
  event.preventDefault();
  const form = event.currentTarget;
  if (form instanceof HTMLFormElement) { void createSchedule(form); }
});

void loadBlogs();
void loadAssets();
void loadSchedules();
