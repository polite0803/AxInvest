// SPDX-License-Identifier: AGPL-3.0-only
/** sales_growth 页面 bundle 的最小静态校验：三条命令只能走 executeCommand。 */
import { readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const bundle = readFileSync(join(root, "web/pages/bundle.js"), "utf8");
const commands = ["opc_create_customer", "opc_list_landing_pages", "opc_list_contacts"];
for (const command of commands) {
  if (!bundle.includes(`"${command}"`)) { throw new Error(`bundle 未包含命令 ${command}`); }
}
if (/client\.invoke\(\s*["']opc_/.test(bundle)) {
  throw new Error('sales_growth 页面不得使用 client.invoke("opc_*")');
}
console.log("PASS: sales_growth bundle 三条命令均走 executeCommand");
