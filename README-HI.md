# AxInvest

**स्थानीय-प्राथमिकता वाला A-शेयर निवेश अनुसंधान वर्कस्टेशन**：Tauri 2 + Rust आधारित डेस्कटॉप ऐप, जो बाज़ार व फंडामेंटल डेटा प्राप्ति, चार-अवधि स्टॉक निर्णय, स्टॉक चयन व समीक्षा, पेपर पोर्टफ़ोलियो, क्वांट बैकटेस्ट, स्वचालित कार्य और AI सहयोग को एक ही वर्कस्पेस में समेटता है। सभी व्यावसायिक डेटा डिफ़ॉल्ट रूप से स्थानीय SQLite में सहेजा जाता है, API Key AES-256-GCM से एन्क्रिप्ट होती हैं, और मुख्य प्रक्रियाएँ किसी तृतीय-पक्ष क्लाउड सेवा पर निर्भर नहीं हैं।

> **भाषा संस्करण**：[English](./README-EN.md) | [繁體中文](./README-ZH-TW.md) | [日本語](./README-JA.md) | [한국어](./README-KO.md) | [Français](./README-FR.md) | [Deutsch](./README-DE.md) | [Español](./README-ES.md) | [Русский](./README-RU.md) | [हिन्दी](./README-HI.md) | [العربية](./README-AR.md)

---

## परियोजना की स्थिति

