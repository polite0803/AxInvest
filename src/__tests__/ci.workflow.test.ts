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

  // ── i18n 三道互补门禁（2026-09-14 补）────────────────────────────────
  // 为什么单独守：这三道查的是**互不重叠**的东西，缺一条就有一种逃逸路径 ——
  //   ① 硬编码文案：代码里直接写死中文字面量
  //   ② key 对齐  ：JSON 语法 / zh-CN 空值 / 代码 t() 引用的 key 是否存在
  //   ③ 值未翻译  ：非 CJK locale 的值是否仍是中文（key 齐全但值是中文 ⇒ 只有这条能拦）
  // 此前 ①② 之一（check_i18n.py）**根本没被 CI 调用**，且它在非 strict 模式下**退出码恒为 0**
  // —— 即「装了门禁但门禁从不失败」。故这里同时守「被调用」与「用 strict」两件事。
  describe("i18n 门禁接线（三道互补，缺一即逃逸）", () => {
    const I18N_STEPS = [
      "Check i18n hardcoded strings",
      "Check i18n key alignment (syntax / empty values / t() key existence)",
      "Check i18n untranslated values (non-CJK locale must not contain CJK chars)",
    ];

    it("三条 i18n 门禁步骤都存在于 ci.yml", () => {
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

    it("三条 i18n 门禁都不得被静默放行（`|| true` / `continue-on-error`）", () => {
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

    it("同步护栏：本地 ci-check.mjs 也跑这三道（防本地绿、CI 红）", () => {
      const src = readFileSync(resolve(process.cwd(), "scripts/ci-check.mjs"), "utf8");
      expect(src).toContain("i18n-scan.mjs --strict");
      expect(src).toContain("check-i18n-untranslated.mjs");
      expect(src).toMatch(/check_i18n\.py\s+--strict/);
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
