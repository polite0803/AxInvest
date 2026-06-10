[English](./README-EN.md) | [简体中文](./README.md) | [繁體中文](./README-ZH-TW.md) | [日本語](./README-JA.md) | [한국어](./README-KO.md) | [Français](./README-FR.md) | **Deutsch** | [Español](./README-ES.md) | [Русский](./README-RU.md) | [हिन्दी](./README-HI.md) | [العربية](./README-AR.md)

[![AxInvest](https://github.com/polite0803/AxAgent/blob/main/src/assets/image/logo.png?raw=true)](https://github.com/polite0803/AxAgent)

<p align="center">
  <a href="https://www.producthunt.com/products/axagent?embed=true&amp;amp;utm_source=badge-featured&amp;amp;&amp;#10;&amp;amp;amp;;utm_medium=badge&amp;amp;#10&amp;amp;;utm_campaign=badge-axagent" target="_blank" rel="noopener noreferrer"><img alt="AxInvest - KI-gesteuerte intelligente Investmentanalyse-Plattform | Product Hunt" width="250" height="54" src="https://api.producthunt.com/widgets/embed-image/v1/featured.svg?post_id=1118403&amp;theme=light&amp;t=1775627359538"></a>
</p>

<p align="center">
  <strong>KI-gesteuerte intelligente Investmentanalyse | Multi-Agenten-Zusammenarbeit | Local-First</strong>
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

## Was ist AxInvest?

**AxInvest v2.3** ist eine KI-gesteuerte intelligente Investmentanalyse-Plattform, die auf dem AxAgent Multi-Agenten-Framework aufbaut. Sie vereint fortschrittliche KI-Agenten-Fähigkeiten mit professioneller A-Aktien-Investmentanalyse und unterstützt mehrere Modellanbieter, KI-Agenten-Forschung, visuelle Workflow-Orchestrierung, lokales Wissensmanagement und ein integriertes API-Gateway für **Windows / macOS / Linux / Android / iOS**, mit adaptivem Layout für **Desktop, Tablet und Smartphone**.

Das Kernmerkmal von AxInvest liegt in der Nutzung von Multi-Agenten-Mechanismen wie adversarischer Debatte, Tiefenrecherche und Faktenprüfung, um umfassende und objektive Analyseunterstützung für Investitionsentscheidungen zu bieten.

---

## Screenshots

| Chat und Modellauswahl | Multi-Agenten-Dashboard |
|:---:|:---:|
| ![](.github/images/s1-0412.png) | ![](.github/images/s5-0412.png) |

| Wissensbasis RAG | Speicher und Kontext |
|:---:|:---:|
| ![](.github/images/s3-0412.png) | ![](.github/images/s4-0412.png) |

| Workflow-Editor | API-Gateway |
|:---:|:---:|
| ![](.github/images/s9-0412.png) | ![](.github/images/s10-0412.png) |

---

## Kernfunktionen

### 📈 Intelligente Investmentanalyse

Das Kernmerkmal von AxInvest — tiefgreifende Verschmelzung von KI-Agenten-Fähigkeiten mit professioneller Investmentanalyse:

**Multi-Quellen-Datenaggregation und Fallback**

- **9 Datenquellen** — Tencent Finance, Tongdaxin (mootdx), Eastmoney, Sina Finance, Baidu Stocks, THS (Tonghuashun), Iwencai, Cninfo (Juchao), AKShare
- **22 Datenrouten** — Jeder Datentyp mit Multi-Quellen-Fallback-Routing; automatische Umschaltung auf Backup-Quelle bei Ausfall der primären Quelle
- **Parallele Datenerfassung** — `tokio::join!` paralleles Abrufen von 16 Aktien-Datentypen + 5 Markt-Datentypen für maximale Effizienz
- **Intelligentes Caching** — LRU-In-Memory-Cache (1000 Einträge Limit), Kursdaten 30s TTL / K-Linien 300s TTL, automatischer Ablauf und Bereinigung
- **Gesundheitsprüfung** — Anbieter-Verbindungsprüfung (Ping'an Bank 000001 als Sonde), Laufzeit-Erkennung der Datenquellenverfügbarkeit

**A-Aktien-Markterkennung und Regeln**

- **Sektorerkennung** — Automatische Erkennung nach Code-Präfix: Shanghai Hauptbörse (6), STAR Market (688), Shenzhen Hauptbörse (0), ChiNext (3), BSE (8)
- **Limit-Up/Limit-Down-Regeln** — STAR Market/ChiNext ±20%, BSE ±30%, Hauptbörse ±10%, ST-Aktien ±5%
- **Handelskalender** — Integrierter A-Aktien-Feiertagskalender 2025-2026 mit Ersatzarbeitstagen, Handelstag-Erkennung

**Aktien-Daten (16 Typen)**

- **Echtzeit-Notierungen** — Preis, Veränderung, Volumen/Umsatz, Umsatzrate, KGV/KBV, Marktkapitalisierung, Limit-Up/Limit-Down-Preis, ST-Kennzeichnung
- **K-Linien-Daten** — 7 Perioden (5 Min/15 Min/30 Min/60 Min/Tag/Woche/Monat), inkl. Volumen, Umsatz, Umsatzrate
- **Finanzanalyse** — Umsatz, Nettogewinn, EPS, BPS, ROE, Verschuldungsgrad, Bruttomarge, Nettomarge, YoY-Umsatzwachstum, YoY-Gewinnwachstum
- **Geldfluss** — Nettozuflüsse nach Haupt-/Super-Groß-/Groß-/Mittel-/Kleinaufträgen
- **Long-Hu-Bang** — Kauf-/Verkaufsbeträge der Handelsabteilungen, Netto, Listungsgründe
- **Sperrfrist-Freigaben** — Freigabedatum, -anteile, -quote, Aktionärsinformationen
- **Margin-Trading** — Margin-Kaufbetrag/Restsaldo, Short-Verkaufsmenge/Restmenge
- **Nordkapitalfluss** — Bestandsmenge, Anteil, Änderungsmenge
- **Branchenklassifikation** — Shenwan Level-1/2-Branchen, Konzeptsektor-Tags
- **Aktionärsänderungen** — Kauf-/Verkaufsaktivitäten wichtiger Aktionäre, Änderungsgründe
- **Dividendenhistorie** — Ex-Dividenden-Datum, Dividende pro Aktie, Umteilungsverhältnis, Stichtag
- **Forschungsberichte** — Broker-Analysen mit Institution, Analyst, Rating, Zielkurs, EPS-Prognose
- **Konsens-EPS** — Institutioneller Konsens-EPS, Konsens-Zielkurs, Durchschnittsrating, Ratinganzahl
- **Konzeptsektoren** — Dreidimensionale Zuordnung (Branche/Konzept/Region), inkl. Sektor-Veränderung
- **Bekanntmachungssuche** — Cninfo-Unternehmensbekanntmachungen mit Typ und PDF-Link
- **Nachrichten und Stimmung** — Nachrichtentitel/Zusammenfassung/Quelle, inkl. Stimmungsbewertung

**Markt-Daten (5 Typen)**

- **Marktweiter Long-Hu-Bang** — Alle gelisteten Aktien des Tages mit Netto-Kauf, Kauf-/Verkaufsbeträgen
- **Heiße Aktien** — THS-Star-Aktien mit Veränderung, Umsatzrate, Ursachen-Tags, Sektorzugehörigkeit
- **Branchenranking** — Shenwan-Branchen-Veränderung, Umsatz, Spitzenreiter-Aktien
- **Cailianshe-Eilmeldungen** — Echtzeit-Finanznachrichten mit Titel, Inhalt, Quelle
- **Nordkapitalfluss** — Shanghai/Shenzhen/Gesamt minutengenauer Kapitalfluss

**Technische Indikatoren (indicators-Modul)**

- **Gleitende Durchschnitte** — MA5/MA10/MA20/MA60, inkl. Anordnungszustandsbestimmung (bullisch/bärisch/schwach-bullisch/Verschlingung/Kreuzung)
- **MACD** — DIF/DEA/Histogramm, inkl. Signalbestimmung (Goldenes Kreuz/Totenkreuz/bullischer Lauf/bärischer Lauf)
- **RSI** — RSI6/RSI12/RSI24, inkl. Signalbestimmung (überkauft/überverkauft/stark/schwach/neutral)
- **Bollinger-Bänder** — Oberes/Mittleres/Unteres Band (20,2), inkl. Positionsbestimmung (über oberem Band/oberer Bereich/nahe Mittellinie/unterer Bereich/unterhalb unterem Band)
- **Abweichungsrate** — MA5-Abweichungsrate, MA20-Abweichungsrate
- **Volumen-Analyse** — Volumen-Verhältnis (Tagesvolumen/5-Tage-Durchschnitt), inkl. Signalbestimmung (Volumen-Anstieg/Volumen-Rückgang bei Korrektur/Volumen-Abfall/steigender Kurs bei niedrigem Volumen/normal)
- **Unterstützung/Widerstand** — Automatische Berechnung basierend auf jüngsten Hochs/Tiefs und gleitenden Durchschnitten

**MCP-Werkzeug-Registrierung (mcp_tools-Modul)**

- Aktien-Datenfähigkeiten werden über das MCP-Protokoll als Standardwerkzeuge registriert, die KI-Agenten direkt in Konversationen aufrufen können
- Registrierte Werkzeuge: search_stock, get_stock_quote, get_stock_kline, get_stock_financials, get_stock_news, get_stock_money_flow, get_stock_dragon_tiger usw.

**KI-Analyse-Pipeline (stock-analysis crate, 23 Submodule)**

- **Analyse-Orchestrierung** — orchestrator (Pipeline-Orchestrierung), pipeline (mehrstufige Pipeline), runner (Aufgaben-Executor)
- **Entscheidungs-Engine** — decision (Investitionsentscheidung), signals (Handelssignal-Generierung), rules (Handelsregel-Engine)
- **Risikobewertung** — risk (Risikobewertungsmodell), portfolio_risk (Portfolio-Risiko), position_limits (Positionsbeschränkungen und Compliance)
- **Aktienselektion und Backtesting** — screener (Mehrkriterien-Aktienselektion), backtest (Strategie-Backtesting-Engine), trading (Handelsstrategie-Framework)
- **Value-Investing** — value (Wertanalyse), value_investing (Value-Investing-Bewertungsframework)
- **Qualitätskontrolle** — quality (Datenqualitätsprüfung), data_clean (Datenbereinigung und Vorverarbeitung), review (Analyseergebnis-Review)
- **Berichte und Scoring** — report (Analysebericht-Generierung), scoring (Gesamt-Bewertungssystem)
- **Hilfsmodule** — key_levels (Schlüsselpreisniveau-Erkennung), monitor (Echtzeitüberwachung und Warnungen), plugin (Analyse-Plugin-Erweiterungen), prompts (KI-Prompt-Vorlagen)

**Frontend-Analysekomponenten (16)**

- StockAnalysisPage, StockQuoteCard, KLineChart, RiskMatrix, TradePanel
- DecisionBanner, DebatePanel, WatchlistPanel, PriceAlertPanel, CompareView
- AnalystReportGrid, AnalystReportCard, HistoricalAnalysisPanel, StockSearchBar
- AnalysisProgress, StockAnalysisSettingsModal, StockAnalysisChatIndicator

**Adversariale Debatte und Entscheidung**

- **Adversariale Debatte** — Multi-Agenten Pro/Contra-Debatte mit Argumentstärke-Bewertung und Widerlegungsverfolgung
- **Entscheidungsbanner** — Kauf/Verkauf/Halten-Entscheidungsvisualisierung mit Konfidenz und Begründung
- **KI-Workflow-Integration** — Nahtlose Integration von Aktienanalyse-Workflows in Konversationen (stockWorkflowChatBridge)

### 🤖 KI-Modellunterstützung

- **Multi-Anbieter-Unterstützung** — Native Integration von OpenAI, Anthropic Claude, Google Gemini, Ollama, OpenClaw, Hermes und allen OpenAI-kompatiblen APIs
- **Multi-Key-Rotation** — Mehrere API-Schlüssel pro Anbieter konfigurieren mit automatischer Rotation zur Verteilung des Rate-Limit-Drucks
- **Lokale Modellunterstützung** — Vollständige Unterstützung für Ollama lokale Modelle, einschließlich GGUF/GGML-Dateiverwaltung
- **Candle-Inferenz-Engine** — Integrierte lokale Candle-Inferenz, Rerank/Judge-Schnittstellen, GGUF-On-Demand-Download
- **Modellverwaltung** — Remote-Modelllisten abrufen, Parameter anpassen (Temperatur, maximale Tokens, Top-P usw.)
- **Streaming-Ausgabe** — Echtzeit-Token-für-Token-Rendering mit einklappbaren Denkblöcken (Claude Extended Thinking)
- **Multi-Modell-Vergleich** — Gleichzeitige Frage an mehrere Modelle mit Side-by-Side-Vergleich
- **Funktionsaufrufe** — Strukturierte Funktionsaufrufe über alle unterstützten Anbieter
- **OpenAI Responses API** — Unterstützung für das OpenAI Responses-Format-Transport
- **Realtime API** — WebSocket-Ereignis-Push kompatibel mit der OpenAI Realtime API
- **KI-Bildgenerierung** — KI-Bildgenerierungs-Panel mit Unterstützung für mehrere Modelle und Parameterkonfiguration

### 🔐 KI-Agenten-System

Das Agentensystem basiert auf einer anspruchsvollen Architektur (agent crate, 70+ Quelldateien) mit folgenden Eigenschaften:

- **ReAct-Reasoning-Engine** — Verschmelzung von Reasoning und Aktion mit integrierter Selbstverifikation für zuverlässige Aufgabenausführung
- **Hierarchischer Planer** — Zerlegung komplexer Aufgaben in strukturierte Pläne mit Phasen und Abhängigkeiten
- **Aufgabenzerleger** — Automatische Zerlegung komplexer Aufgaben in ausführbare Teilaufgaben
- **Gedankenkette** — Visualisierung der Agenten-Entscheidungsfindung, schrittweise Zerlegung
- **Gedankenbaum** — tree_of_thoughts Mehrpfad-Reasoning-Exploration
- **Tiefenrecherche** — Multi-Quellen-Suchorchesterung, Zitationsverfolgung und Glaubwürdigkeitsbewertung
- **Faktenprüfung** — KI-gesteuerte Faktenverifikation und Quellenklassifizierung
- **Suchorchesterung** — Koordination mehrerer Suchanbieter mit Suchplanung und Ergebnissynthese
- **Akademische Suche** — Akademische Literaturrecherche und Zitationsanalyse
- **Computersteuerung** — KI-gesteuerte Mausklicks, Tastatureingaben, Bildlauf mit visuellem Modell-Analyse
- **Bildschirmwahrnehmung** — Screenshot-Erfassung und visuelle Modell-Analyse zur UI-Element-Identifikation
- **Visuelle Pipeline** — vision_pipeline Bildverständnis und -analyse
- **Drei Berechtigungsstufen** — Standard (Genehmigung erforderlich), Bearbeitungen akzeptieren (automatische Genehmigung), Vollzugriff (keine Abfragen)
- **Sandbox-Isolation** — Agenten-Operationen sind strikt auf das angegebene Arbeitsverzeichnis beschränkt
- **Werkzeug-Genehmigungspanel** — Echtzeit-Anzeige von Werkzeugaufruf-Anfragen mit einzelner Genehmigung
- **Kostenverfolgung** — Echtzeit-Token-Nutzung und Kostenstatistiken pro Sitzung
- **Pause/Fortsetzen** — Agenten-Ausführung jederzeit anhalten und später fortsetzen
- **Checkpoint-System** — Persistente Checkpoints für Absturzwiederherstellung und Sitzungs-Wiederverbindung
- **Fehlerwiederherstellungs-Engine** — Automatische Fehlerklassifizierung, Ursachenanalyse und Wiederherstellungsstrategie-Ausführung
- **Schleifenerkennung** — Automatische Erkennung und Unterbrechung von Schleifenverhalten im Agenten-Reasoning
- **Proaktiver Modus** — Agenten können proaktiv Vorschläge machen und Aktionen ausführen
- **Zweckverwaltung** — Pflege und Verfolgung der Ausführungszwecke und des Kontexts des Agenten
- **Selbstverifikation** — self_verifier automatische Verifikation der Agenten-Ausgabekorrektheit
- **Reflektor** — reflector Reflexion und Verbesserung des Reasoning-Prozesses
- **Lenkungs-Input** — steer_manager dynamische Anpassung der Agenten-Verhaltensrichtung
- **Ereignis-Bus** — event_bus / event_emitter Agenten-Ereignis-gesteuerte Architektur
- **Inhaltssynthese** — content_synthesizer Multi-Quellen-Informationssynthese und Berichtsgenerierung
- **Zitationsverfolgung** — citation_tracker automatische Verfolgung und Kennzeichnung von Informationsquellen
- **Glaubwürdigkeitsbewertung** — credibility_evaluator Bewertung der Informationsquellen-Glaubwürdigkeit
- **Gliederungserstellung** — outline_builder automatische Erstellung von Forschungsgliederungen
- **Schema-Verwaltung** — schema_manager Verwaltung von Ausgabestruktur-Schemata
- **Projektspeicher** — project_memory projektübergreifender persistenter Speicher
- **Umgebungserkundung** — environment_probe automatische Erkundung der Laufzeitumgebung
- **Gesundheitsprüfung** — health_checker Agenten-Gesundheitsstatus-Überwachung

### 👥 Multi-Agenten-Kollaboration

- **Sub-Agenten-Koordination** — Master-Slave-Architektur, coordinator koordiniert mehrere kollaborative Agenten
- **Parallele Ausführung** — Parallele Verarbeitung durch mehrere Agenten mit abhängigkeitsbewusster Planung
- **Adversariale Debatte** — adversarial_debate Pro/Contra-Debattenrunden mit Argumentstärke-Bewertung und Widerlegungsverfolgung
- **Agenten-Rollen** — agent_roles vordefinierte Rollen (Forscher, Planer, Entwickler, Prüfer, Synthetisierer) für Teamzusammenarbeit
- **Agenten-Orchestrator** — Zentrales Nachrichten-Routing und Zustandsverwaltung für Multi-Agenten-Teams
- **Kommunikationsgraph** — graph_insights Visualisierung von Agenten-Interaktionen und Nachrichtenflüssen
- **Gemeinsames Blackboard** — shared_blackboard / blackboard agentenübergreifender gemeinsamer Zustandsraum
- **Buddy-System** — Konfigurierbare Agenten-Partner mit Spezies- und Attributdefinition
- **Gemeinsamer Speicher** — Agenten-übergreifender gemeinsamer Speicherplatz mit Statistiken und Abfragen
- **Team-Cron-Registrierung** — Teamweite Cron-Aufgabenplanung
- **Experten-System** — agency_expert Domänen-Experten-Agenten
- **Agenten-Profil** — agent_profile Agenten-Persönlichkeits- und Fähigkeitsprofil-Verwaltung

### ⭐ Skill-System

- **Skill-Marktplatz** — Integrierter Marktplatz zum Durchsuchen und Installieren von Community-Skills
- **Skill-Erstellung** — Automatische Skill-Erstellung aus Vorschlägen mit Markdown-Editor
- **Skill-Evolution** — skill_evolution KI-gesteuerte automatische Analyse und Verbesserung bestehender Skills basierend auf Ausführungsfeedback
- **Skill-Matching** — skill_matcher semantische Übereinstimmung, Empfehlung relevanter Skills zum Gesprächskontext
- **Skill-Zerlegung** — Automatische Zerlegung komplexer Aufgaben in ausführbare atomare Skills (LLM-unterstützt/Multi-Runde/Workflow-Validierung)
- **Generierte Werkzeuge** — KI-gesteuerte automatische Generierung und Registrierung neuer Werkzeuge zur Erweiterung der Agenten-Fähigkeiten
- **Skill-Hub** — skills_hub_adapter zentrale Skill-Entdeckung und Konfigurationsverwaltungsoberfläche
- **Skill-Hub-Client** — skills_hub_client Integration mit Remote-Skill-Hub mit Community-Sharing
- **Skill-Abhängigkeitsprüfung** — Automatische Erkennung von Skill-Abhängigkeiten und Werkzeugverfügbarkeit
- **Skill-Sandbox-Container** — Sichere Ausführung von Skills in einer isolierten Umgebung
- **Atomare Skills** — atomic_skill kleinste ausführbare Skill-Einheit
- **Skill-Vorschlag** — skill_proposal KI-gesteuerter Skill-Erstellungsvorschlag

### 🔄 Workflow-System

Die Workflow-Engine (rt-workflow crate) implementiert ein DAG-basiertes Aufgaben-Orchestrierungssystem:

- **Visueller Workflow-Editor** — Drag-and-Drop-Workflow-Designer mit Knotenverbindung und -konfiguration
- **16 Knotentypen** — Trigger, Agent, LLM, Bedingung, Parallel, Schleife, Merge, Verzögerung, Werkzeug, Code, Sub-Workflow, Vektorsuche, Dokumentanalyse, Validierung, Ende, Fallback
- **16 Eigenschafts-Panels** — Jeder Knotentyp mit eigenem Konfigurations-Panel
- **Workflow-Vorlagen** — Integrierte Voreinstellungen: Code-Review, Bug-Fix, Dokumentation, Tests, Refactoring, Exploration, Performance, Sicherheit, Feature-Entwicklung
- **DAG-Ausführung** — Topologische Sortierung nach Kahn-Algorithmus mit Zykluserkennung
- **Parallele Planung** — Pipeline-Ausführung, schnelle Schritte warten nicht auf langsame
- **Wiederholungsstrategie** — Exponentielles Backoff, konfigurierbare maximale Wiederholungsversuche pro Schritt
- **Teilabschluss** — Fehlgeschlagene Schritte blockieren keine unabhängigen nachgelagerten Schritte
- **Versionsverwaltung** — Versionskontrolle von Workflow-Vorlagen mit Rollback
- **Ausführungsverlauf** — Detaillierte Aufzeichnung mit Statusverfolgung und Debugging
- **KI-Unterstützung** — KI-gestütztes Workflow-Design, Knotenempfehlung und Agenten-Prompt-Optimierung
- **Semantische Prüfung** — Semantische Workflow-Validierung, Erkennung potenzieller Probleme
- **n8n-Import** — Unterstützung für Workflow-Import aus n8n-Verzeichnis
- **Debug-Panel** — Echtzeit-Debugging und Statusanzeige während der Workflow-Ausführung
- **Cache-Schicht** — cache_layer Workflow-Ausführungsergebnis-Cache
- **Marktplatz** — workflow_marketplace Workflow-Vorlagen-Marktplatz und Review

### 📚 Wissen und Speicher

- **Wissensbasis (RAG)** — Multi-Wissensbasis-Unterstützung, Dokument-Upload, automatisches Parsen, Chunking und Vektorindexierung
- **Hybride Suche** — Kombination aus Vektorähnlichkeitssuche und BM25-Volltext-Ranking
- **Reranking** — Cross-Encoder-Reranking zur Verbesserung der Abrufgenauigkeit
- **Dreistufige Recall-Pipeline** — Mehrstufiger Abrufmechanismus mit AST-Index + Vektorsuche + FTS5
- **Self-RAG** — self_rag adaptive Retrieval-Augmented Generation
- **Abfrageverbesserung** — query_enhancement Abfrage-Umschreibung und -Erweiterung
- **Wissensgraph** — Wissensentitäts-Beziehungsvisualisierung (Entitäten, Attribute, Beziehungen, Flüsse, Schnittstellen)
- **Wiki-System** — LLM-Wiki-Compiler und -Validator mit Wissensgraph-Visualisierung und inkrementeller Synchronisation
- **Wiki-Notizen** — Bidirektionales Link-Notizsystem mit Graphansicht und automatischer Link-Synchronisation
- **Speichersystem** — Multi-Namespace-Speicher mit manuellem Eintrag oder KI-gesteuerter automatischer Extraktion
- **Closed-Loop-Speicher** — Integration der persistenten Speicheranbieter Honcho und Mem0
- **Speicher-Vergessen** — memory_forgetting zeitbasierte Speicherabkling-Mechanismus
- **FTS5-Volltextsuche** — Schnelle Suche über Gespräche, Dateien und Speicher
- **Sitzungssuche** — Erweiterte Suche über alle Gesprächssitzungen
- **Kontextverwaltung** — Flexibles Anhängen von Dateien, Suchergebnissen, Wissenspassagen, Speichereinträgen, Werkzeugausgaben
- **Dokument-Parser** — Automatisches Parsen und Inhaltsextraktion von Multi-Format-Dokumenten
- **Inkrementelle Indexierung** — Inkrementelle Indexaktualisierung bei Dateiänderungen
- **Text-Chunking** — text_chunker intelligente Text-Chunking-Strategie
- **Token-Budget** — token_budget Token-Budget-Steuerung für Abrufergebnisse

### 🌐 API-Gateway

- **Lokaler API-Server** — Integrierter OpenAI-kompatibler, Claude- und Gemini-Schnittstellenserver
- **Externe Links** — One-Click-Integration mit Claude CLI, OpenCode, automatische API-Schlüssel- und Modellsynchronisation
- **Schlüsselverwaltung** — Generierung, Widerruf, Aktivierung/Deaktivierung von Zugriffsschlüsseln mit Beschreibungen
- **Nutzungsanalyse** — Anfragevolumen und Token-Nutzung nach Schlüssel, Anbieter und Datum
- **SSL/TLS-Unterstützung** — Integrierte selbstsignierte Zertifikate, Unterstützung für benutzerdefinierte Zertifikate
- **Anfrage-Logs** — Vollständige Aufzeichnung aller API-Anfragen und -Antworten
- **Konfigurationsvorlagen** — Vorgefertigte Vorlagen für Claude, Codex, OpenCode, Gemini
- **Realtime API** — WebSocket-Ereignis-Push kompatibel mit der OpenAI Realtime API
- **Plattform-Integration** — Unterstützung für DingTalk, Feishu, QQ, Slack, WeChat, WhatsApp, Telegram, Discord
- **Gateway-Diagnose** — Verbindungsdiagnose und Programmrichtlinienverwaltung
- **Rate-Limiter** — API-Anfragen-Ratenbegrenzung und Flusskontrolle
- **Persistente Warteschlange** — Persistente Anfragewarteschlangenverwaltung
- **Aktien-API** — stock_handlers Aktien-Daten-spezifische API-Endpunkte
- **SSE-Push** — sse Server-Sent Events Echtzeit-Ereignis-Push

### 🔧 Werkzeuge und Erweiterungen

- **MCP-Protokoll** — Vollständige Model Context Protocol-Implementierung mit stdio- und HTTP/WebSocket-Transporten
- **OAuth-Authentifizierung** — OAuth-Flow-Unterstützung für MCP-Server
- **MCP-Autostart** — Automatischer Start und Lebenszyklusverwaltung von MCP-Servern
- **MCP-Werkzeug-Bridge** — Brücke zwischen MCP-Werkzeugen und dem Agenten-Werkzeugsystem
- **MCP-Gesundheitsprüfung** — mcp_health MCP-Server-Gesundheitsstatus-Überwachung
- **Plugin-System** — OpenClaw-kompatible dreistufige Plugin-Architektur (integriert/gebündelt/extern) mit npm-Paket-Installation, Werkzeugregistrierung, Hooks und Lebenszyklusverwaltung
- **Plugin-Marktplatz** — Integrierte Marktplatz-UI mit npm-Suche, Installation und Bestätigungsdialog
- **Integrierte Werkzeuge** — 40+ Werkzeugmodule: Dateioperationen (Lesen/Schreiben/Bearbeiten/System), Codeausführung, Suche (Grep/Glob), Bash, Websuche/Web-Fetch, Planverwaltung, Cron-Planung, REPL, LSP, Kontextverwaltung, Computersteuerung, Nachrichtenversand, Todo-Liste, Datenbank, DevOps, Dokument-Parser, Git, Wissensabruf, LSP, Medienverarbeitung, Nachrichtenversand, OCR, Push-Benachrichtigungen, Systeminformationen, Aufgabensystem, Tests, Arbeitsbereich/Worktree usw.
- **Werkzeug-Berechtigungssystem** — Werkzeug-Berechtigungsklassifizierung, Regelverwaltung und Nutzungsverfolgung
- **Bash-Sicherheit** — Befehlsanalyse, Pfadvalidierung und Sandbox-Sicherheitskontrolle
- **LSP-Client** — Integriertes Language Server Protocol mit Code-Vervollständigung und Diagnose
- **AST-Index** — AST-Parsing und Indexerstellung für Codedateien
- **Terminal-Backend** — Unterstützung für lokale, Docker- und SSH-Terminalverbindungen
- **Browser-Automatisierung** — Browsersteuerung über CDP-Integration (Navigation, Screenshots, Klicks, Formularausfüllung, Textextraktion usw.)
- **UI-Automatisierung** — Plattformübergreifende UI-Element-Identifikation und -Steuerung
- **Git-Werkzeuge** — Git-Operationen mit Branch-Erkennung und Konfliktbewusstsein
- **Werkzeug-Empfehlung** — Kontextbasiertes intelligentes Werkzeug-Empfehlungssystem
- **Werkzeug-Orchestrierung** — Multi-Werkzeug-Koordinationsausführung und Streaming-Ausgabe
- **Werkzeug-Statistiken** — Werkzeug-Nutzungshäufigkeit und Leistungsstatistiken
- **Werkzeug-Audit** — audit Werkzeugaufruf-Audit-Log

### 📊 Inhaltsrendering

- **Markdown-Rendering** — Vollständige Unterstützung für Code-Hervorhebung, LaTeX-Mathematikformeln, Tabellen, Aufgabenlisten
- **Monaco Code-Editor** — Integrierter Editor mit Syntaxhervorhebung, Kopieren, Diff-Vorschau
- **Diagramm-Rendering** — Mermaid-Flussdiagramme, D2-Architekturdiagramme, ECharts-interaktive Diagramme
- **Artefakt-Panel** — Codeausschnitte, HTML-Entwürfe, React-Komponenten, Markdown-Notizen mit Echtzeitvorschau
- **Vier Vorschaumodi** — Code (Editor), Split (Side-by-Side), Vorschau (nur gerendert), React-Komponentenvorschau
- **Sitzungs-Inspektor** — Baumansicht der Sitzungsstruktur, schnelle Navigation
- **Zitations-Panel** — Verfolgung und Anzeige von Quellenzitationen mit Glaubwürdigkeitsbewertung
- **Infografik-Rendering** — Unterstützung für Infografik-Visualisierung
- **Diagramm-Interpreter** — ChartInterpreter KI-gesteuerte Diagramminterpretation
- **Diff-Viewer** — DiffViewer Code-Differenzvergleich

### 🛡️ Daten und Sicherheit

- **AES-256-Verschlüsselung** — API-Schlüssel und sensible Daten mit AES-256-GCM verschlüsselt
- **Isolierte Speicherung** — Anwendungsstatus in `~/.axinvest/`, Benutzerdateien in `~/Documents/axinvest/`
- **Automatisches Backup** — Geplante Backups in lokale Verzeichnisse oder WebDAV-Speicher
- **S3-Backup** — s3_backup Amazon S3 Cloud-Backup-Unterstützung
- **Backup-Wiederherstellung** — Ein-Klick-Wiederherstellung aus historischen Backups
- **Export-Optionen** — PNG-Screenshots, Markdown, Klartext, JSON
- **Speicherverwaltung** — Visuelle Plattennutzungsanzeige und Bereinigungstools
- **Speichermigration** — storage_migration Datenmigration zwischen Versionen
- **Dateiautorisierung** — Dateizugriffsautorisierung und -widerrufverwaltung
- **Operations-Audit** — Audit-Log-Erfassung kritischer Operationen
- **Befehlsvalidierung** — command_validator Befehlssicherheitsvalidierung
- **Ressourcenlimits** — resource_limits Ressourcenverwendungslimits
- **Sandbox-Ausführung** — sandbox_runner isolierte Umgebungsausführung

### 🖥️ Desktop-Erfahrung

- **Themen-Engine** — Dunkle/helle Themen, Systemfolge oder manuelle Präferenz
- **Oberflächensprache** — 11 Sprachen: Vereinfachtes Chinesisch, Traditionelles Chinesisch, Englisch, Japanisch, Koreanisch, Französisch, Deutsch, Spanisch, Russisch, Hindi, Arabisch
- **Systemtray** — Minimierung in den Systemtray ohne Unterbrechung von Hintergrunddiensten
- **Immer im Vordergrund** — Fenster über allen anderen Fenstern anheften
- **Globale Tastenkürzel** — Anpassbare Tastenkürzel zum Aufrufen des Hauptfensters
- **QuickBar** — Schnellzugriff-Schwebelleiste, Ein-Klick-Aufruf
- **Autostart** — Optionaler Start beim Systemstart
- **Proxy-Unterstützung** — HTTP- und SOCKS5-Proxy-Konfiguration
- **Automatische Updates** — Automatische Versionsprüfung, Update-Benachrichtigung
- **Befehlspalette** — `Cmd/Ctrl+K` für schnellen Zugriff auf Befehle
- **Onboarding-Assistent** — Interaktiver Erstnutzungs-Assistent und Ollama-Erkennung
- **Benachrichtigungscenter** — Unified In-App-Benachrichtigungsverwaltung
- **Cloud-Arbeitsbereich** — cloud_workspace Cloud-Arbeitsbereichsauswahl
- **Absturzbericht** — crash_report automatische Absturzbericht-Erfassung
- **Sprachanruf** — VoiceCall Sprachkonversationsfähigkeit

### 🔬 Erweiterte Funktionen

- **Tiefenrecherche** — Multi-Quellen-Suche, Zitationsverfolgung, Glaubwürdigkeitsbewertung und Inhaltssynthese
- **Faktenprüfung** — KI-gesteuerte Faktenverifikation und Quellenklassifizierung
- **Cron-Planer** — Automatisierte Aufgabenplanung mit täglichen/wöchentlichen/monatlichen Vorlagen und benutzerdefinierten Cron-Ausdrücken
- **Webhook-System** — Ereignisabonnement, Werkzeugabschluss-, Agentenfehler-, Sitzungsende-Benachrichtigungen
- **Benutzerprofil** — Automatisches Lernen von Code-Stil, Namenskonventionen, Einrückung, Kommentarstil, Kommunikationspräferenzen
- **RL-Optimierer** — Reinforcement-Learning-Optimierung der Werkzeugauswahl und Aufgabenstrategien
- **LoRA-Feinabstimmung** — Benutzerdefinierte Modelladaption mit lokalem LoRA-Feintuning
- **Proaktive Vorschläge** — Kontextbewusste Hinweise basierend auf Gesprächsinhalt und Benutzermustern
- **Kontextvorhersage** — Vorhersage der nächsten Benutzeraktion und Vorabladen relevanter Ressourcen
- **Traum-Integration** — dream_consolidation automatische Hintergrund-Integration von Speicher und Mustern, Optimierung von Langzeitwissen
- **Fehlerwiederherstellung** — Automatische Fehlerklassifizierung, Ursachenanalyse und Wiederherstellungsvorschläge
- **Entwicklerwerkzeuge** — Trace, Span, Timeline-Visualisierung für Debugging und Performance-Analyse
- **Benchmark-System** — SWE-bench / Terminal-bench Leistungsbewertung und Metriken mit Scorecards
- **Stiltransfer** — style_migrator Anwendung gelernter Code-Stil-Präferenzen auf generierten Code
- **Dashboard-Plugins** — Erweiterbares Dashboard mit benutzerdefinierten Panels und Widgets
- **Kollaboration und Freigabe** — CRDT-Echtzeit-Kollaboration und Ein-Klick-Sitzungsfreigabe
- **Browser-Erweiterung** — Wiki Clipper Browser-Erweiterung zum schnellen Clipping von Webseiten ins LLM-Wiki
- **Python SDK** — Python SDK zur Integration mit AxInvest
- **Intelligentes Routing** — Intelligentes Routing und Klassifizierung von Anfragen
- **Semantischer Cache** — Semantikbasierter Antwort-Cache zur Reduzierung redundanter Berechnungen
- **Kontextkompression** — Automatische Kompression langer Kontexte, Optimierung der Token-Nutzung
- **Nachrichten-Batching** — Nachrichten-Stapelversand und -optimierung
- **Verbindungspool** — Datenbank- und API-Verbindungspool-Verwaltung
- **Feature Flags** — Konfigurierbares Feature-Flag-System
- **Policy-Engine** — Zentrale Verwaltung von Berechtigungs- und Operationsrichtlinien
- **Ressourcen-Governor** — Agenten-Ressourcennutzungslimitierung und -Governance
- **LAN-Transfer** — Lokale Netzwerk-Dateiübertragungsfähigkeit
- **Koevolution** — coevolution Skill- und Agenten-Koevolution
- **Verhaltenslernen** — behavior_learner / behavior_tracker Benutzer-Verhaltenslernen und -Verfolgung
- **Präferenzenlernen** — preference_learner automatisches Lernen von Benutzerpräferenzen
- **Intrinsische Belohnung** — intrinsic_reward intrinsisch motivierte Exploration
- **Prozessbelohnung** — process_reward prozessuale Belohnungssignale
- **TextGrad** — text_grad textgradientenbasierte automatische Optimierung
- **Trajektorienkompression** — trajectory_compressor automatische Kompression langer Trajektorien
- **Erinnerungsverwaltung** — reminder_manager intelligente Erinnerungsplanung
- **Aufgaben-Prefetching** — task_prefetcher prädiktives Aufgaben-Ressourcen-Prefetching

### 🛡️ Prompt-Injection-Schutz (Prompt-Guard)

- **Vierstufiges Schutzsystem** — L1 Mustererkennung (Hochrisiko-Blockierung + Mittleres-Risiko-Markierung) → L2 Trennzeichen-Escaping → L3 XML-Wrapper → L4 Vertrauens-Tags
- **Pipeline-Orchestrator** — Mehrstufige Erkennungspipeline in Reihe, anpassbare Risikoschwellen
- **Token-Smuggling-Erkennung** — Spezielle Erkennung von Kodierungsverschleierung und Token-Schmuggel-Angriffen
- **Trennzeichen-Escaping-Erkennung** — delimiter_escape Erkennung von Prompt-Trennzeichen-Escaping-Angriffen
- **Mustererkennung** — pattern_detect Regex + heuristische Injection-Mustererkennung
- **Vertrauens-Tags** — trust_labels vertrauenswürdige Inhaltsmarkierung und -verifikation
- **Strict-Modus** — Strikt-Modus-Tests + Mittleres-Risiko-Ursachenbenennung + benutzerdefinierte Modus-Dokumentation
- **Vollständige Pipeline-Integration** — Integriert in Session / Prompt / Git / RAG

### 📱 Mobile Unterstützung

- **Android Nativ** — APK/AAB-Build, Unterstützung für arm64-v8a / armeabi-v7a / x86_64
- **iOS Nativ** — IPA-Build, Unterstützung für arm64
- **Adaptives Layout** — Automatische Anpassung für Desktop/Tablet/Smartphone (useResponsive Hook)
- **Mobile Navigation** — Drawer-Slide-Navigation + untere Navigationsleiste + schnelles Floating-Button
- **Safe-Area-Anpassung** — Android System-Statusleiste/Navigationsleiste CSS env() Anpassung
- **CSP-Optimierung** — Android WebView CSP-Protokoll-Whitelist
- **Bedingte Kompilierung** — `#[cfg(not(mobile))]` Desktop-exklusive Funktionen (Browser, Computersteuerung, Desktop, QuickBar, Terminal, Bildschirm-Sehen) automatisch ausgeschlossen

---

## Technische Architektur

### Technologie-Stack

| Schicht | Technologie |
|---------|------------|
| **Framework** | Tauri 2 + React 19 + TypeScript 6 |
| **UI** | Ant Design 6 + TailwindCSS 4 |
| **Zustandsverwaltung** | Zustand 5 |
| **Routing** | React Router 7 |
| **i18n** | i18next + react-i18next |
| **Backend** | Rust 2024 + SeaORM 2 + SQLite |
| **Vektor-DB** | sqlite-vec |
| **Code-Editor** | Monaco Editor |
| **Diagramme** | Mermaid + D2 + ECharts (CDN) |
| **Terminal** | xterm.js 6 |
| **Workflow** | ReactFlow 11 |
| **Infografik** | @antv/infographic |
| **Icons** | Iconify + Lucide |
| **Drag & Drop** | @dnd-kit |
| **Build** | Vite 8 + npm |
| **Tests** | Vitest + Playwright + cargo-nextest |
| **Formatierung** | dprint (TS/JSON) + rustfmt |
| **Lint** | TS: eslint + oxlint / Rust: clippy + cargo-deny |
| **Mobil** | Tauri Android + iOS Nativ-Build |
| **Desktop** | Windows (MSI) · macOS (DMG) · Linux (AppImage/deb/rpm) |

### Plattformunterstützung

| Plattform | Architektur |
|-----------|------------|
| Windows | x86_64, ARM64 |
| macOS | Apple Silicon (arm64), Intel (x86_64) |
| Linux | x86_64, ARM64 |
| Android | arm64-v8a, armeabi-v7a, x86_64 (Emulator) |
| iOS | arm64 |

### Rust-Backend-Architektur

Das Backend ist als Rust-Workspace mit **20** spezialisierten Crates organisiert:

```
src-tauri/crates/
├── agent/            # KI-Agenten-Kern (70+ Quelldateien: ReAct-Engine, Koordination, Planung, Tiefenrecherche, Faktenprüfung usw.)
├── astock-data/      # A-Aktien-Datenquellen (9 Datenquellen, 22 Datenrouten, technische Indikatoren, Handelskalender, MCP-Werkzeug-Registrierung)
├── core/             # Kernwerkzeuge (85+ Datenbank-Entitäten, 40+ Repositories, RAG, Verschlüsselung, MCP, Browser-Automatisierung, AST-Index usw.)
├── gateway/          # API-Gateway (HTTP-Server, Authentifizierung, Routing, OpenAI-kompatible Schnittstelle, Aktien-API-Endpunkte)
├── migration/        # Datenbankmigrationen (5 Migrationen: Aktienanalyse/Watchlist-Kombination/Analyse-Scheduling/Preiswarnungen/Handel)
├── npm/              # npm-Paket-Parsing und Registry
├── plugins/          # Plugin-System (OpenClaw-kompatibel, npm-Paket-Installation, inkl. Beispiel-Plugin)
├── prompt-guard/     # Prompt-Injection-Schutz (L1-L4 mehrstufige Erkennung und Abwehr, 4 Detektoren)
├── providers/        # Modellanbieter-Adapter (OpenAI, Anthropic, Gemini, Ollama, OpenClaw, Hermes, Bildgenerierung)
├── rt-dashboard/     # Dashboard-Plugin-System
├── rt-messaging/     # Nachrichten-Gateway (9 Plattformen: DingTalk/Feishu/QQ/Slack/WeChat/WhatsApp/Telegram/Discord)
├── rt-theme/         # Themen-Engine
├── rt-webhook/       # Webhook-Server und Versand
├── rt-workflow/      # Workflow-Engine (DAG-Orchestrierung, 16 Knoten-Executors, Scheduler, Cache-Schicht)
├── runtime/          # Laufzeitdienste (70+ Quelldateien: Sitzungsverwaltung, MCP, Terminal, Rate-Limiting, Webhook, Berechtigungen, Benchmarking usw.)
├── runtime-core/     # Laufzeit-Abstraktionsschicht (gemeinsame Typen, Trait-Definitionen, Konfiguration, Feature Flags, Berechtigungs-Executor)
├── stock-analysis/   # Intelligente Investmentanalyse (23 Submodule: Pipeline, Entscheidungs-Engine, Risikobewertung, Backtesting, Screener, Value-Investing)
├── telemetry/        # Telemetrie und verteiltes Tracing (OpenTelemetry-kompatibel)
├── tools/            # Werkzeugsystem (40+ integrierte Werkzeuge, Bash-Sicherheit, MCP-Bridge, Berechtigungssystem, Orchestrierung, Audit)
└── trajectory/       # Lernsystem (55+ Quelldateien: Speicher, Skills, RL, Benutzerprofil, Traum-Integration, Stiltransfer, Koevolution)
```

#### stock-analysis Crate-Modulstruktur (23 Submodule)

```
stock-analysis/
├── backtest.rs         # Strategie-Backtesting-Engine
├── data_clean.rs       # Datenbereinigung und Vorverarbeitung
├── decision.rs         # Investment-Entscheidungs-Engine
├── key_levels.rs       # Schlüsselpreisniveau-Erkennung
├── monitor.rs          # Echtzeitüberwachung und Warnungen
├── orchestrator.rs     # Analyse-Pipeline-Orchestrierung
├── pipeline.rs         # Mehrstufige Analyse-Pipeline
├── plugin.rs           # Analyse-Plugin-Erweiterungen
├── portfolio_risk.rs   # Portfolio-Risikobewertung
├── position_limits.rs  # Positionsbeschränkungen und Compliance
├── prompts.rs          # KI-Prompt-Vorlagen
├── quality.rs          # Datenqualitätsprüfung
├── report.rs           # Analysebericht-Generierung
├── review.rs           # Analyseergebnis-Review
├── risk.rs             # Risikobewertungsmodelle
├── rules.rs            # Handelsregel-Engine
├── runner.rs           # Analyse-Aufgaben-Executor
├── scoring.rs          # Gesamt-Bewertungssystem
├── screener.rs         # Aktienscreener
├── signals.rs          # Handelssignal-Generierung
├── trading.rs          # Handelsstrategie-Framework
├── value.rs            # Wertanalyse
└── value_investing.rs  # Value-Investing-Bewertung
```

#### astock-data Crate-Datenquellen

| Datenquelle | Kennung | Unterstützte Datentypen |
|-------------|---------|------------------------|
| Tencent Finance | tencent | Echtzeit-Notierungen, K-Linien |
| Tongdaxin | mootdx | Echtzeit-Notierungen, K-Linien |
| Eastmoney | eastmoney | Notierungen, K-Linien, Finanzen, Geldfluss, Long-Hu-Bang, Sperrfrist-Freigaben, Margin-Trading, Nordkapitalfluss, Branchenklassifikation, Aktionärsänderungen, Dividenden, Forschungsberichte, marktweiter Long-Hu-Bang, Cailianshe-Eilmeldungen |
| Sina Finance | sina | Notierungen, K-Linien, Nachrichten |
| Baidu Stocks | baidu_stock | Notierungen, Nachrichten, Geldfluss, Long-Hu-Bang, Sperrfrist-Freigaben, Margin-Trading, Nordkapitalfluss, Branchenklassifikation, Aktionärsänderungen, Dividenden, Forschungsberichte, heiße Aktien, Branchenranking, Konzeptsektoren, Nordkapitalfluss |
| THS (Tonghuashun) | ths | Notierungen, Branchenklassifikation, Konsens-EPS, Konzeptsektoren, heiße Aktien, Branchenranking, Nordkapitalfluss |
| Iwencai | iwencai | Aktiensuche, Branchenklassifikation, Konsens-EPS, Konzeptsektoren, heiße Aktien |
| Cninfo (Juchao) | cninfo | Bekanntmachungen |
| AKShare | akshare | Finanzen, Nachrichten, Konsens-EPS, Cailianshe-Eilmeldungen |

Jeder Datentyp ist mit Multi-Quellen-Fallback-Routing konfiguriert — bei Ausfall der primären Quelle wird automatisch auf die Backup-Quelle umgeschaltet.

#### astock-data Zusatzmodule

| Modul | Funktion |
|-------|----------|
| calendar | A-Aktien-Handelskalender (2025-2026 Feiertage + Ersatzarbeitstage) |
| indicators | Technische Indikatoren (MA/MACD/RSI/Bollinger-Bänder/Abweichungsrate/Volumen-Verhältnis/Unterstützung-Widerstand) |
| mcp_tools | MCP-Werkzeug-Registrierung (Aktien-Datenfähigkeiten als KI-aufrufbare Werkzeuge registriert) |

### Frontend-Architektur

```
src/
├── stores/                    # Zustand State Management (65 Stores)
│   ├── domain/               # Kerngeschäftslogik-State (9 Stores)
│   │   ├── agentDomainStore.ts
│   │   ├── compressStore.ts
│   │   ├── conversationPreferences.ts
│   │   ├── conversationStore.ts
│   │   ├── conversationStoreEvents.ts
│   │   ├── conversationStoreSend.ts
│   │   ├── multiModelStore.ts
│   │   ├── preferenceStore.ts
│   │   └── streamStore.ts
│   ├── feature/               # Feature-Modul-State (46 Stores)
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
│   ├── devtools/              # Entwicklerwerkzeug-State (5 Stores)
│   │   ├── evaluatorStore.ts
│   │   ├── fineTuneStore.ts
│   │   ├── recommendationStore.ts
│   │   ├── rlStore.ts
│   │   └── tracerStore.ts
│   └── shared/                # Geteilter State (5 Stores)
│       ├── artifactStore.ts
│       ├── chatWorkspaceStore.ts
│       ├── rightPanelStore.ts
│       ├── tabStore.ts
│       └── uiStore.ts
│
├── components/                # React-Komponenten (25 Module)
│   ├── chat/                # Chat-Interface (100+ Komponenten: Agenten-Ausführungs-Panel, Branch-Vergleich, Browser-Automatisierung, Code-Executor, Kollaborations-Panel, Tiefenrecherche, Faktenprüfung, Git-Commit, Bildgenerierung/-analyse, Wissensabruf, Speicherextraktion, Modell-Routing, Multi-Modell-Anzeige, Berechtigungsverwaltung, Plugin-Marktplatz, Reflexions-Panel, Skill-Erstellung/Evolution, strukturiertes Denken, Sub-Agenten-Karte, Werkzeugaufruf-Karte, Trajektorien-Wiedergabe, Sprachanruf, Wiki-Abruf, Workflow-Fortschritt usw.)
│   ├── stock-analysis/      # Intelligente Investmentanalyse (16 Komponenten)
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
│   ├── workflow/            # Workflow-Editor (16 Knotentypen + 16 Eigenschafts-Panels + KI-Panel + Vorlagen + Debug)
│   ├── gateway/             # API-Gateway-UI (Übersicht/Schlüssel/Metriken/Monitoring/Einstellungen/Vorlagen/Diagnose)
│   ├── settings/            # Einstellungs-Panels (50+ Komponenten: Anbieter/Modelle/MCP/Wissen/Speicher/Proxy/Tastenkürzel/Themen/Werkzeuge/Webhook/Cron/Aktienanalyse-Konfiguration usw.)
│   ├── terminal/            # Terminal-UI (integriertes Terminal/Docker/SSH/Backend-Auswahl/Pfad-Vervollständigung/Slash-Vervollständigung)
│   ├── skill/               # Skill-Editor und -Renderer (Aktionsketten-Editor/Frontend-Editor/Sandbox-Container/Abhängigkeitsprüfung/Statistik-Panel)
│   ├── benchmark/           # Benchmark-Panel (Konfiguration/Bericht/Auswahl/Aufgabenliste/Ergebnisse)
│   ├── files/               # Dateiverwaltungsseite
│   ├── fine-tune/           # LoRA-Feinabstimmungs-Konfiguration (Datensatz/Trainingsaufgaben/LoRA-Konfiguration)
│   ├── link/                # Externe Links-Verwaltung (Übersicht/Modelle/Strategie/Skills/Strategie-Details)
│   ├── llm-wiki/            # LLM-Wiki-Editor (Qualitätsbewertung/Synchronisationsstatus)
│   ├── proactive/           # Proaktives Vorschlagssystem (Kontextvorhersage/Prefetch-Indikator/Vorschlagsleiste/Erinnerungsliste)
│   ├── wiki/                # Wiki-Verwaltung (Rückverweise/Graphansicht/Ingest/Code-Prüfung/Aktions-Timeline/Tag-Aggregation/Versionshistorie)
│   ├── devtools/            # Trace/Span-Timeline (Kostendiagramm/Dauerdiagramm/Details/Filter/Liste)
│   ├── decomposition/       # Skill-Zerlegung (Zerlegungsvorschau/Werkzeugabhängigkeiten/Werkzeuggenerierung/Werkzeuginstallation)
│   ├── recommendation/      # Werkzeug-Empfehlungs-Panel
│   ├── style/               # Code-Stiltransfer (Beispiele/Anpassungs-Slider/Vergleich/Vorschau-Panel)
│   ├── layout/              # Layout-Komponenten (Titelleiste/Sidebar/Befehlspalette/Globales Kopieren/Fehlergrenze/Statusleiste/Benachrichtigungsglocke/Benutzerprofil-Modal)
│   ├── help/                # Hilfe-Panel
│   ├── notification/        # Benachrichtigungscenter
│   ├── search/              # Sitzungssuche
│   ├── onboarding/          # Onboarding-Assistent (interaktives Tutorial/Willkommens-Assistent)
│   ├── common/              # Gemeinsame Komponenten (Kopieren/Icons/Modellparameter-Slider/Einfügen)
│   └── shared/              # Geteilte Komponenten (Avatar-Bearbeitung/Modals/Diagramm-Rendering/Dynamische Icons/Embedding-Modell-Auswahl/Emoji-Auswahl/Wissensbasis-Icon/MCP-Icon/Modellauswahl/Monaco-Editor/Namespace-Icon/Suchanbieter-Icon)
│
├── pages/                    # Seitenkomponenten (22 Seiten)
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
├── hooks/                    # React-Hooks (12)
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
├── lib/                      # Hilfsfunktionen (33 Module + Web Worker)
│   ├── workers/            # Web Worker (heavy.worker.ts)
│   ├── actionRouter.ts     # Aktions-Routing
│   ├── artifactRenderer.ts # Artefakt-Rendering
│   ├── chartGenerator.ts   # Diagrammgenerierung
│   ├── chatMarkdown.ts     # Markdown-Rendering
│   ├── codeExecutor.ts     # Codeausführung
│   ├── invoke.ts           # Tauri IPC-Wrapper
│   ├── skillActionExecutor.ts  # Skill-Aktionsausführung
│   ├── skillEventBus.ts    # Skill-Ereignis-Bus
│   ├── skillLifecycle.ts   # Skill-Lebenszyklus
│   ├── skillPermissions.ts # Skill-Berechtigungen
│   ├── storeRegistry.ts    # Store-Registrierung
│   ├── tokenEstimator.ts   # Token-Schätzung
│   ├── workflowLayout.ts   # Workflow-Layout
│   └── ...                 # Weitere Hilfsmodule
│
├── types/                    # TypeScript-Typdefinitionen (22)
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
├── sdk/                      # SDK (inkl. Python SDK)
│   ├── index.ts
│   ├── types.ts
│   ├── rpcBridge.ts
│   ├── sandboxTemplate.ts
│   └── python/              # Python SDK
│       ├── setup.py
│       └── axagent_sdk/__init__.py
│
└── i18n/                     # 11-Sprach-Übersetzungen
```

## Erste Schritte

### Vorab-Builds herunterladen

Besuchen Sie die [Releases](https://github.com/polite0803/AxAgent/releases)-Seite und laden Sie das Installationsprogramm für Ihre Plattform herunter.

### Aus dem Quellcode erstellen

#### Voraussetzungen

- [Node.js](https://nodejs.org/) 20+
- [Rust](https://www.rust-lang.org/) 1.75+
- [npm](https://www.npmjs.com/) 10+
- Windows: [Visual Studio Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/) + Rust MSVC-Ziele

#### Build-Schritte

```bash
# Repository klonen
git clone https://github.com/polite0803/AxAgent.git
cd AxAgent

# Abhängigkeiten installieren
npm install

# Entwicklungsmodus
npm run tauri dev

# Nur Frontend bauen
npm run build

# Desktop-Anwendung bauen
npm run tauri build
```

Build-Artefakte befinden sich in `src-tauri/target/release/`.

### Tests

```bash
# Unit-Tests
npm run test          # Vitest Watch-Modus
npm run test:run      # Vitest Einzeldurchlauf

# E2E-Tests
npm run test:e2e      # Playwright
npm run test:e2e:ui   # Playwright UI-Modus

# Rust-Backend-Tests
cd src-tauri && cargo nextest run   # cargo-nextest (2-3x schneller)
cd src-tauri && cargo test          # Standard-Tests

# Typprüfung
npm run typecheck     # TypeScript
cd src-tauri && cargo clippy -- -D warnings  # Rust

# Code-Formatierung
npm run format        # dprint
cd src-tauri && cargo fmt

# CI-Vollprüfung
npm run ci:check
```

---

## Projektstruktur

```
AxInvest/
├── src/                         # Frontend-Quellcode (React + TypeScript)
│   ├── components/              # React-Komponenten (25 Module)
│   │   ├── chat/               # Chat-Interface (100+ Komponenten)
│   │   ├── stock-analysis/     # Intelligente Investmentanalyse (16 Komponenten)
│   │   ├── workflow/           # Workflow-Editor (16 Knotentypen + Eigenschafts-Panels + KI-Panel)
│   │   ├── gateway/            # API-Gateway-Komponenten
│   │   ├── settings/           # Einstellungs-Panels (50+ Komponenten)
│   │   ├── terminal/           # Terminal-Komponenten
│   │   ├── skill/              # Skill-Editor und -Renderer
│   │   ├── benchmark/          # Benchmarks
│   │   ├── files/              # Dateiverwaltung
│   │   ├── fine-tune/          # LoRA-Feinabstimmung
│   │   ├── link/               # Externe Links
│   │   ├── llm-wiki/           # LLM-Wiki
│   │   ├── proactive/          # Proaktive Vorschläge
│   │   ├── wiki/               # Wiki-Verwaltung
│   │   ├── devtools/           # Entwicklerwerkzeuge
│   │   ├── decomposition/      # Skill-Zerlegung
│   │   ├── recommendation/     # Werkzeug-Empfehlung
│   │   ├── style/              # Code-Stil
│   │   ├── layout/             # Layout-Komponenten
│   │   ├── help/               # Hilfe-Panel
│   │   ├── notification/       # Benachrichtigungscenter
│   │   ├── search/             # Sitzungssuche
│   │   ├── onboarding/         # Onboarding-Assistent
│   │   ├── common/             # Gemeinsame Komponenten
│   │   └── shared/             # Geteilte Komponenten
│   ├── pages/                   # Seitenkomponenten (22 Seiten)
│   ├── stores/                  # Zustand State Management (65 Stores)
│   │   ├── domain/            # Kerngeschäftslogik-State (9 Stores)
│   │   ├── feature/           # Feature-Modul-State (46 Stores)
│   │   ├── devtools/          # Entwicklerwerkzeug-State (5 Stores)
│   │   └── shared/            # Geteilter State (5 Stores)
│   ├── hooks/                   # React-Hooks (12)
│   ├── lib/                     # Hilfsfunktionen (33 Module + Web Worker)
│   ├── types/                   # TypeScript-Typdefinitionen (22)
│   ├── sdk/                     # SDK (TypeScript + Python)
│   └── i18n/                    # 11-Sprach-Übersetzungen
│
├── src-tauri/                    # Backend-Quellcode (Rust)
│   ├── crates/                  # Rust-Workspace (20 Crates)
│   │   ├── agent/             # KI-Agenten-Kern (70+ Quelldateien)
│   │   ├── astock-data/       # A-Aktien-Datenquellen (9 Datenquellen, 22 Datenrouten, technische Indikatoren, Handelskalender)
│   │   ├── core/              # Kernwerkzeuge (85+ Entitäten, 40+ Repositories, RAG, Verschlüsselung, MCP)
│   │   ├── gateway/           # API-Gateway (inkl. Aktien-API-Endpunkte)
│   │   ├── migration/         # Datenbankmigrationen (5 Migrationen)
│   │   ├── npm/               # npm-Paket-Parsing
│   │   ├── plugins/           # Plugin-System
│   │   ├── prompt-guard/      # Prompt-Injection-Schutz
│   │   ├── providers/         # Modellanbieter-Adapter
│   │   ├── rt-dashboard/      # Dashboard-Plugin
│   │   ├── rt-messaging/      # Nachrichten-Gateway (9 Plattformen)
│   │   ├── rt-theme/          # Themen-Engine
│   │   ├── rt-webhook/        # Webhook-Server
│   │   ├── rt-workflow/       # Workflow-Engine (16 Knoten-Executors)
│   │   ├── runtime/           # Laufzeitdienste (70+ Quelldateien)
│   │   ├── runtime-core/      # Laufzeit-Abstraktionsschicht
│   │   ├── stock-analysis/    # Intelligente Investmentanalyse (23 Submodule)
│   │   ├── telemetry/         # Tracing und Metriken
│   │   ├── tools/             # Werkzeugsystem (40+ integrierte Werkzeuge)
│   │   └── trajectory/        # Lernsystem (55+ Quelldateien)
│   └── src/                    # Tauri-Einstiegspunkt (91 Befehlsmodule)
│       ├── commands/          # Befehlsmodule
│       │   ├── stock_analysis.rs        # Aktienanalyse-Befehle
│       │   ├── stock_analysis_setup.rs  # Aktienanalyse-Konfiguration
│       │   ├── stock_workflow.rs        # Aktien-Workflow-Befehle
│       │   ├── agency_expert.rs         # Experten-Agenten
│       │   ├── agent_advanced.rs        # Erweiterte Agenten
│       │   ├── agent_analytics.rs       # Agenten-Analyse
│       │   ├── agent_insight.rs         # Agenten-Insights
│       │   ├── agent_nudge.rs           # Agenten-Hinweise
│       │   ├── agent_profile.rs         # Agenten-Profil
│       │   ├── agent_role.rs            # Agenten-Rollen
│       │   ├── background_tasks.rs      # Hintergrundaufgaben
│       │   ├── browser.rs              # Browser-Automatisierung
│       │   ├── chart_generator.rs       # Diagrammgenerierung
│       │   ├── cloud_workspace.rs       # Cloud-Arbeitsbereich
│       │   ├── computer_control.rs      # Computersteuerung
│       │   ├── context_breakdown.rs     # Kontext-Aufschlüsselung
│       │   ├── conversation_categories.rs  # Gesprächskategorien
│       │   ├── conversations_search.rs  # Gesprächssuche
│       │   ├── crash_report.rs          # Absturzbericht
│       │   ├── dream.rs                # Traum-Integration
│       │   ├── evolution.rs            # Skill-Evolution
│       │   ├── fine_tune.rs            # LoRA-Feinabstimmung
│       │   ├── gateway.rs              # API-Gateway
│       │   ├── gateway_link.rs         # Externe Links
│       │   ├── generated_tool.rs        # Generierte Werkzeuge
│       │   ├── image_gen.rs            # Bildgenerierung
│       │   ├── knowledge.rs            # Wissensbasis
│       │   ├── llm_wiki.rs             # LLM-Wiki
│       │   ├── local_models.rs         # Lokale Modelle
│       │   ├── mcp.rs                  # MCP-Protokoll
│       │   ├── memory.rs              # Speichersystem
│       │   ├── message_continuation.rs  # Nachrichtenfortsetzung
│       │   ├── onboarding.rs           # Onboarding-Assistent
│       │   ├── parallel_execution.rs    # Parallele Ausführung
│       │   ├── plan.rs                 # Planverwaltung
│       │   ├── platform_integration.rs  # Plattform-Integration
│       │   ├── plugin.rs               # Plugin-Verwaltung
│       │   ├── proactive.rs            # Proaktive Vorschläge
│       │   ├── prompt_templates.rs      # Prompt-Vorlagen
│       │   ├── providers.rs            # Modellanbieter
│       │   ├── quickbar.rs             # QuickBar
│       │   ├── reflection.rs           # Reflexion
│       │   ├── research.rs             # Tiefenrecherche
│       │   ├── rl.rs                   # Reinforcement Learning
│       │   ├── sandbox.rs              # Sandbox
│       │   ├── scheduled_task.rs        # Geplante Aufgaben
│       │   ├── screen_vision.rs        # Bildschirm-Sehen
│       │   ├── search.rs               # Suche
│       │   ├── session_share.rs         # Sitzungsfreigabe
│       │   ├── shell.rs                # Shell
│       │   ├── skill_decomposition.rs   # Skill-Zerlegung
│       │   ├── skills_hub.rs           # Skill-Hub
│       │   ├── tool_recommender.rs      # Werkzeug-Empfehlung
│       │   ├── tracer.rs               # Tracing
│       │   ├── user_profile.rs          # Benutzerprofil
│       │   ├── webdav.rs               # WebDAV
│       │   ├── webhook.rs              # Webhook
│       │   ├── wiki.rs                 # Wiki
│       │   ├── work_engine.rs          # Arbeits-Engine
│       │   ├── workflow_ai.rs          # KI-Workflow
│       │   ├── workflow_template.rs     # Workflow-Vorlagen
│       │   └── ...                     # Weitere Befehle
│       ├── init/              # Initialisierungsmodule
│       ├── stock_scheduler.rs # Aktien-Scheduler
│       └── ...                # Weitere Kernmodule
│
├── extension/                  # Browser-Erweiterung (Wiki Clipper: popup/content/background)
├── e2e/                        # Playwright E2E-Tests (9 Test-Suites)
├── scripts/                    # Build- und Werkzeugskripte
└── website/                    # Projekt-Website (VitePress, 11-Sprach-Dokumentation)
```

## Datenverzeichnis

```
~/.axinvest/                     # Konfigurationsverzeichnis
├── axinvest.db                  # SQLite-Datenbank
├── master.key                   # AES-256-Hauptschlüssel
├── vector_db/                   # Vektordatenbank (sqlite-vec)
└── ssl/                         # SSL-Zertifikate

~/Documents/axinvest/           # Benutzerdateiverzeichnis
├── images/                     # Bildanhänge
├── files/                      # Dateianhänge
└── backups/                    # Backup-Dateien
```

---

## FAQ

### macOS: „App ist beschädigt" oder „Entwickler kann nicht überprüft werden"

Da die Anwendung nicht von Apple signiert ist:

**1. Apps aus „Beliebiger Herkunft" zulassen**
```bash
sudo spctl --master-disable
```

Gehen Sie dann zu **Systemeinstellungen → Datenschutz & Sicherheit → Sicherheit** und wählen Sie **Beliebige Herkunft**.

**2. Das Quarantäne-Attribut entfernen**
```bash
sudo xattr -dr com.apple.quarantine /Applications/AxInvest.app
```

**3. Zusätzlicher Schritt für macOS Ventura+**
Gehen Sie zu **Systemeinstellungen → Datenschutz & Sicherheit** und klicken Sie auf **Trotzdem öffnen**.

---

## Community

- [LinuxDO](https://linux.do)

## Lizenz

Dieses Projekt ist unter der [AGPL-3.0](LICENSE)-Lizenz lizenziert.
