[**English**](./README-EN.md) | [简体中文](./README.md) | [繁體中文](./README-ZH-TW.md) | [日本語](./README-JA.md) | **한국어** | [Français](./README-FR.md) | [Deutsch](./README-DE.md) | [Español](./README-ES.md) | [Русский](./README-RU.md) | [हिन्दी](./README-HI.md) | [العربية](./README-AR.md)

[![AxInvest](https://github.com/polite0803/AxAgent/blob/main/src/assets/image/logo.png?raw=true)](https://github.com/polite0803/AxAgent)

<p align="center">
  <a href="https://www.producthunt.com/products/axagent?embed=true&amp&amp&utm_source=badge-featured&amp&amp;&amp;#10;&amp;amp&amp&amp;;utm_medium=badge&amp&amp;#10&amp;amp;;utm_campaign=badge-axagent" target="_blank" rel="noopener noreferrer"><img alt="AxInvest - AI 기반 스마트 투자 분석 플랫폼 | Product Hunt" width="250" height="54" src="https://api.producthunt.com/widgets/embed-image/v1/featured.svg?post_id=1118403&amp;theme=light&amp;t=1775627359538"></a>
</p>

<p align="center">
  <strong>AI 기반 스마트 투자 분석 | 멀티 에이전트 협업 | 로컬 우선</strong>
</p>

<p align="center">
  <a href="https://github.com/polite0803/AxAgent/releases" target="_blank">
    <img src="https://img.shields.io/github/v/release/polite0803/AxAgent?style=flat-square" alt="Release">
  </a>
  <a href="https://github.com/polite0803/AxAgent/actions" target="_blank">
    <img src="https://img.shields.io/github/actions/workflow_status/polite0803/AxAgent/release.yml?style=flat-square" alt="Build">
  </a>
  <img src="https://img.shields.io/badge/platform-Windows%20%7C%20macOS%20%7C%20Linux%20%7C%20Android%20%7C%20iOS-blue?style=flat-square" alt="Platform">
  <img src="https://img.shields.io/badge/license-AGPL--3.0-green?style=flat-square" alt="License">
</p>

---

## AxInvest란?

**AxInvest v2.3**은 AI 기반 스마트 투자 분석 플랫폼으로, AxAgent 멀티 에이전트 프레임워크를 기반으로 구축되었습니다. 고급 AI 에이전트 능력과 전문적인 A주 투자 분석을 심층적으로 융합하여, 멀티 모델 프로바이더, AI 에이전트 연구, 시각적 워크플로 오케스트레이션, 로컬 지식 관리, 내장 API 게이트웨이를 지원하며 **Windows / macOS / Linux / Android / iOS** 5개 플랫폼을 지원하고, **데스크톱, 태블릿, 모바일** 3단계 기기에 적응형 레이아웃을 제공합니다.

AxInvest의 핵심 특징은 멀티 에이전트 적대적 디베이트, 심층 연구 및 팩트체크 메커니즘을 활용하여 투자 결정에 포괄적이고 객관적인 분석 지원을 제공하는 것입니다.

---

## 스크린샷

| 채팅 및 모델 선택 | 멀티 에이전트 대시보드 |
|:---:|:---:|
| ![](.github/images/s1-0412.png) | ![](.github/images/s5-0412.png) |

| 지식 베이스 RAG | 메모리 및 컨텍스트 |
|:---:|:---:|
| ![](.github/images/s3-0412.png) | ![](.github/images/s4-0412.png) |

| 워크플로 편집기 | API 게이트웨이 |
|:---:|:---:|
| ![](.github/images/s9-0412.png) | ![](.github/images/s10-0412.png) |

---

## 핵심 기능

### 📈 스마트 투자 분석

AxInvest의 핵심 특징 모듈로, AI 에이전트 능력과 전문 투자 분석을 심층 융합합니다:

**다중 소스 데이터 집계 및 장애 조치**

- **9개 데이터 소스** — 텐센트 금융, 통달신(mootdx), 동방부자, 신랑 금융, 바이두 주식, 동화순(THS), 문재(Iwencai), 거조 정보(cninfo), AKShare
- **22종 데이터 라우팅** — 각 데이터 유형별 다중 소스 장애 조치 라우팅 구성, 주 소스 사용 불가 시 자동으로 백업 소스로 전환
- **동시성 데이터 수집** — `tokio::join!` 동시 수집으로 16종 개별 주식 데이터 + 5종 시장 데이터를 수집하여 최대 효율 달성
- **스마트 캐시** — LRU 메모리 캐시(1000개 상한), 시세 30s TTL / K선 300s TTL, 자동 만료 제거
- **헬스 체크** — 공급자 연결성 프로브(평안은행 000001을 프로브로 사용), 런타임에서 데이터 소스 가용성 감지 지원

**A주 시장 식별 및 규칙**

- **섹터 식별** — 코드 접두사로 자동 식별: 상해 메인보드(6), 과창판(688), 심천 메인보드(0), 창업판(3), 북경거래소(8)
- **상한가/하한가 규칙** — 과창판/창업판 ±20%, 북경거래소 ±30%, 메인보드 ±10%, ST주 ±5%
- **거래일 캘린더** — 내장 2025-2026년 A주 공휴일 및 조정 출근일, 거래일 판단 지원

**개별 주식 데이터(16종)**

- **실시간 시세** — 가격, 등락률, 거래량/거래대금, 회전율, PE/PB, 총시가총액, 상한가/하한가, ST 표시
- **K선 데이터** — 7종 주기(5분/15분/30분/60분/일/주/월), 거래량, 거래대금, 회전율 포함
- **재무 분석** — 매출, 순이익, EPS, BPS, ROE, 부채비율, 매출총이익률, 순이익률, 매출 전년 동기 대비, 이익 전년 동기 대비
- **자금 흐름** — 메인/초대형/대형/중형/소형 주문 순유입
- **용호방** — 영업부 매수/매도 금액, 순액, 등록 사유
- **제한매도 해제** — 해제 일자, 해제 주식 수, 해제 비율, 주주 정보
- **신용거래** — 신용매수액/잔액, 공매도량/잔량
- **북향 자금** — 보유 수량, 보유 비율, 변동 수량
- **업종 분류** — 신만 1급/2급 업종, 컨셉 섹터 태그
- **주요 주주 증감** — 주요 주주 증감 동향, 증감 사유
- **배당 기록** — 권리락일, 주당 배당금, 송전 비율, 기준일
- **리서치 리포트 집계** — 증권사 리서치 리포트, 기관, 애널리스트, 평가, 목표가, EPS 예측 포함
- **컨센서스 EPS** — 기관 컨센서스 EPS, 컨센서스 목표가, 평균 평가, 평가 수
- **컨셉 섹터** — 3차원 귀속(업종/컨셉/지역), 섹터 등락률 포함
- **공시 검색** — 거조 정보 상장 기업 공시, 공시 유형 및 PDF 링크 포함
- **뉴스 여론** — 뉴스 제목/요약/출처, 감정 점수 포함

**시장 데이터(5종)**

- **전 시장 용호방** — 당일 모든 등록 주식, 순매수, 매수/매도 금액 포함
- **인기 주식** — 동화순 강세주, 등락률, 회전율, 사유 태그, 소속 섹터 포함
- **업종 순위** — 신만 업종 등락률, 거래대금, 선도주
- **재련사 속보** — 실시간 재련 속보, 제목, 내용, 출처 포함
- **북향 자금 흐름** — 상해/심천/합계 분 단위 자금 흐름

**기술 지표 계산(indicators 모듈)**

- **이동평균선 시스템** — MA5/MA10/MA20/MA60, 배열 상태 판단(강세/약세/약강세/얽힘 교차) 포함
- **MACD** — DIF/DEA/히스토그램, 신호 판단(골든크로스/데드크로스/강세 진행/약세 진행) 포함
- **RSI** — RSI6/RSI12/RSI24, 신호 판단(과매수/과매도/강세/약세/중립) 포함
- **볼린저 밴드** — 상한/중간/하한 (20,2), 위치 판단(상한 이상/상한 구간/중간 근처/하한 구간/하한 이하) 포함
- **이격률** — MA5 이격률, MA20 이격률
- **거래량 분석** — 거래량 비율(당일 거래량/5일 평균 거래량), 신호 판단(거래량 증가 상승/거래량 감소 조정/거래량 증가 하락/거래량 감소 상승/정상) 포함
- **지지/저항선** — 최근 고저점 및 이동평균선 기반 자동 계산

**MCP 도구 등록(mcp_tools 모듈)**

- 주식 데이터 능력이 MCP 프로토콜을 통해 표준 도구로 등록되며, AI 에이전트가 대화에서 직접 호출 가능
- 등록 도구: search_stock, get_stock_quote, get_stock_kline, get_stock_financials, get_stock_news, get_stock_money_flow, get_stock_dragon_tiger 등

**AI 분석 파이프라인(stock-analysis crate, 23개 서브모듈)**

- **분석 오케스트레이션** — orchestrator(파이프라인 오케스트레이션), pipeline(다단계 파이프라인), runner(작업 실행기)
- **의사결정 엔진** — decision(투자 의사결정), signals(거래 시그널 생성), rules(거래 규칙 엔진)
- **리스크 평가** — risk(리스크 평가 모델), portfolio_risk(포트폴리오 리스크), position_limits(포지션 제한 및 컴플라이언스)
- **선별 및 백테스트** — screener(다중 조건 스크리너), backtest(전략 백테스트 엔진), trading(거래 전략 프레임워크)
- **가치 투자** — value(가치 분석), value_investing(가치 투자 평가 프레임워크)
- **품질 관리** — quality(데이터 품질 검사), data_clean(데이터 클리닝 및 전처리), review(분석 결과 복核查)
- **리포트 및 평가** — report(분석 리포트 생성), scoring(종합 평가 시스템)
- **보조 모듈** — key_levels(핵심 가격대 식별), monitor(실시간 모니터링 및 알림), plugin(분석 플러그인 확장), prompts(AI 프롬프트 템플릿)

**프론트엔드 분석 컴포넌트(16개)**

- StockAnalysisPage, StockQuoteCard, KLineChart, RiskMatrix, TradePanel
- DecisionBanner, DebatePanel, WatchlistPanel, PriceAlertPanel, CompareView
- AnalystReportGrid, AnalystReportCard, HistoricalAnalysisPanel, StockSearchBar
- AnalysisProgress, StockAnalysisSettingsModal, StockAnalysisChatIndicator

**적대적 디베이트 및 의사결정**

- **적대적 디베이트** — 멀티 에이전트 Pro/Con 디베이트, 논점 강도 점수 및 반박 추적 지원
- **의사결정 배너** — 매수/매도/보유 의사결정 시각화, 신뢰도 및 사유 포함
- **AI 워크플로 통합** — 주식 분석 워크플로와 대화의 원활한 연결(stockWorkflowChatBridge)

### 🤖 AI 모델 지원

- **멀티 프로바이더 지원** — OpenAI, Anthropic Claude, Google Gemini, Ollama, OpenClaw, Hermes 및 모든 OpenAI 호환 API와 네이티브 통합
- **멀티 키 로테이션** — 각 프로바이더에 여러 API 키를 구성하고 자동 로테이션으로 비율 제한 분산
- **로컬 모델 추론** — Ollama 로컬 모델 및 GGUF/GGML 파일 관리를 완벽하게 지원
- **Candle 추론 엔진** — 내장 Candle 로컬 추론, rerank/judge 인터페이스 지원, GGUF 온디맨드 다운로드
- **모델 관리** — 원격 모델 목록 가져오기, 사용자 지정 가능한 매개변수(temperature, max tokens, top-p 등)
- **스트리밍 출력** — 실시간 토큰 단위 렌더링, 접이식 사고 블록(Claude 확장 사고) 지원
- **멀티 모델 비교** — 여러 모델에 동시에 동일한 질문을 전송하고 나란히 비교
- **함수 호출** — 지원되는 모든 프로바이더에 걸친 구조화된 함수 호출
- **OpenAI Responses API** — OpenAI Responses 형식 전송 지원
- **실시간 API** — OpenAI 실시간 API 호환 WebSocket 이벤트 푸시
- **이미지 생성** — AI 이미지 생성 패널, 다양한 모델 및 매개변수 구성 지원

### 🔐 AI 에이전트 시스템

에이전트 시스템은 정교한 아키텍처를 기반으로 구축되어(agent crate, 70+ 소스 파일) 다음 기능을 제공합니다:

- **ReAct 추론 엔진** — 추론과 행동을 통합하고 자체 검증을 내장하여 작업 실행의 신뢰성 보장
- **계층적 플래너** — 복잡한 작업을 단계 및 의존성을 가진 구조화된 계획으로 분해
- **작업 분해기** — 복잡한 작업을 자동으로 실행 가능한 하위 작업으로 분해
- **사고 체인** — 에이전트 결정 추론의 시각화, 단계별 분해
- **사고 트리** — tree_of_thoughts 다중 경로 추론 탐색
- **심층 연구** — 다중 소스 검색 오케스트레이션, 인용 추적 및 신뢰도 평가
- **팩트체크** — AI 기반 사실 검증 및 출처 분류
- **검색 오케스트레이션** — 다중 검색 프로바이더 조정, 검색 계획 및 결과 종합 지원
- **학술 검색** — 학술 문헌 검색 및 인용 분석
- **컴퓨터 제어** — AI 제어 마우스 클릭, 키보드 입력, 화면 스크롤, 비전 모델 분석과 연계
- **화면 인식** — 스크린샷 캡처 및 비전 모델 분석으로 UI 요소 식별
- **비전 파이프라인** — vision_pipeline 이미지 이해 및 분석
- **3단계 권한 모드** — 기본(승인 필요), 편집 수락(자동 승인), 전체 액세스(프롬프트 없음)
- **샌드박스 격리** — 에이전트 작업은 지정된 작업 디렉토리로 엄격히 제한
- **도구 승인 패널** — 도구 호출 요청의 실시간 표시, 항목별 승인 지원
- **비용 추적** — 각 세션의 토큰 사용량 및 비용 통계 실시간 표시
- **일시 중지/재개** — 에이전트 실행을 언제든지 일시 중지하고 나중에 재개
- **체크포인트 시스템** — 크래시 복구 및 세션 재연결을 위한 영속성 체크포인트
- **오류 복구 엔진** — 자동 오류 분류, 근본 원인 분석 및 복구 전략 실행
- **루프 감지** — 에이전트 추론에서 순환 동작 자동 감지 및 중단
- **능동적 모드** — 에이전트가 자발적으로 제안 및 작업 실행 가능
- **목적 관리** — 에이전트의 실행 목적 및 컨텍스트 유지 및 추적
- **자체 검증** — self_verifier 에이전트 출력 정확성 자동 검증
- **반성기** — reflector 추론 과정에 대한 반성 및 개선
- **방향 제어 입력** — steer_manager 에이전트 행동 방향의 동적 조정
- **이벤트 버스** — event_bus / event_emitter 에이전트 이벤트 기반 아키텍처
- **콘텐츠 종합** — content_synthesizer 다중 소스 정보 종합 및 리포트 생성
- **인용 추적** — citation_tracker 정보 출처 자동 추적 및 표시
- **신뢰도 평가** — credibility_evaluator 정보 출처 신뢰도 평가
- **개요 구축** — outline_builder 연구 개요 자동 구축
- **스키마 관리** — schema_manager 출력 구조 스키마 관리
- **프로젝트 메모리** — project_memory 프로젝트 수준의 영속성 메모리
- **환경 탐지** — environment_probe 실행 환경 정보 자동 탐지
- **헬스 체크** — health_checker 에이전트 건강 상태 모니터링

### 👥 멀티 에이전트 협업

- **하위 에이전트 조정** — 마스터-슬레이브 아키텍처로 coordinator가 여러 협업 에이전트 조정
- **병렬 실행** — 여러 에이전트가 작업을 병렬 처리, 의존성 인식 스케줄링 지원
- **적대적 디베이트** — adversarial_debate Pro/Con 디베이트 라운드, 논점 강도 점수 및 반박 추적 지원
- **에이전트 역할** — agent_roles 팀 협업을 위한 사전 정의된 역할(연구자, 플래너, 개발자, 검토자, 종합자)
- **에이전트 오케스트레이터** — 멀티 에이전트 팀을 위한 중앙 집중식 메시지 라우팅 및 상태 관리
- **통신 그래프** — graph_insights 에이전트 상호작용 및 메시지 흐름의 시각화
- **공유 블랙보드** — shared_blackboard / blackboard 에이전트 간 공유 상태 공간
- **Buddy 파트너 시스템** — 구성 가능한 에이전트 파트너, 종족 및 속성 정의 지원
- **공유 메모리** — 에이전트 간 공유 메모리 공간, 통계 및 쿼리 지원
- **팀 Cron 등록** — 팀 수준의 정기 작업 스케줄링
- **전문가 시스템** — agency_expert 도메인 전문가 에이전트
- **에이전트 프로필** — agent_profile 에이전트 개성 및 능력 프로필 관리

### ⭐ 스킬 시스템

- **스킬 마켓플레이스** — 커뮤니티 기여 스킬을 검색하고 설치할 수 있는 내장 마켓플레이스
- **스킬 생성** — 제안에서 자동으로 스킬 생성, Markdown 편집기 지원
- **스킬 진화** — skill_evolution 실행 피드백에 기반한 AI 구동 기존 스킬의 자동 분석 및 개선
- **스킬 매칭** — skill_matcher 의미적 매칭으로 대화 컨텍스트와 관련된 스킬 추천
- **스킬 분해** — 복잡한 작업을 자동으로 실행 가능한 원자 스킬로 분해(LLM 보조/다중 라운드/워크플로 검증)
- **생성 도구** — AI가 자동으로 새로운 도구를 생성하고 등록하여 에이전트 능력 확장
- **스킬 허브** — skills_hub_adapter 중앙 집중식 스킬 발견 및 구성 관리 인터페이스
- **스킬 허브 클라이언트** — skills_hub_client 원격 스킬 허브와의 통합, 커뮤니티 공유 지원
- **스킬 의존성 검사** — 스킬 의존성 및 도구 가용성 자동 검사
- **스킬 샌드박스 컨테이너** — 격리된 환경에서 스킬 안전 실행
- **원자 스킬** — atomic_skill 최소 실행 가능 스킬 단위
- **스킬 제안** — skill_proposal AI 구동 스킬 생성 제안

### 🔄 워크플로 시스템

워크플로 엔진(rt-workflow crate)은 DAG 기반 작업 오케스트레이션 시스템을 구현합니다:

- **시각적 워크플로 편집기** — 드래그 앤 드롭 워크플로 디자이너, 노드 연결 및 구성 지원
- **16종 노드 유형** — 트리거, 에이전트, LLM, 조건, 병렬, 루프, 병합, 지연, 도구, 코드, 하위 워크플로, 벡터 검색, 문서 파서, 검증, 종료, 폴백(fallback)
- **16종 속성 패널** — 각 노드 유형에 대응하는 독립 구성 패널
- **워크플로 템플릿** — 내장 프리셋: 코드 리뷰, 버그 수정, 문서, 테스트, 리팩토링, 탐색, 성능, 보안, 기능 개발
- **DAG 실행** — Kahn 알고리즘 위상 정렬, 순환 감지 지원
- **병렬 디스패치** — 파이프라인 스타일 실행, 빠른 단계가 느린 단계를 기다리지 않음
- **재시도 정책** — 지수 백오프, 각 단계별 구성 가능한 최대 재시도 횟수
- **부분 완료** — 실패한 단계가 독립적인 하류 단계를 차단하지 않음
- **버전 관리** — 워크플로 템플릿 버전 관리, 롤백 지원
- **실행 기록** — 상세한 기록, 상태 추적 및 디버깅 지원
- **AI 지원** — AI 지원 워크플로 설계, 노드 추천 및 에이전트 프롬프트 최적화
- **의미 검사** — 워크플로 의미 검증, 잠재적 문제 감지
- **n8n 가져오기** — n8n 디렉토리에서 워크플로 가져오기 지원
- **디버그 패널** — 워크플로 실행 과정의 실시간 디버깅 및 상태 확인
- **캐시 레이어** — cache_layer 워크플로 실행 결과 캐시
- **마켓플레이스** — workflow_marketplace 워크플로 템플릿 마켓플레이스 및 리뷰

### 📚 지식 및 메모리

- **지식 베이스(RAG)** — 멀티 지식 베이스 지원, 문서 업로드, 자동 파싱, 청킹 및 벡터 인덱싱 지원
- **하이브리드 검색** — 벡터 유사성 검색과 BM25 전체 텍스트 순위 조합
- **리랭킹** — 교차 인코더 리랭킹으로 검색 정확도 향상
- **3단계 리콜 파이프라인** — AST 인덱스 + 벡터 검색 + FTS5의 다단계 리콜 메커니즘
- **Self-RAG** — self_rag 자기 검색 증강 생성
- **쿼리 강화** — query_enhancement 쿼리 재작성 및 확장
- **지식 그래프** — 지식 연결의 엔티티 관계 시각화(엔티티, 속성, 관계, 흐름, 인터페이스)
- **Wiki 시스템** — LLM Wiki 컴파일러 및 검증기, 지식 그래프 시각화 및 증분 동기화 지원
- **Wiki 노트** — 양방향 링크 노트 시스템, 그래프 뷰 및 자동 링크 동기화 지원
- **메모리 시스템** — 멀티 네임스페이스 메모리, 수동 입력 또는 AI 자동 추출 지원
- **폐쇄 루프 메모리** — Honcho 및 Mem0 영속성 메모리 프로바이더와의 통합
- **메모리 망각** — memory_forgetting 시간 기반 메모리 감쇠 메커니즘
- **FTS5 전체 텍스트 검색** — 대화, 파일, 메모리 전체의 빠른 검색
- **세션 검색** — 모든 대화 세션 전체의 고급 검색
- **컨텍스트 관리** — 파일, 검색 결과, 지식 스니펫, 메모리, 도구 출력의 유연한 첨부
- **문서 파싱** — 다중 형식 문서 자동 파싱 및 콘텐츠 추출
- **증분 인덱싱** — 파일 변경에 대한 증분 인덱스 업데이트
- **텍스트 청킹** — text_chunker 스마트 텍스트 청킹 전략
- **토큰 예산** — token_budget 검색 결과 토큰 예산 제어

### 🌐 API 게이트웨이

- **로컬 API 서버** — 내장 OpenAI 호환, Claude 및 Gemini 인터페이스 서버
- **외부 링크** — 원클릭 Claude CLI, OpenCode 통합, API 키 및 모델 자동 동기화
- **키 관리** — 생성, 취소, 활성화/비활성화 액세스 키, 설명 지원
- **사용량 분석** — 키, 프로바이더, 날짜별 요청량 및 토큰 사용량
- **SSL/TLS 지원** — 내장 자체 서명 인증서, 사용자 정의 인증서 지원
- **요청 로깅** — 모든 API 요청 및 응답의 완전한 기록
- **구성 템플릿** — Claude, Codex, OpenCode, Gemini의 사전 구축된 템플릿
- **실시간 API** — OpenAI 실시간 API 호환 WebSocket 이벤트 푸시
- **플랫폼 통합** — 딩톡, 페이슈, QQ, Slack, 위챗, WhatsApp, Telegram, Discord 지원
- **게이트웨이 진단** — 연결 진단 및 프로그램 정책 관리
- **속도 제한기** — API 요청 속도 제한 및 트래픽 제어
- **영속성 큐** — 요청 영속성 큐 관리
- **주식 API** — stock_handlers 주식 데이터 전용 API 엔드포인트
- **SSE 푸시** — sse Server-Sent Events 실시간 이벤트 푸시

### 🔧 도구 및 확장

- **MCP 프로토콜** — 완전한 모델 컨텍스트 프로토콜 구현, stdio 및 HTTP/WebSocket 전송 지원
- **OAuth 인증** — MCP 서버의 OAuth 흐름 지원
- **MCP 자동 시작** — MCP 서버 자동 시작 및 수명 주기 관리
- **MCP 도구 브릿지** — MCP 도구와 에이전트 도구 시스템의 브릿지
- **MCP 헬스 체크** — mcp_health MCP 서버 건강 상태 모니터링
- **플러그인 시스템** — OpenClaw 호환 3단계 플러그인 아키텍처(내장/번들/외부), npm 패키지 설치, 도구 등록, 훅 및 수명 주기 관리 지원
- **플러그인 마켓플레이스** — 내장 마켓 UI, npm 검색 설치, 확인 팝업 지원
- **내장 도구** — 40+ 도구 모듈: 파일 작업(읽기/쓰기/편집/시스템), 코드 실행, 검색(Grep/Glob), Bash, 웹 검색/스크래핑, 계획 관리, Cron 스케줄링, REPL, LSP, 컨텍스트 관리, 컴퓨터 제어, 메시지 푸시, 할 일, 데이터베이스, DevOps, 문서 파싱, Git, 지식 검색, LSP, 미디어 처리, 메시지 푸시, OCR, 푸시 알림, 시스템 정보, 작업 시스템, 테스트, 워크스페이스/워크트리 등
- **도구 권한 시스템** — 도구 권한 분류, 규칙 관리 및 사용 추적
- **Bash 보안** — 명령 파싱, 경로 검증 및 샌드박스 보안 제어
- **LSP 클라이언트** — 내장 언어 서버 프로토콜, 코드 완성 및 진단 지원
- **AST 인덱스** — 코드 파일의 AST 파싱 및 인덱스 구축
- **터미널 백엔드** — 로컬, Docker 및 SSH 터미널 연결 지원
- **브라우저 자동화** — CDP를 통한 브라우저 제어 기능 통합(탐색, 스크린샷, 클릭, 폼 작성, 텍스트 추출 등)
- **UI 자동화** — 크로스 플랫폼 UI 요소 식별 및 제어
- **Git 도구** — 분기 감지 및 충돌 인식을 지원하는 Git 작업
- **도구 추천** — 컨텍스트 기반 스마트 도구 추천 엔진
- **도구 오케스트레이션** — 다중 도구 조정 실행 및 스트리밍 출력
- **도구 통계** — 도구 사용 빈도 및 성능 통계
- **도구 감사** — audit 도구 호출 감사 로그

### 📊 콘텐츠 렌더링

- **Markdown 렌더링** — 코드 하이라이트, LaTeX 수학 공식, 표, 작업 목록의 완전한 지원
- **Monaco 코드 편집기** — 내장 편집기, 구문 하이라이트, 복사, 차이점 미리보기 지원
- **다이어그램 렌더링** — Mermaid 플로우차트, D2 아키텍처 다이어그램, ECharts 대화형 차트
- **아티팩트 패널** — 코드 스니펫, HTML 초안, React 컴포넌트, Markdown 노트, 실시간 미리보기 지원
- **4가지 미리보기 모드** — 코드(편집기), 분할(나란히), 미리보기(렌더링만), React 컴포넌트 미리보기
- **세션 검사기** — 세션 구조의 트리 뷰, 빠른 탐색
- **인용 패널** — 소스 인용 추적 및 표시, 신뢰도 점수 지원
- **인포그래픽 렌더링** — 인포그래픽 시각화 표시 지원
- **차트 인터프리터** — ChartInterpreter AI 구동 차트 해석
- **Diff 뷰어** — DiffViewer 코드 차이점 비교

### 🛡️ 데이터 및 보안

- **AES-256 암호화** — API 키 및 민감한 데이터는 AES-256-GCM으로 암호화
- **분리 저장소** — 애플리케이션 상태는 `~/.axinvest/`에, 사용자 파일은 `~/Documents/axinvest/`에 저장
- **자동 백업** — 로컬 디렉토리 또는 WebDAV 저장소로 예약된 백업
- **S3 백업** — s3_backup Amazon S3 클라우드 백업 지원
- **백업 복원** — 원클릭으로 이전 백업에서 복원
- **내보내기 옵션** — PNG 스크린샷, Markdown, 일반 텍스트, JSON 형식
- **저장소 관리** — 시각적 디스크 사용량 표시 및 정리 도구
- **저장소 마이그레이션** — storage_migration 버전 간 데이터 마이그레이션
- **파일 권한 부여** — 파일 액세스 권한 부여 및 취소 관리
- **작업 감사** — 주요 작업의 감사 로그 기록
- **명령 검증** — command_validator 명령 보안 검증
- **리소스 제한** — resource_limits 리소스 사용 제한
- **샌드박스 실행** — sandbox_runner 격리 환경 실행

### 🖥️ 데스크톱 환경

- **테마 엔진** — 다크/라이트 테마, 시스템 따르기 또는 수동 기본 설정 지원
- **인터페이스 언어** — 11개 언어: 간체 중국어, 번체 중국어, 영어, 일본어, 한국어, 프랑스어, 독일어, 스페인어, 러시아어, 힌디어, 아랍어
- **시스템 트레이** — 백그라운드 서비스를 중단하지 않고 트레이로 최소화
- **항상 위에** — 다른 창보다 앞에 창 고정
- **전역 단축키** — 주 창을 호출하기 위한 사용자 정의 가능한 단축키
- **QuickBar** — 빠른 액세스 플로팅 바, 원클릭 호출
- **자동 시작** — 시스템 시작 시 선택적 실행
- **프록시 지원** — HTTP 및 SOCKS5 프록시 구성
- **자동 업데이트** — 자동 버전 확인 및 업데이트 프롬프트
- **명령 팔레트** — `Cmd/Ctrl+K` 빠른 명령 액세스
- **온보딩 마법사** — 최초 사용 시 대화형 가이드 및 Ollama 감지
- **알림 센터** — 통합 앱 내 알림 관리
- **클라우드 워크스페이스** — cloud_workspace 클라우드 워크스페이스 선택
- **크래시 리포트** — crash_report 자동 크래시 리포트 수집
- **음성 통화** — VoiceCall 음성 대화 능력

### 🔬 고급 기능

- **심층 연구** — 다중 소스 검색, 인용 추적, 신뢰도 평가 및 콘텐츠 종합
- **팩트체크** — AI 기반 사실 검증 및 출처 분류
- **Cron 스케줄러** — 매일/매주/매월 템플릿 및 사용자 정의 cron 표현식을 통한 자동화된 작업 스케줄링
- **Webhook 시스템** — 도구 완료, 에이전트 오류, 세션 종료 알림의 이벤트 구독
- **사용자 프로파일링** — 코드 스타일, 명명 규칙, 들여쓰기, 주석 스타일, 커뮤니케이션 기본 설정의 자동 학습
- **RL 옵티마이저** — 도구 선택 및 작업 전략 최적화를 위한 강화 학습
- **LoRA 미세 조정** — LoRA를 사용한 로컬 훈련으로 사용자 정의 모델 어댑테이션
- **능동적 제안** — 대화 내용 및 사용자 패턴에 기반한 컨텍스트 인식 힌트
- **컨텍스트 예측** — 사용자의 다음 작업을 예측하고 관련 리소스 사전 로드
- **드림 통합** — dream_consolidation 백그라운드에서 메모리와 패턴을 자동 통합하여 장기 지식 최적화
- **오류 복구** — 자동 오류 분류, 근본 원인 분석 및 복구 제안
- **개발자 도구** — 디버깅 및 성능 분석을 위한 Trace, Span, 타임라인 시각화
- **벤치마크 시스템** — SWE-bench / Terminal-bench 작업 성능 평가 및 지표, 점수 카드 포함
- **스타일 전송** — style_migrator 학습한 코드 스타일 기본 설정을 생성된 코드에 적용
- **대시보드 플러그인** — 사용자 정의 패널 및 위젯을 지원하는 확장 가능한 대시보드
- **협업 공유** — CRDT 실시간 협업 및 원클릭 세션 공유
- **브라우저 확장** — Wiki Clipper 브라우저 확장, 웹 페이지를 LLM Wiki로 빠르게 클리핑
- **Python SDK** — AxInvest와의 통합을 위한 Python SDK 제공
- **스마트 라우팅** — 요청 스마트 라우팅 및 분류
- **의미 캐시** — 의미 기반 응답 캐시, 중복 계산 감소
- **컨텍스트 압축** — 긴 컨텍스트 자동 압축, 토큰 사용 최적화
- **메시지 배치 처리** — 메시지 배치 전송 및 최적화
- **연결 풀** — 데이터베이스 및 API 연결 풀 관리
- **기능 플래그** — 구성 가능한 기능 특성 토글 시스템
- **정책 엔진** — 권한 및 작업 정책의 중앙 집중식 관리
- **리소스 거버넌스** — 에이전트 리소스 사용 제한 및 거버넌스
- **LAN 전송** — 로컬 영역 네트워크 파일 전송 기능
- **공진화** — coevolution 스킬과 에이전트의 공동 진화
- **행동 학습** — behavior_learner / behavior_tracker 사용자 행동 학습 및 추적
- **선호도 학습** — preference_learner 사용자 선호도 자동 학습
- **내재적 보상** — intrinsic_reward 내재적 동기 기반 탐색
- **과정 보상** — process_reward 과정 수준 보상 신호
- **TextGrad** — text_grad 텍스트 그래디언트 기반 자동 최적화
- **궤적 압축** — trajectory_compressor 긴 궤적 자동 압축
- **알림 관리** — reminder_manager 스마트 알림 스케줄링
- **작업 프리페치** — task_prefetcher 예측적 작업 리소스 프리페치

### 🛡️ 프롬프트 인젝션 방어(Prompt-Guard)

- **4단계 방어 체계** — L1 패턴 감지(고위험 차단 + 중위험 표시) → L2 구분자 이스케이프 → L3 XML 래퍼 → L4 신뢰 태그
- **파이프라인 오케스트레이터** — 다단계 감지 파이프라인 직렬 연결, 사용자 정의 위험 임계값 지원
- **Token Smuggling 감지** — 인코딩 난독화 및 토큰 밀수 공격에 대한 전문 감지
- **구분자 이스케이프 감지** — delimiter_escape 프롬프트 구분자 탈출 공격 감지
- **패턴 감지** — pattern_detect 정규식+휴리스틱 인젝션 패턴 매칭
- **신뢰 태그** — trust_labels 신뢰할 수 있는 콘텐츠 마킹 및 검증
- **Strict 모드** — 엄격 모드 테스트 + 중위험 사유 명명 + 사용자 정의 모드 문서
- **전체 파이프라인 통합** — session / prompt / git / RAG 각 단계에 통합 완료

### 📱 모바일 지원

- **Android 네이티브** — APK/AAB 빌드, arm64-v8a / armeabi-v7a / x86_64 지원
- **iOS 네이티브** — IPA 빌드, arm64 지원
- **적응형 레이아웃** — 데스크톱/태블릿/모바일 3단계 자동 적응(useResponsive hook)
- **모바일 내비게이션** — Drawer 슬라이드 내비게이션 + 하단 내비게이션 바 + 플래시 플로팅 버튼
- **안전 영역 적응** — Android 시스템 상태바/내비게이션바 CSS env() 적응
- **CSP 최적화** — Android WebView CSP 프로토콜 화이트리스트
- **조건부 컴파일** — `#[cfg(not(mobile))]` 데스크톱 전용 기능(브라우저, 컴퓨터 제어, 데스크톱, QuickBar, 터미널, 화면 비전) 자동 제외

---

## 기술 아키텍처

### 기술 스택

| 레이어 | 기술 |
|--------|------|
| **프레임워크** | Tauri 2 + React 19 + TypeScript 6 |
| **UI** | Ant Design 6 + TailwindCSS 4 |
| **상태 관리** | Zustand 5 |
| **라우팅** | React Router 7 |
| **국제화** | i18next + react-i18next |
| **백엔드** | Rust 2024 + SeaORM 2 + SQLite |
| **벡터 DB** | sqlite-vec |
| **코드 편집기** | Monaco Editor |
| **다이어그램** | Mermaid + D2 + ECharts(CDN) |
| **터미널** | xterm.js 6 |
| **워크플로** | ReactFlow 11 |
| **차트 렌더링** | @antv/infographic |
| **아이콘** | Iconify + Lucide |
| **드래그 앤 드롭** | @dnd-kit |
| **빌드** | Vite 8 + npm |
| **테스트** | Vitest + Playwright + cargo-nextest |
| **포맷팅** | dprint (TS/JSON) + rustfmt |
| **Lint** | TS: eslint + oxlint / Rust: clippy + cargo-deny |
| **모바일** | Tauri Android + iOS 네이티브 빌드 |
| **데스크톱** | Windows (MSI) · macOS (DMG) · Linux (AppImage/deb/rpm) |

### 플랫폼 지원

| 플랫폼 | 아키텍처 |
|--------|----------|
| Windows | x86_64, ARM64 |
| macOS | Apple Silicon (arm64), Intel (x86_64) |
| Linux | x86_64, ARM64 |
| Android | arm64-v8a, armeabi-v7a, x86_64 (에뮬레이터) |
| iOS | arm64 |

### Rust 백엔드 아키텍처

백엔드는 전문화된 **20개의** crates로 구성된 Rust workspace로 구성됩니다:

```
src-tauri/crates/
├── agent/            # AI 에이전트 코어(70+ 소스 파일: ReAct 엔진, 조정, 계획, 심층 연구, 팩트체크 등)
├── astock-data/      # A주 데이터 소스(9개 데이터 소스, 22종 데이터 라우팅, 기술 지표, 거래일 캘린더, MCP 도구 등록)
├── core/             # 코어 유틸리티(85+ 데이터베이스 엔티티, 40+ 리포지토리, RAG, 암호화, MCP, 브라우저 자동화, AST 인덱스 등)
├── gateway/          # API 게이트웨이(HTTP 서버, 인증, 라우팅, OpenAI 호환 인터페이스, 주식 API 엔드포인트)
├── migration/        # 데이터베이스 마이그레이션(5개 마이그레이션: 주식 분석/관심종목 포트폴리오/분석 스케줄/가격 알림/거래)
├── npm/              # npm 패키지 파싱 및 레지스트리
├── plugins/          # 플러그인 시스템(OpenClaw 호환, npm 패키지 설치, 예제 플러그인 포함)
├── prompt-guard/     # 프롬프트 인젝션 방어(L1-L4 다단계 감지 및 방어, 4종 감지기)
├── providers/        # 모델 프로바이더 어댑터(OpenAI, Anthropic, Gemini, Ollama, OpenClaw, Hermes, 이미지 생성)
├── rt-dashboard/     # 대시보드 플러그인 시스템
├── rt-messaging/     # 메시지 게이트웨이(9개 플랫폼: 딩톡/페이슈/QQ/Slack/위챗/WhatsApp/Telegram/Discord)
├── rt-theme/         # 테마 엔진
├── rt-webhook/       # Webhook 서버 및 디스패치
├── rt-workflow/      # 워크플로 엔진(DAG 오케스트레이션, 16종 노드 실행기, 스케줄러, 캐시 레이어)
├── runtime/          # 런타임 서비스(70+ 소스 파일: 세션 관리, MCP, 터미널, 속도 제한, Webhook, 권한, 벤치마크 등)
├── runtime-core/     # 런타임 추상화 레이어(공용 타입, trait 정의, 구성, 기능 플래그, 권한 실행기)
├── stock-analysis/   # 스마트 투자 분석(23개 서브모듈: 파이프라인, 의사결정 엔진, 리스크 평가, 백테스트, 스크리너, 가치 투자)
├── telemetry/        # 원격 측정 및 분산 추적(OpenTelemetry 호환)
├── tools/            # 도구 시스템(40+ 내장 도구, Bash 보안, MCP 브릿지, 권한 시스템, 오케스트레이션, 감사)
└── trajectory/       # 학습 시스템(55+ 소스 파일: 메모리, 스킬, RL, 사용자 프로파일링, 드림 통합, 스타일 전송, 공진화)
```

#### stock-analysis crate 모듈 구조(23개 서브모듈)

```
stock-analysis/
├── backtest.rs         # 전략 백테스트 엔진
├── data_clean.rs       # 데이터 클리닝 및 전처리
├── decision.rs         # 투자 의사결정 엔진
├── key_levels.rs       # 핵심 가격대 식별
├── monitor.rs          # 실시간 모니터링 및 알림
├── orchestrator.rs     # 분석 파이프라인 오케스트레이션
├── pipeline.rs         # 다단계 분석 파이프라인
├── plugin.rs           # 분석 플러그인 확장
├── portfolio_risk.rs   # 포트폴리오 리스크 평가
├── position_limits.rs  # 포지션 제한 및 컴플라이언스
├── prompts.rs          # AI 프롬프트 템플릿
├── quality.rs          # 데이터 품질 검사
├── report.rs           # 분석 리포트 생성
├── review.rs           # 분석 결과 복核查
├── risk.rs             # 리스크 평가 모델
├── rules.rs            # 거래 규칙 엔진
├── runner.rs           # 분석 작업 실행기
├── scoring.rs          # 종합 평가 시스템
├── screener.rs         # 스크리너
├── signals.rs          # 거래 시그널 생성
├── trading.rs          # 거래 전략 프레임워크
├── value.rs            # 가치 분석
└── value_investing.rs  # 가치 투자 평가
```

#### astock-data crate 데이터 소스

| 데이터 소스 | 식별자 | 지원 데이터 유형 |
|------------|--------|-----------------|
| 텐센트 금융 | tencent | 실시간 시세, K선 |
| 통달신 | mootdx | 실시간 시세, K선 |
| 동방부자 | eastmoney | 시세, K선, 재무, 자금 흐름, 용호방, 제한매도 해제, 신용거래, 북향 자금, 업종 분류, 주요 주주 증감, 배당, 리서치 리포트, 전 시장 용호방, 재련사 속보 |
| 신랑 금융 | sina | 시세, K선, 뉴스 |
| 바이두 주식 | baidu_stock | 시세, 뉴스, 자금 흐름, 용호방, 제한매도 해제, 신용거래, 북향 자금, 업종 분류, 주요 주주 증감, 배당, 리서치 리포트, 인기 주식, 업종 순위, 컨셉 섹터, 북향 자금 흐름 |
| 동화순 | ths | 시세, 업종 분류, 컨센서스 EPS, 컨셉 섹터, 인기 주식, 업종 순위, 북향 자금 흐름 |
| 문재 | iwencai | 주식 검색, 업종 분류, 컨센서스 EPS, 컨셉 섹터, 인기 주식 |
| 거조 정보 | cninfo | 공시 |
| AKShare | akshare | 재무, 뉴스, 컨센서스 EPS, 재련사 속보 |

각 데이터 유형은 다중 소스 장애 조치 라우팅이 구성되어 있어, 주 데이터 소스를 사용할 수 없을 때 자동으로 백업 소스로 전환됩니다.

#### astock-data 추가 모듈

| 모듈 | 기능 |
|------|------|
| calendar | A주 거래일 캘린더(2025-2026년 공휴일 + 조정 출근일) |
| indicators | 기술 지표 계산(MA/MACD/RSI/볼린저 밴드/이격률/거래량 비율/지지 저항선) |
| mcp_tools | MCP 도구 등록(주식 데이터 능력을 AI 호출 가능 도구로 등록) |

### 프론트엔드 아키텍처

```
src/
├── stores/                    # Zustand 상태 관리(65개 store)
│   ├── domain/               # 코어 비즈니스 상태(9개)
│   │   ├── agentDomainStore.ts
│   │   ├── compressStore.ts
│   │   ├── conversationPreferences.ts
│   │   ├── conversationStore.ts
│   │   ├── conversationStoreEvents.ts
│   │   ├── conversationStoreSend.ts
│   │   ├── multiModelStore.ts
│   │   ├── preferenceStore.ts
│   │   └── streamStore.ts
│   ├── feature/               # 기능 모듈 상태(46개)
│   │   ├── agentProfileStore.ts
│   │   ├── agentStore.ts
│   │   ├── appConfigStore.ts
│   │   ├── backupStore.ts
│   │   ├── buddyStore.ts
│   │   ├── cacheStore.ts
│   │   ├── categoryStore.ts
│   │   ├── citationStore.ts
│   │   ├── continuationStore.ts
│   │   ├── decompositionStore.ts
│   │   ├── dreamStore.ts
│   │   ├── executionStore.ts
│   │   ├── expertStore.ts
│   │   ├── fileStore.ts
│   │   ├── gatewayLinkStore.ts
│   │   ├── gatewayStore.ts
│   │   ├── generatedToolStore.ts
│   │   ├── helpStore.ts
│   │   ├── knowledgeStore.ts
│   │   ├── llmWikiStore.ts
│   │   ├── localToolStore.ts
│   │   ├── mcpStore.ts
│   │   ├── memoryStore.ts
│   │   ├── nudgeStore.ts
│   │   ├── onboardingStore.ts
│   │   ├── planStore.ts
│   │   ├── platformStore.ts
│   │   ├── proactiveStore.ts
│   │   ├── promptTemplateStore.ts
│   │   ├── providerStore.ts
│   │   ├── searchStore.ts
│   │   ├── settingsStore.ts
│   │   ├── skillExtensionStore.ts
│   │   ├── skillStore.ts
│   │   ├── sourceStore.ts
│   │   ├── stockAnalysisStore.ts
│   │   ├── stockWorkflowChatBridge.ts
│   │   ├── styleStore.ts
│   │   ├── terminalStore.ts
│   │   ├── themeStore.ts
│   │   ├── topicGroupStore.ts
│   │   ├── trajectoryStore.ts
│   │   ├── userProfileStore.ts
│   │   ├── wikiStore.ts
│   │   ├── workEngineStore.ts
│   │   └── workflowEditorStore.ts
│   ├── devtools/              # 개발자 도구 상태(5개)
│   │   ├── evaluatorStore.ts
│   │   ├── fineTuneStore.ts
│   │   ├── recommendationStore.ts
│   │   ├── rlStore.ts
│   │   └── tracerStore.ts
│   └── shared/                # 공유 상태(5개)
│       ├── artifactStore.ts
│       ├── chatWorkspaceStore.ts
│       ├── rightPanelStore.ts
│       ├── tabStore.ts
│       └── uiStore.ts
│
├── components/                # React 컴포넌트(25개 모듈)
│   ├── chat/                # 채팅 인터페이스(100+ 컴포넌트: 에이전트 실행 패널, 분기 비교, 브라우저 자동화, 코드 실행기, 협업 패널, 심층 연구, 팩트체크, Git 커밋, 이미지 생성/분석, 지식 검색, 메모리 추출, 모델 라우팅, 멀티 모델 표시, 권한 관리, 플러그인 마켓, 반성 패널, 스킬 생성/진화, 구조화된 사고, 하위 에이전트 카드, 도구 호출 카드, 궤적 재생, 음성 통화, Wiki 검색, 워크플로 진행 등)
│   ├── stock-analysis/      # 스마트 투자 분석(16개 컴포넌트)
│   │   ├── StockAnalysisPage.tsx
│   │   ├── StockQuoteCard.tsx
│   │   ├── KLineChart.tsx
│   │   ├── RiskMatrix.tsx
│   │   ├── TradePanel.tsx
│   │   ├── DecisionBanner.tsx
│   │   ├── DebatePanel.tsx
│   │   ├── WatchlistPanel.tsx
│   │   ├── PriceAlertPanel.tsx
│   │   ├── CompareView.tsx
│   │   ├── AnalystReportGrid.tsx
│   │   ├── AnalystReportCard.tsx
│   │   ├── HistoricalAnalysisPanel.tsx
│   │   ├── StockSearchBar.tsx
│   │   ├── AnalysisProgress.tsx
│   │   └── StockAnalysisSettingsModal.tsx
│   │   └── StockAnalysisChatIndicator.tsx
│   ├── workflow/            # 워크플로 편집기(16종 노드 + 16종 속성 패널 + AI 패널 + 템플릿 + 디버그)
│   ├── gateway/             # API 게이트웨이 UI(개요/키/지표/모니터링/설정/템플릿/진단)
│   ├── settings/            # 설정 패널(50+ 컴포넌트: 프로바이더/모델/MCP/지식/메모리/프록시/단축키/테마/도구/Webhook/Cron/주식 분석 구성 등)
│   ├── terminal/            # 터미널 UI(통합 터미널/Docker/SSH/백엔드 선택/경로 완성/슬래시 완성)
│   ├── skill/               # 스킬 편집기 및 렌더러(액션 체인 편집/프론트엔드 편집기/샌드박스 컨테이너/의존성 검사/통계 패널)
│   ├── benchmark/           # 벤치마크 패널(구성/리포트/선택기/작업 목록/결과)
│   ├── files/               # 파일 관리 페이지
│   ├── fine-tune/           # LoRA 미세 조정 구성(데이터셋/훈련 작업/LoRA 구성)
│   ├── link/                # 외부 링크 관리(개요/모델/전략/스킬/전략 상세)
│   ├── llm-wiki/            # LLM Wiki 편집기(품질 점수/동기화 상태)
│   ├── proactive/           # 능동적 제안 시스템(컨텍스트 예측/프리페치 표시기/제안 바/알림 목록)
│   ├── wiki/                # Wiki 관리(역방향 링크/그래프 뷰/수집/코드 검사/작업 타임라인/태그 집계/버전 기록)
│   ├── devtools/            # Trace/Span 타임라인(비용 차트/지속 시간 차트/상세/필터/목록)
│   ├── decomposition/       # 스킬 분해(분해 미리보기/도구 의존성/도구 생성/도구 설치)
│   ├── recommendation/      # 도구 추천 패널
│   ├── style/               # 코드 스타일 전송(샘플/조정 슬라이더/비교/미리보기 패널)
│   ├── layout/              # 레이아웃 컴포넌트(제목 표시줄/사이드바/명령 팔레트/전역 복사/에러 바운더리/상태바/알림 벨/사용자 프로필 모달)
│   ├── help/                # 도움말 패널
│   ├── notification/        # 알림 센터
│   ├── search/              # 세션 검색
│   ├── onboarding/          # 온보딩 마법사(대화형 튜토리얼/환영 마법사)
│   ├── common/              # 공통 컴포넌트(복사/아이콘/모델 매개변수 슬라이더/붙여넣기)
│   └── shared/              # 공유 컴포넌트(아바타 편집/모달/차트 렌더링/동적 아이콘/임베딩 모델 선택/Emoji 선택/지식 베이스 아이콘/MCP 아이콘/모델 선택/Monaco 편집기/네임스페이스 아이콘/검색 프로바이더 아이콘)
│
├── pages/                    # 페이지 컴포넌트(22개 페이지)
│   ├── ChatPage.tsx
│   ├── StockAnalysisPage.tsx
│   ├── KnowledgeHubPage.tsx
│   ├── MemoryPage.tsx
│   ├── WorkflowPage.tsx
│   ├── WorkflowMarketplace.tsx
│   ├── GatewayPage.tsx
│   ├── GatewayLinkPage.tsx
│   ├── LinkPage.tsx
│   ├── FilesPage.tsx
│   ├── FineTunePage.tsx
│   ├── SkillsPage.tsx
│   ├── WikiEditPage.tsx
│   ├── WikiEditorPage.tsx
│   ├── WikiGraphPage.tsx
│   ├── IngestPage.tsx
│   ├── QuickBarPage.tsx
│   ├── SettingsPage.tsx
│   ├── TerminalPage.tsx
│   └── DevTools/
│       ├── TraceExplorer.tsx
│       ├── BenchmarkRunner.tsx
│       └── ToolRecommender.tsx
│
├── hooks/                    # React hooks(12개)
│   ├── useCommandPalette.ts
│   ├── useCopyToClipboard.ts
│   ├── useDebounce.ts
│   ├── useGlobalOverlayScrollbars.ts
│   ├── useGlobalShortcutManager.ts
│   ├── useKeyboardShortcuts.ts
│   ├── usePageRouting.ts
│   ├── useResolvedAvatarSrc.ts
│   ├── useResolvedDarkMode.ts
│   ├── useResponsive.ts
│   ├── useUpdateChecker.tsx
│   └── useVoiceChat.ts
│
├── lib/                      # 유틸리티 함수(33개 모듈 + Web Worker)
│   ├── workers/            # Web Worker(heavy.worker.ts)
│   ├── actionRouter.ts     # 액션 라우팅
│   ├── artifactRenderer.ts # 아티팩트 렌더링
│   ├── chartGenerator.ts   # 차트 생성
│   ├── chatMarkdown.ts     # Markdown 렌더링
│   ├── codeExecutor.ts     # 코드 실행
│   ├── invoke.ts           # Tauri IPC 래핑
│   ├── skillActionExecutor.ts  # 스킬 액션 실행
│   ├── skillEventBus.ts    # 스킬 이벤트 버스
│   ├── skillLifecycle.ts   # 스킬 수명 주기
│   ├── skillPermissions.ts # 스킬 권한
│   ├── storeRegistry.ts    # Store 레지스트리
│   ├── tokenEstimator.ts   # 토큰 추정
│   ├── workflowLayout.ts   # 워크플로 레이아웃
│   └── ...                 # 기타 유틸리티 모듈
│
├── types/                    # TypeScript 타입 정의(22개)
│   ├── agent.ts
│   ├── agentProfile.ts
│   ├── artifact.ts
│   ├── backup.ts
│   ├── citation.ts
│   ├── evaluator.ts
│   ├── expert.ts
│   ├── index.ts
│   ├── knowledge.ts
│   ├── llmWiki.ts
│   ├── localTool.ts
│   ├── mcp.ts
│   ├── memory.ts
│   ├── nudge.ts
│   ├── permission.ts
│   ├── platform.ts
│   ├── proactive.ts
│   ├── search.ts
│   ├── stock-analysis.ts
│   ├── style.ts
│   ├── tracer.ts
│   └── wiki.ts
│
├── sdk/                      # SDK(Python SDK 포함)
│   ├── index.ts
│   ├── types.ts
│   ├── rpcBridge.ts
│   ├── sandboxTemplate.ts
│   └── python/              # Python SDK
│       ├── setup.py
│       └── axagent_sdk/__init__.py
│
└── i18n/                     # 11개 언어 번역
```

## 시작하기

### 사전 빌드 다운로드

[Releases](https://github.com/polite0803/AxAgent/releases) 페이지에서 플랫폼용 인스톨러를 다운로드하세요.

### 소스에서 빌드

#### 요구 사항

- [Node.js](https://nodejs.org/) 20+
- [Rust](https://www.rust-lang.org/) 1.75+
- [npm](https://www.npmjs.com/) 10+
- Windows: [Visual Studio Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/) + Rust MSVC targets

#### 빌드 단계

```bash
# 리포지토리 복제
git clone https://github.com/polite0803/AxAgent.git
cd AxAgent

# 종속성 설치
npm install

# 개발 모드
npm run tauri dev

# 프론트엔드만 빌드
npm run build

# 데스크톱 애플리케이션 빌드
npm run tauri build
```

빌드 아티팩트는 `src-tauri/target/release/`에 있습니다.

### 테스트

```bash
# 단위 테스트
npm run test          # Vitest watch
npm run test:run      # Vitest 단일 실행

# E2E 테스트
npm run test:e2e      # Playwright
npm run test:e2e:ui   # Playwright UI 모드

# Rust 백엔드 테스트
cd src-tauri && cargo nextest run   # cargo-nextest(2-3배 빠름)
cd src-tauri && cargo test          # 표준 테스트

# 타입 확인
npm run typecheck     # TypeScript
cd src-tauri && cargo clippy -- -D warnings  # Rust

# 코드 포맷팅
npm run format        # dprint
cd src-tauri && cargo fmt

# CI 전체 검사
npm run ci:check
```

---

## 프로젝트 구조

```
AxInvest/
├── src/                         # 프론트엔드 소스 (React + TypeScript)
│   ├── components/              # React 컴포넌트(25개 모듈)
│   │   ├── chat/               # 채팅 인터페이스(100+ 컴포넌트)
│   │   ├── stock-analysis/     # 스마트 투자 분석(16개 컴포넌트)
│   │   ├── workflow/           # 워크플로 편집기(16종 노드 + 속성 패널 + AI 패널)
│   │   ├── gateway/            # API 게이트웨이 컴포넌트
│   │   ├── settings/           # 설정 패널(50+ 컴포넌트)
│   │   ├── terminal/           # 터미널 컴포넌트
│   │   ├── skill/              # 스킬 편집기 및 렌더러
│   │   ├── benchmark/          # 벤치마크
│   │   ├── files/              # 파일 관리
│   │   ├── fine-tune/          # LoRA 미세 조정
│   │   ├── link/               # 외부 링크
│   │   ├── llm-wiki/           # LLM Wiki
│   │   ├── proactive/          # 능동적 제안
│   │   ├── wiki/               # Wiki 관리
│   │   ├── devtools/           # 개발자 도구
│   │   ├── decomposition/      # 스킬 분해
│   │   ├── recommendation/     # 도구 추천
│   │   ├── style/              # 코드 스타일
│   │   ├── layout/             # 레이아웃 컴포넌트
│   │   ├── help/               # 도움말 패널
│   │   ├── notification/       # 알림 센터
│   │   ├── search/             # 세션 검색
│   │   ├── onboarding/         # 온보딩 마법사
│   │   ├── common/             # 공통 컴포넌트
│   │   └── shared/             # 공유 컴포넌트
│   ├── pages/                   # 페이지 컴포넌트(22개 페이지)
│   ├── stores/                  # Zustand 상태 관리(65개 store)
│   │   ├── domain/            # 코어 비즈니스 상태(9개)
│   │   ├── feature/           # 기능 모듈 상태(46개)
│   │   ├── devtools/          # 개발자 도구 상태(5개)
│   │   └── shared/            # 공유 상태(5개)
│   ├── hooks/                   # React hooks(12개)
│   ├── lib/                     # 유틸리티 함수(33개 모듈 + Web Worker)
│   ├── types/                   # TypeScript 타입 정의(22개)
│   ├── sdk/                     # SDK(TypeScript + Python)
│   └── i18n/                    # 11개 언어 번역
│
├── src-tauri/                    # 백엔드 소스 (Rust)
│   ├── crates/                  # Rust workspace(20개 crates)
│   │   ├── agent/             # AI 에이전트 코어(70+ 소스 파일)
│   │   ├── astock-data/       # A주 데이터 소스(9개 데이터 소스, 22종 데이터 라우팅, 기술 지표, 거래일 캘린더)
│   │   ├── core/              # 코어 유틸리티(85+ 엔티티, 40+ 리포지토리, RAG, 암호화, MCP)
│   │   ├── gateway/           # API 게이트웨이(주식 API 엔드포인트 포함)
│   │   ├── migration/         # 데이터베이스 마이그레이션(5개 마이그레이션)
│   │   ├── npm/               # npm 패키지 파싱
│   │   ├── plugins/           # 플러그인 시스템
│   │   ├── prompt-guard/      # 프롬프트 인젝션 방어
│   │   ├── providers/         # 모델 프로바이더 어댑터
│   │   ├── rt-dashboard/      # 대시보드 플러그인
│   │   ├── rt-messaging/      # 메시지 게이트웨이(9개 플랫폼)
│   │   ├── rt-theme/          # 테마 엔진
│   │   ├── rt-webhook/        # Webhook 서버
│   │   ├── rt-workflow/       # 워크플로 엔진(16종 노드 실행기)
│   │   ├── runtime/           # 런타임 서비스(70+ 소스 파일)
│   │   ├── runtime-core/      # 런타임 추상화 레이어
│   │   ├── stock-analysis/    # 스마트 투자 분석(23개 서브모듈)
│   │   ├── telemetry/         # 추적 및 지표
│   │   ├── tools/             # 도구 시스템(40+ 내장 도구)
│   │   └── trajectory/        # 학습 시스템(55+ 소스 파일)
│   └── src/                    # Tauri 진입점(91개 명령 모듈)
│       ├── commands/          # 명령 모듈
│       │   ├── stock_analysis.rs        # 주식 분석 명령
│       │   ├── stock_analysis_setup.rs  # 주식 분석 구성
│       │   ├── stock_workflow.rs        # 주식 워크플로 명령
│       │   ├── agency_expert.rs         # 전문가 에이전트
│       │   ├── agent_advanced.rs        # 고급 에이전트
│       │   ├── agent_analytics.rs       # 에이전트 분석
│       │   ├── agent_insight.rs         # 에이전트 인사이트
│       │   ├── agent_nudge.rs           # 에이전트 넛지
│       │   ├── agent_profile.rs         # 에이전트 프로필
│       │   ├── agent_role.rs            # 에이전트 역할
│       │   ├── background_tasks.rs      # 백그라운드 작업
│       │   ├── browser.rs              # 브라우저 자동화
│       │   ├── chart_generator.rs       # 차트 생성
│       │   ├── cloud_workspace.rs       # 클라우드 워크스페이스
│       │   ├── computer_control.rs      # 컴퓨터 제어
│       │   ├── context_breakdown.rs     # 컨텍스트 분해
│       │   ├── conversation_categories.rs  # 대화 분류
│       │   ├── conversations_search.rs  # 대화 검색
│       │   ├── crash_report.rs          # 크래시 리포트
│       │   ├── dream.rs                # 드림 통합
│       │   ├── evolution.rs            # 스킬 진화
│       │   ├── fine_tune.rs            # LoRA 미세 조정
│       │   ├── gateway.rs              # API 게이트웨이
│       │   ├── gateway_link.rs         # 외부 링크
│       │   ├── generated_tool.rs        # 생성 도구
│       │   ├── image_gen.rs            # 이미지 생성
│       │   ├── knowledge.rs            # 지식 베이스
│       │   ├── llm_wiki.rs             # LLM Wiki
│       │   ├── local_models.rs         # 로컬 모델
│       │   ├── mcp.rs                  # MCP 프로토콜
│       │   ├── memory.rs              # 메모리 시스템
│       │   ├── message_continuation.rs  # 메시지 속행
│       │   ├── onboarding.rs           # 온보딩 마법사
│       │   ├── parallel_execution.rs    # 병렬 실행
│       │   ├── plan.rs                 # 계획 관리
│       │   ├── platform_integration.rs  # 플랫폼 통합
│       │   ├── plugin.rs               # 플러그인 관리
│       │   ├── proactive.rs            # 능동적 제안
│       │   ├── prompt_templates.rs      # 프롬프트 템플릿
│       │   ├── providers.rs            # 모델 프로바이더
│       │   ├── quickbar.rs             # QuickBar
│       │   ├── reflection.rs           # 반성
│       │   ├── research.rs             # 심층 연구
│       │   ├── rl.rs                   # 강화 학습
│       │   ├── sandbox.rs              # 샌드박스
│       │   ├── scheduled_task.rs        # 정기 작업
│       │   ├── screen_vision.rs        # 화면 비전
│       │   ├── search.rs               # 검색
│       │   ├── session_share.rs         # 세션 공유
│       │   ├── shell.rs                # Shell
│       │   ├── skill_decomposition.rs   # 스킬 분해
│       │   ├── skills_hub.rs           # 스킬 허브
│       │   ├── tool_recommender.rs      # 도구 추천
│       │   ├── tracer.rs               # 추적
│       │   ├── user_profile.rs          # 사용자 프로필
│       │   ├── webdav.rs               # WebDAV
│       │   ├── webhook.rs              # Webhook
│       │   ├── wiki.rs                 # Wiki
│       │   ├── work_engine.rs          # 워크 엔진
│       │   ├── workflow_ai.rs          # AI 워크플로
│       │   ├── workflow_template.rs     # 워크플로 템플릿
│       │   └── ...                     # 기타 명령
│       ├── init/              # 초기화 모듈
│       ├── stock_scheduler.rs # 주식 스케줄러
│       └── ...                # 기타 코어 모듈
│
├── extension/                  # 브라우저 확장(Wiki Clipper: popup/content/background)
├── e2e/                        # Playwright E2E 테스트(9개 테스트 스위트)
├── scripts/                    # 빌드 및 도구 스크립트
└── website/                    # 프로젝트 웹사이트(VitePress, 11개 언어 문서)
```

## 데이터 디렉토리

```
~/.axinvest/                     # 구성 디렉토리
├── axinvest.db                  # SQLite 데이터베이스
├── master.key                   # AES-256 마스터 키
├── vector_db/                   # 벡터 데이터베이스 (sqlite-vec)
└── ssl/                         # SSL 인증서

~/Documents/axinvest/           # 사용자 파일 디렉토리
├── images/                     # 이미지 첨부 파일
├── files/                      # 파일 첨부 파일
└── backups/                    # 백업 파일
```

---

## FAQ

### macOS: "앱이 손상되었습니다" 또는 "개발자를 확인할 수 없습니다"

앱이 Apple에서 서명하지 않았기 때문에:

**1. "모든 곳"의 앱 허용**
```bash
sudo spctl --master-disable
```

그런 다음 **시스템 설정 → 개인정보 보호 및 보안 → 보안**으로 이동하여 **모든 곳**을 선택합니다.

**2. 격리 속성 제거**
```bash
sudo xattr -dr com.apple.quarantine /Applications/AxInvest.app
```

**3. macOS Ventura+ 추가 단계**
**시스템 설정 → 개인정보 보호 및 보안**으로 이동하여 **그래도 열기**를 클릭합니다.

---

## 커뮤니티

- [LinuxDO](https://linux.do)

## 라이선스

이 프로젝트는 [AGPL-3.0](LICENSE) 라이선스 하에 오픈소스로 제공됩니다.
