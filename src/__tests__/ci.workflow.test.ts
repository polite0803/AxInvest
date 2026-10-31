// 集成测试: ci.yml 的**静态门禁**接线检查
//
// 为什么需要这个文件：
//   2026-09-14 03:05 之前，`ci.yml` **完全没有测试守卫** —— 门禁步骤被误删、
//   被改名、被降级为 `|| true` / `continue-on-error: true` 时，**没有任何测试会红**。
//   「装了门禁但门禁被绕过」比「没装门禁」更危险 —— 因为它给人已装的安全感。
//
// ⚠ 本文件**刻意不用 `it.skipIf`**：
//   反面教材是仓库里**曾经存在**的 `pr-ci.workflow.test.ts`（2026-09-14 已删除，
//   备份见 `output/backup-2026-09-14/del-pr-ci-guard/`）：它用
//   `it.skipIf(!existsSync(...))` 守 `.github/workflows/pr-ci.yml`，而那个 yml
//   早已不存在 ⇒ 它的 2 个用例**永远 skip、永远绿** = **空转守卫**
//   （比没有守卫更坏：它冒充有守卫）。删掉它就是为了不留一个「假守卫」在仓里。
//   所以这里把「ci.yml 必须存在」做成**硬断言**：文件没了就红，绝不静默跳过。
//
// 本文件断言的是**不变量**，不是逐字文本：
//   ① 三个静态门禁步骤（领域语义 / 分层护栏 / 领域本体）存在，且都**先自检再判定**
//      （自检不过则结论不可信）；
//   ② 判定用的是 **strict / 默认（拦截）模式**，不是 report-only；
//   ③ 三个步骤都位于 `frontend-check` job 内，且排在 dprint **之前**（纯 Node、秒级、早失败）；
//   ④ 反向断言：门禁步骤块内**不得**出现 `|| true` / `continue-on-error`（静默放行）；
//   ⑤ `test-e2e` job 存在、跑 `--project=chromium`、且浏览器由 Playwright 自装
//      （2026-09-14 裁定：**不恢复**「用 runner 系统 Chrome 省 165MB」那条优化 ——
//       版本随 runner 镜像漂移、不受 pin，flake 风险 > 固定成本）。

