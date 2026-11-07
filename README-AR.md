# AxInvest

**منصة عمل لبحوث الاستثمار في أسهم A، محلية أولاً**：تطبيق سطح مكتب مبني على Tauri 2 + Rust، يجمع استرجاع بيانات السوق والأساسيات، وقرارات الأسهم على أربعة أطر زمنية، واختيار الأسهم والمراجعة، ومحافظ المحاكاة، والاختبار الخلفي الكمي، والمهام الآلية، والتعاون مع الذكاء الاصطناعي في مساحة عمل واحدة. تُحفظ جميع بيانات الأعمال افتراضياً في SQLite محلية، وتُشفَّر مفاتيح API باستخدام AES-256-GCM، ولا تعتمد العمليات الأساسية على خدمات سحابية تابعة لجهات خارجية.

> **إصدارات اللغة**：[English](./README-EN.md) | [繁體中文](./README-ZH-TW.md) | [日本語](./README-JA.md) | [한국어](./README-KO.md) | [Français](./README-FR.md) | [Deutsch](./README-DE.md) | [Español](./README-ES.md) | [Русский](./README-RU.md) | [हिन्दी](./README-HI.md) | [العربية](./README-AR.md)

---

## مكانة المشروع

AxInvest مشتق من [`polite0803/AxAgent`](https://github.com/polite0803/AxAgent)، حيث يرث الأساس العام للذكاء الاصطناعي من المشروع الأصلي، بينما تتطور قدرات أعمال الاستثمار في هذا المستودع. وهو موجَّه نحو「سلسلة بحث استثماري قابلة للمراجعة」، لا نحو مجموعة من الميزات：

- **رؤية زمنية واحدة**：`live` و`as-of` تسريان عبر جميع قدرات الاستثمار، ويمكن إعادة تشغيل الدفعات التاريخية كما هي، ويمكن معايرة العينات.
- **سلسلة قرار واحدة**：من بناء مجموعة المرشحين، إلى قرارات مستقلة على أربعة أطر زمنية، ثم الحفظ في قاعدة البيانات والإسناد والتطور عبر التأمل، هناك معيار واحد فقط.
- **محلي أولاً**：حفظ افتراضي في SQLite، دون أي اعتماد على السحابة، وتبقى البيانات والمفاتيح على الجهاز المحلي.

---

## دليل الميزات (16 نقطة ميزة)

مدخل `docs/` هو الفهرس ثنائي العرض [docs/INDEX.md](./docs/INDEX.md)：يُنظَّم عرض الميزات وفق 16 نقطة ميزة، ويُصنَّف عرض الأرشيف حسب الخطة / المخطط / التدقيق / المرجع، وقد سُجِّل حتى الآن 265 وثيقة. لكل نقطة ميزة دليل يوضح بوضوح السلوك الحالي، ومدخل الكود، ومفاتيح الإعداد، والقيود المعروفة، وسجل تغييرات الإصدار.

**نطاقات الأعمال (9)**

| نقطة الميزة                    | slug                   | الدليل                                                             |
| ------------------------------ | ---------------------- | ------------------------------------------------------------------ |
| سلسلة قرارات الأسهم             | `stock-decision-chain` | [stock-decision-chain.md](./docs/features/stock-decision-chain.md) |
| الاختيار الذكي للاتجاه           | `serenity-screening`   | [serenity-screening.md](./docs/features/serenity-screening.md)     |
| إعادة التشغيل بالسفر عبر الزمن | `time-travel-replay`   | [time-travel-replay.md](./docs/features/time-travel-replay.md)     |
| بيانات السوق ومصادر البيانات   | `market-data`          | [market-data.md](./docs/features/market-data.md)                   |
| حزم قدرات OPC                  | `opc-capability-packs` | [opc-capability-packs.md](./docs/features/opc-capability-packs.md) |
| الاختبار الخلفي الكمي           | `quant-backtest`       | [quant-backtest.md](./docs/features/quant-backtest.md)             |
| التأمل والتطور                 | `reflection-evolution` | [reflection-evolution.md](./docs/features/reflection-evolution.md) |
| مركز المهام والجدولة           | `task-center`          | [task-center.md](./docs/features/task-center.md)                   |
| المحفظة الافتراضية وجسر التداول | `paper-portfolio`      | [paper-portfolio.md](./docs/features/paper-portfolio.md)           |

**نطاقات المنصة (7)**

| نقطة الميزة                   | slug                 | الدليل                                                         |
| ----------------------------- | -------------------- | -------------------------------------------------------------- |
| نظام الإضافات                  | `plugin-system`      | [plugin-system.md](./docs/features/plugin-system.md)           |
| جودة البيانات وبوابات التحقق  | `data-quality-gates` | [data-quality-gates.md](./docs/features/data-quality-gates.md) |
| بيئة التشغيل والوكلاء          | `agent-runtime`      | [agent-runtime.md](./docs/features/agent-runtime.md)           |
| مزودو النماذج والبث المتدفق   | `provider-models`    | [provider-models.md](./docs/features/provider-models.md)       |
| التخزين وترحيل قاعدة البيانات | `storage-migration`  | [storage-migration.md](./docs/features/storage-migration.md)   |
| قشرة الواجهة الأمامية          | `ui-shell`           | [ui-shell.md](./docs/features/ui-shell.md)                     |
| البناء و CI                   | `build-ci`           | [build-ci.md](./docs/features/build-ci.md)                     |

---

## البدء السريع

المتطلبات：Node.js ≥ 24، وRust ≥ 1.85 (edition 2024)، وnpm ≥ 10. على Windows يلزم MSVC Build Tools، وعلى macOS يلزم Xcode Command Line Tools، وعلى Linux يلزم `build-essential` و`libwebkit2gtk-4.1-dev` و`libssl-dev`.

```bash
npm install
npm run dev           # وضع المتصفح（تعمل الواجهة الأمامية على localStorage mock، دون الحاجة إلى Tauri）
npm run tauri dev     # تطبيق سطح المكتب（Vite HMR + نافذة Tauri）
```

البناء والمخرجات：

```bash
npm run build         # فحص الأنواع + بناء الإنتاج للواجهة الأمامية
npm run tauri:build   # حزمة التثبيت لسطح المكتب
```

---

## طبقات البنية (8 جذور workspace)

يُقسَّم الجزء الخاص بـ Rust وفق اتجاه التبعيات إلى **8 جذور Cargo workspace**، بإجمالي **42 crate مكتبة**، ويكون `src-tauri` جذر التركيب（crate ثنائي رئيسي + `axagent-mobile`）. قاعدة اتجاه التبعيات：`المكوّن → harness ← التنفيذ`.

| جذر workspace     | الدور                      | عدد crates | المحتوى الرئيسي                                                                                      |
| ----------------- | -------------------------- | ---------- | ---------------------------------------------------------------------------------------------------- |
| `axagent-core`    | foundation                 | 7          | `harness` trait و DTO, `entities`, `plugin-proto`, `disk-cache`, `rt-theme`                          |
| `axagent-kit`     | implementor                | 1          | طبقة التنفيذ المشتركة `kit`                                                                          |
| `axagent-ai`      | consumer + implementor     | 6          | `agent`, `orchestrator`, `providers`, `mcp`, `prompt-guard`, `trajectory`                            |
| `axagent-infra`   | implementor                | 12         | `dao`, `storage`, `migration`, `crypto`, `search`, `telemetry`, `document-parser` وغيرها             |
| `axagent-quant`   | implementor / consumer     | 5          | `astock-data`, `analysis-engine`, `company-runtime`, `quant`, `market-sim`                           |
| `axagent-plugins` | implementor + foundation   | 2          | `plugins`, `schema-gen`                                                                              |
| `axagent-runtime` | wiring + consumer + hybrid | 8          | `runtime`, `runtime-core`, `tools`, `rt-workflow`, `rt-messaging`, `gateway`, `crdt`, `notification` |
| `src-tauri`       | wiring                     | 2          | أوامر Tauri، والتهيئة والتجميع، و`axagent-mobile`                                                    |

الواجهة الأمامية تطبيق أحادي الصفحة مبني على Vite：تُقسَّم `src/` وفق `pages / components / stores / hooks / lib / types / i18n`، ويُقسَّم Zustand store بدوره إلى أربع طبقات：`domain / feature / shared / devtools`.

انظر [AGENTS.md](./AGENTS.md) في قسم「خلفية Rust：مبادئ بنية Harness」لمعرفة الانتماء الطبقي ومجموعات التبعيات المسموح بها وتسجيل دور كل crate.

---

## بوابات التحقق

```bash
npm run format:check                          # فحص تنسيق dprint ونهايات الأسطر
npm run lint:eslint                           # oxlint（ESLint لا يدعم TS 7）
npm run typecheck                             # tsc --noEmit
npm run test:run                              # تشغيل Vitest مرة واحدة
npm run test:e2e                              # Playwright E2E
npm run test:rust                             # اختبارات Rust（على Windows باستخدام __TAURI_WORKSPACE__=true）
npm run ci:check                              # بوابة مرآة CI المحلية
node scripts/cargo-all-workspaces.mjs fmt-check
node scripts/cargo-all-workspaces.mjs clippy
node scripts/cargo-all-workspaces.mjs check
node scripts/cargo-all-workspaces.mjs nextest
```

نجاح `cargo check` / `cargo test` لا يعني اجتياز بوابة Clippy：يجب أن يأتي `fmt` مع `--all`، ويجب أن يأتي `clippy` مع `--all-targets --all-features -- -D warnings`، وأن يُنفَّذ ذلك لكل workspace على حدة. شغِّل ذلك مرة واحدة وفق معيار CI قبل الإيداع. راجع قائمة فحص الإصدار في [CHECKS.md](./CHECKS.md).

---

## الإصدار وسجل التغييرات

الإصدار المنشور الحالي **v3.0.0** (2026-10-31). المصدر الوحيد لإدخالات الإصدار هو [CHANGELOG.md](./CHANGELOG.md)، ويشمل قسم `[unreleased]` غير المنشور；ويجب أن يتوافق معه `package.json` و`Cargo.toml` لكل workspace و「الإصدار المقابل」في رأس الوثائق.

---

## الترخيص والعلاقة بالمشروع الأصلي

- الترخيص：[AGPL-3.0-only](./LICENSE)。
- المشروع الأصلي：هذا المستودع مشتق من [`polite0803/AxAgent`](https://github.com/polite0803/AxAgent)（git remote `upstream`）; وقدرات الأساس العام للذكاء الاصطناعي موروثة من المشروع الأصلي.
- عنوان هذا المستودع：[`polite0803/AxInvest`](https://github.com/polite0803/AxInvest)。