AxInvest [`polite0803/AxAgent`](https://github.com/polite0803/AxAgent) से व्युत्पन्न है; सामान्य AI आधार upstream से विरासत में मिलता है, और निवेश-व्यवसाय की क्षमताएँ इस रिपॉज़िटरी में विकसित होती हैं। यह「एक समीक्षायोग्य निवेश-अनुसंधान श्रृंखला」पर केंद्रित है, न कि सुविधाओं के संग्रह पर：

- **एक ही समय-दृष्टि**：`live` और `as-of` सभी निवेश क्षमताओं में व्याप्त हैं, ऐतिहासिक बैच जैसे-के-तैसे रीप्ले किए जा सकते हैं, और नमूनों का कैलिब्रेशन किया जा सकता है।
- **एक ही निर्णय श्रृंखला**：उम्मीदवार पूल निर्माण से लेकर चार अवधियों के स्वतंत्र निर्णय, फिर डेटाबेस में सहेजना, कारण-निर्धारण और प्रतिबिंब-विकास तक, मानक केवल एक ही है।
- **स्थानीय-प्राथमिकता**：डिफ़ॉल्ट SQLite सहेजना, कोई क्लाउड निर्भरता नहीं, डेटा और कुंजियाँ स्थानीय मशीन पर रहती हैं।

---

## सुविधा नेविगेशन (16 सुविधा बिंदु)

`docs/` का प्रवेश द्वि-दृश्य सूचकांक [docs/INDEX.md](./docs/INDEX.md) है：सुविधा दृश्य 16 सुविधा बिंदुओं के अनुसार संगठित है, संग्रह दृश्य योजना / प्लान / ऑडिट / संदर्भ के अनुसार वर्गीकृत है, और वर्तमान में कुल 265 दस्तावेज़ पंजीकृत हैं। प्रत्येक सुविधा बिंदु का एक मैनुअल है, जिसमें वर्तमान व्यवहार, कोड प्रवेश, कॉन्फ़िगरेशन स्विच, ज्ञात सीमाएँ और संस्करण परिवर्तन रिकॉर्ड स्पष्ट रूप से लिखे गए हैं।

**व्यवसाय डोमेन (9)**

| सुविधा बिंदु                    | slug                   | मैनुअल                                                               |
| ---------------------------- | ---------------------- | ------------------------------------------------------------------ |
| स्टॉक निर्णय श्रृंखला             | `stock-decision-chain` | [stock-decision-chain.md](./docs/features/stock-decision-chain.md) |
| ट्रेंड स्मार्ट चयन                | `serenity-screening`   | [serenity-screening.md](./docs/features/serenity-screening.md)     |
| टाइम-ट्रैवल रीप्ले               | `time-travel-replay`   | [time-travel-replay.md](./docs/features/time-travel-replay.md)     |
| बाज़ार डेटा और डेटा स्रोत        | `market-data`          | [market-data.md](./docs/features/market-data.md)                   |
| OPC क्षमता पैक                 | `opc-capability-packs` | [opc-capability-packs.md](./docs/features/opc-capability-packs.md) |
| क्वांट बैकटेस्ट                   | `quant-backtest`       | [quant-backtest.md](./docs/features/quant-backtest.md)             |
| प्रतिबिंब और विकास             | `reflection-evolution` | [reflection-evolution.md](./docs/features/reflection-evolution.md) |
| टास्क सेंटर और शेड्यूलिंग           | `task-center`          | [task-center.md](./docs/features/task-center.md)                   |
| पेपर पोर्टफ़ोलियो और ट्रेडिंग ब्रिज | `paper-portfolio`      | [paper-portfolio.md](./docs/features/paper-portfolio.md)           |

**प्लेटफ़ॉर्म डोमेन (7)**

| सुविधा बिंदु              | slug                 | मैनुअल                                                           |
| ---------------------- | -------------------- | -------------------------------------------------------------- |
| प्लगइन सिस्टम            | `plugin-system`      | [plugin-system.md](./docs/features/plugin-system.md)           |
| डेटा गुणवत्ता और गेट       | `data-quality-gates` | [data-quality-gates.md](./docs/features/data-quality-gates.md) |
| रनटाइम और एजेंट          | `agent-runtime`      | [agent-runtime.md](./docs/features/agent-runtime.md)           |
| मॉडल प्रदाता और स्ट्रीमिंग | `provider-models`    | [provider-models.md](./docs/features/provider-models.md)       |
| स्टोरेज और डेटाबेस माइग्रेशन | `storage-migration`  | [storage-migration.md](./docs/features/storage-migration.md)   |
| फ़्रंटएंड शेल               | `ui-shell`           | [ui-shell.md](./docs/features/ui-shell.md)                     |
| बिल्ड और CI             | `build-ci`           | [build-ci.md](./docs/features/build-ci.md)                     |

---

## शुरुआत करें

आवश्यकताएँ：Node.js ≥ 24, Rust ≥ 1.85 (edition 2024), npm ≥ 10। Windows पर MSVC Build Tools, macOS पर Xcode Command Line Tools, Linux पर `build-essential`, `libwebkit2gtk-4.1-dev`, `libssl-dev` आवश्यक हैं।

```bash
npm install
npm run dev           # ब्राउज़र मोड（फ़्रंटएंड localStorage mock पर चलता है, Tauri की आवश्यकता नहीं）
npm run tauri dev     # डेस्कटॉप ऐप（Vite HMR + Tauri विंडो）
```

बिल्ड और आउटपुट：

```bash
npm run build         # टाइप जाँच + फ़्रंटएंड प्रोडक्शन बिल्ड
npm run tauri:build   # डेस्कटॉप इंस्टॉलर
```

---

## आर्किटेक्चर परतें (8 workspace रूट)

Rust भाग को निर्भरता की दिशा के अनुसार **8 Cargo workspace रूट** में विभाजित किया गया है, कुल **42 लाइब्रेरी crate**, और `src-tauri` संयोजन रूट है（मुख्य बाइनरी crate + `axagent-mobile`）。निर्भरता दिशा का नियम：`घटक → harness ← कार्यान्वयन`।

| workspace रूट      | भूमिका                      | crate संख्या | मुख्य सामग्री                                                                                           |
| ----------------- | -------------------------- | ---------- | ---------------------------------------------------------------------------------------------------- |
| `axagent-core`    | foundation                 | 7          | `harness` trait और DTO, `entities`, `plugin-proto`, `disk-cache`, `rt-theme`                         |
| `axagent-kit`     | implementor                | 1          | साझा कार्यान्वयन परत `kit`                                                                             |
| `axagent-ai`      | consumer + implementor     | 6          | `agent`, `orchestrator`, `providers`, `mcp`, `prompt-guard`, `trajectory`                            |
| `axagent-infra`   | implementor                | 12         | `dao`, `storage`, `migration`, `crypto`, `search`, `telemetry`, `document-parser` आदि                |
| `axagent-quant`   | implementor / consumer     | 5          | `astock-data`, `analysis-engine`, `company-runtime`, `quant`, `market-sim`                           |
| `axagent-plugins` | implementor + foundation   | 2          | `plugins`, `schema-gen`                                                                              |
| `axagent-runtime` | wiring + consumer + hybrid | 8          | `runtime`, `runtime-core`, `tools`, `rt-workflow`, `rt-messaging`, `gateway`, `crdt`, `notification` |
| `src-tauri`       | wiring                     | 2          | Tauri कमांड, इनिशियलाइज़ेशन और असेंबली, `axagent-mobile`                                                  |

फ़्रंटएंड Vite एकल-पृष्ठ ऐप है：`src/` में `pages / components / stores / hooks / lib / types / i18n` के अनुसार परतें हैं, और Zustand store आगे `domain / feature / shared / devtools` चार परतों में विभाजित है।

परत-सदस्यता, अनुमत निर्भरता संयोजन और प्रत्येक crate की भूमिका पंजीकरण [AGENTS.md](./AGENTS.md) के「Rust बैकएंड：Harness आर्किटेक्चर सिद्धांत」में देखें।

---

## सत्यापन गेट

```bash
npm run format:check                          # dprint प्रारूप और पंक्ति-अंत जाँच
npm run lint:eslint                           # oxlint（ESLint TS 7 का समर्थन नहीं करता）
npm run typecheck                             # tsc --noEmit
npm run test:run                              # Vitest एकल रन
npm run test:e2e                              # Playwright E2E
npm run test:rust                             # Rust परीक्षण（Windows पर __TAURI_WORKSPACE__=true）
npm run ci:check                              # स्थानीय CI मिरर गेट
node scripts/cargo-all-workspaces.mjs fmt-check
node scripts/cargo-all-workspaces.mjs clippy
node scripts/cargo-all-workspaces.mjs check
node scripts/cargo-all-workspaces.mjs nextest
```

`cargo check` / `cargo test` का हरा होना Clippy गेट पास होने के बराबर नहीं है：`fmt` के साथ `--all` अनिवार्य है, `clippy` के साथ `--all-targets --all-features -- -D warnings` अनिवार्य है, और इसे प्रत्येक workspace पर चलाना होता है। सबमिट करने से पहले CI के मानक के अनुसार एक बार चलाएँ। रिलीज़ जाँच [CHECKS.md](./CHECKS.md) में देखें।

---

## संस्करण और चेंजलॉग

वर्तमान रिलीज़ संस्करण **v3.0.0** (2026-10-31)। संस्करण प्रविष्टियों का एकमात्र स्रोत [CHANGELOG.md](./CHANGELOG.md) है, जिसमें अप्रकाशित `[unreleased]` अनुभाग भी शामिल है; `package.json`, प्रत्येक workspace का `Cargo.toml` और दस्तावेज़ शीर्ष की「संबंधित संस्करण」जानकारी उससे मेल खानी चाहिए।

---

## लाइसेंस और upstream संबंध

- लाइसेंस：[AGPL-3.0-only](./LICENSE)।
- Upstream：यह रिपॉज़िटरी [`polite0803/AxAgent`](https://github.com/polite0803/AxAgent)（git remote `upstream`）से व्युत्पन्न है; सामान्य AI आधार की क्षमताएँ upstream से विरासत में मिली हैं।
- इस रिपॉज़िटरी का पता：[`polite0803/AxInvest`](https://github.com/polite0803/AxInvest)।
