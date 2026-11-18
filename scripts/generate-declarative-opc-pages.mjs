#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-only
/**
 * 从主仓 `config/opc/domain_packs/**` 生成声明式 OPC 插件页面数据。
 *
 * 产物随插件 workspace 提交，所以拷出独立 SDK 树后打包不再依赖主仓 `config/`。
 * `--check` 用同一生成结果与盘上文件逐字比较；YAML 改了而生成物未更新时 CI 会红。
 */
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, relative, resolve } from "node:path";
import { parse } from "yaml";

const ROOT = resolve(import.meta.dirname, "..");
const REGISTRY = join(ROOT, "axagent-plugins", "scripts", "opc-domain-pack-plugins.json");
const argv = process.argv.slice(2);
const CHECK = argv.includes("--check");

const PAGE_HTML = `<!doctype html>
<html lang="zh">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>OPC 域包</title>
    <style>
      :root { color-scheme: light dark; font-family: system-ui, sans-serif; }
      body { margin: 0; padding: 16px; font-size: 13px; line-height: 1.6; }
      h1 { font-size: 16px; margin: 0 0 4px; }
      h2 { font-size: 14px; margin: 18px 0 8px; }
      .muted { color: #888; }
      .grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(240px, 1fr)); gap: 16px; }
      section { padding: 12px; border: 1px solid #8884; border-radius: 6px; }
      ul, ol { margin: 0; padding-left: 20px; }
      li { margin: 3px 0; }
      .critical { color: #d4380d; }
    </style>
  </head>
  <body>
    <h1 id="title">OPC 域包</h1>
    <p id="description" class="muted"></p>
    <div class="grid">
      <section><h2>能力</h2><ul id="capabilities"></ul></section>
      <section><h2>KPI</h2><ul id="kpis"></ul></section>
      <section><h2>工作流步骤</h2><ol id="workflow-steps"></ol></section>
      <section><h2>数据源</h2><ul id="data-sources"></ul></section>
      <section><h2>验收标准</h2><ul id="acceptance"></ul></section>
    </div>
    <script src="./domain-data.js"></script>
    <script>
      const data = globalThis.AX_OPC_DOMAIN_DATA;
      if (!data) {
        document.body.insertAdjacentHTML("beforeend", '<p class="critical">页面数据加载失败</p>');
      } else {
        document.title = "OPC " + data.name;
        document.querySelector("#title").textContent = "OPC " + data.name;
        document.querySelector("#description").textContent = data.description;
        const list = (selector, rows, render) => {
          const root = document.querySelector(selector);
          if (!rows.length) {
            const empty = document.createElement("li");
            empty.className = "muted";
            empty.textContent = "未声明";
            root.append(empty);
            return;
          }
          for (const row of rows) {
            const item = document.createElement(root.tagName === "OL" ? "li" : "li");
            item.textContent = render(row);
            root.append(item);
          }
        };
        list("#capabilities", data.capabilities, (x) => x);
        list("#kpis", data.kpis, (x) => x.name + " (" + x.key + ")" + (x.unit ? " · " + x.unit : ""));
        list("#workflow-steps", data.workflowSteps, (x) => x.name + (x.description ? " · " + x.description : ""));
        list("#data-sources", data.dataSources, (x) => x.id + (x.chain.length ? " · " + x.chain.join(" → ") : ""));
        list("#acceptance", data.acceptanceCriteria, (x) => x.name + (x.critical ? " · 关键" : ""));
      }
    </script>
  </body>
</html>
`;

const LIMIT = 30;

function readYaml(path) {
  return existsSync(path) ? parse(readFileSync(path, "utf8")) ?? {} : {};
}

function strings(value) {
  return Array.isArray(value) ? value.filter((item) => typeof item === "string") : [];
}

function buildData(domainPackId, capabilities) {
  const dir = join(ROOT, "config", "opc", "domain_packs", domainPackId);
  const manifest = readYaml(join(dir, "manifest.yaml"));
  const runtime = readYaml(join(dir, "runtime.yaml"));
  const learning = readYaml(join(dir, "learning.yaml"));
  const analysis = readYaml(join(dir, "analysis.yaml"));
  const adapter = learning.adapter ?? {};
  return {
    id: domainPackId,
    name: typeof manifest.name === "string" ? manifest.name : domainPackId,
    description: typeof manifest.description === "string" ? manifest.description : "",
    domain: typeof manifest.domain === "string" ? manifest.domain : "",
    capabilities: strings(capabilities).slice(0, LIMIT),
    kpis: (Array.isArray(runtime.kpi_definitions) ? runtime.kpi_definitions : [])
      .slice(0, LIMIT)
      .map((item) => ({
        key: typeof item?.key === "string" ? item.key : "",
        name: typeof item?.name === "string" ? item.name : "",
        unit: typeof item?.unit === "string" ? item.unit : "",
      }))
      .filter((item) => item.key),
    workflowSteps: (Array.isArray(runtime.workflow_steps) ? runtime.workflow_steps : [])
      .slice(0, LIMIT)
      .map((item) => ({
        id: typeof item?.id === "string" ? item.id : "",
        name: typeof item?.name === "string" ? item.name : "",
        description: typeof item?.description === "string" ? item.description : "",
      }))
      .filter((item) => item.id),
    dataSources: (Array.isArray(analysis.data_sources) ? analysis.data_sources : [])
      .slice(0, LIMIT)
      .map((item) => ({
        id: typeof item?.id === "string" ? item.id : "",
        chain: strings(item?.chain),
      }))
      .filter((item) => item.id),
    acceptanceCriteria: (Array.isArray(adapter.acceptance_criteria) ? adapter.acceptance_criteria : [])
      .slice(0, LIMIT)
      .map((item) => ({
        id: typeof item?.id === "string" ? item.id : "",
        name: typeof item?.name === "string" ? item.name : "",
        description: typeof item?.description === "string" ? item.description : "",
        critical: item?.is_critical === true,
      }))
      .filter((item) => item.id),
  };
}

function renderData(data) {
  return `globalThis.AX_OPC_DOMAIN_DATA = ${JSON.stringify(data, null, 2)};\n`;
}

function writeOrCheck(path, content, problems) {
  if (CHECK) {
    const actual = existsSync(path) ? readFileSync(path, "utf8") : "";
    if (actual !== content) { problems.push(`${relative(ROOT, path)} 已过期，请运行 node scripts/generate-declarative-opc-pages.mjs`); }
    return;
  }
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, content);
}

function main() {
  const registry = JSON.parse(readFileSync(REGISTRY, "utf8"));
  const problems = [];
  const declarative = registry.policy.verticalSlices.filter((slice) => slice.manifestStrategy === "declarative-only");
  for (const slice of declarative) {
    const pack = registry.packs.find((item) => item.id === slice.domainPackId);
    if (!pack) {
      problems.push(`底册缺少域包 ${slice.domainPackId}`);
      continue;
    }
    const pageDir = join(ROOT, "axagent-plugins", "crates", slice.pluginId, "web", "pages");
    writeOrCheck(join(pageDir, "index.html"), PAGE_HTML, problems);
    writeOrCheck(join(pageDir, "domain-data.js"), renderData(buildData(pack.id, pack.capabilities)), problems);
  }
  if (problems.length) {
    for (const problem of problems) { console.error(`FAIL: ${problem}`); }
    process.exit(1);
  }
  if (CHECK) {
    console.log(`PASS: ${declarative.length} 个声明式 OPC 页面数据与权威 YAML 一致`);
  } else {
    console.log(`已生成 ${declarative.length} 个声明式 OPC 页面数据`);
  }
}

main();
