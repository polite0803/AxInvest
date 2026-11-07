# AxInvest

**로컬 우선의 A주 투자 리서치 워크스테이션**: Tauri 2 + Rust 기반 데스크톱 애플리케이션으로, 시세와 펀더멘털 데이터 수집, 4주기 개별 종목 판단, 종목 선정과 복기, 모의 포트폴리오, 퀀트 백테스트, 자동화 작업, AI 협업을 하나의 워크스페이스에 담았다. 모든 업무 데이터는 기본적으로 로컬 SQLite에 저장되고, API Key는 AES-256-GCM으로 암호화되며, 핵심 흐름은 서드파티 클라우드 서비스에 의존하지 않는다.

> **언어 버전**: [English](./README-EN.md) | [繁體中文](./README-ZH-TW.md) | [日本語](./README-JA.md) | [한국어](./README-KO.md) | [Français](./README-FR.md) | [Deutsch](./README-DE.md) | [Español](./README-ES.md) | [Русский](./README-RU.md) | [हिन्दी](./README-HI.md) | [العربية](./README-AR.md)

---

## 프로젝트 포지셔닝

AxInvest는 [`polite0803/AxAgent`](https://github.com/polite0803/AxAgent)에서 파생되었으며, 범용 AI 기반은 업스트림을 계승하고 투자 업무 역량은 이 저장소에서 발전한다. 지향하는 것은 "복기 가능한 투자 리서치 체인"이지 기능 집합이 아니다:

- **동일한 시간 세계관**: `live`와 `as-of`가 모든 투자 역량에 관통하며, 과거 배치를 원형 그대로 재생하고 샘플을 보정할 수 있다.
- **동일한 의사결정 체인**: 후보 풀 구성과 4주기 독립 판단에서 데이터베이스 기록, 원인 분석과 성찰·진화까지 기준은 단 하나뿐이다.
- **로컬 우선**: 기본 SQLite 저장, 클라우드 의존 없음, 데이터와 키는 로컬에 남는다.

---

## 기능 내비게이션 (16개 기능 포인트)

`docs/`의 진입점은 이중 뷰 인덱스 [docs/INDEX.md](./docs/INDEX.md)다: 기능 뷰는 16개 기능 포인트로 구성되고, 아카이브 뷰는 방안 / 계획 / 감사 / 참고로 분류되며, 현재 총 265편의 문서가 등록되어 있다. 각 기능 포인트에는 설명서 한 편이 있어 현재 동작, 코드 진입점, 설정 스위치, 알려진 제한, 버전 변경 기록을 명시한다.

**비즈니스 영역(9개)**

| 기능 포인트                   | slug                   | 설명서                                                             |
| ----------------------------- | ---------------------- | ------------------------------------------------------------------ |
| 주식 의사결정 체인            | `stock-decision-chain` | [stock-decision-chain.md](./docs/features/stock-decision-chain.md) |
| 트렌드 스마트 선별            | `serenity-screening`   | [serenity-screening.md](./docs/features/serenity-screening.md)     |
| 타임 트래블 리플레이          | `time-travel-replay`   | [time-travel-replay.md](./docs/features/time-travel-replay.md)     |
| 시세와 데이터 소스            | `market-data`          | [market-data.md](./docs/features/market-data.md)                   |
| OPC 역량 패키지               | `opc-capability-packs` | [opc-capability-packs.md](./docs/features/opc-capability-packs.md) |
| 퀀트 백테스트                 | `quant-backtest`       | [quant-backtest.md](./docs/features/quant-backtest.md)             |
| 성찰과 진화                   | `reflection-evolution` | [reflection-evolution.md](./docs/features/reflection-evolution.md) |
| 태스크 센터와 스케줄링        | `task-center`          | [task-center.md](./docs/features/task-center.md)                   |
| 모의 포지션과 트레이딩 브리지 | `paper-portfolio`      | [paper-portfolio.md](./docs/features/paper-portfolio.md)           |

**플랫폼 영역(7개)**

| 기능 포인트                          | slug                 | 설명서                                                         |
| ------------------------------------ | -------------------- | -------------------------------------------------------------- |
| 플러그인 체계                        | `plugin-system`      | [plugin-system.md](./docs/features/plugin-system.md)           |
| 데이터 품질과 게이트                 | `data-quality-gates` | [data-quality-gates.md](./docs/features/data-quality-gates.md) |
| 런타임과 에이전트                    | `agent-runtime`      | [agent-runtime.md](./docs/features/agent-runtime.md)           |
| 모델 프로바이더와 스트리밍           | `provider-models`    | [provider-models.md](./docs/features/provider-models.md)       |
| 스토리지와 데이터베이스 마이그레이션 | `storage-migration`  | [storage-migration.md](./docs/features/storage-migration.md)   |
| 프런트엔드 셸                        | `ui-shell`           | [ui-shell.md](./docs/features/ui-shell.md)                     |
| 빌드와 CI                            | `build-ci`           | [build-ci.md](./docs/features/build-ci.md)                     |

---

## 빠른 시작

환경 요구 사항: Node.js ≥ 24, Rust ≥ 1.85(edition 2024), npm ≥ 10. Windows는 MSVC Build Tools, macOS는 Xcode Command Line Tools, Linux는 `build-essential`, `libwebkit2gtk-4.1-dev`, `libssl-dev`가 필요하다.

```bash
npm install
npm run dev           # 브라우저 모드(프런트엔드는 localStorage mock, Tauri 불필요)
npm run tauri dev     # 데스크톱 앱(Vite HMR + Tauri 창)
```

빌드와 산출물:

```bash
npm run build         # 타입 검사 + 프런트엔드 프로덕션 빌드
npm run tauri:build   # 데스크톱 설치 패키지
```

---

## 아키텍처 계층(8개 workspace 루트)

Rust 측은 의존 방향에 따라 **8개의 Cargo workspace 루트**로 분리되며, 총 **42개의 라이브러리 crate**가 있고 `src-tauri`가 조합 루트(메인 바이너리 crate + `axagent-mobile`)다. 의존 방향 철칙: `컴포넌트 → harness ← 구현`.

| workspace 루트    | 역할                       | crate 수 | 주요 내용                                                                                            |
| ----------------- | -------------------------- | -------- | ---------------------------------------------------------------------------------------------------- |
| `axagent-core`    | foundation                 | 7        | `harness` trait과 DTO, `entities`, `plugin-proto`, `disk-cache`, `rt-theme`                          |
| `axagent-kit`     | implementor                | 1        | 공통 구현 계층 `kit`                                                                                 |
| `axagent-ai`      | consumer + implementor     | 6        | `agent`, `orchestrator`, `providers`, `mcp`, `prompt-guard`, `trajectory`                            |
| `axagent-infra`   | implementor                | 12       | `dao`, `storage`, `migration`, `crypto`, `search`, `telemetry`, `document-parser` 등                 |
| `axagent-quant`   | implementor / consumer     | 5        | `astock-data`, `analysis-engine`, `company-runtime`, `quant`, `market-sim`                           |
| `axagent-plugins` | implementor + foundation   | 2        | `plugins`, `schema-gen`                                                                              |
| `axagent-runtime` | wiring + consumer + hybrid | 8        | `runtime`, `runtime-core`, `tools`, `rt-workflow`, `rt-messaging`, `gateway`, `crdt`, `notification` |
| `src-tauri`       | wiring                     | 2        | Tauri 명령, 초기화와 조립, `axagent-mobile`                                                          |

프런트엔드는 Vite 단일 페이지 애플리케이션이다: `src/` 아래를 `pages / components / stores / hooks / lib / types / i18n`으로 계층화하고, Zustand store는 다시 `domain / feature / shared / devtools` 4개 계층으로 나뉜다.

계층 귀속, 허용되는 의존 조합, 각 crate 역할 등록은 [AGENTS.md](./AGENTS.md)의 "Rust 백엔드: Harness 아키텍처 원칙"을 참고한다.

---

## 검증 게이트

```bash
npm run format:check                          # dprint 포맷과 줄바꿈 검사
npm run lint:eslint                           # oxlint(ESLint는 TS 7 미지원)
npm run typecheck                             # tsc --noEmit
npm run test:run                              # Vitest 1회 실행
npm run test:e2e                              # Playwright E2E
npm run test:rust                             # Rust 테스트(Windows는 __TAURI_WORKSPACE__=true)
npm run ci:check                              # 로컬 CI 미러 게이트
node scripts/cargo-all-workspaces.mjs fmt-check
node scripts/cargo-all-workspaces.mjs clippy
node scripts/cargo-all-workspaces.mjs check
node scripts/cargo-all-workspaces.mjs nextest
```

`cargo check` / `cargo test`가 모두 통과한다고 해서 Clippy 게이트를 통과한 것은 아니다: `fmt`는 반드시 `--all`을, `clippy`는 반드시 `--all-targets --all-features -- -D warnings`를 붙여 workspace별로 실행해야 한다. 제출 전 CI 기준으로 한 번 실행한다. 릴리스 점검은 [CHECKS.md](./CHECKS.md)를 참고한다.

---

## 버전과 변경 로그

현재 릴리스 버전 **v3.0.0**(2026-10-31). 버전 항목의 유일한 출처는 [CHANGELOG.md](./CHANGELOG.md)이며, 미출시 `[unreleased]` 구간을 포함한다. `package.json`, 각 workspace의 `Cargo.toml`, 문서 머리말의 "대응 버전"은 이와 일치해야 한다.

---

## 라이선스와 업스트림 관계

- 라이선스: [AGPL-3.0-only](./LICENSE).
- 업스트림: 이 저장소는 [`polite0803/AxAgent`](https://github.com/polite0803/AxAgent)에서 파생되었으며(git remote `upstream`), 범용 AI 기반 역량은 업스트림에서 계승한다.
- 이 저장소 주소: [`polite0803/AxInvest`](https://github.com/polite0803/AxInvest).