import { existsSync, readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const ymlPath = resolve(process.cwd(), ".github/workflows/ci.yml");
const hasCi = existsSync(ymlPath);

function loadCi(): string {
  return hasCi ? readFileSync(ymlPath, "utf8") : "";
}

/** 去掉整行注释 —— 注释里可能提到 `|| true` 之类的字样，会制造假阳性。 */
function stripComments(s: string): string {
  return s
    .split("\n")
    .filter((l) => !l.trim().startsWith("#"))
    .join("\n");
}

/**
 * 取出以 `- name: <name>` 开头、到下一个同级步骤（6 空格缩进的 `- `）为止的片段。
 * 找不到则返回 null（调用方断言其非空 ⇒ 步骤被删/改名时会红）。
 */
function stepBlock(yml: string, name: string): string | null {
  const start = yml.indexOf(`      - name: ${name}`);
  if (start < 0) { return null; }
  const rest = yml.slice(start + 1);
  const next = rest.search(/\n {6}- /);
  return next < 0 ? yml.slice(start) : yml.slice(start, start + 1 + next);
}

/** 取出某个 job 的片段（`<job>:` 到下一个 2 空格缩进的 `key:`）。 */
function jobBlock(yml: string, job: string): string | null {
  const start = yml.indexOf(`\n  ${job}:`);
  if (start < 0) { return null; }
  const rest = yml.slice(start + 1);
  const next = rest.slice(1).search(/\n {2}[A-Za-z][A-Za-z0-9_-]*:/);
  return next < 0 ? yml.slice(start) : yml.slice(start, start + 1 + next);
}

describe(".github/workflows/ci.yml — 静态门禁接线", () => {
  const DOMAIN_STEP = "Check domain semantics (unit registry / one-name-many-concepts)";
  const LAYER_STEP = "Check layer discipline (dependency direction / cross-layer)";
  const ONTOLOGY_STEP = "Check domain ontology consistency (authority vs copies)";

  it("ci.yml 存在（**缺失即红，不 skip** —— skip 的守卫等于空转守卫）", () => {
    expect(hasCi, `${ymlPath} 不存在 ⇒ 门禁接线无人守卫`).toBe(true);
    expect(loadCi().length).toBeGreaterThan(0);
  });

  it("领域语义门禁步骤存在，且「先自检、再 strict」", () => {
    const block = stepBlock(loadCi(), DOMAIN_STEP);
    expect(block, `找不到步骤「${DOMAIN_STEP}」—— 被删或改名了？`).not.toBeNull();
    const b = block as string;
    expect(b).toContain("check-domain-semantics.mjs --selftest");
    // ⚠ 必须是 --strict。若被降级成 report-only（去掉 --strict），门禁即失效。
    expect(b).toContain("check-domain-semantics.mjs --strict");
    // 自检必须排在判定之前，否则「自检不过也照样出结论」
    expect(b.indexOf("--selftest")).toBeLessThan(b.indexOf("--strict"));
  });

  it("分层护栏门禁步骤存在，且「先自检、再判定」", () => {
    const block = stepBlock(loadCi(), LAYER_STEP);
    expect(block, `找不到步骤「${LAYER_STEP}」—— 被删或改名了？`).not.toBeNull();
    const b = block as string;
    expect(b).toContain("check-layer-discipline.mjs --selftest");
    expect(b).toContain("check-layer-discipline.mjs");
    expect(b.indexOf("--selftest")).toBeLessThan(b.lastIndexOf("check-layer-discipline.mjs"));
    // 不得被降级为「只报告不拦截」
    expect(b).not.toContain("--report-only");
    expect(b).not.toContain("--only=");
  });

  // ── 领域本体门禁（2026-09-14 补）─────────────────────────────────────
  // 为什么单独守：三力权重原本有 7 处副本（rhai 回退值 ×2、分档阈值 ×2、
  //   seeder 变量定义 ×3、prompt 文案），且其中两处**互相矛盾**（D1：复合分
  //   在 prompt 里写成「三力/3」等权，在 rhai 里是 Σwᵢfᵢ 加权）。
  //   权威源唯一化之后，唯一能防它重新散开的机制就是这条门禁 ⇒ 它自身
  //   必须被守卫，否则「权威源」会在某次改动里悄悄退化成「又一处副本」。
  it("领域本体门禁步骤存在，且「先自检、再判定」", () => {
    const block = stepBlock(loadCi(), ONTOLOGY_STEP);
    expect(block, `找不到步骤「${ONTOLOGY_STEP}」—— 被删或改名了？`).not.toBeNull();
    const b = block as string;
    expect(b).toContain("check-ontology-consistency.mjs --selftest");
    expect(b).toContain("check-ontology-consistency.mjs");
    // 自检必须排在判定之前，否则「自检不过也照样出结论」
    expect(b.indexOf("--selftest")).toBeLessThan(b.lastIndexOf("check-ontology-consistency.mjs"));
    // 不得被降级为「只报告不拦截」
    expect(b).not.toContain("--report-only");
    expect(b).not.toContain("--only=");
  });

  it("三个门禁都挂在 frontend-check job 内（Node 可用、无需 Rust）", () => {
    const job = jobBlock(loadCi(), "frontend-check");
    expect(job, "找不到 job「frontend-check」").not.toBeNull();
    const j = job as string;
    expect(j).toContain(DOMAIN_STEP);
    expect(j).toContain(LAYER_STEP);
    expect(j).toContain(ONTOLOGY_STEP);
  });

  it("三个门禁都排在 dprint 之前（纯 Node、秒级 ⇒ 尽早失败）", () => {
    const yml = loadCi();
    const dprint = yml.indexOf("Check frontend formatting (dprint)");
    expect(dprint).toBeGreaterThan(0);
    for (const name of [DOMAIN_STEP, LAYER_STEP, ONTOLOGY_STEP]) {
      const at = yml.indexOf(name);
      expect(at, `找不到步骤「${name}」`).toBeGreaterThan(0);
      expect(at, `「${name}」排到了 dprint 之后 ⇒ 早失败失效`).toBeLessThan(dprint);
    }
  });

  it("反向断言：门禁步骤不得被静默放行（`|| true` / `continue-on-error`）", () => {
    const yml = loadCi();
    for (const name of [DOMAIN_STEP, LAYER_STEP, ONTOLOGY_STEP]) {
      const block = stepBlock(yml, name);
      expect(block, `找不到步骤「${name}」`).not.toBeNull();
      const b = stripComments(block as string);
      // 这两种写法都会让门禁「跑了但从不失败」= 等于没装
      expect(b, `${name} 用了 \`|| true\` ⇒ 门禁恒绿`).not.toMatch(/\|\|\s*true/);
      expect(b, `${name} 开了 continue-on-error ⇒ 门禁恒绿`).not.toMatch(/continue-on-error\s*:\s*true/);
    }
  });

  it("同步护栏：本地 `ci-check.mjs` 也跑同样的三条门禁（防本地绿、CI 红）", () => {
    const p = resolve(process.cwd(), "scripts/ci-check.mjs");
    expect(existsSync(p), "找不到 scripts/ci-check.mjs").toBe(true);
    const src = readFileSync(p, "utf8");
    expect(src).toContain("check-domain-semantics.mjs --selftest");
    expect(src).toContain("check-domain-semantics.mjs --strict");
    expect(src).toContain("check-layer-discipline.mjs --selftest");
    expect(src).toContain("check-layer-discipline.mjs");
    expect(src).toContain("check-ontology-consistency.mjs --selftest");
    expect(src).toContain("check-ontology-consistency.mjs");
  });

  // ── i18n **五道**互补门禁（2026-09-14 补三道；2026-10-16 扩到五道）────────────
  // 为什么单独守：这五道查的是**互不重叠**的东西，缺一条就有一种逃逸路径 ——
  //   ① 硬编码文案：代码里直接写死中文字面量
  //   ② key 对齐  ：JSON 语法 / zh-CN 空值 / 代码 t() 引用的 key 是否存在
  //   ③ 值未翻译  ：非 CJK locale 的值是否仍是中文（key 齐全但值是中文 ⇒ 只有这条能拦）
  //   ④ key 集合  ：zh-CN ↔ 其余 10 语言的**双向叶子路径差集**（2026-09-21 加）
  //   ⑤ 占位符    ：资源值与调用点对账（值==键名 / `{{x}}` 从未插值）（2026-10-01 加）
  // 此前 ①② 之一（check_i18n.py）**根本没被 CI 调用**，且它在非 strict 模式下**退出码恒为 0**
  // —— 即「装了门禁但门禁从不失败」。故这里同时守「被调用」与「用 strict」两件事。
  //
  // ⚠ 2026-10-16 补记（本描述块存在的意义）：④⑤ 加了之后**只进了 `ci.yml`，没进本地镜像**，
  //   而本 describe 当时仍只列 ①②③ ⇒ 两道门「CI 有、本地无」持续了近一个月而**没有任何东西红**。
  //   形态与文件头那两条反面教材同源：**守卫只守它被写下的那一刻已存在的门**。
  //   凡往 `ci.yml` 加 i18n 门禁，必须同时改三处：ci.yml、`ci-check.mjs`、本数组。
  describe("i18n 门禁接线（五道互补，缺一即逃逸）", () => {
    const I18N_STEPS = [
      "Check i18n hardcoded strings",
      "Check i18n key alignment (syntax / empty values / t() key existence)",
      "Check i18n untranslated values (non-CJK locale must not contain CJK chars)",
      "Check i18n key parity (all locales must expose the same key paths)",
      "Check i18n placeholder usage (value must not echo its key; {{x}} must be interpolated)",
    ];

    it("五条 i18n 门禁步骤都存在于 ci.yml", () => {
      const yml = loadCi();
      for (const name of I18N_STEPS) {
        expect(stepBlock(yml, name), `找不到门禁步骤「${name}」—— 被删或改名了？`).not.toBeNull();
      }
    });

    it("key 对齐门禁必须带 `--strict`（非 strict 模式退出码恒为 0 ⇒ 门禁形同虚设）", () => {
      const block = stepBlock(loadCi(), I18N_STEPS[1]);
      expect(block, `找不到步骤「${I18N_STEPS[1]}」`).not.toBeNull();
      expect(block as string).toMatch(/check_i18n\.py\s+--strict/);
    });

    it("五条 i18n 门禁都不得被静默放行（`|| true` / `continue-on-error`）", () => {
      const yml = loadCi();
      for (const name of I18N_STEPS) {
        const block = stepBlock(yml, name);
        expect(block, `找不到步骤「${name}」`).not.toBeNull();
        const b = stripComments(block as string);
        expect(b, `${name} 用了 \`|| true\` ⇒ 门禁恒绿`).not.toMatch(/\|\|\s*true/);
        expect(b, `${name} 开了 continue-on-error ⇒ 门禁恒绿`).not.toMatch(
          /continue-on-error\s*:\s*true/,
        );
      }
    });

    it("同步护栏：本地 ci-check.mjs 也跑这五道（防本地绿、CI 红）", () => {
      const src = readFileSync(resolve(process.cwd(), "scripts/ci-check.mjs"), "utf8");
      expect(src).toContain("i18n-scan.mjs --strict");
      expect(src).toContain("check-i18n-untranslated.mjs");
      expect(src).toMatch(/check_i18n\.py\s+--strict/);
      // ④⑤ —— 2026-10-16 补：这两道此前**只在 ci.yml 里**，本地镜像没有（见本 describe 顶部批注）
      expect(src).toContain("check-i18n-key-parity.mjs");
      expect(src).toContain("check-i18n-placeholder-usage.mjs");
    });
  });

  // ── E2E job 接线（2026-09-14 补）────────────────────────────────────
  // 为什么单独守：`test-e2e` job 此前**没有任何测试断言其存在** ——
  // 删掉 job / 改 `runs-on` / 把 `--grep-invert` 扩成吞掉全部用例，
  // **不会有任何东西红**。这与「`ci.yml` 本身无守卫」是同一个病（见文件头）。
  //
  // 并存记录一条**裁决**（2026-09-14，用户裁定「系统 Chrome 不要恢复」）：
  //   原 `pr-ci.workflow.test.ts`（已删）曾断言「macOS runner 用系统 Chrome、
  //   跳过 165MB Chromium 下载」。该优化**不恢复** —— runner 自带 Chrome 的版本
  //   随镜像漂移、不由我们 pin，flake 风险大于那 165MB 的固定成本。
  //   故这里反过来断言 `npx playwright install` **必须存在**：
  //   谁把 install 步骤删掉改用 runner Chrome，这条会红 ⇒ 逼其回到本裁决。
  describe("E2E job 接线（test-e2e）", () => {
    const E2E_JOB = "test-e2e";

    it("job 存在且跑在 macos-latest（**硬断言，不 skip**）", () => {
      const job = jobBlock(loadCi(), E2E_JOB) ?? "";
      expect(job, `找不到 job「${E2E_JOB}」—— 被删或改名了？`).not.toBe("");
      expect(job).toMatch(/runs-on:\s*macos-latest/);
    });

    it("真正执行 Playwright（chromium 项目），且过滤条件不会退化为「零用例」", () => {
      const job = jobBlock(loadCi(), E2E_JOB) ?? "";
      expect(job, `找不到 job「${E2E_JOB}」`).not.toBe("");
      expect(job).toMatch(/npx playwright test\b/);
      expect(job).toMatch(/--project=chromium/);
      // `--grep-invert ""` / `--grep-invert ".*"` ⇒ 排除全部用例 ⇒ 跑 0 个测试却退出码 0
      const m = /--grep-invert\s+"([^"]*)"/.exec(job);
      expect(m, `E2E 缺 \`--grep-invert "…"\`（空或通配 = 排除全部 ⇒ 空转）`).not.toBeNull();
      const pat = (m ?? ["", ""])[1].trim();
      expect(pat.length, "`--grep-invert` 为空串 ⇒ 排除全部用例，E2E 空转").toBeGreaterThan(0);
      expect(pat, "`--grep-invert` 为 `.*` ⇒ 排除全部用例，E2E 空转").not.toBe(".*");
    });

    it("浏览器由 Playwright 自装（**不依赖 runner 自带 Chrome** —— 2026-09-14 裁决）", () => {
      const job = jobBlock(loadCi(), E2E_JOB) ?? "";
      expect(job, "缺 `npx playwright install` ⇒ 退化为依赖 runner 自带 Chrome（已被否决）").toMatch(
        /npx playwright install\b/,
      );
    });

    it("反向断言：E2E job 不得被静默放行（`|| true` / `continue-on-error`）", () => {
      const job = jobBlock(loadCi(), E2E_JOB) ?? "";
      expect(job, `找不到 job「${E2E_JOB}」`).not.toBe("");
      const b = stripComments(job);
      expect(b, "test-e2e 用了 `|| true` ⇒ E2E 恒绿").not.toMatch(/\|\|\s*true/);
      expect(b, "test-e2e 开了 continue-on-error ⇒ E2E 恒绿").not.toMatch(
        /continue-on-error\s*:\s*true/,
      );
    });
  });

  it("结构完整：步骤缩进统一为 6 空格、无错位 run", () => {
    const lines = loadCi().split(/\r?\n/);
    const bad: string[] = [];
    lines.forEach((l, i) => {
      if (/^\s*- name: /.test(l) && !/^ {6}- name: /.test(l)) { bad.push(`${i + 1}: ${l.trim().slice(0, 60)}`); }
      if (/^\s+run: /.test(l) && !/^ {8}run: /.test(l)) { bad.push(`${i + 1}: ${l.trim().slice(0, 60)}`); }
    });
    expect(bad, `缩进异常（会导致 YAML 解析成别的结构）：\n${bad.join("\n")}`).toHaveLength(0);
  });
});

