// SPDX-License-Identifier: AGPL-3.0-only
//
// geospatial 插件页面入口：静态域数据由 domain-data.js 渲染，这里补上 worker 的交互区。
// 所有命令都走插件 SDK 的 executeCommand，不直接 invoke 宿主 IPC。

import { createAxAgentPluginClient } from "../vendor/sdk/pluginClientBridge";

const client = createAxAgentPluginClient();
client.announceReady();

function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function render(value: unknown): string {
  if (typeof value === "string") {
    try {
      return JSON.stringify(JSON.parse(value), null, 2);
    } catch {
      return value;
    }
  }
  return JSON.stringify(value, null, 2);
}

const SAMPLES: Record<string, { command: string; input: unknown }> = {
  "OpcSearchWiki": { command: "opc_geospatial_opc_search_wiki", input: {} },
  "WebSearch": { command: "opc_geospatial_web_search", input: {"query":"搜索关键词"} },
  "OpcCreateContentAsset": { command: "opc_geospatial_opc_create_content_asset", input: {} },
  "OpcCreateProject": { command: "opc_geospatial_opc_create_project", input: {} },
  "FileWrite": { command: "opc_geospatial_file_write", input: {"path":"note.md","content":"正文"} },
  // `capabilityPackId` / `domain_pack_id` 由 worker 按自身身份固定注入；页面只提供实体字段。
  rules: {
    command: "opc_geospatial_run_automation_rules",
    input: { entityType: "geospatial", entityId: "demo" },
  },
};

/** 交互区自建在 body 末尾，避免与生成器产出的静态区模板耦合。 */
function mount(): void {
  const section = document.createElement("section");
  section.id = "geospatial-worker-panel";
  section.innerHTML = `
    <h2>域 worker 命令</h2>
    <p class="muted">命令由 opc-geospatial 的 Rust worker 声明；工具逻辑复用宿主实现。</p>
    <label>命令
      <select id="worker-command">
        <option value="OpcSearchWiki">OpcSearchWiki</option>
        <option value="WebSearch">WebSearch</option>
        <option value="OpcCreateContentAsset">OpcCreateContentAsset</option>
        <option value="OpcCreateProject">OpcCreateProject</option>
        <option value="FileWrite">FileWrite</option>
        <option value="rules">触发自动化规则</option>
      </select>
    </label>
    <label>输入（JSON 对象）
      <textarea id="worker-input" rows="4" spellcheck="false"></textarea>
    </label>
    <button type="button" id="worker-run">执行命令</button>
    <pre id="worker-output" class="worker-output">尚未执行命令</pre>
  `;
  document.body.append(section);

  const select = section.querySelector<HTMLSelectElement>("#worker-command");
  const input = section.querySelector<HTMLTextAreaElement>("#worker-input");
  const output = section.querySelector<HTMLElement>("#worker-output");
  const run = section.querySelector<HTMLButtonElement>("#worker-run");
  if (!select || !input || !output || !run) { return; }

  const fillSample = (): void => {
    const sample = SAMPLES[select.value];
    if (sample) { input.value = JSON.stringify(sample.input, null, 2); }
    output.textContent = "尚未执行命令";
  };
  select.addEventListener("change", fillSample);
  fillSample();

  run.addEventListener("click", () => void (async () => {
    const sample = SAMPLES[select.value];
    if (!sample) { return; }
    let args: unknown;
    try {
      args = JSON.parse(input.value) as unknown;
    } catch (error) {
      output.textContent = `输入不是合法 JSON：${errorText(error)}`;
      return;
    }
    if (args === null || typeof args !== "object" || Array.isArray(args)) {
      output.textContent = "输入必须是 JSON 对象";
      return;
    }
    output.textContent = `执行 ${sample.command}…`;
    try {
      const result = await client.executeCommand({ name: sample.command, input: args });
      output.textContent = `${sample.command}
${render(result)}`;
    } catch (error) {
      output.textContent = `${sample.command} 失败：${errorText(error)}`;
    }
  })());
}

mount();
