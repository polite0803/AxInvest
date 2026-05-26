[**English**](./README-EN.md) | [简体中文](./README.md) | [繁體中文](./README-ZH-TW.md) | [日本語](./README-JA.md) | [한국어](./README-KO.md) | **Français** | [Deutsch](./README-DE.md) | [Español](./README-ES.md) | [Русский](./README-RU.md) | [हिन्दी](./README-HI.md) | [العربية](./README-AR.md)

[![AxInvest](https://github.com/polite0803/AxAgent/blob/main/src/assets/image/logo.png?raw=true)](https://github.com/polite0803/AxAgent)

<p align="center">
  <a href="https://www.producthunt.com/products/axagent?embed=true&amp;amp&utm_source=badge-featured&amp;amp;&amp;#10;&amp;amp;&amp;;utm_medium=badge&amp;#10&amp;amp;;utm_campaign=badge-axagent" target="_blank" rel="noopener noreferrer"><img alt="AxInvest - Plateforme d'analyse d'investissement intelligente alimentée par l'IA | Product Hunt" width="250" height="54" src="https://api.producthunt.com/widgets/embed-image/v1/featured.svg?post_id=1118403&amp;theme=light&amp;t=1775627359538"></a>
</p>

<p align="center">
  <strong>Analyse d'investissement intelligente alimentée par l'IA | Collaboration multi-agents | Local d'abord</strong>
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

## Qu'est-ce qu'AxInvest ?

**AxInvest v2.3** est une plateforme d'analyse d'investissement intelligente alimentée par l'IA, construite sur le framework multi-agents AxAgent. Elle intègre les capacités avancées des agents IA à l'analyse d'investissement professionnelle du marché A, prenant en charge plusieurs fournisseurs de modèles, la recherche par agents IA, l'orchestration de workflows visuels, la gestion locale des connaissances, une passerelle API intégrée, couvrant **Windows / macOS / Linux / Android / iOS**, avec une mise en page adaptative pour **bureau, tablette et mobile**.

La caractéristique principale d'AxInvest réside dans l'utilisation de mécanismes de débat contradictoire multi-agents, de recherche approfondie et de vérification des faits pour fournir un soutien analytique complet et objectif aux décisions d'investissement.

---

## Aperçu par captures d'écran

| Conversation et sélection de modèle | Tableau de bord multi-agents |
|:---:|:---:|
| ![](.github/images/s1-0412.png) | ![](.github/images/s5-0412.png) |

| Base de connaissances RAG | Mémoire et contexte |
|:---:|:---:|
| ![](.github/images/s3-0412.png) | ![](.github/images/s4-0412.png) |

| Éditeur de workflow | Passerelle API |
|:---:|:---:|
| ![](.github/images/s9-0412.png) | ![](.github/images/s10-0412.png) |

---

## Fonctionnalités principales

### 📈 Analyse d'investissement intelligente

Module central d'AxInvest, intégrant les capacités des agents IA à l'analyse d'investissement professionnelle :

**Agrégation et dégradation multi-sources**

- **9 sources de données** — Tencent Finance, Tongdaxin (mootdx), Eastmoney, Sina Finance, Baidu Stock, THS, Iwencai, cninfo, AKShare
- **22 routes de données** — Chaque type de données configure des routes de dégradation multi-sources, basculement automatique vers la source de secours lorsque la source principale est indisponible
- **Collecte de données concurrente** — `tokio::join!` collecte concurrente de 16 types de données individuelles + 5 types de données de marché, maximisant l'efficacité de collecte
- **Cache intelligent** — Cache mémoire LRU (limite 1000 entrées), cotations TTL 30s / K-line TTL 300s, expiration automatique
- **Vérification de santé** — Sonde de connectivité fournisseur (Ping An Bank 000001 comme sonde), détection de disponibilité des sources de données à l'exécution

**Identification et règles du marché A**

- **Identification des secteurs** — Identification automatique par préfixe de code : Shanghai principal (6), STAR Market (688), Shenzhen principal (0), ChiNext (3), BSE (8)
- **Règles de limite haussière/baissière** — STAR Market/ChiNext ±20%, BSE ±30%, marché principal ±10%, actions ST ±5%
- **Calendrier de trading** — Jours fériés et jours de travail ajustés du marché A 2025-2026 intégrés, prise en charge du jugement des jours de trading

**Données individuelles (16 types)**

- **Cotations en temps réel** — Prix, variation, volume/montant, taux de rotation, PE/PB, capitalisation, prix limite haussière/baissière, indicateur ST
- **Données K-line** — 7 périodes (5 min/15 min/30 min/60 min/jour/semaine/mois), incluant volume, montant, taux de rotation
- **Analyse financière** — Chiffre d'affaires, bénéfice net, EPS, BPS, ROE, ratio d'endettement, marge brute, marge nette, croissance annuelle du CA, croissance annuelle du bénéfice
- **Flux de capitaux** — Flux nets principal/super grand ordre/grand ordre/ordre moyen/petit ordre
- **Liste dragon-tigre** — Montants achats/ventes des succursales, montant net, raison de l'inscription
- **Levée de restrictions** — Date de levée, nombre d'actions, ratio de levée, informations actionnaires
- **Marge et vente à découvert** — Montant achats/marge de financement, volume vente à découvert/solde
- **Capitaux nord** — Nombre d'actions détenues, proportion détenue, variation
- **Classification sectorielle** — Industries Shenwan niveau 1/2, étiquettes de secteurs conceptuels
- **Augmentation/réduction par action** — Dynamique d'augmentation/réduction par action, raisons
- **Enregistrement des dividendes** — Date ex-dividende, dividende par action, ratio de distribution, date d'enregistrement
- **Agrégation de rapports** — Rapports de recherche de courtage, incluant institution, analyste, notation, prix cible, prévisions EPS
- **EPS consensus** — EPS consensus institutionnel, prix cible consensus, notation moyenne, nombre de notations
- **Secteurs conceptuels** — Appartenance tridimensionnelle (industrie/concept/région), incluant variation sectorielle
- **Recherche d'annonces** — Annonces d'entreprises cotées cninfo, incluant type d'annonce et lien PDF
- **Sentiment de l'actualité** — Titre/résumé/source de l'actualité, incluant score de sentiment

**Données de marché (5 types)**

- **Liste dragon-tigre du marché complet** — Toutes les actions inscrites du jour, incluant achats nets, montants achats/ventes
- **Actions populaires** — Actions fortes THS, incluant variation, taux de rotation, étiquettes de raison, secteurs d'appartenance
- **Classement sectoriel** — Variation sectorielle Shenwan, montant des transactions, actions en hausse
- **Flash CIF** — Flash financiers en temps réel, incluant titre, contenu, source
- **Flux de capitaux nord** — Flux de capitaux minute par minute Shanghai/Shenzhen/total

**Calcul d'indicateurs techniques (module indicators)**

- **Système de moyennes mobiles** — MA5/MA10/MA20/MA60, incluant jugement d'état d'alignement (haussier/baissier/haussier faible/croisement enroulé)
- **MACD** — DIF/DEA/histogramme, incluant jugement de signal (croix dorée/croix morte/tendance haussière/tendance baissière)
- **RSI** — RSI6/RSI12/RSI24, incluant jugement de signal (surachat/survente/fort/faible/neutre)
- **Bandes de Bollinger** — Bande supérieure/bande médiane/bande inférieure (20,2), incluant jugement de position (au-dessus bande supérieure/zone bande supérieure/près bande médiane/zone bande inférieure/au-dessous bande inférieure)
- **Taux d'écart** — Taux d'écart MA5, taux d'écart MA20
- **Analyse volumique** — Ratio de volume (volume du jour/moyenne 5 jours), incluant jugement de signal (hausse avec volume/baisse avec volume réduit/baisse avec volume/hausse avec volume réduit/normal)
- **Support/Résistance** — Calcul automatique basé sur les hauts/bas récents et les moyennes mobiles

**Enregistrement d'outils MCP (module mcp_tools)**

- Les capacités de données boursières sont enregistrées comme outils standard via le protocole MCP, les agents IA peuvent les appeler directement en conversation
- Outils enregistrés : search_stock, get_stock_quote, get_stock_kline, get_stock_financials, get_stock_news, get_stock_money_flow, get_stock_dragon_tiger, etc.

**Pipeline d'analyse IA (crate stock-analysis, 23 sous-modules)**

- **Orchestration d'analyse** — orchestrator (orchestration de pipeline), pipeline (pipeline multi-étapes), runner (exécuteur de tâches)
- **Moteur de décision** — decision (décision d'investissement), signals (génération de signaux de trading), rules (moteur de règles de trading)
- **Évaluation des risques** — risk (modèle d'évaluation des risques), portfolio_risk (risque de portefeuille), position_limits (limites de position et conformité)
- **Sélection et backtesting** — screener (sélecteur multi-critères), backtest (moteur de backtesting), trading (framework de stratégies de trading)
- **Investissement value** — value (analyse de valeur), value_investing (framework d'évaluation d'investissement value)
- **Contrôle qualité** — quality (vérification qualité des données), data_clean (nettoyage et prétraitement des données), review (révision des résultats d'analyse)
- **Rapports et notation** — report (génération de rapports d'analyse), scoring (système de notation globale)
- **Modules auxiliaires** — key_levels (identification des niveaux clés), monitor (surveillance en temps réel et alertes), plugin (extension de plugins d'analyse), prompts (modèles de prompts IA)

**Composants d'analyse frontend (16)**

- StockAnalysisPage, StockQuoteCard, KLineChart, RiskMatrix, TradePanel
- DecisionBanner, DebatePanel, WatchlistPanel, PriceAlertPanel, CompareView
- AnalystReportGrid, AnalystReportCard, HistoricalAnalysisPanel, StockSearchBar
- AnalysisProgress, StockAnalysisSettingsModal, StockAnalysisChatIndicator

**Débat contradictoire et décision**

- **Débat contradictoire** — Débat Pro/Con multi-agents, support de notation de force d'argument et suivi de réfutation
- **Bannière de décision** — Visualisation de décision acheter/vendre/conserver, incluant confiance et raisons
- **Intégration workflow IA** — Workflow d'analyse boursière connecté de manière transparente à la conversation (stockWorkflowChatBridge)

### 🤖 Support des modèles IA

- **Support multi-fournisseurs** — Intégration native OpenAI, Anthropic Claude, Google Gemini, Ollama, OpenClaw, Hermes et toutes les API compatibles OpenAI
- **Rotation multi-clés** — Configuration de plusieurs clés API par fournisseur, rotation automatique et limitation de débit
- **Inférence de modèle local** — Support complet des modèles locaux Ollama, incluant la gestion de fichiers GGUF/GGML
- **Moteur d'inférence Candle** — Inférence locale Candle intégrée, support des interfaces rerank/judge, téléchargement GGUF à la demande
- **Gestion des modèles** — Récupération de liste de modèles distants, paramètres personnalisables (temperature, max tokens, top-p, etc.)
- **Sortie en streaming** — Rendu token par token en temps réel, support des blocs de pensée repliables (pensée étendue Claude)
- **Comparaison multi-modèles** — Question simultanée à plusieurs modèles, comparaison côte à côte des résultats
- **Appel de fonctions** — Appel de fonctions structuré sur tous les fournisseurs supportés
- **OpenAI Responses API** — Support du format de transmission OpenAI Responses
- **API en temps réel** — Compatible avec les événements WebSocket de l'API en temps réel OpenAI
- **Génération d'images** — Panneau de génération d'images IA, support de plusieurs modèles et configurations de paramètres

### 🔐 Système d'agents IA

Le système d'agents est construit sur une architecture sophistiquée (crate agent, 70+ fichiers sources), avec les caractéristiques suivantes :

- **Moteur de raisonnement ReAct** — Fusion du raisonnement et de l'action, auto-vérification intégrée pour assurer l'exécution fiable des tâches
- **Planificateur hiérarchique** — Décomposition des tâches complexes en plans structurés avec phases et dépendances
- **Décomposeur de tâches** — Décomposition automatique des tâches complexes en sous-tâches exécutables
- **Chaîne de pensée** — Visualisation du raisonnement décisionnel des agents, décomposition étape par étape
- **Arbre de pensées** — tree_of_thoughts exploration de raisonnement multi-chemins
- **Recherche approfondie** — Orchestration de recherche multi-sources, suivi de citations et évaluation de crédibilité
- **Vérification des faits** — Vérification des faits pilotée par IA et classification des sources
- **Orchestration de recherche** — Coordination de multiples fournisseurs de recherche, support de planification de recherche et synthèse de résultats
- **Recherche académique** — Recherche de littérature académique et analyse de citations
- **Contrôle informatique** — Clics de souris, saisie clavier, défilement d'écran contrôlés par IA, avec analyse par modèle visuel
- **Perception d'écran** — Capture d'écran et analyse par modèle visuel, pour l'identification d'éléments UI
- **Pipeline visuel** — vision_pipeline compréhension et analyse d'images
- **Mode de permissions à trois niveaux** — Par défaut (approbation requise), Accepter les modifications (approbation automatique), Accès complet (sans invite)
- **Isolation sandbox** — Opérations des agents strictement limitées au répertoire de travail spécifié
- **Panneau d'approbation d'outils** — Affichage en temps réel des demandes d'appel d'outils, approbation individuelle
- **Suivi des coûts** — Affichage en temps réel de l'utilisation de tokens et des statistiques de coûts par session
- **Pause/Reprise** — Pause de l'exécution de l'agent à tout moment, reprise ultérieure
- **Système de points de contrôle** — Points de contrôle persistants pour la récupération après crash et la reconnexion de session
- **Moteur de récupération d'erreurs** — Classification automatique des erreurs, analyse des causes racines et exécution de stratégies de récupération
- **Détection de boucles** — Détection et interruption automatiques des comportements en boucle dans le raisonnement des agents
- **Mode proactif** — Les agents peuvent proposer des suggestions et exécuter des actions de manière proactive
- **Gestion des objectifs** — Maintien et suivi des objectifs d'exécution et du contexte des agents
- **Auto-vérification** — self_verifier vérification automatique de la correction des sorties des agents
- **Réflecteur** — reflector réflexion et amélioration du processus de raisonnement
- **Entrée de direction** — steer_manager ajustement dynamique de la direction comportementale des agents
- **Bus d'événements** — event_bus / event_emitter architecture événementielle des agents
- **Synthèse de contenu** — content_synthesizer synthèse multi-sources et génération de rapports
- **Suivi de citations** — citation_tracker suivi et annotation automatiques des sources d'information
- **Évaluation de crédibilité** — credibility_evaluator évaluation de la crédibilité des sources d'information
- **Construction de plan** — outline_builder construction automatique de plans de recherche
- **Gestion de schémas** — schema_manager gestion des schémas de structure de sortie
- **Mémoire de projet** — project_memory mémoire persistante au niveau du projet
- **Détection d'environnement** — environment_probe détection automatique des informations d'environnement d'exécution
- **Vérification de santé** — health_checker surveillance de l'état de santé des agents

### 👥 Collaboration multi-agents

- **Coordination de sous-agents** — Architecture maître-esclave, coordinator coordonne plusieurs agents collaboratifs
- **Exécution parallèle** — Traitement parallèle des tâches par plusieurs agents, support de l'ordonnancement sensible aux dépendances
- **Débat contradictoire** — adversarial_debate tours de débat Pro/Con, support de notation de force d'argument et suivi de réfutation
- **Rôles d'agents** — agent_roles rôles prédéfinis (chercheur, planificateur, développeur, réviseur, synthétiseur) pour la collaboration en équipe
- **Orchestrateur d'agents** — Routage centralisé des messages et gestion d'état pour les équipes multi-agents
- **Graphe de communication** — graph_insights visualisation des interactions et flux de messages entre agents
- **Tableau noir partagé** — shared_blackboard / blackboard espace d'état partagé inter-agents
- **Système Buddy** — Partenaires d'agents configurables, support de définition d'espèces et d'attributs
- **Mémoire partagée** — Espace mémoire partagé inter-agents, support de statistiques et requêtes
- **Enregistrement Cron d'équipe** — Ordonnancement de tâches planifiées au niveau de l'équipe
- **Système d'experts** — agency_expert agent expert de domaine
- **Profil d'agent** — agent_profile gestion du profil de personnalité et de capacités des agents

### ⭐ Système de compétences

- **Marché des compétences** — Marché intégré, navigation et installation de compétences contribuées par la communauté
- **Création de compétences** — Création automatique de compétences à partir de propositions, support d'éditeur Markdown
- **Évolution des compétences** — skill_evolution analyse et amélioration automatiques des compétences existantes basées sur le feedback d'exécution
- **Correspondance de compétences** — skill_matcher correspondance sémantique, recommandation de compétences pertinentes au contexte de conversation
- **Décomposition de compétences** — Décomposition automatique de tâches complexes en compétences atomiques exécutables (assistée par LLM/multi-tours/validation de workflow)
- **Outils générés** — Génération et enregistrement automatiques de nouveaux outils par l'IA, étendant les capacités des agents
- **Centre de compétences** — skills_hub_adapter interface centralisée de découverte et de gestion de configuration des compétences
- **Client du centre de compétences** — skills_hub_client intégration avec le centre de compétences distant, support du partage communautaire
- **Vérification des dépendances** — Détection automatique des dépendances de compétences et de la disponibilité des outils
- **Conteneur sandbox de compétences** — Exécution sécurisée des compétences dans un environnement isolé
- **Compétences atomiques** — atomic_skill plus petite unité de compétence exécutable
- **Proposition de compétences** — skill_proposal proposition de création de compétences pilotée par IA

### 🔄 Système de workflows

Le moteur de workflows (crate rt-workflow) implémente un système d'orchestration de tâches basé sur les DAG :

- **Éditeur de workflows visuel** — Concepteur de workflows par glisser-déposer, support de connexion et configuration de nœuds
- **16 types de nœuds** — Déclencheur, agent, LLM, condition, parallèle, boucle, fusion, délai, outil, code, sous-workflow, recherche vectorielle, analyse de document, validation, fin, fallback
- **16 panneaux de propriétés** — Chaque type de nœud dispose d'un panneau de configuration indépendant
- **Modèles de workflows** — Préréglages intégrés : revue de code, correction de bugs, documentation, test, refactoring, exploration, performance, sécurité, développement de fonctionnalités
- **Exécution DAG** — Tri topologique par algorithme de Kahn, support de détection de cycles
- **Ordonnancement parallèle** — Exécution en pipeline, les étapes rapides n'attendent pas les étapes lentes
- **Stratégie de retry** — Backoff exponentiel, nombre maximal de retries configurable par étape
- **Achèvement partiel** — Les étapes en échec ne bloquent pas les étapes en aval indépendantes
- **Gestion de versions** — Contrôle de version des modèles de workflows, support de rollback
- **Historique d'exécution** — Enregistrement détaillé, support de suivi d'état et de débogage
- **Assistance IA** — Conception de workflows assistée par IA, recommandation de nœuds et optimisation de prompts d'agents
- **Vérification sémantique** — Validation sémantique des workflows, détection de problèmes potentiels
- **Import n8n** — Support d'import de workflows depuis un répertoire n8n
- **Panneau de débogage** — Débogage en temps réel et visualisation d'état de l'exécution des workflows
- **Couche de cache** — cache_layer cache des résultats d'exécution des workflows
- **Marché** — workflow_marketplace marché et revue de modèles de workflows

### 📚 Connaissances et mémoire

- **Base de connaissances (RAG)** — Support multi-bases de connaissances, upload de documents, analyse automatique, segmentation et indexation vectorielle
- **Recherche hybride** — Combinaison de recherche par similarité vectorielle et classement BM25 en texte intégral
- **Reranking** — Reranking par cross-encoder, amélioration de la précision de récupération
- **Pipeline de rappel à trois niveaux** — Mécanisme de rappel multi-niveaux avec index AST + recherche vectorielle + FTS5
- **Self-RAG** — self_rag génération augmentée par récupération adaptative
- **Enrichissement de requêtes** — query_enhancement réécriture et expansion de requêtes
- **Graphe de connaissances** — Visualisation des relations d'entités de connaissances (entités, attributs, relations, flux, interfaces)
- **Système Wiki** — Compilateur et validateur LLM Wiki, support de visualisation de graphe de connaissances et synchronisation incrémentale
- **Notes Wiki** — Système de notes à liens bidirectionnels, support de vue en graphe et synchronisation automatique des liens
- **Système de mémoire** — Mémoire multi-espaces de noms, support de saisie manuelle ou d'extraction automatique par IA
- **Mémoire en boucle fermée** — Intégration de fournisseurs de mémoire persistante Honcho et Mem0
- **Oubli de mémoire** — memory_forgetting mécanisme de dégradation de mémoire basé sur le temps
- **Recherche plein texte FTS5** — Recherche rapide across conversations, fichiers, mémoires
- **Recherche de sessions** — Recherche avancée across toutes les sessions de conversation
- **Gestion du contexte** — Ajout flexible de fichiers, résultats de recherche, fragments de connaissances, mémoires, sorties d'outils
- **Analyse de documents** — Analyse automatique et extraction de contenu de documents multi-formats
- **Indexation incrémentale** — Mise à jour incrémentale de l'index des modifications de fichiers
- **Segmentation de texte** — text_chunker stratégie de segmentation de texte intelligente
- **Budget de tokens** — token_budget contrôle du budget de tokens des résultats de récupération

### 🌐 Passerelle API

- **Serveur API local** — Serveur intégré compatible OpenAI, Claude et Gemini
- **Liens externes** — Intégration en un clic de Claude CLI, OpenCode, synchronisation automatique des clés API et modèles
- **Gestion des clés** — Génération, révocation, activation/désactivation des clés d'accès, support de descriptions
- **Analyse d'utilisation** — Volume de requêtes et utilisation de tokens par clé, fournisseur, date
- **Support SSL/TLS** — Certificat auto-signé intégré, support de certificats personnalisés
- **Journal des requêtes** — Enregistrement complet de toutes les requêtes et réponses API
- **Modèles de configuration** — Modèles prédéfinis pour Claude, Codex, OpenCode, Gemini
- **API en temps réel** — Compatible avec les événements WebSocket de l'API en temps réel OpenAI
- **Intégration de plateformes** — Support DingTalk, Feishu, QQ, Slack, WeChat, WhatsApp, Telegram, Discord
- **Diagnostics de passerelle** — Diagnostics de connexion et gestion des politiques de programme
- **Limiteur de débit** — Limitation du débit des requêtes API et contrôle du trafic
- **File persistante** — Gestion de file persistante des requêtes
- **API boursière** — stock_handlers endpoints API dédiés aux données boursières
- **Push SSE** — sse Server-Sent Events push d'événements en temps réel

### 🔧 Outils et extensions

- **Protocole MCP** — Implémentation complète du Model Context Protocol, support des transports stdio et HTTP/WebSocket
- **Authentification OAuth** — Support du flux OAuth pour les serveurs MCP
- **Démarrage automatique MCP** — Démarrage automatique et gestion du cycle de vie des serveurs MCP
- **Pontage d'outils MCP** — Pontage des outils MCP avec le système d'outils des agents
- **Vérification de santé MCP** — mcp_health surveillance de l'état de santé des serveurs MCP
- **Système de plugins** — Architecture de plugins à trois niveaux compatible OpenClaw (intégré/bundlé/externe), support d'installation de paquets npm, enregistrement d'outils, hooks et gestion du cycle de vie
- **Marché de plugins** — Interface de marché intégrée, support de recherche et installation npm, popup de confirmation
- **Outils intégrés** — 40+ modules d'outils : opérations de fichiers (lecture/écriture/édition/système), exécution de code, recherche (Grep/Glob), Bash, recherche/extraction Web, gestion de plans, ordonnancement Cron, REPL, LSP, gestion de contexte, contrôle informatique, push de messages, tâches, base de données, DevOps, analyse de documents, Git, récupération de connaissances, LSP, traitement média, push de messages, OCR, notifications push, informations système, système de tâches, test, espace de travail/worktree, etc.
- **Système de permissions d'outils** — Classification des permissions d'outils, gestion des règles et suivi d'utilisation
- **Sécurité Bash** — Analyse de commandes, validation de chemins et contrôle de sécurité sandbox
- **Client LSP** — Protocole de serveur de langage intégré, support de complétion de code et de diagnostics
- **Index AST** — Analyse AST et construction d'index des fichiers de code
- **Backend terminal** — Support de connexions terminal locales, Docker et SSH
- **Automatisation de navigateur** — Intégration du contrôle de navigateur via CDP (navigation, captures d'écran, clics, remplissage, extraction de texte, etc.)
- **Automatisation UI** — Identification et contrôle d'éléments UI multi-plateformes
- **Outils Git** — Opérations Git, support de détection de branches et de sensibilité aux conflits
- **Recommandation d'outils** — Moteur de recommandation intelligente d'outils basé sur le contexte
- **Orchestration d'outils** — Coordination d'exécution multi-outils et sortie en streaming
- **Statistiques d'outils** — Fréquence d'utilisation et statistiques de performance des outils
- **Audit d'outils** — audit journal d'audit des appels d'outils

### 📊 Rendu de contenu

- **Rendu Markdown** — Support complet de la coloration syntaxique, formules mathématiques LaTeX, tableaux, listes de tâches
- **Éditeur de code Monaco** — Éditeur intégré, support de coloration syntaxique, copie, aperçu des différences
- **Rendu de graphiques** — Diagrammes Mermaid, diagrammes d'architecture D2, graphiques interactifs ECharts
- **Panneau d'artefacts** — Extraits de code, brouillons HTML, composants React, notes Markdown, support de prévisualisation en temps réel
- **Quatre modes de prévisualisation** — Code (éditeur), écran partagé (côte à côte), prévisualisation (rendu uniquement), prévisualisation de composants React
- **Inspecteur de session** — Vue arborescente de la structure de session, navigation rapide
- **Panneau de citations** — Suivi et affichage des citations sources, support de notation de crédibilité
- **Rendu d'infographies** — Support de visualisation d'infographies
- **Interpréteur de graphiques** — ChartInterpreter interprétation de graphiques pilotée par IA
- **Visionneuse de différences** — DiffViewer comparaison de différences de code

### 🛡️ Données et sécurité

- **Chiffrement AES-256** — Clés API et données sensibles chiffrées avec AES-256-GCM
- **Stockage isolé** — État de l'application stocké dans `~/.axinvest/`, fichiers utilisateur stockés dans `~/Documents/axinvest/`
- **Sauvegarde automatique** — Sauvegarde planifiée vers un répertoire local ou un stockage WebDAV
- **Sauvegarde S3** — s3_backup support de sauvegarde cloud Amazon S3
- **Restauration de sauvegarde** — Restauration en un clic depuis un historique de sauvegardes
- **Options d'export** — Captures d'écran PNG, Markdown, texte brut, format JSON
- **Gestion du stockage** — Affichage visuel de l'utilisation disque et outils de nettoyage
- **Migration de stockage** — storage_migration migration de données entre versions
- **Autorisation de fichiers** — Autorisation et révocation d'accès aux fichiers
- **Audit des opérations** — Journal d'audit des opérations critiques
- **Validation de commandes** — command_validator validation de sécurité des commandes
- **Limites de ressources** — resource_limits limites d'utilisation des ressources
- **Exécution sandbox** — sandbox_runner exécution en environnement isolé

### 🖥️ Expérience bureau

- **Moteur de thèmes** — Thèmes sombre/clair, support du suivi système ou préférence manuelle
- **Langue de l'interface** — 11 langues : chinois simplifié, chinois traditionnel, anglais, japonais, coréen, français, allemand, espagnol, russe, hindi, arabe
- **Barre d'état système** — Minimisation dans la barre d'état, les services en arrière-plan ne sont pas interrompus
- **Fenêtre toujours au-dessus** — Fenêtre positionnée au-dessus des autres fenêtres
- **Raccourcis globaux** — Raccourcis clavier personnalisables pour afficher la fenêtre principale
- **QuickBar** — Barre flottante d'accès rapide, invocation en un clic
- **Démarrage automatique** — Option de lancement au démarrage du système
- **Support proxy** — Configuration proxy HTTP et SOCKS5
- **Mise à jour automatique** — Vérification automatique de version, notification en cas de mise à jour
- **Palette de commandes** — `Cmd/Ctrl+K` accès rapide aux commandes
- **Assistant de configuration** — Guide interactif de première utilisation et détection Ollama
- **Centre de notifications** — Gestion unifiée des notifications dans l'application
- **Espace de travail cloud** — cloud_workspace sélection d'espace de travail cloud
- **Rapport de crash** — crash_report collecte automatique des rapports de crash
- **Appel vocal** — VoiceCall capacité de conversation vocale

### 🔬 Fonctionnalités avancées

- **Recherche approfondie** — Recherche multi-sources, suivi de citations, évaluation de crédibilité et synthèse de contenu
- **Vérification des faits** — Vérification des faits pilotée par IA et classification des sources
- **Ordonnanceur Cron** — Ordonnancement de tâches automatisées, support de modèles quotidien/hebdomadaire/mensuel et d'expressions cron personnalisées
- **Système Webhook** — Abonnement aux événements, support de notifications de complétion d'outils, d'erreurs d'agents, de fin de session
- **Profil utilisateur** — Apprentissage automatique du style de code, conventions de nommage, indentation, style de commentaires, préférences de communication
- **Optimiseur RL** — Optimisation par apprentissage par renforcement de la sélection d'outils et des stratégies de tâches
- **Fine-tuning LoRA** — Adaptation de modèle personnalisé par fine-tuning local avec LoRA
- **Suggestions proactives** — Invites contextuelles basées sur le contenu de conversation et les modèles utilisateur
- **Prédiction de contexte** — Prédiction des prochaines actions de l'utilisateur et pré-récupération des ressources pertinentes
- **Consolidation onirique** — dream_consolidation consolidation automatique en arrière-plan des mémoires et modèles, optimisation des connaissances à long terme
- **Récupération d'erreurs** — Classification automatique des erreurs, analyse des causes racines et suggestions de récupération
- **Outils développeur** — Trace, Span, visualisation de timeline, pour le débogage et l'analyse de performance
- **Système de benchmark** — Évaluation de performance des tâches SWE-bench / Terminal-bench et métriques, avec scorecard
- **Migration de style** — style_migrator application des préférences de style de code apprises au code généré
- **Plugin de tableau de bord** — Tableau de bord extensible, support de panneaux et widgets personnalisés
- **Partage collaboratif** — Collaboration en temps réel CRDT et partage de session en un clic
- **Extension de navigateur** — Extension de navigateur Wiki Clipper, clippage rapide de pages web vers LLM Wiki
- **SDK Python** — SDK Python pour l'intégration avec AxInvest
- **Routage intelligent** — Routage et classification intelligents des requêtes
- **Cache sémantique** — Cache de réponses basé sur la sémantique, réduction du calcul répétitif
- **Compression de contexte** — Compression automatique des contextes longs, optimisation de l'utilisation des tokens
- **Traitement par lots de messages** — Envoi et optimisation par lots de messages
- **Pool de connexions** — Gestion du pool de connexions base de données et API
- **Feature flags** — Système de feature flags configurable
- **Moteur de politiques** — Gestion centralisée des politiques de permissions et d'opérations
- **Gouvernance des ressources** — Limites et gouvernance de l'utilisation des ressources par les agents
- **Transfert LAN** — Capacité de transfert de fichiers en réseau local
- **Coévolution** — coevolution coévolution des compétences et des agents
- **Apprentissage comportemental** — behavior_learner / behavior_tracker apprentissage et suivi du comportement utilisateur
- **Apprentissage des préférences** — preference_learner apprentissage automatique des préférences utilisateur
- **Récompense intrinsèque** — intrinsic_reward exploration pilotée par motivation intrinsèque
- **Récompense de processus** — process_reward signal de récompense au niveau du processus
- **TextGrad** — text_grad optimisation automatique basée sur les gradients de texte
- **Compression de trajectoire** — trajectory_compressor compression automatique des trajectoires longues
- **Gestion des rappels** — reminder_manager ordonnancement intelligent des rappels
- **Pré-récupération de tâches** — task_prefetcher pré-récupération prédictive des ressources de tâches

### 🛡️ Protection contre l'injection de prompts (Prompt-Guard)

- **Système de protection à quatre niveaux** — L1 Détection de motifs (interception haut risque + marquage risque moyen) → L2 Échappement de délimiteurs → L3 Wrapper XML → L4 Étiquettes de confiance
- **Orchestrateur de pipeline** — Pipeline de détection multi-niveaux en série, support de seuils de risque personnalisables
- **Détection de Token Smuggling** — Détection spécialisée contre l'obfuscation d'encodage et les attaques de contrebande de tokens
- **Détection d'échappement de délimiteurs** — delimiter_escape détection des attaques d'évasion de délimiteurs de prompts
- **Détection de motifs** — pattern_detect correspondance de motifs d'injection par regex + heuristiques
- **Étiquettes de confiance** — trust_labels marquage et vérification de contenu de confiance
- **Mode Strict** — Tests en mode strict + nommage des raisons de risque moyen + documentation de motifs personnalisés
- **Intégration pipeline complète** — Intégré dans les sessions / prompts / git / RAG

### 📱 Support mobile

- **Android natif** — Build APK/AAB, support arm64-v8a / armeabi-v7a / x86_64
- **iOS natif** — Build IPA, support arm64
- **Mise en page adaptative** — Adaptation automatique bureau/tablette/mobile (hook useResponsive)
- **Navigation mobile** — Navigation Drawer + barre de navigation inférieure + bouton flottant flash
- **Adaptation zone de sécurité** — Adaptation CSS env() de la barre d'état/barre de navigation Android
- **Optimisation CSP** — Liste blanche de protocoles CSP Android WebView
- **Compilation conditionnelle** — `#[cfg(not(mobile))]` exclusion automatique des fonctionnalités bureau exclusives (navigateur, contrôle informatique, bureau, QuickBar, terminal, vision écran)

---

## Architecture technique

### Stack technique

| Couche | Technologie |
|--------|-------------|
| **Framework** | Tauri 2 + React 19 + TypeScript 6 |
| **UI** | Ant Design 6 + TailwindCSS 4 |
| **Gestion d'état** | Zustand 5 |
| **Routage** | React Router 7 |
| **Internationalisation** | i18next + react-i18next |
| **Backend** | Rust 2024 + SeaORM 2 + SQLite |
| **Base de données vectorielle** | sqlite-vec |
| **Éditeur de code** | Monaco Editor |
| **Graphiques** | Mermaid + D2 + ECharts (CDN) |
| **Terminal** | xterm.js 6 |
| **Workflow** | ReactFlow 11 |
| **Rendu de graphiques** | @antv/infographic |
| **Icônes** | Iconify + Lucide |
| **Glisser-déposer** | @dnd-kit |
| **Build** | Vite 8 + npm |
| **Test** | Vitest + Playwright + cargo-nextest |
| **Formatage** | dprint (TS/JSON) + rustfmt |
| **Lint** | TS: eslint + oxlint / Rust: clippy + cargo-deny |
| **Mobile** | Tauri Android + iOS build natif |
| **Bureau** | Windows (MSI) · macOS (DMG) · Linux (AppImage/deb/rpm) |

### Support de plateformes

| Plateforme | Architecture |
|------------|-------------|
| Windows | x86_64, ARM64 |
| macOS | Apple Silicon (arm64), Intel (x86_64) |
| Linux | x86_64, ARM64 |
| Android | arm64-v8a, armeabi-v7a, x86_64 (émulateur) |
| iOS | arm64 |

### Architecture du backend Rust

Le backend est organisé en workspace Rust, comprenant **20** crates spécialisés :

```
src-tauri/crates/
├── agent/            # Noyau des agents IA (70+ fichiers sources : moteur ReAct, coordination, planification, recherche approfondie, vérification des faits, etc.)
├── astock-data/      # Sources de données du marché A (9 sources de données, 22 routes de données, indicateurs techniques, calendrier de trading, enregistrement d'outils MCP)
├── core/             # Utilitaires principaux (85+ entités de base de données, 40+ dépôts, RAG, chiffrement, MCP, automatisation de navigateur, index AST, etc.)
├── gateway/          # Passerelle API (serveur HTTP, authentification, routage, interface compatible OpenAI, endpoints API boursière)
├── migration/        # Migrations de base de données (5 migrations : analyse boursière/portefeuille surveillé/ordonnancement analyse/alertes de prix/trading)
├── npm/              # Analyse de paquets npm et registre
├── plugins/          # Système de plugins (compatible OpenClaw, installation de paquets npm, incluant des plugins d'exemple)
├── prompt-guard/     # Protection contre l'injection de prompts (détection et défense multi-niveaux L1-L4, 4 détecteurs)
├── providers/        # Adaptateurs de fournisseurs de modèles (OpenAI, Anthropic, Gemini, Ollama, OpenClaw, Hermes, génération d'images)
├── rt-dashboard/     # Système de plugins de tableau de bord
├── rt-messaging/     # Passerelle de messagerie (9 plateformes : DingTalk/Feishu/QQ/Slack/WeChat/WhatsApp/Telegram/Discord)
├── rt-theme/         # Moteur de thèmes
├── rt-webhook/       # Serveur et distribution Webhook
├── rt-workflow/      # Moteur de workflows (orchestration DAG, 16 exécuteurs de nœuds, ordonnanceur, couche de cache)
├── runtime/          # Services d'exécution (70+ fichiers sources : gestion de sessions, MCP, terminal, limitation de débit, Webhook, permissions, benchmark, etc.)
├── runtime-core/     # Couche d'abstraction d'exécution (types publics, définitions de traits, configuration, feature flags, exécuteur de permissions)
├── stock-analysis/   # Analyse d'investissement intelligente (23 sous-modules : pipeline, moteur de décision, évaluation des risques, backtesting, sélecteur, investissement value)
├── telemetry/        # Télémétrie et traçage distribué (compatible OpenTelemetry)
├── tools/            # Système d'outils (40+ outils intégrés, sécurité Bash, pontage MCP, système de permissions, orchestration, audit)
└── trajectory/       # Système d'apprentissage (55+ fichiers sources : mémoire, compétences, RL, profil utilisateur, consolidation onirique, migration de style, coévolution)
```

#### Structure des modules du crate stock-analysis (23 sous-modules)

```
stock-analysis/
├── backtest.rs         # Moteur de backtesting de stratégies
├── data_clean.rs       # Nettoyage et prétraitement des données
├── decision.rs         # Moteur de décision d'investissement
├── key_levels.rs       # Identification des niveaux clés
├── monitor.rs          # Surveillance en temps réel et alertes
├── orchestrator.rs     # Orchestration de pipeline d'analyse
├── pipeline.rs         # Pipeline d'analyse multi-étapes
├── plugin.rs           # Extension de plugins d'analyse
├── portfolio_risk.rs   # Évaluation des risques de portefeuille
├── position_limits.rs  # Limites de position et conformité
├── prompts.rs          # Modèles de prompts IA
├── quality.rs          # Vérification qualité des données
├── report.rs           # Génération de rapports d'analyse
├── review.rs           # Révision des résultats d'analyse
├── risk.rs             # Modèle d'évaluation des risques
├── rules.rs            # Moteur de règles de trading
├── runner.rs           # Exécuteur de tâches d'analyse
├── scoring.rs          # Système de notation globale
├── screener.rs         # Sélecteur d'actions
├── signals.rs          # Génération de signaux de trading
├── trading.rs          # Framework de stratégies de trading
├── value.rs            # Analyse de valeur
└── value_investing.rs  # Évaluation d'investissement value
```

#### Sources de données du crate astock-data

| Source de données | Identifiant | Types de données supportés |
|-------------------|-------------|---------------------------|
| Tencent Finance | tencent | Cotations en temps réel, K-line |
| Tongdaxin | mootdx | Cotations en temps réel, K-line |
| Eastmoney | eastmoney | Cotations, K-line, financier, flux de capitaux, liste dragon-tigre, levée de restrictions, marge et vente à découvert, capitaux nord, classification sectorielle, augmentation/réduction par action, dividendes, rapports de recherche, liste dragon-tigre du marché complet, flash CIF |
| Sina Finance | sina | Cotations, K-line, actualités |
| Baidu Stock | baidu_stock | Cotations, actualités, flux de capitaux, liste dragon-tigre, levée de restrictions, marge et vente à découvert, capitaux nord, classification sectorielle, augmentation/réduction par action, dividendes, rapports de recherche, actions populaires, classement sectoriel, secteurs conceptuels, flux de capitaux nord |
| THS | ths | Cotations, classification sectorielle, EPS consensus, secteurs conceptuels, actions populaires, classement sectoriel, flux de capitaux nord |
| Iwencai | iwencai | Recherche d'actions, classification sectorielle, EPS consensus, secteurs conceptuels, actions populaires |
| cninfo | cninfo | Annonces |
| AKShare | akshare | Financier, actualités, EPS consensus, flash CIF |

Chaque type de données configure des routes de dégradation multi-sources, basculement automatique vers la source de secours lorsque la source principale est indisponible.

#### Modules supplémentaires astock-data

| Module | Fonctionnalité |
|--------|---------------|
| calendar | Calendrier de trading du marché A (jours fériés 2025-2026 + jours de travail ajustés) |
| indicators | Calcul d'indicateurs techniques (MA/MACD/RSI/Bandes de Bollinger/Taux d'écart/Volume/Support-Résistance) |
| mcp_tools | Enregistrement d'outils MCP (capacités de données boursières enregistrées comme outils appelables par l'IA) |

### Architecture frontend

```
src/
├── stores/                    # Gestion d'état Zustand (65 stores)
│   ├── domain/               # État métier principal (9)
│   │   ├── agentDomainStore.ts
│   │   ├── compressStore.ts
│   │   ├── conversationPreferences.ts
│   │   ├── conversationStore.ts
│   │   ├── conversationStoreEvents.ts
│   │   ├── conversationStoreSend.ts
│   │   ├── multiModelStore.ts
│   │   ├── preferenceStore.ts
│   │   └── streamStore.ts
│   ├── feature/               # État des modules fonctionnels (46)
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
│   ├── devtools/              # État des outils développeur (5)
│   │   ├── evaluatorStore.ts
│   │   ├── fineTuneStore.ts
│   │   ├── recommendationStore.ts
│   │   ├── rlStore.ts
│   │   └── tracerStore.ts
│   └── shared/                # État partagé (5)
│       ├── artifactStore.ts
│       ├── chatWorkspaceStore.ts
│       ├── rightPanelStore.ts
│       ├── tabStore.ts
│       └── uiStore.ts
│
├── components/                # Composants React (25 modules)
│   ├── chat/                # Interface de conversation (100+ composants : panneau d'exécution d'agent, comparaison de branches, automatisation de navigateur, exécuteur de code, panneau de collaboration, recherche approfondie, vérification des faits, commit Git, génération/analyse d'images, récupération de connaissances, extraction de mémoire, routage de modèles, affichage multi-modèles, gestion de permissions, marché de plugins, panneau de réflexion, création/évolution de compétences, pensée structurée, carte de sous-agent, carte d'appel d'outils, rejeu de trajectoire, appel vocal, recherche Wiki, progression de workflow, etc.)
│   ├── stock-analysis/      # Analyse d'investissement intelligente (16 composants)
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
│   ├── workflow/            # Éditeur de workflows (16 types de nœuds + 16 panneaux de propriétés + panneau IA + modèles + débogage)
│   ├── gateway/             # UI passerelle API (aperçu/clés/métriques/surveillance/paramètres/modèles/diagnostics)
│   ├── settings/            # Panneau de paramètres (50+ composants : fournisseurs/modèles/MCP/connaissances/mémoire/proxy/raccourcis/thème/outils/Webhook/Cron/configuration analyse boursière, etc.)
│   ├── terminal/            # UI terminal (terminal intégré/Docker/SSH/sélection backend/complétion de chemin/complétion slash)
│   ├── skill/               # Éditeur et rendu de compétences (édition chaîne d'actions/éditeur frontend/conteneur sandbox/vérification dépendances/panneau statistiques)
│   ├── benchmark/           # Panneau de benchmark (configuration/rapport/sélecteur/liste de tâches/résultats)
│   ├── files/               # Page de gestion de fichiers
│   ├── fine-tune/           # Configuration fine-tuning LoRA (dataset/tâches d'entraînement/configuration LoRA)
│   ├── link/                # Gestion des liens externes (aperçu/modèles/stratégies/compétences/détails stratégie)
│   ├── llm-wiki/            # Éditeur LLM Wiki (score qualité/statut synchronisation)
│   ├── proactive/           # Système de suggestions proactives (prédiction de contexte/indicateur de pré-récupération/barre de suggestions/liste de rappels)
│   ├── wiki/                # Gestion Wiki (liens retour/vue grappe/ingestion/vérification code/timeline opérations/agrégation tags/historique versions)
│   ├── devtools/            # Timeline Trace/Span (graphique coûts/graphique durées/détails/filtres/liste)
│   ├── decomposition/       # Décomposition de compétences (aperçu décomposition/dépendances outils/génération outils/installation outils)
│   ├── recommendation/      # Panneau de recommandation d'outils
│   ├── style/               # Migration de style de code (échantillons/curseurs d'ajustement/comparaison/panneau de prévisualisation)
│   ├── layout/              # Composants de mise en page (barre de titre/barre latérale/palette de commandes/copie globale/limites d'erreur/barre d'état/cloche de notification/modal profil utilisateur)
│   ├── help/                # Panneau d'aide
│   ├── notification/        # Centre de notifications
│   ├── search/              # Recherche de sessions
│   ├── onboarding/          # Assistant de configuration (tutoriel interactif/assistant de bienvenue)
│   ├── common/              # Composants communs (copie/icônes/curseurs de paramètres de modèle/coller)
│   └── shared/              # Composants partagés (édition avatar/modales/rendu de graphiques/icônes dynamiques/sélection modèle d'embedding/sélection Emoji/icône base de connaissances/icône MCP/sélection de modèle/éditeur Monaco/icône espace de noms/icône fournisseur de recherche)
│
├── pages/                    # Composants de page (22 pages)
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
├── hooks/                    # React hooks (12)
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
├── lib/                      # Fonctions utilitaires (33 modules + Web Worker)
│   ├── workers/            # Web Worker (heavy.worker.ts)
│   ├── actionRouter.ts     # Routage d'actions
│   ├── artifactRenderer.ts # Rendu d'artefacts
│   ├── chartGenerator.ts   # Génération de graphiques
│   ├── chatMarkdown.ts     # Rendu Markdown
│   ├── codeExecutor.ts     # Exécution de code
│   ├── invoke.ts           # Encapsulation Tauri IPC
│   ├── skillActionExecutor.ts  # Exécution d'actions de compétences
│   ├── skillEventBus.ts    # Bus d'événements de compétences
│   ├── skillLifecycle.ts   # Cycle de vie des compétences
│   ├── skillPermissions.ts # Permissions des compétences
│   ├── storeRegistry.ts    # Registre de stores
│   ├── tokenEstimator.ts   # Estimation de tokens
│   ├── workflowLayout.ts   # Mise en page de workflows
│   └── ...                 # Autres modules utilitaires
│
├── types/                    # Définitions de types TypeScript (22)
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
├── sdk/                      # SDK (incluant SDK Python)
│   ├── index.ts
│   ├── types.ts
│   ├── rpcBridge.ts
│   ├── sandboxTemplate.ts
│   └── python/              # SDK Python
│       ├── setup.py
│       └── axagent_sdk/__init__.py
│
└── i18n/                     # Traductions en 11 langues
```

## Démarrage rapide

### Télécharger les versions pré-construites

Visitez la page [Releases](https://github.com/polite0803/AxAgent/releases) pour télécharger l'installateur adapté à votre plateforme.

### Construire depuis les sources

#### Prérequis

- [Node.js](https://nodejs.org/) 20+
- [Rust](https://www.rust-lang.org/) 1.75+
- [npm](https://www.npmjs.com/) 10+
- Windows : [Visual Studio Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/) + Rust MSVC targets

#### Étapes de construction

```bash
# Cloner le dépôt
git clone https://github.com/polite0803/AxAgent.git
cd AxAgent

# Installer les dépendances
npm install

# Mode développement
npm run tauri dev

# Construire uniquement le frontend
npm run build

# Construire l'application bureau
npm run tauri build
```

Les artefacts de construction se trouvent dans `src-tauri/target/release/`.

### Tests

```bash
# Tests unitaires
npm run test          # Vitest watch
npm run test:run      # Vitest exécution unique

# Tests E2E
npm run test:e2e      # Playwright
npm run test:e2e:ui   # Playwright mode UI

# Tests du backend Rust
cd src-tauri && cargo nextest run   # cargo-nextest (2-3x plus rapide)
cd src-tauri && cargo test          # Tests standard

# Vérification de types
npm run typecheck     # TypeScript
cd src-tauri && cargo clippy -- -D warnings  # Rust

# Formatage du code
npm run format        # dprint
cd src-tauri && cargo fmt

# Vérification CI complète
npm run ci:check
```

---

## Structure du projet

```
AxInvest/
├── src/                         # Code source frontend (React + TypeScript)
│   ├── components/              # Composants React (25 modules)
│   │   ├── chat/               # Interface de conversation (100+ composants)
│   │   ├── stock-analysis/     # Analyse d'investissement intelligente (16 composants)
│   │   ├── workflow/           # Éditeur de workflows (16 types de nœuds + panneaux de propriétés + panneau IA)
│   │   ├── gateway/            # Composants passerelle API
│   │   ├── settings/           # Panneau de paramètres (50+ composants)
│   │   ├── terminal/           # Composants terminal
│   │   ├── skill/              # Éditeur et rendu de compétences
│   │   ├── benchmark/          # Benchmark
│   │   ├── files/              # Gestion de fichiers
│   │   ├── fine-tune/          # Fine-tuning LoRA
│   │   ├── link/               # Liens externes
│   │   ├── llm-wiki/           # LLM Wiki
│   │   ├── proactive/          # Suggestions proactives
│   │   ├── wiki/               # Gestion Wiki
│   │   ├── devtools/           # Outils développeur
│   │   ├── decomposition/      # Décomposition de compétences
│   │   ├── recommendation/     # Recommandation d'outils
│   │   ├── style/              # Style de code
│   │   ├── layout/             # Composants de mise en page
│   │   ├── help/               # Panneau d'aide
│   │   ├── notification/       # Centre de notifications
│   │   ├── search/             # Recherche de sessions
│   │   ├── onboarding/         # Assistant de configuration
│   │   ├── common/             # Composants communs
│   │   └── shared/             # Composants partagés
│   ├── pages/                   # Composants de page (22 pages)
│   ├── stores/                  # Gestion d'état Zustand (65 stores)
│   │   ├── domain/            # État métier principal (9)
│   │   ├── feature/           # État des modules fonctionnels (46)
│   │   ├── devtools/          # État des outils développeur (5)
│   │   └── shared/            # État partagé (5)
│   ├── hooks/                   # React hooks (12)
│   ├── lib/                     # Fonctions utilitaires (33 modules + Web Worker)
│   ├── types/                   # Définitions de types TypeScript (22)
│   ├── sdk/                     # SDK (TypeScript + Python)
│   └── i18n/                    # Traductions en 11 langues
│
├── src-tauri/                    # Code source backend (Rust)
│   ├── crates/                  # Workspace Rust (20 crates)
│   │   ├── agent/             # Noyau des agents IA (70+ fichiers sources)
│   │   ├── astock-data/       # Sources de données du marché A (9 sources, 22 routes, indicateurs techniques, calendrier de trading)
│   │   ├── core/              # Utilitaires principaux (85+ entités, 40+ dépôts, RAG, chiffrement, MCP)
│   │   ├── gateway/           # Passerelle API (incluant endpoints API boursière)
│   │   ├── migration/         # Migrations de base de données (5 migrations)
│   │   ├── npm/               # Analyse de paquets npm
│   │   ├── plugins/           # Système de plugins
│   │   ├── prompt-guard/      # Protection contre l'injection de prompts
│   │   ├── providers/         # Adaptateurs de fournisseurs de modèles
│   │   ├── rt-dashboard/      # Plugins de tableau de bord
│   │   ├── rt-messaging/      # Passerelle de messagerie (9 plateformes)
│   │   ├── rt-theme/          # Moteur de thèmes
│   │   ├── rt-webhook/        # Serveur Webhook
│   │   ├── rt-workflow/       # Moteur de workflows (16 exécuteurs de nœuds)
│   │   ├── runtime/           # Services d'exécution (70+ fichiers sources)
│   │   ├── runtime-core/      # Couche d'abstraction d'exécution
│   │   ├── stock-analysis/    # Analyse d'investissement intelligente (23 sous-modules)
│   │   ├── telemetry/         # Traçage et métriques
│   │   ├── tools/             # Système d'outils (40+ outils intégrés)
│   │   └── trajectory/        # Système d'apprentissage (55+ fichiers sources)
│   └── src/                    # Point d'entrée Tauri (91 modules de commandes)
│       ├── commands/          # Modules de commandes
│       │   ├── stock_analysis.rs        # Commandes d'analyse boursière
│       │   ├── stock_analysis_setup.rs  # Configuration analyse boursière
│       │   ├── stock_workflow.rs        # Commandes workflow boursier
│       │   ├── agency_expert.rs         # Agent expert
│       │   ├── agent_advanced.rs        # Agent avancé
│       │   ├── agent_analytics.rs       # Analytique agent
│       │   ├── agent_insight.rs         # Insight agent
│       │   ├── agent_nudge.rs           # Prompt agent
│       │   ├── agent_profile.rs         # Profil agent
│       │   ├── agent_role.rs            # Rôle agent
│       │   ├── background_tasks.rs      # Tâches en arrière-plan
│       │   ├── browser.rs              # Automatisation de navigateur
│       │   ├── chart_generator.rs       # Génération de graphiques
│       │   ├── cloud_workspace.rs       # Espace de travail cloud
│       │   ├── computer_control.rs      # Contrôle informatique
│       │   ├── context_breakdown.rs     # Décomposition de contexte
│       │   ├── conversation_categories.rs  # Catégories de conversation
│       │   ├── conversations_search.rs  # Recherche de conversations
│       │   ├── crash_report.rs          # Rapport de crash
│       │   ├── dream.rs                # Consolidation onirique
│       │   ├── evolution.rs            # Évolution de compétences
│       │   ├── fine_tune.rs            # Fine-tuning LoRA
│       │   ├── gateway.rs              # Passerelle API
│       │   ├── gateway_link.rs         # Liens externes
│       │   ├── generated_tool.rs        # Outils générés
│       │   ├── image_gen.rs            # Génération d'images
│       │   ├── knowledge.rs            # Base de connaissances
│       │   ├── llm_wiki.rs             # LLM Wiki
│       │   ├── local_models.rs         # Modèles locaux
│       │   ├── mcp.rs                  # Protocole MCP
│       │   ├── memory.rs              # Système de mémoire
│       │   ├── message_continuation.rs  # Suite de messages
│       │   ├── onboarding.rs           # Assistant de configuration
│       │   ├── parallel_execution.rs    # Exécution parallèle
│       │   ├── plan.rs                 # Gestion de plans
│       │   ├── platform_integration.rs  # Intégration de plateformes
│       │   ├── plugin.rs               # Gestion de plugins
│       │   ├── proactive.rs            # Suggestions proactives
│       │   ├── prompt_templates.rs      # Modèles de prompts
│       │   ├── providers.rs            # Fournisseurs de modèles
│       │   ├── quickbar.rs             # QuickBar
│       │   ├── reflection.rs           # Réflexion
│       │   ├── research.rs             # Recherche approfondie
│       │   ├── rl.rs                   # Apprentissage par renforcement
│       │   ├── sandbox.rs              # Sandbox
│       │   ├── scheduled_task.rs        # Tâches planifiées
│       │   ├── screen_vision.rs        # Vision écran
│       │   ├── search.rs               # Recherche
│       │   ├── session_share.rs         # Partage de session
│       │   ├── shell.rs                # Shell
│       │   ├── skill_decomposition.rs   # Décomposition de compétences
│       │   ├── skills_hub.rs           # Centre de compétences
│       │   ├── tool_recommender.rs      # Recommandation d'outils
│       │   ├── tracer.rs               # Traçage
│       │   ├── user_profile.rs          # Profil utilisateur
│       │   ├── webdav.rs               # WebDAV
│       │   ├── webhook.rs              # Webhook
│       │   ├── wiki.rs                 # Wiki
│       │   ├── work_engine.rs          # Moteur de travail
│       │   ├── workflow_ai.rs          # Workflow IA
│       │   ├── workflow_template.rs     # Modèles de workflows
│       │   └── ...                     # Autres commandes
│       ├── init/              # Modules d'initialisation
│       ├── stock_scheduler.rs # Ordonnanceur boursier
│       └── ...                # Autres modules principaux
│
├── extension/                  # Extension de navigateur (Wiki Clipper : popup/content/background)
├── e2e/                        # Tests E2E Playwright (9 suites de tests)
├── scripts/                    # Scripts de build et d'outils
└── website/                    # Site du projet (VitePress, documentation en 11 langues)
```

## Répertoire de données

```
~/.axinvest/                     # Répertoire de configuration
├── axinvest.db                  # Base de données SQLite
├── master.key                   # Clé maîtresse AES-256
├── vector_db/                   # Base de données vectorielle (sqlite-vec)
└── ssl/                         # Certificats SSL

~/Documents/axinvest/           # Répertoire des fichiers utilisateur
├── images/                     # Pièces jointes images
├── files/                      # Pièces jointes fichiers
└── backups/                    # Fichiers de sauvegarde
```

---

## FAQ

### macOS : Message « L'application est endommagée » ou « Impossible de vérifier le développeur »

L'application n'étant pas signée par Apple :

**1. Autoriser les applications de « Toute source »**
```bash
sudo spctl --master-disable
```

Puis aller dans **Paramètres système → Confidentialité et sécurité → Sécurité**, sélectionner **Toute source**.

**2. Supprimer l'attribut de quarantaine**
```bash
sudo xattr -dr com.apple.quarantine /Applications/AxInvest.app
```

**3. Étape supplémentaire pour macOS Ventura+**
Aller dans **Paramètres système → Confidentialité et sécurité**, cliquer sur **Ouvrir quand même**.

---

## Communauté

- [LinuxDO](https://linux.do)

## Licence

Ce projet est sous licence [AGPL-3.0](LICENSE).