// ── #5 自动提取护栏（2026-10-31）─────────────────────────────────────────────
// 为什么需要：上面那两段「同步护栏」是**手写白名单**，只守 7 个门；ci.yml 实有约 60 个
//   scripts/** 门，≈80% 不受保护。④⑤「只进了 ci.yml、没进本地镜像」持续近一个月无人红
//   （见上方 i18n describe 批注）—— 手写清单挡不住「漏登记」。
// 形态：从两侧**自动提取** scripts/** 域命令（ci.yml 的 run 行 ↔ ci-check.mjs 的
//   step() 第二参），归一化后做**对称差集**。任一侧新增命令而另一侧没有 ⇒ 红。
// 口径（qa-verifier 规格二终稿）：
//   · 只比 `scripts/**` 域 —— dprint / oxlint / tsc / vitest / cargo 两侧拼写不同
//     （`npx dprint check` vs `dprint check --incremental=false` 等），v1 不纳入；
//   · canon()：去解释器/env 前缀 → 折叠空白 → 参数 token 排序（`--tier all --strict`
//     ≡ `--strict --tier all`）；
//   · 已知不对称必须以 {ciYml, ciCheck, reason} 写进 allowlist（≤8 条，无 reason 即红，
//     失效不删也红 —— 防 allowlist 腐化成新白名单，pr-ci.workflow.test.ts 教训）；
//   · 分母非零断言（提取器写坏 ⇒ 差集恒空 = 空转守卫）。
describe("ci.yml ↔ ci-check.mjs 命令清单自动提取比对（对称差集）", () => {
  const MIN_COMMANDS_PER_SIDE = 40;
  const MAX_ALLOWLIST = 8;

  /** 去掉 `//` 行注释与 `/* *\/` 块注释（`[^:]` 防吃掉 `node:fs` 这类协议前缀）。 */
  function stripJsComments(s: string): string {
    const noBlock = s.replace(/\/\*[\s\S]*?\*\//g, " ");
    return noBlock
      .split("\n")
      .map((l) => {
        const m = /(^|[^:])\/\//.exec(l);
        return m ? l.slice(0, (m.index ?? 0) + (m[1] ? m[1].length : 0)) : l;
      })
      .join("\n");
  }

  /**
   * 从 ci.yml 提取 `scripts/**` 命令：逐行匹配解释器 + 脚本路径 + 参数。
   * 允许 `run: node …`、`NODE_PATH=… node …`、`&&` 串联等形态（不锚行首 ——
   * `- run: node …` 单行形态是真实存在的主流写法，锚行首会整批漏抓）；
   * 整行注释已在 stripComments 剥除。
   */
  function extractCiYmlCommands(text: string): Set<string> {
    const out = new Set<string>();
    const re =
      /(?:[A-Za-z_][A-Za-z0-9_]*=\S+\s+)*(?:node|bash|sh|python3?|npx)\s+(scripts\/[A-Za-z0-9_./-]+\.(?:mjs|sh|py|ts)[^\n"'|;&\\]*(?:\s+[^"'|;&\\\n]+)*)/g;
    for (const raw of stripComments(text).split("\n")) {
      let m: RegExpExecArray | null;
      re.lastIndex = 0;
      while ((m = re.exec(raw)) !== null) {
        const canon = canonCommand(m[1]);
        if (canon.startsWith("scripts/")) { out.add(canon); }
      }
    }
    return out;
  }

  /** 从 ci-check.mjs 提取 step() 的第二个参数（label/cmd 接受 " ' ` 三种引号形态）。 */
  function extractCiCheckCommands(text: string): Set<string> {
    const out = new Set<string>();
    const src = stripJsComments(text);
    const BT = "`";
    // 单个字符串字面量：双引号 / 单引号 / 反引号（不含未转义的同种引号与换行）
    const str = `(?:"(?:[^"\\\\]|\\\\.)*"|'(?:[^'\\\\]|\\\\.)*'|${BT}(?:[^${BT}\\\\]|\\\\.)*${BT})`;
    const re = new RegExp(`step\\(\\s*${str}\\s*,\\s*(${str})`, "g");
    let m: RegExpExecArray | null;
    while ((m = re.exec(src)) !== null) {
      const quoted = m[1];
      const canon = canonCommand(quoted.slice(1, -1));
      // 只比 scripts/** 域（cargo/npx/vitest/tsc 两侧拼写不同，v1 不纳入 —— 见规格）
      if (canon.startsWith("scripts/")) { out.add(canon); }
    }
    return out;
  }

  /** 归一化：剥解释器/env/`${var}`/`./` 前缀 → 折叠空白 → 脚本路径之后的参数 token 排序。 */
  function canonCommand(cmd: string): string {
    let s = cmd.trim().replace(/\\+$/, "").trim();
    // 反复剥前缀：env 赋值、解释器、${var}、./
    for (let i = 0; i < 6; i++) {
      const before = s;
      s = s
        .replace(/^(?:[A-Za-z_][A-Za-z0-9_]*=\S+\s+)+/, "")
        .replace(/^(?:node|bash|sh|python3|python|npx)\s+/, "")
        .replace(/^\$\{[A-Za-z_][A-Za-z0-9_]*\}\s+/, "")
        .replace(/^\.?\//, "");
      if (s === before) { break; }
    }
    const tokens = s.split(/\s+/).filter(Boolean).map((t) => t.replace(/\\+$/, ""));
    if (tokens.length === 0) { return ""; }
    const [script, ...args] = tokens;
    // 行尾连接符（\ | ; &）在参数位置不该出现，出现即剥
    const cleanArgs = args.filter((a) => a !== "\\" && a !== "|" && a !== ";" && a !== "&");
    return [script, ...cleanArgs.sort()].join(" ");
  }

  /** 纯函数：两侧命令集合的对称差集（未套 allowlist 的原始差集）。 */
  function diffCommands(
    ciYmlText: string,
    ciCheckText: string,
  ): { ymlOnly: string[]; checkOnly: string[] } {
    const yml = extractCiYmlCommands(ciYmlText);
    const check = extractCiCheckCommands(ciCheckText);
    const ymlOnly = [...yml].filter((c) => !check.has(c)).sort();
    const checkOnly = [...check].filter((c) => !yml.has(c)).sort();
    return { ymlOnly, checkOnly };
  }

  // 已知不对称（每条必须带 reason；两边命令取 canon 形态）。
  const ALLOWLIST: { ciYml: string | null; ciCheck: string | null; reason: string }[] = [
    {
      ciYml: "scripts/check-hardcoded-i18n.sh --strict",
      ciCheck: "scripts/i18n-scan.mjs --strict",
      reason: "同一门两种实现：CI 用 .sh，本地 node 直调 i18n-scan.mjs（Windows 无 bash，见 ci-check.mjs i18n 段注释）",
    },
    {
      ciYml: "scripts/check-line-endings.sh",
      ciCheck: null,
      reason:
        "本地镜像没有行尾门（ci-check.mjs 头部自陈缺 9 类之一）；本机无 bash 时可用 node scripts/check-line-endings.mjs 同核心替代",
    },
    {
      ciYml: "scripts/check-ngram-consistency.mjs --only=consistency",
      ciCheck: "scripts/check-ngram-consistency.mjs",
      reason: "同门不同参数：CI 库是干净的（无 v227 迁移），限定 consistency 子集；本地跑全量",
    },
    {
      ciYml: null,
      ciCheck: "scripts/check-accent-foreground.mjs --selftest",
      reason: "本地跑、CI 不跑（台账类别 2）：主题前景 token 门只挂在本地镜像",
    },
    {
      ciYml: null,
      ciCheck: "scripts/check-accent-foreground.mjs",
      reason: "本地跑、CI 不跑（台账类别 2）：主题前景 token 门只挂在本地镜像",
    },
    {
      ciYml: null,
      ciCheck: "scripts/check-decision-validation-integrity.mjs",
      reason: "本地跑、CI 不跑（台账类别 2）：决策验证表引用完整性门只挂在本地镜像（需本机 PG）",
    },
    {
      ciYml: null,
      ciCheck: "scripts/check-replay-live-consistency.mjs",
      reason: "本地跑、CI 不跑（台账类别 2）：回放↔实盘一致性门只挂在本地镜像（需本机 PG）",
    },
  ];

  it("ci.yml 与 ci-check.mjs 都存在（硬断言，不 skip）", () => {
    expect(hasCi, `${ymlPath} 不存在`).toBe(true);
    expect(existsSync(resolve(process.cwd(), "scripts/ci-check.mjs")), "scripts/ci-check.mjs 不存在").toBe(true);
  });

  it("提取分母非零：两侧各 > 40 条（提取器写坏 ⇒ 差集恒空 = 空转守卫）", () => {
    const yml = extractCiYmlCommands(loadCi());
    const check = extractCiCheckCommands(readFileSync(resolve(process.cwd(), "scripts/ci-check.mjs"), "utf8"));
    expect(
      yml.size,
      `ci.yml 只提取到 ${yml.size} 条 scripts 命令（实测基线 >90）—— 提取器或文件被改坏`,
    ).toBeGreaterThan(MIN_COMMANDS_PER_SIDE);
    expect(
      check.size,
      `ci-check.mjs 只提取到 ${check.size} 条 step 命令（实测基线 >90）—— 提取器或文件被改坏`,
    ).toBeGreaterThan(MIN_COMMANDS_PER_SIDE);
  });

  it("提取器负控：ci.yml 侧独有的合成命令必须出现在差集里", () => {
    const d = diffCommands(
      "jobs:\n  a:\n    steps:\n      - run: node scripts/zzz-new-gate.mjs --strict\n",
      'step("已有", "node scripts/known.mjs")\n',
    );
    expect(d.ymlOnly).toContain("scripts/zzz-new-gate.mjs --strict");
  });

  it("提取器负控：ci-check 侧独有的合成命令必须出现在差集里（反方向）", () => {
    const d = diffCommands(
      "- run: node scripts/known.mjs\n",
      'step(\n  "新门",\n  `node scripts/zzz-other-gate.mjs`,\n);\n',
    );
    expect(d.checkOnly).toContain("scripts/zzz-other-gate.mjs");
  });

  it("canon 负控：参数顺序 / env 前缀 / 解释器差异必须归一为同一条", () => {
    const yml = extractCiYmlCommands("- run: NODE_PATH=/x node scripts/foo.mjs --strict --tier all\n");
    const check = extractCiCheckCommands('step("x", `${pythonBin} scripts/foo.mjs --tier all --strict`)\n');
    expect([...yml][0]).toBe("scripts/foo.mjs --strict --tier all");
    expect([...check][0]).toBe("scripts/foo.mjs --strict --tier all");
  });

  it("allowlist 条数 ≤ 8 且每条必须带 reason（防白名单再生 / 腐化）", () => {
    expect(ALLOWLIST.length).toBeLessThanOrEqual(MAX_ALLOWLIST);
    for (const e of ALLOWLIST) {
      expect(e.reason.trim().length, `allowlist 条目缺 reason：${JSON.stringify(e)}`).toBeGreaterThan(4);
      expect(e.ciYml !== null || e.ciCheck !== null, "allowlist 条目两侧皆空").toBe(true);
    }
  });

  it("真实数据：对称差集必须恰好等于 allowlist（多一条 = 漂移，红）", () => {
    const d = diffCommands(loadCi(), readFileSync(resolve(process.cwd(), "scripts/ci-check.mjs"), "utf8"));
    const allowYml = new Set(ALLOWLIST.map((e) => e.ciYml).filter((x): x is string => x !== null));
    const allowCheck = new Set(ALLOWLIST.map((e) => e.ciCheck).filter((x): x is string => x !== null));
    const unexplainedYml = d.ymlOnly.filter((c) => !allowYml.has(c));
    const unexplainedCheck = d.checkOnly.filter((c) => !allowCheck.has(c));
    expect(
      unexplainedYml,
      "ci.yml 有、ci-check.mjs 没有的门（本地绿会冒充 CI 绿）；若是新门请同步加进本地镜像，若是已知不对称请进 ALLOWLIST 并写 reason",
    ).toEqual([]);
    expect(
      unexplainedCheck,
      "ci-check.mjs 有、ci.yml 没有的门（本地严 CI 松，违规可绕过 CI 合入）；若是新门请同步加进 ci.yml，若是已知不对称请进 ALLOWLIST 并写 reason",
    ).toEqual([]);
  });

  it("allowlist 防腐化：每条都必须仍命中真实差集（失效即红，逼人删除）", () => {
    const d = diffCommands(loadCi(), readFileSync(resolve(process.cwd(), "scripts/ci-check.mjs"), "utf8"));
    const ymlSet = new Set(d.ymlOnly);
    const checkSet = new Set(d.checkOnly);
    for (const e of ALLOWLIST) {
      const hitYml = e.ciYml === null || ymlSet.has(e.ciYml);
      const hitCheck = e.ciCheck === null || checkSet.has(e.ciCheck);
      expect(
        hitYml && hitCheck,
        `allowlist 条目已失效（对称差集里已不存在）：ciYml=${e.ciYml} ciCheck=${e.ciCheck} —— 请删除该条目，不要留着糊弄下一代`,
      ).toBe(true);
    }
  });
});
