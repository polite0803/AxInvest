[**English**](./README-EN.md) | [简体中文](./README.md) | [繁體中文](./README-ZH-TW.md) | **日本語** | [한국어](./README-KO.md) | [Français](./README-FR.md) | [Deutsch](./README-DE.md) | [Español](./README-ES.md) | [Русский](./README-RU.md) | [हिन्दी](./README-HI.md) | [العربية](./README-AR.md)

[![AxInvest](https://github.com/polite0803/AxAgent/blob/main/src/assets/image/logo.png?raw=true)](https://github.com/polite0803/AxAgent)

<p align="center">
  <a href="https://www.producthunt.com/products/axagent?embed=true&amp;amp&utm_source=badge-featured&amp;amp;&amp;#10;&amp;amp&amp;amp;;utm_medium=badge&amp;amp;#10&amp;amp;;utm_campaign=badge-axagent" target="_blank" rel="noopener noreferrer"><img alt="AxInvest - AI 駆動のスマート投資分析プラットフォーム | Product Hunt" width="250" height="54" src="https://api.producthunt.com/widgets/embed-image/v1/featured.svg?post_id=1118403&amp;theme=light&amp;t=1775627359538"></a>
</p>

<p align="center">
  <strong>AI 駆動のスマート投資分析 | マルチエージェント協調 | ローカルファースト</strong>
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

## AxInvest とは？

**AxInvest v2.3** は、AI 駆動のスマート投資分析プラットフォームであり、AxAgent マルチエージェントフレームワーク上に構築されています。先進的な AI エージェント能力とプロフェッショナルな A 株投資分析を深く融合し、マルチプロバイダーモデルサポート、AI エージェントリサーチ、ビジュアルワークフローオーケストレーション、ローカルナレッジ管理、内蔵 API ゲートウェイを備え、**Windows / macOS / Linux / Android / iOS** の 5 プラットフォームに対応し、**デスクトップ、タブレット、スマートフォン**の 3 段階デバイスにアダプティブレイアウトを適用します。

AxInvest の核心的な特色は、マルチエージェント対抗ディベート、ディープリサーチ、ファクトチェックなどのメカニズムを活用し、投資判断に包括的かつ客観的な分析サポートを提供することにあります。

---

## スクリーンショット

| チャットとモデル選択 | マルチエージェントダッシュボード |
|:---:|:---:|
| ![](.github/images/s1-0412.png) | ![](.github/images/s5-0412.png) |

| ナレッジベース RAG | メモリとコンテキスト |
|:---:|:---:|
| ![](.github/images/s3-0412.png) | ![](.github/images/s4-0412.png) |

| ワークフローエディタ | API ゲートウェイ |
|:---:|:---:|
| ![](.github/images/s9-0412.png) | ![](.github/images/s10-0412.png) |

---

## コア機能

### 📈 スマート投資分析

AxInvest の核心的特色モジュール。AI エージェント能力とプロフェッショナルな投資分析を深く融合：

**マルチソースデータ集約とフェイルオーバー**

- **9 大データソース** — 騰訊財経、通達信 (mootdx)、東方財富、新浪財経、百度股票、同花順 (THS)、問財 (Iwencai)、巨潮資訊 (cninfo)、AKShare
- **22 種データルーティング** — 各データタイプにマルチソースフェイルオーバールーティングを設定、プライマリソースが利用不可の場合に自動的にバックアップソースへ切替
- **並行データ収集** — `tokio::join!` で 16 種の個別銘柄データ + 5 種の市場データを並行取得、収集効率を最大化
- **インテリジェントキャッシュ** — LRU メモリキャッシュ（上限 1000 件）、行情 30s TTL / K 線 300s TTL、自動期限切れ排除
- **ヘルスチェック** — プロバイダー接続性プローブ（平安銀行 000001 をプローブに使用）、ランタイムでのデータソース可用性検出をサポート

**A 株市場識別とルール**

- **セクター識別** — コードプレフィックスに基づく自動識別：上海メインボード(6)、科創板(688)、深センメインボード(0)、創業板(3)、北京証券取引所(8)
- **ストップ高/ストップ安ルール** — 科創板/創業板 ±20%、北京証券取引所 ±30%、メインボード ±10%、ST 銘柄 ±5%
- **取引カレンダー** — 2025-2026 年の A 株祝日・振替営業日を内蔵、取引日判定をサポート

**個別銘柄データ（16 種）**

- **リアルタイム行情** — 価格、騰落率、出来高/売買代金、回転率、PE/PB、時価総額、ストップ高/ストップ安価格、ST 表示
- **K 線データ** — 7 種の周期（5 分/15 分/30 分/60 分/日/週/月）、出来高、売買代金、回転率を含む
- **財務分析** — 売上高、純利益、EPS、BPS、ROE、負債率、粗利率、純利率、売上高前年同期比、利益前年同期比
- **資金フロー** — 主力/超大口/大口/中口/小口の純流入
- **龍虎榜** — 営業部売買金額、純額、上榜理由
- **限售解除** — 解除日、解除株数、解除割合、株主情報
- **信用取引** — 融資買付額/残高、貸株売付量/残量
- **北向資金** — 持株数、持株割合、変動数量
- **業種分類** — 申万一次/二次業種、コンセプトセクタータグ
- **主要株主増減持** — 重要株主の増減持動向、増減持理由
- **配当記録** — 権利落ち日、1 株当たり配当金、送転比率、権利確定日
- **レポート集約** — 証券会社リサーチレポート、機関、アナリスト、レーティング、目標株価、EPS 予測を含む
- **コンセンサス EPS** — 機関コンセンサス EPS、コンセンサス目標株価、平均レーティング、レーティング数
- **コンセプトセクター** — 3 次元帰属（業種/コンセプト/地域）、セクター騰落率を含む
- **公告検索** — 巨潮資訊の上場企業公告、公告タイプと PDF リンクを含む
- **ニュース・センチメント** — ニュースタイトル/要約/ソース、センチメントスコアを含む

**市場データ（5 種）**

- **全市場龍虎榜** — 当日全上榜銘柄、純買入、売買金額を含む
- **人気銘柄** — 同花順の強勢銘柄、騰落率、回転率、理由タグ、所属セクターを含む
- **業種ランキング** — 申万業種騰落率、売買代金、リード銘柄
- **財聯社速報** — リアルタイム財経速報、タイトル、内容、ソースを含む
- **北向資金フロー** — 上海/深セン/合計の分単位資金フロー

**テクニカル指標計算（indicators モジュール）**

- **移動平均線システム** — MA5/MA10/MA20/MA60、配列状態判定を含む（強気/弱気/弱強気/絡み合いクロス）
- **MACD** — DIF/DEA/ヒストグラム、シグナル判定を含む（ゴールデンクロス/デッドクロス/強気推移/弱気推移）
- **RSI** — RSI6/RSI12/RSI24、シグナル判定を含む（買われすぎ/売られすぎ/強勢/弱勢/ニュートラル）
- **ボリンジャーバンド** — 上バンド/中央バンド/下バンド (20,2)、位置判定を含む（上バンド以上/上バンド域/中央バンド付近/下バンド域/下バンド以下）
- **乖離率** — MA5 乖離率、MA20 乖離率
- **出来高分析** — 出来高比率（当日出来高/5 日平均出来高）、シグナル判定を含む（出来高増加上昇/出来高減少調整/出来高増加下落/出来高減少上昇/正常）
- **サポート/レジスタンス** — 直近の高値・安値と移動平均線に基づく自動計算

**MCP ツール登録（mcp_tools モジュール）**

- 株式データ機能を MCP プロトコルを通じて標準ツールとして登録、AI エージェントが会話中に直接呼び出し可能
- 登録ツール：search_stock、get_stock_quote、get_stock_kline、get_stock_financials、get_stock_news、get_stock_money_flow、get_stock_dragon_tiger 等

**AI 分析パイプライン（stock-analysis crate、23 サブモジュール）**

- **分析オーケストレーション** — orchestrator（パイプラインオーケストレーション）、pipeline（多段階パイプライン）、runner（タスク実行器）
- **意思決定エンジン** — decision（投資意思決定）、signals（取引シグナル生成）、rules（取引ルールエンジン）
- **リスク評価** — risk（リスク評価モデル）、portfolio_risk（ポートフォリオリスク）、position_limits（ポジション制限とコンプライアンス）
- **スクリーニングとバックテスト** — screener（多条件スクリーナー）、backtest（ストラテジーバックテストエンジン）、trading（取引ストラテジーフレームワーク）
- **バリュー投資** — value（バリュー分析）、value_investing（バリュー投資評価フレームワーク）
- **品質管理** — quality（データ品質チェック）、data_clean（データクリーニングと前処理）、review（分析結果レビュー）
- **レポートとスコアリング** — report（分析レポート生成）、scoring（総合スコアリングシステム）
- **補助モジュール** — key_levels（キーレベル識別）、monitor（リアルタイム監視とアラート）、plugin（分析プラグイン拡張）、prompts（AI プロンプトテンプレート）

**フロントエンド分析コンポーネント（16 個）**

- StockAnalysisPage、StockQuoteCard、KLineChart、RiskMatrix、TradePanel
- DecisionBanner、DebatePanel、WatchlistPanel、PriceAlertPanel、CompareView
- AnalystReportGrid、AnalystReportCard、HistoricalAnalysisPanel、StockSearchBar
- AnalysisProgress、StockAnalysisSettingsModal、StockAnalysisChatIndicator

**対抗ディベートと意思決定**

- **対抗ディベート** — マルチエージェント Pro/Con ディベート、論点強度スコアリングと反論追跡をサポート
- **意思決定バナー** — 買い/売り/ホールド意思決定の可視化、信頼度と理由を含む
- **AI ワークフロー統合** — 株式分析ワークフローとチャットのシームレスな連携（stockWorkflowChatBridge）

### 🤖 AI モデルサポート

- **マルチプロバイダーサポート** — OpenAI、Anthropic Claude、Google Gemini、Ollama、OpenClaw、Hermes およびすべての OpenAI 互換 API とのネイティブ統合
- **マルチキーローテーション** — 各プロバイダーに対して複数の API キーを設定可能、自動ローテーションでレート制限を分散
- **ローカルモデル推論** — Ollama ローカルモデルの完全なサポート、GGUF/GGML ファイル管理を含む
- **Candle 推論エンジン** — 内蔵 Candle ローカル推論、rerank/judge インターフェース対応、GGUF オンデマンドダウンロード
- **モデル管理** — リモートモデルリストの取得、カスタマイズ可能なパラメータ（temperature、max tokens、top-p など）
- **ストリーミング出力** — リアルタイムのトークン単位レンダリング、折りたたみ可能な思考ブロック（Claude 拡張思考）をサポート
- **マルチモデル比較** — 複数のモデルに同時に同じ質問を送信し、サイドバイサイドで結果を比較
- **関数呼び出し** — サポートされているすべてのプロバイダーにわたる構造化関数呼び出し
- **OpenAI Responses API** — OpenAI Responses 形式の転送をサポート
- **リアルタイム API** — OpenAI リアルタイム API 互換の WebSocket イベントプッシュ
- **AI 画像生成** — AI 画像生成パネル、複数モデルとパラメータ設定をサポート

### 🔐 AI エージェントシステム

エージェントシステムは高度なアーキテクチャに基づいて構築され（agent crate、70+ ソースファイル）、以下の機能を備えています：

- **ReAct 推論エンジン** — 推論と行動を統合し、自己検証を組み込んでタスク実行の信頼性を確保
- **階層的プランナー** — 複雑なタスクを段階と依存関係を持つ構造化されたプランに分解
- **タスク分解器** — 複雑なタスクを実行可能なサブタスクに自動分解
- **思考チェーン** — エージェントの意思決定推論の可視化、ステップバイステップ分解
- **思考の木** — tree_of_thoughts マルチパス推論探索
- **ディープリサーチ** — マルチソース検索オーケストレーション、引用追跡と信頼性評価
- **ファクトチェック** — AI 駆動の事実検証とソース分類
- **検索オーケストレーション** — マルチ検索プロバイダー調整、検索計画と結果統合をサポート
- **学術検索** — 学術文献検索と引用分析
- **コンピュータ制御** — AI 制御のマウスクリック、キーボード入力、画面スクロール、ビジョンモデル分析との連携
- **画面知覚** — スクリーンキャプチャとビジョンモデル分析、UI 要素の識別に使用
- **ビジョンパイプライン** — vision_pipeline 画像理解と分析
- **3 段階の権限モード** — デフォルト（承認が必要）、編集を受け入れる（自動承認）、完全アクセス（プロンプトなし）
- **サンドボックス分離** — エージェント操作は指定された作業ディレクトリに厳密に制限
- **ツール承認パネル** — ツール呼び出しリクエストのリアルタイム表示、項目ごとの承認をサポート
- **コスト追跡** — 各セッションのトークン使用量とコスト統計のリアルタイム表示
- **一時停止/再開** — エージェントの実行をいつでも一時停止し、後から再開
- **チェックポイントシステム** — クラッシュ回復とセッション再接続のための永続化チェックポイント
- **エラー回復エンジン** — 自動エラー分類、根本原因分析と回復戦略の実行
- **ループ検出** — エージェント推論中の循環動作の自動検出と中断
- **プロアクティブモード** — エージェントが自発的に提案と操作を実行
- **目的管理** — エージェントの実行目的とコンテキストの維持と追跡
- **自己検証** — self_verifier エージェント出力の正確性を自動検証
- **リフレクター** — reflector 推論プロセスの振り返りと改善
- **ステアリング入力** — steer_manager エージェントの行動方向を動的に調整
- **イベントバス** — event_bus / event_emitter エージェントイベント駆動アーキテクチャ
- **コンテンツ統合** — content_synthesizer マルチソース情報統合とレポート生成
- **引用追跡** — citation_tracker 情報ソースの自動追跡と注記
- **信頼性評価** — credibility_evaluator 情報ソースの信頼性評価
- **アウトライン構築** — outline_builder リサーチアウトラインの自動構築
- **スキーマ管理** — schema_manager 出力構造スキーマの管理
- **プロジェクトメモリ** — project_memory プロジェクトレベルの永続化メモリ
- **環境プローブ** — environment_probe 実行環境情報の自動検出
- **ヘルスチェック** — health_checker エージェントヘルス状態監視

### 👥 マルチエージェント協調

- **サブエージェント調整** — マスター・スレーブアーキテクチャ、coordinator が複数の協調エージェントを調整
- **並列実行** — 複数のエージェントがタスクを並行処理、依存関係認識スケジューリングをサポート
- **敵対的ディベート** — adversarial_debate Pro/Con ディベートラウンド、論点強度スコアリングと反論追跡をサポート
- **エージェントロール** — agent_roles チームコラボレーションのための定義済みロール（研究者、プランナー、開発者、レビュアー、シンセサイザー）
- **エージェントオーケストレーター** — マルチエージェントチームの集中型メッセージルーティングと状態管理
- **コミュニケーショングラフ** — graph_insights エージェントの相互作用とメッセージフローの可視化
- **共有ブラックボード** — shared_blackboard / blackboard エージェント間共有状態空間
- **Buddy パートナーシステム** — 設定可能なエージェントパートナー、種と属性の定義をサポート
- **共有メモリ** — エージェント間で共有されるメモリ空間、統計とクエリをサポート
- **チーム Cron 登録** — チームレベルの定期タスクスケジューリング
- **エキスパートシステム** — agency_expert ドメインエキスパートエージェント
- **エージェントプロファイル** — agent_profile エージェントの個性と能力プロファイル管理

### ⭐ スキルシステム

- **スキルマーケットプレイス** — 組み込みマーケットプレイスでコミュニティ貢献のスキルを閲覧とインストール
- **スキル作成** — プロポーザルから自動的にスキルを作成、Markdown エディタをサポート
- **スキル進化** — skill_evolution 実行フィードバックに基づく AI 駆動の既存スキルの自動分析と改善
- **スキルマッチング** — skill_matcher 意味的マッチングで会話コンテキストに関連するスキルを推奨
- **スキル分解** — 複雑なタスクの自動分解と実行可能なアトミックスキルへの変換（LLM 支援/マルチターン/ワークフロー検証）
- **生成ツール** — AI による新しいツールの自動生成と登録、エージェント能力を拡張
- **スキルハブ** — skills_hub_adapter 集中型のスキル発見と設定管理インターフェース
- **スキルハブクライアント** — skills_hub_client リモートスキルハブとの統合、コミュニティ共有をサポート
- **スキル依存チェック** — スキル依存関係とツール可用性の自動検出
- **スキルサンドボックスコンテナ** — スキルを隔離環境で安全に実行
- **アトミックスキル** — atomic_skill 最小実行可能スキルユニット
- **スキルプロポーザル** — skill_proposal AI 駆動のスキル作成プロポーザル

### 🔄 ワークフローシステム

ワークフローエンジン（rt-workflow crate）は DAG ベースのタスクオーケストレーションシステムを実装しています：

- **ビジュアルワークフローエディタ** — ドラッグ＆ドロップ式のワークフローデザイナー、ノード接続と設定をサポート
- **16 種ノードタイプ** — トリガー、エージェント、LLM、条件、並列、ループ、マージ、遅延、ツール、コード、サブワークフロー、ベクター検索、ドキュメントパーサー、検証、終了、フォールバック（fallback）
- **16 種プロパティパネル** — 各ノードタイプに対応する独立した設定パネル
- **ワークフローテンプレート** — 組み込みプリセット：コードレビュー、バグ修正、ドキュメント、テスト、リファクタリング、探索、パフォーマンス、セキュリティ、機能開発
- **DAG 実行** — Kahn アルゴリズムによるトポロジカルソート、循環検出をサポート
- **並列ディスパッチ** — パイプラインスタイルの実行、高速ステップは低速ステップを待ちません
- **再試行ポリシー** — 指数バックオフ、各ステップで設定可能な最大再試行回数
- **部分完了** — 失敗したステップは独立した下流ステップをブロックしません
- **バージョン管理** — ワークフローテンプレートのバージョン管理、ロールバックをサポート
- **実行履歴** — 詳細な記録、ステータス追跡とデバッグをサポート
- **AI 支援** — AI 支援ワークフロー設計、ノード推奨とエージェントプロンプト最適化
- **セマンティックチェック** — ワークフローのセマンティック検証、潜在的な問題を検出
- **n8n インポート** — n8n ディレクトリからのワークフローインポートをサポート
- **デバッグパネル** — ワークフロー実行プロセスのリアルタイムデバッグと状態確認
- **キャッシュレイヤー** — cache_layer ワークフロー実行結果キャッシュ
- **マーケットプレイス** — workflow_marketplace ワークフローテンプレートマーケットプレイスとレビュー

### 📚 ナレッジとメモリ

- **ナレッジベース（RAG）** — マルチナレッジベースサポート、ドキュメントアップロード、自動解析、チャンク化、ベクターインデックスをサポート
- **ハイブリッド検索** — ベクター類似性検索と BM25 全文ランキングの組み合わせ
- **リランキング** — クロスエンコーダーリランキング、取得精度の向上
- **3 段階リコールパイプライン** — AST インデックス + ベクター検索 + FTS5 のマルチレベルリコール機構
- **Self-RAG** — self_rag 自己検索拡張生成
- **クエリ拡張** — query_enhancement クエリの書き換えと拡張
- **ナレッジグラフ** — ナレッジ関連性のエンティティ関係可視化（エンティティ、属性、関係、フロー、インターフェース）
- **Wiki システム** — LLM Wiki コンパイラとバリデーター、ナレッジグラフ可視化と増分同期をサポート
- **Wiki ノート** — 双方向リンクノートシステム、グラフビューと自動リンク同期をサポート
- **メモリシステム** — マルチ名前空間メモリ、手動入力または AI 自動抽出をサポート
- **クローズドループメモリ** — Honcho と Mem0 永続化メモリプロバイダーとの統合
- **メモリ忘却** — memory_forgetting 時間ベースのメモリ減衰機構
- **FTS5 全文検索** — 会話、ファイル、メモリ全体の高速検索
- **セッション検索** — すべての会話セッション全体の高度な検索
- **コンテキスト管理** — ファイル、検索結果、ナレッジスニペット、メモリ、ツール出力の柔軟な添付
- **ドキュメントパーサー** — マルチフォーマットドキュメントの自動解析とコンテンツ抽出
- **増分インデックス** — ファイル変更の増分インデックス更新
- **テキストチャンカー** — text_chunker インテリジェントテキストチャンキング戦略
- **トークン予算** — token_budget 検索結果のトークン予算制御

### 🌐 API ゲートウェイ

- **ローカル API サーバー** — 組み込みの OpenAI 互換、Claude、Gemini インターフェースサーバー
- **外部リンク** — ワンクリックで Claude CLI、OpenCode との統合、API キーとモデルの自動同期
- **キー管理** — 生成、取り消し、有効化/無効化、説明付きアクセスキーの管理
- **使用量分析** — キー、プロバイダー、日付ごとのリクエスト量とトークン使用量
- **SSL/TLS サポート** — 組み込み自己署名証明書、カスタム証明書をサポート
- **リクエストログ** — すべての API リクエストとレスポンスの完全な記録
- **設定テンプレート** — Claude、Codex、OpenCode、Gemini のプリセットテンプレート
- **リアルタイム API** — OpenAI リアルタイム API 互換の WebSocket イベントプッシュ
- **プラットフォーム統合** — 钉钉、飛書、QQ、Slack、WeChat、WhatsApp、Telegram、Discord のサポート
- **ゲートウェイ診断** — 接続診断とプログラムポリシー管理
- **レートリミッター** — API リクエストのレート制限とトラフィック制御
- **永続化キュー** — リクエストの永続化キュー管理
- **株式 API** — stock_handlers 株式データ専用 API エンドポイント
- **SSE プッシュ** — sse Server-Sent Events リアルタイムイベントプッシュ

### 🔧 ツールと拡張

- **MCP プロトコル** — 完全なモデルコンテキストプロトコル実装、stdio と HTTP/WebSocket トランスポートをサポート
- **OAuth 認証** — MCP サーバーの OAuth フローサポート
- **MCP 自動起動** — MCP サーバーの自動起動とライフサイクル管理
- **MCP ツールブリッジ** — MCP ツールとエージェントツールシステムのブリッジング
- **MCP ヘルスチェック** — mcp_health MCP サーバーヘルス状態監視
- **プラグインシステム** — OpenClaw 互換の 3 層プラグインアーキテクチャ（内蔵/バンドル/外部）、npm パッケージインストール、ツール登録、フックとライフサイクル管理をサポート
- **プラグインマーケットプレイス** — 内蔵マーケットプレイス UI、npm 検索インストール、確認ダイアログをサポート
- **組み込みツール** — 40+ ツールモジュール：ファイル操作（読み/書き/編集/システム）、コード実行、検索（Grep/Glob）、Bash、Web 検索/スクレイピング、プラン管理、Cron スケジューリング、REPL、LSP、コンテキスト管理、コンピュータ制御、メッセージプッシュ、ToDo、データベース、DevOps、ドキュメント解析、Git、ナレッジ検索、LSP、メディア処理、メッセージプッシュ、OCR、プッシュ通知、システム情報、タスクシステム、テスト、ワークスペース/ワークツリー等
- **ツール権限システム** — ツール権限の分類、ルール管理と使用追跡
- **Bash セキュリティ** — コマンド解析、パス検証とサンドボックスセキュリティ制御
- **LSP クライアント** — 組み込み言語サーバープロトコル、コード補完と診断をサポート
- **AST インデックス** — コードファイルの AST 解析とインデックス構築
- **ターミナルバックエンド** — ローカル、Docker、SSH ターミナル接続をサポート
- **ブラウザ自動化** — CDP によるブラウザ制御機能の統合（ナビゲーション、スクリーンショット、クリック、入力、テキスト抽出など）
- **UI 自動化** — クロスプラットフォーム UI 要素識別と制御
- **Git ツール** — ブランチ検出と競合認識をサポートする Git 操作
- **ツール推奨** — コンテキストベースのインテリジェントツール推奨エンジン
- **ツールオーケストレーション** — マルチツールの協調実行とストリーミング出力
- **ツール統計** — ツール使用頻度とパフォーマンス統計
- **ツール監査** — audit ツール呼び出し監査ログ

### 📊 コンテンツレンダリング

- **Markdown レンダリング** — コードハイライト、LaTeX 数式、テーブル、タスクリストの完全なサポート
- **Monaco コードエディタ** — 組み込みエディタ、構文ハイライト、コピー、差分プレビューをサポート
- **ダイアグラムレンダリング** — Mermaid フローチャート、D2 アーキテクチャダイアグラム、ECharts インタラクティブチャート
- **アーティファクトパネル** — コードスニペット、HTML ドラフト、React コンポーネント、Markdown ノート、リアルタイムプレビューをサポート
- **4 つのプレビューモード** — コード（エディタ）、スプリット（並列）、プレビュー（レンダリングのみ）、React コンポーネントプレビュー
- **セッションインスペクター** — セッション構造のツリービュー、クイックナビゲーション
- **引用パネル** — ソース引用の追跡と表示、信頼性スコアリングをサポート
- **インフォグラフィックレンダリング** — インフォグラフィックの可視化表示をサポート
- **チャートインタープリター** — ChartInterpreter AI 駆動のチャート解釈
- **Diff ビューアー** — DiffViewer コード差分比較

### 🛡️ データとセキュリティ

- **AES-256 暗号化** — API キーと機密データは AES-256-GCM で暗号化
- **分離ストレージ** — アプリケーション状態は `~/.axinvest/`、ユーザーファイルは `~/Documents/axinvest/` に保存
- **自動バックアップ** — ローカルディレクトリまたは WebDAV ストレージへのスケジュールバックアップ
- **S3 バックアップ** — s3_backup Amazon S3 クラウドバックアップをサポート
- **バックアップ復元** — ワンクリックで履歴バックアップから復元
- **エクスポートオプション** — PNG スクリーンショット、Markdown、プレーンテキスト、JSON 形式
- **ストレージ管理** — 視覚的なディスク使用量表示とクリーンアップツール
- **ストレージマイグレーション** — storage_migration バージョン間のデータマイグレーション
- **ファイル認可** — ファイルアクセスの認可と取り消し管理
- **操作監査** — 重要操作の監査ログ記録
- **コマンド検証** — command_validator コマンドセキュリティ検証
- **リソース制限** — resource_limits リソース使用制限
- **サンドボックス実行** — sandbox_runner 隔離環境での実行

### 🖥️ デスクトップ体験

- **テーマエンジン** — ダーク/ライトテーマ、システムフォローまたは手動設定をサポート
- **インターフェース言語** — 11 の言語：簡体字中文、繁体字中文、英語、日本語、韓国語、フランス語、ドイツ語、スペイン語、ロシア語、ヒンディー語、アラビア語
- **システムトレイ** — バックグラウンドサービスを中断せずにトレイに最小化
- **常に手前** — 他のウィンドウより前にウィンドウを固定
- **グローバルショートカット** — カスタマイズ可能なショートカットでメインウィンドウを呼び出し
- **QuickBar** — クイックアクセスフローティングバー、ワンクリックで起動
- **自動起動** — システム起動時のオプションの起動
- **プロキシサポート** — HTTP と SOCKS5 プロキシ設定
- **自動更新** — 自動バージョン確認と更新プロンプト
- **コマンドパレット** — `Cmd/Ctrl+K` クイックコマンドアクセス
- **オンボーディングウィザード** — 初回起動時のインタラクティブガイドと Ollama 検出
- **通知センター** — 統合されたアプリ内通知管理
- **クラウドワークスペース** — cloud_workspace クラウドワークスペース選択
- **クラッシュレポート** — crash_report 自動クラッシュレポート収集
- **音声通話** — VoiceCall 音声会話機能

### 🔬 上級機能

- **ディープリサーチ** — マルチソース検索、引用追跡、信頼性評価とコンテンツ統合
- **ファクトチェック** — AI 駆動の事実検証とソース分類
- **Cron スケジューラー** — 毎日/毎週/毎月テンプレートとカスタム cron 式による自動化タスクスケジューリング
- **Webhook システム** — ツール完了、エージェントエラー、セッション終了通知のイベントサブスクリプション
- **ユーザープロファイリング** — コードスタイル、命名規則、インデント、コメントスタイル、コミュニケーション設定の自動学習
- **RL オプティマイザー** — ツール選択とタスク戦略の最適化のための強化学習
- **LoRA ファインチューニング** — LoRA によるローカルトレーニングを使用したカスタムモデル適応
- **プロアクティブ提案** — 会話内容とユーザーパターンに基づくコンテキスト対応のヒント
- **コンテキスト予測** — ユーザーの次の操作を予測し、関連リソースを事前取得
- **ドリーム統合** — dream_consolidation バックグラウンドでメモリとパターンを自動統合、長期知識を最適化
- **エラー回復** — 自動エラー分類、根本原因分析、回復提案
- **開発者ツール** — デバッグとパフォーマンス分析のための Trace、Span、タイムライン可視化
- **ベンチマークシステム** — SWE-bench / Terminal-bench タスクパフォーマンス評価と指標、スコアカード付き
- **スタイル転送** — style_migrator 学習したコードスタイル設定を生成されたコードに適用
- **ダッシュボードプラグイン** — カスタムパネルとウィジェットをサポートする拡張可能なダッシュボード
- **コラボレーション共有** — CRDT リアルタイムコラボレーションとワンクリックセッション共有
- **ブラウザ拡張** — Wiki Clipper ブラウザ拡張、Web ページを LLM Wiki に素早くクリップ
- **Python SDK** — AxInvest との統合のための Python SDK を提供
- **スマートルーター** — リクエストのインテリジェントルーティングと分類
- **セマンティックキャッシュ** — セマンティクスベースのレスポンスキャッシュ、重複計算を削減
- **コンテキスト圧縮** — 長いコンテキストの自動圧縮、トークン使用量を最適化
- **メッセージバッチ処理** — メッセージの一括送信と最適化
- **接続プール** — データベースと API 接続プール管理
- **フィーチャーフラグ** — 設定可能な機能フィーチャーフラグシステム
- **ポリシーエンジン** — 権限と操作ポリシーの集中管理
- **リソースガバナー** — エージェントのリソース使用制限とガバナンス
- **LAN 転送** — ローカルエリアネットワークファイル転送機能
- **共進化** — coevolution スキルとエージェントの協調進化
- **行動学習** — behavior_learner / behavior_tracker ユーザー行動の学習と追跡
- **嗜好学習** — preference_learner ユーザー嗜好の自動学習
- **内発的報酬** — intrinsic_reward 内発的動機に基づく探索
- **プロセス報酬** — process_reward プロセスレベル報酬シグナル
- **TextGrad** — text_grad テキスト勾配に基づく自動最適化
- **軌跡圧縮** — trajectory_compressor 長軌跡の自動圧縮
- **リマインダー管理** — reminder_manager インテリジェントリマインダースケジューリング
- **タスクプリフェッチ** — task_prefetcher 予測的タスクリソースプリフェッチ

### 🛡️ プロンプトインジェクション防護（Prompt-Guard）

- **4 段階防護体系** — L1 パターン検出（高リスクブロック + 中リスクマーク）→ L2 デリミタエスケープ → L3 XML ラッパー → L4 トラストタグ
- **パイプラインオーケストレーター** — 多段階検出パイプライン直列、カスタムリスク閾値をサポート
- **Token Smuggling 検出** — エンコーディング難読化とトークン密輸攻撃に対する特化検出
- **デリミタエスケープ検出** — delimiter_escape プロンプトデリミタエスケープ攻撃の検出
- **パターン検出** — pattern_detect 正規表現 + ヒューリスティックインジェクションパターンマッチング
- **トラストタグ** — trust_labels 信頼できるコンテンツのマーキングと検証
- **Strict モード** — 厳格モードテスト + 中リスク原因命名 + カスタムモードドキュメント
- **全パイプライン統合** — session / prompt / git / RAG 各セクションに統合済み

### 📱 モバイルサポート

- **Android ネイティブ** — APK/AAB ビルド、arm64-v8a / armeabi-v7a / x86_64 対応
- **iOS ネイティブ** — IPA ビルド、arm64 対応
- **アダプティブレイアウト** — デスクトップ/タブレット/スマートフォンの 3 段階自動適応（useResponsive hook）
- **モバイルナビゲーション** — Drawer スライドナビゲーション + ボトムナビゲーションバー + フラッシュフローティングボタン
- **セーフエリア適応** — Android システムステータスバー/ナビゲーションバー CSS env() 自動適応
- **CSP 最適化** — Android WebView CSP プロトコルホワイトリスト
- **条件コンパイル** — `#[cfg(not(mobile))]` デスクトップ専用機能（ブラウザ、コンピュータ制御、デスクトップ、QuickBar、ターミナル、画面ビジョン）を自動除外

---

## 技術アーキテクチャ

### 技術スタック

| レイヤー | 技術 |
|---------|------|
| **フレームワーク** | Tauri 2 + React 19 + TypeScript 6 |
| **UI** | Ant Design 6 + TailwindCSS 4 |
| **状態管理** | Zustand 5 |
| **ルーティング** | React Router 7 |
| **国際化** | i18next + react-i18next |
| **バックエンド** | Rust 2024 + SeaORM 2 + SQLite |
| **ベクトル DB** | sqlite-vec |
| **コードエディタ** | Monaco Editor |
| **ダイアグラム** | Mermaid + D2 + ECharts（CDN） |
| **ターミナル** | xterm.js 6 |
| **ワークフロー** | ReactFlow 11 |
| **チャートレンダリング** | @antv/infographic |
| **アイコン** | Iconify + Lucide |
| **ドラッグ＆ドロップ** | @dnd-kit |
| **ビルド** | Vite 8 + npm |
| **テスト** | Vitest + Playwright + cargo-nextest |
| **フォーマット** | dprint (TS/JSON) + rustfmt |
| **Lint** | TS: eslint + oxlint / Rust: clippy + cargo-deny |
| **モバイル** | Tauri Android + iOS ネイティブビルド |
| **デスクトップ** | Windows (MSI) · macOS (DMG) · Linux (AppImage/deb/rpm) |

### プラットフォームサポート

| プラットフォーム | アーキテクチャ |
|----------------|---------------|
| Windows | x86_64, ARM64 |
| macOS | Apple Silicon (arm64), Intel (x86_64) |
| Linux | x86_64, ARM64 |
| Android | arm64-v8a, armeabi-v7a, x86_64 (エミュレータ) |
| iOS | arm64 |

### Rust バックエンドアーキテクチャ

バックエンドは、Rust workspace として組織された **20 個** の専門化した crates で構成されています：

```
src-tauri/crates/
├── agent/            # AI エージェントコア（70+ ソースファイル：ReAct エンジン、調整、プランニング、ディープリサーチ、ファクトチェック等）
├── astock-data/      # A 株データソース（9 大データソース、22 種データルーティング、テクニカル指標、取引カレンダー、MCP ツール登録）
├── core/             # コアユーティリティ（85+ データベースエンティティ、40+ リポジトリ、RAG、暗号化、MCP、ブラウザ自動化、AST インデックス等）
├── gateway/          # API ゲートウェイ（HTTP サーバー、認証、ルーティング、OpenAI 互換インターフェース、株式 API エンドポイント）
├── migration/        # データベースマイグレーション（5 マイグレーション：株式分析/ウォッチリスト組み合わせ/分析スケジューリング/価格アラート/取引）
├── npm/              # npm パッケージ解析とレジストリ
├── plugins/          # プラグインシステム（OpenClaw 互換、npm パッケージインストール、サンプルプラグイン付き）
├── prompt-guard/     # プロンプトインジェクション防護（L1-L4 多段階検出と防御、4 種検出器）
├── providers/        # モデルプロバイダーアダプター（OpenAI、Anthropic、Gemini、Ollama、OpenClaw、Hermes、画像生成）
├── rt-dashboard/     # ダッシュボードプラグインシステム
├── rt-messaging/     # メッセージゲートウェイ（9 プラットフォーム：钉钉/飛書/QQ/Slack/WeChat/WhatsApp/Telegram/Discord）
├── rt-theme/         # テーマエンジン
├── rt-webhook/       # Webhook サーバーとディスパッチ
├── rt-workflow/      # ワークフローエンジン（DAG オーケストレーション、16 種ノード実行器、スケジューラー、キャッシュレイヤー）
├── runtime/          # ランタイムサービス（70+ ソースファイル：セッション管理、MCP、ターミナル、レートリミッター、Webhook、権限、ベンチマーク等）
├── runtime-core/     # ランタイム抽象層（共通型、trait 定義、設定、フィーチャーフラグ、権限エグゼキューター）
├── stock-analysis/   # スマート投資分析（23 サブモジュール：パイプライン、意思決定エンジン、リスク評価、バックテスト、スクリーナー、バリュー投資）
├── telemetry/        # テレメトリと分散トレーシング（OpenTelemetry 互換）
├── tools/            # ツールシステム（40+ 内蔵ツール、Bash セキュリティ、MCP ブリッジ、権限システム、オーケストレーション、監査）
└── trajectory/       # 学習システム（55+ ソースファイル：メモリ、スキル、RL、ユーザープロファイリング、ドリーム統合、スタイル転送、共進化）
```

#### stock-analysis crate モジュール構造（23 サブモジュール）

```
stock-analysis/
├── backtest.rs         # ストラテジーバックテストエンジン
├── data_clean.rs       # データクリーニングと前処理
├── decision.rs         # 投資意思決定エンジン
├── key_levels.rs       # キーレベル識別
├── monitor.rs          # リアルタイム監視とアラート
├── orchestrator.rs     # 分析パイプラインオーケストレーション
├── pipeline.rs         # 多段階分析パイプライン
├── plugin.rs           # 分析プラグイン拡張
├── portfolio_risk.rs   # ポートフォリオリスク評価
├── position_limits.rs  # ポジション制限とコンプライアンス
├── prompts.rs          # AI プロンプトテンプレート
├── quality.rs          # データ品質チェック
├── report.rs           # 分析レポート生成
├── review.rs           # 分析結果レビュー
├── risk.rs             # リスク評価モデル
├── rules.rs            # 取引ルールエンジン
├── runner.rs           # 分析タスク実行器
├── scoring.rs          # 総合スコアリングシステム
├── screener.rs         # スクリーナー
├── signals.rs          # 取引シグナル生成
├── trading.rs          # 取引ストラテジーフレームワーク
├── value.rs            # バリュー分析
└── value_investing.rs  # バリュー投資評価
```

#### astock-data crate データソース

| データソース | 識別子 | サポートするデータタイプ |
|------------|--------|----------------------|
| 騰訊財経 | tencent | リアルタイム行情、K 線 |
| 通達信 | mootdx | リアルタイム行情、K 線 |
| 東方財富 | eastmoney | 行情、K 線、財務、資金フロー、龍虎榜、限售解除、信用取引、北向資金、業種分類、主要株主増減持、配当、レポート、全市場龍虎榜、財聯社速報 |
| 新浪財経 | sina | 行情、K 線、ニュース |
| 百度股票 | baidu_stock | 行情、ニュース、資金フロー、龍虎榜、限售解除、信用取引、北向資金、業種分類、主要株主増減持、配当、レポート、人気銘柄、業種ランキング、コンセプトセクター、北向資金フロー |
| 同花順 | ths | 行情、業種分類、コンセンサス EPS、コンセプトセクター、人気銘柄、業種ランキング、北向資金フロー |
| 問財 | iwencai | 銘柄検索、業種分類、コンセンサス EPS、コンセプトセクター、人気銘柄 |
| 巨潮資訊 | cninfo | 公告 |
| AKShare | akshare | 財務、ニュース、コンセンサス EPS、財聯社速報 |

各データタイプにはマルチソースフェイルオーバールーティングが設定されており、プライマリデータソースが利用不可の場合、自動的にバックアップソースに切り替わります。

#### astock-data 追加モジュール

| モジュール | 機能 |
|-----------|------|
| calendar | A 株取引カレンダー（2025-2026 年祝日 + 振替営業日） |
| indicators | テクニカル指標計算（MA/MACD/RSI/ボリンジャーバンド/乖離率/出来高比率/サポートレジスタンス） |
| mcp_tools | MCP ツール登録（株式データ機能を AI 呼び出し可能ツールとして登録） |

### フロントエンドアーキテクチャ

```
src/
├── stores/                    # Zustand 状態管理（65 store）
│   ├── domain/               # コアビジネス状態（9 個）
│   │   ├── agentDomainStore.ts
│   │   ├── compressStore.ts
│   │   ├── conversationPreferences.ts
│   │   ├── conversationStore.ts
│   │   ├── conversationStoreEvents.ts
│   │   ├── conversationStoreSend.ts
│   │   ├── multiModelStore.ts
│   │   ├── preferenceStore.ts
│   │   └── streamStore.ts
│   ├── feature/               # 機能モジュール状態（46 個）
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
│   ├── devtools/              # 開発者ツール状態（5 個）
│   │   ├── evaluatorStore.ts
│   │   ├── fineTuneStore.ts
│   │   ├── recommendationStore.ts
│   │   ├── rlStore.ts
│   │   └── tracerStore.ts
│   └── shared/                # 共有状態（5 個）
│       ├── artifactStore.ts
│       ├── chatWorkspaceStore.ts
│       ├── rightPanelStore.ts
│       ├── tabStore.ts
│       └── uiStore.ts
│
├── components/                # React コンポーネント（25 モジュール）
│   ├── chat/                # チャットインターフェース（100+ コンポーネント：エージェント実行パネル、ブランチ比較、ブラウザ自動化、コード実行器、コラボレーションパネル、ディープリサーチ、ファクトチェック、Git コミット、画像生成/分析、ナレッジ検索、メモリ抽出、モデルルーティング、マルチモデル表示、権限管理、プラグインマーケットプレイス、リフレクションパネル、スキル作成/進化、構造化思考、サブエージェントカード、ツール呼び出しカード、軌跡再生、音声通話、Wiki 検索、ワークフロー進捗等）
│   ├── stock-analysis/      # スマート投資分析（16 コンポーネント）
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
│   ├── workflow/            # ワークフローエディタ（16 種ノード + 16 種プロパティパネル + AI パネル + テンプレート + デバッグ）
│   ├── gateway/             # API ゲートウェイ UI（概要/キー/メトリクス/モニタリング/設定/テンプレート/診断）
│   ├── settings/            # 設定パネル（50+ コンポーネント：プロバイダー/モデル/MCP/ナレッジ/メモリ/プロキシ/ショートカット/テーマ/ツール/Webhook/Cron/株式分析設定等）
│   ├── terminal/            # ターミナル UI（統合ターミナル/Docker/SSH/バックエンド選択/パス補完/スラッシュ補完）
│   ├── skill/               # スキルエディタとレンダラー（アクションチェーン編集/フロントエンドエディタ/サンドボックスコンテナ/依存チェック/統計パネル）
│   ├── benchmark/           # ベンチマークパネル（設定/レポート/セレクター/タスクリスト/結果）
│   ├── files/               # ファイル管理ページ
│   ├── fine-tune/           # LoRA ファインチューニング設定（データセット/トレーニングタスク/LoRA 設定）
│   ├── link/                # 外部リンク管理（概要/モデル/ストラテジー/スキル/ストラテジー詳細）
│   ├── llm-wiki/            # LLM Wiki エディタ（品質スコア/同期ステータス）
│   ├── proactive/           # プロアクティブ提案システム（コンテキスト予測/プリフェッチインジケーター/提案バー/リマインダーリスト）
│   ├── wiki/                # Wiki 管理（バックリンク/グラフビュー/インジェスト/コードチェック/操作タイムライン/タグ集約/バージョン履歴）
│   ├── devtools/            # Trace/Span タイムライン（コストグラフ/所要時間グラフ/詳細/フィルター/リスト）
│   ├── decomposition/       # スキル分解（分解プレビュー/ツール依存/ツール生成/ツールインストール）
│   ├── recommendation/      # ツール推奨パネル
│   ├── style/               # コードスタイル転送（サンプル/調整スライダー/比較/プレビューパネル）
│   ├── layout/              # レイアウトコンポーネント（タイトルバー/サイドバー/コマンドパレット/グローバルコピー/エラーバウンダリー/ステータスバー/通知ベル/ユーザープロファイルモーダル）
│   ├── help/                # ヘルプパネル
│   ├── notification/        # 通知センター
│   ├── search/              # セッション検索
│   ├── onboarding/          # オンボーディングウィザード（インタラクティブチュートリアル/ウェルカムウィザード）
│   ├── common/              # 共通コンポーネント（コピー/アイコン/モデルパラメータスライダー/ペースト）
│   └── shared/              # 共有コンポーネント（アバター編集/モーダル/チャートレンダリング/ダイナミックアイコン/埋め込みモデル選択/Emoji 選択/ナレッジベースアイコン/MCP アイコン/モデル選択/Monaco エディタ/ネームスペースアイコン/検索プロバイダーアイコン）
│
├── pages/                    # ページコンポーネント（22 ページ）
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
├── hooks/                    # React hooks（12 個）
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
├── lib/                      # ユーティリティ関数（33 モジュール + Web Worker）
│   ├── workers/            # Web Worker（heavy.worker.ts）
│   ├── actionRouter.ts     # アクションルーティング
│   ├── artifactRenderer.ts # アーティファクトレンダリング
│   ├── chartGenerator.ts   # チャート生成
│   ├── chatMarkdown.ts     # Markdown レンダリング
│   ├── codeExecutor.ts     # コード実行
│   ├── invoke.ts           # Tauri IPC ラッパー
│   ├── skillActionExecutor.ts  # スキルアクション実行
│   ├── skillEventBus.ts    # スキルイベントバス
│   ├── skillLifecycle.ts   # スキルライフサイクル
│   ├── skillPermissions.ts # スキル権限
│   ├── storeRegistry.ts    # Store レジストリ
│   ├── tokenEstimator.ts   # トークン推定
│   ├── workflowLayout.ts   # ワークフローレイアウト
│   └── ...                 # その他ユーティリティモジュール
│
├── types/                    # TypeScript 型定義（22 個）
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
├── sdk/                      # SDK（Python SDK 含む）
│   ├── index.ts
│   ├── types.ts
│   ├── rpcBridge.ts
│   ├── sandboxTemplate.ts
│   └── python/              # Python SDK
│       ├── setup.py
│       └── axagent_sdk/__init__.py
│
└── i18n/                     # 11 言語翻訳
```

## クイックスタート

### ビルド済みダウンロード

[Releases](https://github.com/polite0803/AxAgent/releases) ページにアクセスし、お使いのプラットフォーム用のインストーラをダウンロードしてください。

### ソースからビルド

#### 必要環境

- [Node.js](https://nodejs.org/) 20+
- [Rust](https://www.rust-lang.org/) 1.75+
- [npm](https://www.npmjs.com/) 10+
- Windows: [Visual Studio Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/) + Rust MSVC targets

#### ビルド手順

```bash
# リポジトリをクローン
git clone https://github.com/polite0803/AxAgent.git
cd AxAgent

# 依存関係をインストール
npm install

# 開発モード
npm run tauri dev

# フロントエンドのみビルド
npm run build

# デスクトップアプリケーションをビルド
npm run tauri build
```

ビルド成果物は `src-tauri/target/release/` にあります。

### テスト

```bash
# ユニットテスト
npm run test          # Vitest watch
npm run test:run      # Vitest 単回実行

# E2E テスト
npm run test:e2e      # Playwright
npm run test:e2e:ui   # Playwright UI モード

# Rust バックエンドテスト
cd src-tauri && cargo nextest run   # cargo-nextest（2-3x 高速）
cd src-tauri && cargo test          # 標準テスト

# 型チェック
npm run typecheck     # TypeScript
cd src-tauri && cargo clippy -- -D warnings  # Rust

# コードフォーマット
npm run format        # dprint
cd src-tauri && cargo fmt

# CI フルチェック
npm run ci:check
```

---

## プロジェクト構造

```
AxInvest/
├── src/                         # フロントエンドソース (React + TypeScript)
│   ├── components/              # React コンポーネント（25 モジュール）
│   │   ├── chat/               # チャットインターフェース（100+ コンポーネント）
│   │   ├── stock-analysis/     # スマート投資分析（16 コンポーネント）
│   │   ├── workflow/           # ワークフローエディタ（16 種ノード + プロパティパネル + AI パネル）
│   │   ├── gateway/            # API ゲートウェイコンポーネント
│   │   ├── settings/           # 設定パネル（50+ コンポーネント）
│   │   ├── terminal/           # ターミナルコンポーネント
│   │   ├── skill/              # スキルエディタとレンダラー
│   │   ├── benchmark/          # ベンチマーク
│   │   ├── files/              # ファイル管理
│   │   ├── fine-tune/          # LoRA ファインチューニング
│   │   ├── link/               # 外部リンク
│   │   ├── llm-wiki/           # LLM Wiki
│   │   ├── proactive/          # プロアクティブ提案
│   │   ├── wiki/               # Wiki 管理
│   │   ├── devtools/           # 開発者ツール
│   │   ├── decomposition/      # スキル分解
│   │   ├── recommendation/     # ツール推奨
│   │   ├── style/              # コードスタイル
│   │   ├── layout/             # レイアウトコンポーネント
│   │   ├── help/               # ヘルプパネル
│   │   ├── notification/       # 通知センター
│   │   ├── search/             # セッション検索
│   │   ├── onboarding/         # オンボーディングウィザード
│   │   ├── common/             # 共通コンポーネント
│   │   └── shared/             # 共有コンポーネント
│   ├── pages/                   # ページコンポーネント（22 ページ）
│   ├── stores/                  # Zustand 状態管理（65 store）
│   │   ├── domain/            # コアビジネス状態（9 個）
│   │   ├── feature/           # 機能モジュール状態（46 個）
│   │   ├── devtools/          # 開発者ツール状態（5 個）
│   │   └── shared/            # 共有状態（5 個）
│   ├── hooks/                   # React hooks（12 個）
│   ├── lib/                     # ユーティリティ関数（33 モジュール + Web Worker）
│   ├── types/                   # TypeScript 型定義（22 個）
│   ├── sdk/                     # SDK（TypeScript + Python）
│   └── i18n/                    # 11 言語翻訳
│
├── src-tauri/                    # バックエンドソース (Rust)
│   ├── crates/                  # Rust workspace（20 crates）
│   │   ├── agent/             # AI エージェントコア（70+ ソースファイル）
│   │   ├── astock-data/       # A 株データソース（9 大データソース、22 種データルーティング、テクニカル指標、取引カレンダー）
│   │   ├── core/              # コアユーティリティ（85+ エンティティ、40+ リポジトリ、RAG、暗号化、MCP）
│   │   ├── gateway/           # API ゲートウェイ（株式 API エンドポイント含む）
│   │   ├── migration/         # データベースマイグレーション（5 マイグレーション）
│   │   ├── npm/               # npm パッケージ解析
│   │   ├── plugins/           # プラグインシステム
│   │   ├── prompt-guard/      # プロンプトインジェクション防護
│   │   ├── providers/         # モデルプロバイダーアダプター
│   │   ├── rt-dashboard/      # ダッシュボードプラグイン
│   │   ├── rt-messaging/      # メッセージゲートウェイ（9 プラットフォーム）
│   │   ├── rt-theme/          # テーマエンジン
│   │   ├── rt-webhook/        # Webhook サーバー
│   │   ├── rt-workflow/       # ワークフローエンジン（16 種ノード実行器）
│   │   ├── runtime/           # ランタイムサービス（70+ ソースファイル）
│   │   ├── runtime-core/      # ランタイム抽象層
│   │   ├── stock-analysis/    # スマート投資分析（23 サブモジュール）
│   │   ├── telemetry/         # トレーシングとメトリクス
│   │   ├── tools/             # ツールシステム（40+ 内蔵ツール）
│   │   └── trajectory/        # 学習システム（55+ ソースファイル）
│   └── src/                    # Tauri エントリーポイント（91 コマンドモジュール）
│       ├── commands/          # コマンドモジュール
│       │   ├── stock_analysis.rs        # 株式分析コマンド
│       │   ├── stock_analysis_setup.rs  # 株式分析設定
│       │   ├── stock_workflow.rs        # 株式ワークフローコマンド
│       │   ├── agency_expert.rs         # エキスパートエージェント
│       │   ├── agent_advanced.rs        # 高度エージェント
│       │   ├── agent_analytics.rs       # エージェント分析
│       │   ├── agent_insight.rs         # エージェントインサイト
│       │   ├── agent_nudge.rs           # エージェントナッジ
│       │   ├── agent_profile.rs         # エージェントプロファイル
│       │   ├── agent_role.rs            # エージェントロール
│       │   ├── background_tasks.rs      # バックグラウンドタスク
│       │   ├── browser.rs              # ブラウザ自動化
│       │   ├── chart_generator.rs       # チャート生成
│       │   ├── cloud_workspace.rs       # クラウドワークスペース
│       │   ├── computer_control.rs      # コンピュータ制御
│       │   ├── context_breakdown.rs     # コンテキスト分解
│       │   ├── conversation_categories.rs  # 会話カテゴリ
│       │   ├── conversations_search.rs  # 会話検索
│       │   ├── crash_report.rs          # クラッシュレポート
│       │   ├── dream.rs                # ドリーム統合
│       │   ├── evolution.rs            # スキル進化
│       │   ├── fine_tune.rs            # LoRA ファインチューニング
│       │   ├── gateway.rs              # API ゲートウェイ
│       │   ├── gateway_link.rs         # 外部リンク
│       │   ├── generated_tool.rs        # 生成ツール
│       │   ├── image_gen.rs            # 画像生成
│       │   ├── knowledge.rs            # ナレッジベース
│       │   ├── llm_wiki.rs             # LLM Wiki
│       │   ├── local_models.rs         # ローカルモデル
│       │   ├── mcp.rs                  # MCP プロトコル
│       │   ├── memory.rs              # メモリシステム
│       │   ├── message_continuation.rs  # メッセージ続き
│       │   ├── onboarding.rs           # オンボーディングウィザード
│       │   ├── parallel_execution.rs    # 並列実行
│       │   ├── plan.rs                 # プラン管理
│       │   ├── platform_integration.rs  # プラットフォーム統合
│       │   ├── plugin.rs               # プラグイン管理
│       │   ├── proactive.rs            # プロアクティブ提案
│       │   ├── prompt_templates.rs      # プロンプトテンプレート
│       │   ├── providers.rs            # モデルプロバイダー
│       │   ├── quickbar.rs             # QuickBar
│       │   ├── reflection.rs           # リフレクション
│       │   ├── research.rs             # ディープリサーチ
│       │   ├── rl.rs                   # 強化学習
│       │   ├── sandbox.rs              # サンドボックス
│       │   ├── scheduled_task.rs        # 定期タスク
│       │   ├── screen_vision.rs        # 画面ビジョン
│       │   ├── search.rs               # 検索
│       │   ├── session_share.rs         # セッション共有
│       │   ├── shell.rs                # Shell
│       │   ├── skill_decomposition.rs   # スキル分解
│       │   ├── skills_hub.rs           # スキルハブ
│       │   ├── tool_recommender.rs      # ツール推奨
│       │   ├── tracer.rs               # トレーシング
│       │   ├── user_profile.rs          # ユーザープロファイル
│       │   ├── webdav.rs               # WebDAV
│       │   ├── webhook.rs              # Webhook
│       │   ├── wiki.rs                 # Wiki
│       │   ├── work_engine.rs          # ワークエンジン
│       │   ├── workflow_ai.rs          # AI ワークフロー
│       │   ├── workflow_template.rs     # ワークフローテンプレート
│       │   └── ...                     # その他コマンド
│       ├── init/              # 初期化モジュール
│       ├── stock_scheduler.rs # 株式スケジューラー
│       └── ...                # その他コアモジュール
│
├── extension/                  # ブラウザ拡張（Wiki Clipper：popup/content/background）
├── e2e/                        # Playwright E2E テスト（9 テストスイート）
├── scripts/                    # ビルドとツールスクリプト
└── website/                    # プロジェクトウェブサイト（VitePress、11 言語ドキュメント）
```

## データディレクトリ

```
~/.axinvest/                     # 設定ディレクトリ
├── axinvest.db                  # SQLite データベース
├── master.key                   # AES-256 マスターキー
├── vector_db/                   # ベクトルデータベース (sqlite-vec)
└── ssl/                         # SSL 証明書

~/Documents/axinvest/           # ユーザーファイルディレクトリ
├── images/                     # 画像添付ファイル
├── files/                      # ファイル添付ファイル
└── backups/                    # バックアップファイル
```

---

## よくある質問

### macOS：「アプリが破損しています」または「開発者を検証できません」

アプリが Apple によって署名されていないため：

**1. 「すべてのソース」からのアプリを許可**
```bash
sudo spctl --master-disable
```

次に **システム設定 → プライバシーとセキュリティ → セキュリティ** に移動し、**すべてのソース** を選択します。

**2. 検疫属性を削除**
```bash
sudo xattr -dr com.apple.quarantine /Applications/AxInvest.app
```

**3. macOS Ventura+ の追加手順**
**システム設定 → プライバシーとセキュリティ** に移動し、**それでも開く** をクリックします。

---

## コミュニティ

- [LinuxDO](https://linux.do)

## ライセンス

このプロジェクトは [AGPL-3.0](LICENSE) ライセンスの下で公開されています。
