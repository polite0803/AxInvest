// SPDX-License-Identifier: AGPL-3.0-only
/** finance_invest 页面 bundle 的最小静态校验：页面命令只能走 executeCommand。 */
import { readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const bundle = readFileSync(join(root, "web/pages/bundle.js"), "utf8");
const commands = [
  "opc_finance_invest_opc_get_dashboard",
  "opc_finance_invest_opc_list_kpis",
  "opc_finance_invest_opc_search_wiki",
  "opc_finance_invest_opc_list_projects",
  "opc_finance_invest_opc_get_financial_report",
  "opc_finance_invest_opc_record_kpi",
  "opc_finance_invest_batch_validate_entities",
  "opc_finance_invest_opc_list_customers",
  "opc_finance_invest_web_search",
  "opc_finance_invest_run_automation_rules",
];
for (const command of commands) {
  if (!bundle.includes(`"${command}"`)) { throw new Error(`bundle 未包含命令 ${command}`); }
}
if (/client\.invoke\(\s*["']opc_/.test(bundle)) {
  throw new Error('finance_invest 页面不得使用 client.invoke("opc_*")');
}
console.log("PASS: finance_invest bundle 页面命令均走 executeCommand");
