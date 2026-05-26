[**English**](./README-EN.md) | [简体中文](./README.md) | [繁體中文](./README-ZH-TW.md) | [日本語](./README-JA.md) | [한국어](./README-KO.md) | [Français](./README-FR.md) | [Deutsch](./README-DE.md) | **Español** | [Русский](./README-RU.md) | [हिन्दी](./README-HI.md) | [العربية](./README-AR.md)

[![AxInvest](https://github.com/polite0803/AxAgent/blob/main/src/assets/image/logo.png?raw=true)](https://github.com/polite0803/AxAgent)

<p align="center">
  <a href="https://www.producthunt.com/products/axagent?embed=true&amp;amp;utm_source=badge-featured&amp;amp;&amp;#10;&amp;amp;amp;;utm_medium=badge&amp;amp;#10&amp;amp;;utm_campaign=badge-axagent" target="_blank" rel="noopener noreferrer"><img alt="AxInvest - Plataforma de análisis de inversión inteligente impulsada por IA | Product Hunt" width="250" height="54" src="https://api.producthunt.com/widgets/embed-image/v1/featured.svg?post_id=1118403&amp;theme=light&amp;t=1775627359538"></a>
</p>

<p align="center">
  <strong>Análisis de inversión inteligente impulsado por IA | Colaboración multi-agente | Local primero</strong>
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

## ¿Qué es AxInvest?

**AxInvest v2.3** es una plataforma de análisis de inversión inteligente impulsada por IA, construida sobre el framework multi-agente AxAgent. Integra capacidades avanzadas de agentes IA con análisis de inversión profesional de acciones A, soportando múltiples proveedores de modelos, investigación con agentes IA, orquestación visual de flujos de trabajo, gestión local de conocimientos, pasarela API integrada, cubriendo **Windows / macOS / Linux / Android / iOS** cinco plataformas, con diseño adaptativo para dispositivos de **escritorio, tablet y teléfono**.

La característica principal de AxInvest radica en aprovechar mecanismos como debate adversarial multi-agente, investigación profunda y verificación de hechos para proporcionar un análisis completo y objetivo que respalde las decisiones de inversión.

---

## Capturas de pantalla

| Conversación y selección de modelo | Panel multi-agente |
|:---:|:---:|
| ![](.github/images/s1-0412.png) | ![](.github/images/s5-0412.png) |

| Base de conocimientos RAG | Memoria y contexto |
|:---:|:---:|
| ![](.github/images/s3-0412.png) | ![](.github/images/s4-0412.png) |

| Editor de flujo de trabajo | Pasarela API |
|:---:|:---:|
| ![](.github/images/s9-0412.png) | ![](.github/images/s10-0412.png) |

---

## Funcionalidades principales

### 📈 Análisis de inversión inteligente

El módulo central de AxInvest, que integra capacidades de agentes IA con análisis de inversión profesional:

**Agregación multi-fuente y degradación**

- **9 fuentes de datos** — Tencent Finance, Tongdaxin (mootdx), Eastmoney, Sina Finance, Baidu Stocks, THS (Tonghuashun), Iwencai, Cninfo, AKShare
- **22 rutas de datos** — Cada tipo de datos configura rutas de degradación multi-fuente; cuando la fuente principal no está disponible, se conmuta automáticamente a la fuente de respaldo
- **Recopilación concurrente de datos** — `tokio::join!` recopilación concurrente de 16 tipos de datos de acciones individuales + 5 tipos de datos de mercado, maximizando la eficiencia de recopilación
- **Caché inteligente** — Caché en memoria LRU (límite de 1000 entradas), cotizaciones TTL 30s / K-line TTL 300s, expiración y eliminación automáticas
- **Verificación de salud** — Sonda de conectividad de proveedores (Banco Ping An 000001 como sonda), soporte para detección en tiempo de ejecución de disponibilidad de fuentes de datos

**Identificación y reglas del mercado de acciones A**

- **Identificación de placas** — Identificación automática por prefijo de código: Placa principal de Shanghái (6), Placa STAR (688), Placa principal de Shenzhen (0), ChiNext (3), BSE (8)
- **Reglas de límite de subida/bajada** — Placa STAR/ChiNext ±20%, BSE ±30%, placa principal ±10%, acciones ST ±5%
- **Calendario de trading** — Calendario integrado de feriados y días laborables ajustados de acciones A para 2025-2026, soporte para determinación de días de trading

**Datos de acciones individuales (16 tipos)**

- **Cotizaciones en tiempo real** — Precio, cambio porcentual, volumen/monto, tasa de rotación, PE/PB, capitalización de mercado total, precio límite de subida/bajada, identificación ST
- **Datos K-line** — 7 periodos (5 min/15 min/30 min/60 min/diario/semanal/mensual), incluyendo volumen, monto y tasa de rotación
- **Análisis financiero** — Ingresos, beneficio neto, EPS, BPS, ROE, ratio de endeudamiento, margen bruto, margen neto, crecimiento interanual de ingresos, crecimiento interanual de beneficios
- **Flujo de fondos** — Flujo neto de capital principal/super grande/grande/mediano/pequeño
- **Lista de dragón y tigre** — Montos de compra/venta de sucursales, monto neto, motivo de inclusión
- **Desbloqueo de acciones restringidas** — Fecha de desbloqueo, número de acciones desbloqueadas, proporción desbloqueada, información de accionistas
- **Financiamiento y préstamo de valores** — Monto de compra/saldo de financiamiento, volumen de venta/existencia de préstamo de valores
- **Fondos norteños** — Cantidad de acciones en cartera, proporción de tenencia, variación de cantidad
- **Clasificación sectorial** — Sectores Shenwan de primer/segundo nivel, etiquetas de placas conceptuales
- **Incremento/reducción de accionistas** — Dinámica de incrementos y reducciones de accionistas importantes, motivos de incremento/reducción
- **Registros de dividendos** — Fecha de ex-dividendo/ex-derechos, dividendo por acción, proporción de bonificación/transferencia, fecha de registro
- **Agregación de informes de investigación** — Informes de investigación de corretaje, incluyendo institución, analista, calificación, precio objetivo, predicción de EPS
- **EPS de consenso** — EPS de consenso institucional, precio objetivo de consenso, calificación promedio, número de calificaciones
- **Placas conceptuales** — Pertenencia tridimensional (sector/concepto/región), incluyendo cambio porcentual de placas
- **Búsqueda de anuncios** — Anuncios de empresas cotizadas de Cninfo, incluyendo tipo de anuncio y enlace PDF
- **Sentimiento de noticias** — Título/resumen/fuente de noticias, incluyendo puntuación de sentimiento

**Datos de mercado (5 tipos)**

- **Lista de dragón y tigre de todo el mercado** — Todas las acciones incluidas en el día, incluyendo compra neta, montos de compra/venta
- **Acciones populares** — Acciones fuertes de THS, incluyendo cambio porcentual, tasa de rotación, etiquetas de motivo, placas pertenecientes
- **Ranking sectorial** — Cambio porcentual de sectores Shenwan, monto de transacción, acciones líderes al alza
- **Flashes de Cailianshe** — Flashes financieros en tiempo real, incluyendo título, contenido, fuente
- **Flujo de fondos norteños** — Flujo de fondos minuto a minuto de Shanghái/Shenzhen/Total

**Cálculo de indicadores técnicos (módulo indicators)**

- **Sistema de medias móviles** — MA5/MA10/MA20/MA60, incluyendo determinación de estado de alineación (alcista/bajista/alcista débil/cruce entrelazado)
- **MACD** — DIF/DEA/Histograma, incluyendo señal de determinación (cruce dorado/cruce muerto/ejecución alcista/ejecución bajista)
- **RSI** — RSI6/RSI12/RSI24, incluyendo señal de determinación (sobrecompra/sobreventa/fuerte/débil/neutro)
- **Bandas de Bollinger** — Banda superior/banda media/banda inferior (20,2), incluyendo determinación de posición (sobre banda superior/rango de banda superior/cerca de banda media/rango de banda inferior/bajo banda inferior)
- **Tasa de desviación** — Tasa de desviación MA5, tasa de desviación MA20
- **Análisis de volumen** — Ratio de volumen (volumen del día/volumen promedio de 5 días), incluyendo señal de determinación (alza con volumen/baja con volumen reducido/baja con volumen/alza con volumen reducido/normal)
- **Soporte/Resistencia** — Cálculo automático basado en máximos y mínimos recientes y medias móviles

**Registro de herramientas MCP (módulo mcp_tools)**

- Las capacidades de datos de acciones se registran como herramientas estándar a través del protocolo MCP, los agentes IA pueden invocarlas directamente en la conversación
- Herramientas registradas: search_stock, get_stock_quote, get_stock_kline, get_stock_financials, get_stock_news, get_stock_money_flow, get_stock_dragon_tiger, etc.

**Pipeline de análisis IA (crate stock-analysis, 23 submódulos)**

- **Orquestación de análisis** — orchestrator (orquestación de pipeline), pipeline (pipeline multi-etapa), runner (ejecutor de tareas)
- **Motor de decisiones** — decision (decisión de inversión), signals (generación de señales de trading), rules (motor de reglas de trading)
- **Evaluación de riesgos** — risk (modelo de evaluación de riesgos), portfolio_risk (riesgo de cartera), position_limits (límites de posición y cumplimiento)
- **Selección de acciones y backtesting** — screener (selector multi-criterio), backtest (motor de backtesting de estrategias), trading (framework de estrategias de trading)
- **Inversión de valor** — value (análisis de valor), value_investing (marco de evaluación de inversión de valor)
- **Control de calidad** — quality (verificación de calidad de datos), data_clean (limpieza y preprocesamiento de datos), review (revisión de resultados de análisis)
- **Informes y puntuación** — report (generación de informes de análisis), scoring (sistema de puntuación integral)
- **Módulos auxiliares** — key_levels (identificación de niveles clave de precio), monitor (monitoreo y alertas en tiempo real), plugin (extensiones de plugins de análisis), prompts (plantillas de prompts IA)

**Componentes de análisis frontend (16)**

- StockAnalysisPage, StockQuoteCard, KLineChart, RiskMatrix, TradePanel
- DecisionBanner, DebatePanel, WatchlistPanel, PriceAlertPanel, CompareView
- AnalystReportGrid, AnalystReportCard, HistoricalAnalysisPanel, StockSearchBar
- AnalysisProgress, StockAnalysisSettingsModal, StockAnalysisChatIndicator

**Debate adversarial y decisión**

- **Debate adversarial** — Debate Pro/Con multi-agente, soporte para puntuación de fuerza de argumentos y seguimiento de refutaciones
- **Banner de decisión** — Visualización de decisión comprar/vender/mantener, con confianza y razones
- **Integración con flujo de trabajo IA** — Flujo de trabajo de análisis de acciones integrado con conversaciones (stockWorkflowChatBridge)

### 🤖 Soporte de modelos IA

- **Soporte multi-proveedor** — Integración nativa de OpenAI, Anthropic Claude, Google Gemini, Ollama, OpenClaw, Hermes y todas las APIs compatibles con OpenAI
- **Rotación multi-clave** — Configure múltiples claves API por proveedor con rotación automática para distribuir la limitación de tasa
- **Inferencia de modelos locales** — Soporte completo para modelos locales Ollama, incluyendo gestión de archivos GGUF/GGML
- **Motor de inferencia Candle** — Inferencia local Candle integrada, soporte para interfaces rerank/judge, descarga GGUF bajo demanda
- **Gestión de modelos** — Obtención de listas de modelos remotos, personalización de parámetros (temperatura, tokens máximos, top-p, etc.)
- **Salida en streaming** — Renderizado en tiempo real token a token, soporte para bloques de pensamiento plegables (pensamiento extendido de Claude)
- **Comparación multi-modelo** — Pregunte a múltiples modelos simultáneamente con comparación lado a lado
- **Llamadas de funciones** — Llamadas de funciones estructuradas en todos los proveedores soportados
- **API Responses de OpenAI** — Soporte para transporte en formato OpenAI Responses
- **API Realtime** — Push de eventos WebSocket compatible con la API Realtime de OpenAI
- **Generación de imágenes** — Panel de generación de imágenes IA, soporte para múltiples modelos y configuración de parámetros

### 🔐 Sistema de agentes IA

El sistema de agentes está construido sobre una arquitectura sofisticada (crate agent, 70+ archivos fuente), con las siguientes características:

- **Motor de razonamiento ReAct** — Fusión de razonamiento y acción, con autoverificación integrada para ejecución fiable de tareas
- **Planificador jerárquico** — Descomposición de tareas complejas en planes estructurados con fases y dependencias
- **Descomponedor de tareas** — Descomposición automática de tareas complejas en subtareas ejecutables
- **Cadena de pensamiento** — Visualización del razonamiento decisional del agente, descomposición paso a paso
- **Árbol de pensamiento** — tree_of_thoughts exploración de razonamiento multi-ruta
- **Investigación profunda** — Orquestación de búsqueda multi-fuente, seguimiento de citas y evaluación de credibilidad
- **Verificación de hechos** — Verificación de hechos impulsada por IA y clasificación de fuentes
- **Orquestación de búsqueda** — Coordinación de múltiples proveedores de búsqueda, con planificación y síntesis de resultados
- **Búsqueda académica** — Búsqueda de literatura académica y análisis de citas
- **Control informático** — Clics de ratón, entrada de teclado, desplazamiento de pantalla controlados por IA, con análisis de modelo visual
- **Percepción de pantalla** — Captura de pantalla y análisis por modelo visual para identificación de elementos UI
- **Pipeline visual** — vision_pipeline comprensión y análisis de imágenes
- **Tres niveles de permisos** — Predeterminado (aprobación requerida), Aceptar ediciones (aprobación automática), Acceso completo (sin indicaciones)
- **Aislamiento sandbox** — Las operaciones del agente están estrictamente limitadas al directorio de trabajo especificado
- **Panel de aprobación de herramientas** — Visualización en tiempo real de solicitudes de llamadas a herramientas con aprobación individual
- **Seguimiento de costos** — Visualización en tiempo real del uso de tokens y estadísticas de costos por sesión
- **Pausa/Reanudación** — Suspenda la ejecución del agente en cualquier momento y reanude más tarde
- **Sistema de puntos de control** — Puntos de control persistentes para recuperación tras fallos y reconexión de sesiones
- **Motor de recuperación de errores** — Clasificación automática de errores, análisis de causas raíz y ejecución de estrategias de recuperación
- **Detección de bucles** — Detección e interrupción automáticas de comportamientos de bucle en el razonamiento del agente
- **Modo proactivo** — El agente puede ofrecer sugerencias y ejecutar acciones proactivamente
- **Gestión de propósitos** — Mantenimiento y seguimiento de los propósitos de ejecución y contexto del agente
- **Autoverificación** — self_verifier verificación automática de la corrección de la salida del agente
- **Reflexión** — reflector reflexión y mejora del proceso de razonamiento
- **Entrada de dirección** — steer_manager ajuste dinámico de la dirección de comportamiento del agente
- **Bus de eventos** — event_bus / event_emitter arquitectura basada en eventos del agente
- **Síntesis de contenido** — content_synthesizer síntesis de información multi-fuente y generación de informes
- **Seguimiento de citas** — citation_tracker seguimiento y anotación automática de fuentes de información
- **Evaluación de credibilidad** — credibility_evaluator evaluación de la credibilidad de las fuentes de información
- **Construcción de esquemas** — outline_builder construcción automática de esquemas de investigación
- **Gestión de esquemas** — schema_manager gestión de esquemas de estructura de salida
- **Memoria de proyecto** — project_memory memoria persistente a nivel de proyecto
- **Detección de entorno** — environment_probe detección automática de información del entorno de ejecución
- **Verificación de salud** — health_checker monitoreo del estado de salud del agente

### 👥 Colaboración multi-agente

- **Coordinación de sub-agentes** — Arquitectura maestro-esclavo, coordinator coordina múltiples agentes colaborativos
- **Ejecución paralela** — Procesamiento paralelo por múltiples agentes con planificación consciente de dependencias
- **Debate adversarial** — adversarial_debate rondas de debate Pro/Con, soporte para puntuación de fuerza de argumentos y seguimiento de refutaciones
- **Roles de agentes** — agent_roles roles predefinidos (investigador, planificador, desarrollador, revisor, sintetizador) para colaboración en equipo
- **Orquestador de agentes** — Enrutamiento centralizado de mensajes y gestión de estado para equipos multi-agente
- **Grafo de comunicación** — graph_insights visualización de interacciones y flujos de mensajes entre agentes
- **Pizarra compartida** — shared_blackboard / blackboard espacio de estado compartido entre agentes
- **Sistema Buddy** — Agentes compañeros configurables con definición de especies y atributos
- **Memoria compartida** — Espacio de memoria compartido entre agentes con estadísticas y consultas
- **Cron de equipo** — Planificación de tareas cron a nivel de equipo
- **Sistema de expertos** — agency_expert agente experto de dominio
- **Perfil de agente** — agent_profile gestión de perfil de personalidad y capacidades del agente

### ⭐ Sistema de habilidades

- **Mercado de habilidades** — Mercado integrado para explorar e instalar habilidades contribuidas por la comunidad
- **Creación de habilidades** — Creación automática de habilidades a partir de propuestas, con editor Markdown
- **Evolución de habilidades** — skill_evolution análisis y mejora automáticos impulsados por IA de habilidades existentes basados en retroalimentación de ejecución
- **Coincidencia de habilidades** — skill_matcher coincidencia semántica, recomendación de habilidades relevantes al contexto de conversación
- **Descomposición de habilidades** — Descomposición automática de tareas complejas en habilidades atómicas ejecutables (asistida por LLM/multi-ronda/validación por flujo de trabajo)
- **Herramientas generadas** — Generación y registro automáticos por IA de nuevas herramientas para expandir las capacidades del agente
- **Hub de habilidades** — skills_hub_adapter interfaz centralizada de descubrimiento y gestión de configuración de habilidades
- **Cliente del hub de habilidades** — skills_hub_client integración con hub de habilidades remoto, con compartir comunitario
- **Verificación de dependencias de habilidades** — Detección automática de dependencias de habilidades y disponibilidad de herramientas
- **Contenedor sandbox de habilidades** — Ejecución segura de habilidades en un entorno aislado
- **Habilidad atómica** — atomic_skill unidad de habilidad ejecutable mínima
- **Propuesta de habilidad** — skill_proposal propuesta de creación de habilidad impulsada por IA

### 🔄 Sistema de flujo de trabajo

El motor de flujo de trabajo (crate rt-workflow) implementa un sistema de orquestación de tareas basado en DAG:

- **Editor de flujo de trabajo visual** — Diseñador de flujos de trabajo por arrastrar y soltar con conexión y configuración de nodos
- **16 tipos de nodos** — Disparador, agente, LLM, condición, paralelo, bucle, fusión, retraso, herramienta, código, sub-flujo de trabajo, búsqueda vectorial, análisis de documento, validación, fin, fallback
- **16 paneles de propiedades** — Cada tipo de nodo con panel de configuración independiente
- **Plantillas de flujo de trabajo** — Preajustes integrados: revisión de código, corrección de bugs, documentación, pruebas, refactoring, exploración, rendimiento, seguridad, desarrollo de funcionalidades
- **Ejecución DAG** — Ordenamiento topológico por algoritmo de Kahn, con detección de ciclos
- **Planificación paralela** — Ejecución en pipeline, los pasos rápidos no esperan a los lentos
- **Estrategia de reintento** — Backoff exponencial, número máximo de reintentos configurable por paso
- **Completado parcial** — Los pasos fallidos no bloquean los pasos descendentes independientes
- **Gestión de versiones** — Control de versiones de plantillas de flujo de trabajo con rollback
- **Historial de ejecución** — Registro detallado con seguimiento de estado y depuración
- **Asistencia IA** — Diseño de flujo de trabajo asistido por IA, recomendación de nodos y optimización de prompts de agentes
- **Verificación semántica** — Validación semántica de flujos de trabajo, detección de problemas potenciales
- **Importación n8n** — Soporte para importación de flujos de trabajo desde directorio n8n
- **Panel de depuración** — Depuración en tiempo real y visualización de estado durante la ejecución del flujo de trabajo
- **Capa de caché** — cache_layer caché de resultados de ejecución de flujo de trabajo
- **Mercado** — workflow_marketplace mercado y revisión de plantillas de flujo de trabajo

### 📚 Conocimiento y memoria

- **Base de conocimientos (RAG)** — Soporte multi-base de conocimientos, carga de documentos, análisis automático, fragmentación e indexación vectorial
- **Búsqueda híbrida** — Combinación de búsqueda por similitud vectorial y ranking BM25 de texto completo
- **Reranking** — Reranking por cross-encoder para mejorar la precisión de recuperación
- **Pipeline de recall de tres niveles** — Mecanismo de recall multinivel con índice AST + búsqueda vectorial + FTS5
- **Self-RAG** — self_rag generación aumentada por recuperación adaptativa
- **Mejora de consultas** — query_enhancement reescritura y expansión de consultas
- **Grafo de conocimientos** — Visualización de relaciones entidad-conocimiento (entidades, atributos, relaciones, flujos, interfaces)
- **Sistema Wiki** — Compilador y validador LLM Wiki, con visualización de grafo de conocimientos y sincronización incremental
- **Notas Wiki** — Sistema de notas con enlaces bidireccionales, vista de grafo y sincronización automática de enlaces
- **Sistema de memoria** — Memoria multi-espacio de nombres, con entrada manual o extracción automática por IA
- **Memoria de bucle cerrado** — Integración de proveedores de memoria persistente Honcho y Mem0
- **Olvido de memoria** — memory_forgetting mecanismo de decaimiento de memoria basado en tiempo
- **Búsqueda de texto completo FTS5** — Búsqueda rápida en conversaciones, archivos y memorias
- **Búsqueda de sesiones** — Búsqueda avanzada en todas las sesiones de conversación
- **Gestión de contexto** — Adjuntar de forma flexible archivos, resultados de búsqueda, pasajes de conocimientos, memorias, salidas de herramientas
- **Parser de documentos** — Análisis automático y extracción de contenido de documentos multi-formato
- **Indexación incremental** — Actualización incremental del índice ante cambios de archivos
- **Fragmentación de texto** — text_chunker estrategia inteligente de fragmentación de texto
- **Presupuesto de tokens** — token_budget control de presupuesto de tokens para resultados de recuperación

### 🌐 Pasarela API

- **Servidor API local** — Servidor integrado compatible con OpenAI, Claude y Gemini
- **Enlaces externos** — Integración en un clic con Claude CLI, OpenCode, sincronización automática de claves API y modelos
- **Gestión de claves** — Generación, revocación, activación/desactivación de claves de acceso con descripciones
- **Análisis de uso** — Volumen de solicitudes y uso de tokens por clave, proveedor y fecha
- **Soporte SSL/TLS** — Certificados autofirmados integrados, soporte para certificados personalizados
- **Registros de solicitudes** — Registro completo de todas las solicitudes y respuestas API
- **Plantillas de configuración** — Plantillas preconstruidas para Claude, Codex, OpenCode, Gemini
- **API Realtime** — Push de eventos WebSocket compatible con la API Realtime de OpenAI
- **Integración de plataforma** — Soporte para DingTalk, Feishu, QQ, Slack, WeChat, WhatsApp, Telegram, Discord
- **Diagnóstico de pasarela** — Diagnóstico de conexión y gestión de políticas de programa
- **Limitador de tasa** — Limitación de tasa de solicitudes API y control de flujo
- **Cola persistente** — Gestión de cola de solicitudes persistente
- **API de acciones** — stock_handlers endpoints API dedicados para datos de acciones
- **Push SSE** — sse Server-Sent Events push de eventos en tiempo real

### 🔧 Herramientas y extensiones

- **Protocolo MCP** — Implementación completa del Model Context Protocol, soporte de transportes stdio y HTTP/WebSocket
- **Autenticación OAuth** — Soporte de flujo OAuth para servidores MCP
- **Inicio automático MCP** — Inicio automático y gestión del ciclo de vida de servidores MCP
- **Puente de herramientas MCP** — Puente entre herramientas MCP y el sistema de herramientas del agente
- **Verificación de salud MCP** — mcp_health monitoreo del estado de salud de servidores MCP
- **Sistema de plugins** — Arquitectura de plugins de tres niveles compatible con OpenClaw (integrado/empaquetado/externo), soporte para instalación de paquetes npm, registro de herramientas, hooks y gestión del ciclo de vida
- **Mercado de plugins** — UI de mercado integrada, soporte para búsqueda e instalación npm, diálogos de confirmación
- **Herramientas integradas** — 40+ módulos de herramientas: operaciones de archivos (lectura/escritura/edición/sistema), ejecución de código, búsqueda (Grep/Glob), Bash, búsqueda/extracción web, gestión de planes, planificación Cron, REPL, LSP, gestión de contexto, control informático, envío de mensajes, lista de tareas, base de datos, DevOps, análisis de documentos, Git, recuperación de conocimientos, LSP, procesamiento multimedia, envío de mensajes, OCR, notificaciones push, información del sistema, sistema de tareas, pruebas, workspace/worktree, etc.
- **Sistema de permisos de herramientas** — Clasificación de permisos de herramientas, gestión de reglas y seguimiento de uso
- **Seguridad Bash** — Análisis de comandos, validación de rutas y control de seguridad sandbox
- **Cliente LSP** — Protocolo Language Server integrado, completación de código y diagnósticos
- **Índice AST** — Análisis e indexación AST de archivos de código
- **Backend de terminal** — Soporte para conexiones de terminal locales, Docker y SSH
- **Automatización de navegador** — Control de navegador vía integración CDP (navegación, capturas, clics, relleno, extracción de texto, etc.)
- **Automatización UI** — Identificación y control de elementos UI multiplataforma
- **Herramientas Git** — Operaciones Git con detección de ramas y sensibilidad a conflictos
- **Recomendación de herramientas** — Motor de recomendación inteligente de herramientas basado en contexto
- **Orquestación de herramientas** — Coordinación y ejecución multi-herramienta con salida en streaming
- **Estadísticas de herramientas** — Estadísticas de frecuencia de uso y rendimiento de herramientas
- **Auditoría de herramientas** — audit registro de auditoría de llamadas a herramientas

### 📊 Renderizado de contenido

- **Renderizado Markdown** — Soporte completo para resaltado de código, fórmulas matemáticas LaTeX, tablas, listas de tareas
- **Editor de código Monaco** — Editor integrado con resaltado de sintaxis, copia, vista previa diff
- **Renderizado de diagramas** — Diagramas de flujo Mermaid, diagramas de arquitectura D2, gráficos interactivos ECharts
- **Panel de artefactos** — Fragmentos de código, borradores HTML, componentes React, notas Markdown, con vista previa en tiempo real
- **Cuatro modos de vista previa** — Código (editor), Dividido (lado a lado), Vista previa (solo renderizado), Vista previa de componente React
- **Inspector de sesión** — Vista de árbol de la estructura de sesión, navegación rápida
- **Panel de citas** — Seguimiento y visualización de citas fuente con puntuación de credibilidad
- **Renderizado de infografías** — Soporte para visualización de infografías
- **Intérprete de gráficos** — ChartInterpreter interpretación de gráficos impulsada por IA
- **Visor de diff** — DiffViewer comparación de diferencias de código

### 🛡️ Datos y seguridad

- **Cifrado AES-256** — Claves API y datos sensibles cifrados con AES-256-GCM
- **Almacenamiento aislado** — Estado de la aplicación en `~/.axinvest/`, archivos de usuario en `~/Documents/axinvest/`
- **Copia de seguridad automática** — Copias de seguridad programadas a directorio local o almacenamiento WebDAV
- **Copia de seguridad S3** — s3_backup soporte para copia de seguridad en la nube Amazon S3
- **Restauración de copia de seguridad** — Restauración en un clic desde copias de seguridad históricas
- **Opciones de exportación** — Capturas PNG, Markdown, texto plano, JSON
- **Gestión de almacenamiento** — Visualización del uso del disco y herramientas de limpieza
- **Migración de almacenamiento** — storage_migration migración de datos entre versiones
- **Autorización de archivos** — Gestión de autorización y revocación de acceso a archivos
- **Auditoría de operaciones** — Registro de auditoría de operaciones críticas
- **Validación de comandos** — command_validator validación de seguridad de comandos
- **Límites de recursos** — resource_limits límites de uso de recursos
- **Ejecución sandbox** — sandbox_runner ejecución en entorno aislado

### 🖥️ Experiencia de escritorio

- **Motor de temas** — Temas oscuro/claro, seguimiento del sistema o preferencia manual
- **Idioma de interfaz** — 11 idiomas: chino simplificado, chino tradicional, inglés, japonés, coreano, francés, alemán, español, ruso, hindi, árabe
- **Bandeja del sistema** — Minimización a la bandeja sin interrumpir servicios en segundo plano
- **Ventana siempre visible** — Ventana fijada sobre todas las demás ventanas
- **Atajos globales** — Atajos de teclado globales personalizables para invocar la ventana principal
- **QuickBar** — Barra flotante de acceso rápido, invocación en un clic
- **Inicio automático** — Lanzamiento opcional al iniciar el sistema
- **Soporte de proxy** — Configuración de proxy HTTP y SOCKS5
- **Actualización automática** — Verificación automática de versiones, notificación de actualización
- **Paleta de comandos** — `Cmd/Ctrl+K` para acceso rápido a comandos
- **Asistente de incorporación** — Guía interactiva de primer uso y detección de Ollama
- **Centro de notificaciones** — Gestión unificada de notificaciones en la aplicación
- **Workspace en la nube** — cloud_workspace selección de workspace en la nube
- **Informe de fallos** — crash_report recopilación automática de informes de fallos
- **Llamada de voz** — VoiceCall capacidad de conversación por voz

### 🔬 Funcionalidades avanzadas

- **Investigación profunda** — Búsqueda multi-fuente, seguimiento de citas, evaluación de credibilidad y síntesis de contenido
- **Verificación de hechos** — Verificación de hechos impulsada por IA y clasificación de fuentes
- **Planificador Cron** — Planificación de tareas automatizada con plantillas diarias/semanales/mensuales y expresiones cron personalizadas
- **Sistema Webhook** — Suscripción a eventos, notificaciones de completado de herramientas, errores de agentes, fin de sesión
- **Perfil de usuario** — Aprendizaje automático de estilo de código, convenciones de nomenclatura, indentación, estilo de comentarios, preferencias de comunicación
- **Optimizador RL** — Optimización por aprendizaje por refuerzo de selección de herramientas y estrategias de tareas
- **Ajuste fino LoRA** — Adaptación de modelo personalizada con ajuste fino LoRA local
- **Sugerencias proactivas** — Indicaciones contextuales basadas en contenido de conversación y patrones de usuario
- **Predicción de contexto** — Predicción de las próximas acciones del usuario y precarga de recursos relevantes
- **Integración onírica** — dream_consolidation integración automática en segundo plano de memorias y patrones, optimización del conocimiento a largo plazo
- **Recuperación de errores** — Clasificación automática de errores, análisis de causas raíz y sugerencias de recuperación
- **Herramientas de desarrollo** — Trace, Span, visualización de timeline para depuración y análisis de rendimiento
- **Sistema de benchmark** — Evaluación de rendimiento SWE-bench / Terminal-bench con scorecards
- **Transferencia de estilo** — style_migrator aplicación de preferencias de estilo de código aprendidas al código generado
- **Plugins de panel** — Panel extensible con paneles y widgets personalizados
- **Colaboración y compartir** — Colaboración en tiempo real CRDT y compartir sesión en un clic
- **Extensión de navegador** — Extensión de navegador Wiki Clipper para recorte rápido de páginas web al Wiki LLM
- **SDK Python** — SDK Python para integración con AxInvest
- **Enrutador inteligente** — Enrutamiento y clasificación inteligentes de solicitudes
- **Caché semántico** — Caché de respuestas basado en semántica, reducción de cálculo redundante
- **Compresión de contexto** — Compresión automática de contextos largos, optimización del uso de tokens
- **Procesamiento por lotes de mensajes** — Envío y optimización por lotes de mensajes
- **Pool de conexiones** — Gestión del pool de conexiones de base de datos y API
- **Feature flags** — Sistema de feature flags configurable
- **Motor de políticas** — Gestión centralizada de políticas de permisos y operaciones
- **Gobernanza de recursos** — Limitación y gobernanza del uso de recursos por agentes
- **Transferencia LAN** — Capacidad de transferencia de archivos en red local
- **Coevolución** — coevolution coevolución de habilidades y agentes
- **Aprendizaje de comportamiento** — behavior_learner / behavior_tracker aprendizaje y seguimiento de comportamiento del usuario
- **Aprendizaje de preferencias** — preference_learner aprendizaje automático de preferencias del usuario
- **Recompensa intrínseca** — intrinsic_reward exploración impulsada por motivación intrínseca
- **Recompensa de proceso** — process_reward señal de recompensa a nivel de proceso
- **TextGrad** — text_grad optimización automática basada en gradientes de texto
- **Compresión de trayectoria** — trajectory_compressor compresión automática de trayectorias largas
- **Gestión de recordatorios** — reminder_manager programación inteligente de recordatorios
- **Prefetch de tareas** — task_prefetcher prefetch predictivo de recursos de tareas

### 🛡️ Protección contra inyección de prompts (Prompt-Guard)

- **Sistema de protección de cuatro niveles** — L1 Detección de patrones (intercepción de alto riesgo + marcado de riesgo medio) → L2 Escape de delimitadores → L3 Envoltorio XML → L4 Etiquetas de confianza
- **Orquestador de pipeline** — Pipeline de detección multinivel en serie, soporte para umbrales de riesgo personalizados
- **Detección de Token Smuggling** — Detección especializada contra ofuscación de codificación y ataques de contrabando de tokens
- **Detección de escape de delimitadores** — delimiter_escape detección de ataques de escape de delimitadores de prompts
- **Detección de patrones** — pattern_detect coincidencia de patrones de inyección por regex + heurística
- **Etiquetas de confianza** — trust_labels marcado y verificación de contenido confiable
- **Modo Strict** — Pruebas en modo estricto + nombramiento de razones de riesgo medio + documentación de modos personalizados
- **Integración de pipeline completo** — Integrado en sesión / prompt / git / RAG

### 📱 Soporte móvil

- **Android nativo** — Construcción APK/AAB, soporte para arm64-v8a / armeabi-v7a / x86_64
- **iOS nativo** — Construcción IPA, soporte para arm64
- **Diseño adaptativo** — Adaptación automática en tres niveles: escritorio/tableta/teléfono (hook useResponsive)
- **Navegación móvil** — Navegación deslizante Drawer + barra de navegación inferior + botón flotante flash
- **Adaptación de zona segura** — Adaptación CSS env() para barra de estado/navegación del sistema Android
- **Optimización CSP** — Lista blanca de protocolo CSP de Android WebView
- **Compilación condicional** — `#[cfg(not(mobile))]` exclusión automática de funciones exclusivas de escritorio (navegador, control informático, escritorio, QuickBar, terminal, visión de pantalla)

---

## Arquitectura técnica

### Pila de tecnología

| Capa | Tecnología |
|------|------------|
| **Framework** | Tauri 2 + React 19 + TypeScript 6 |
| **UI** | Ant Design 6 + TailwindCSS 4 |
| **Gestión de estado** | Zustand 5 |
| **Enrutamiento** | React Router 7 |
| **i18n** | i18next + react-i18next |
| **Backend** | Rust 2024 + SeaORM 2 + SQLite |
| **Base de datos vectorial** | sqlite-vec |
| **Editor de código** | Monaco Editor |
| **Diagramas** | Mermaid + D2 + ECharts (CDN) |
| **Terminal** | xterm.js 6 |
| **Flujo de trabajo** | ReactFlow 11 |
| **Renderizado de gráficos** | @antv/infographic |
| **Iconos** | Iconify + Lucide |
| **Arrastrar y soltar** | @dnd-kit |
| **Build** | Vite 8 + npm |
| **Testing** | Vitest + Playwright + cargo-nextest |
| **Formateo** | dprint (TS/JSON) + rustfmt |
| **Lint** | TS: eslint + oxlint / Rust: clippy + cargo-deny |
| **Móvil** | Tauri Android + iOS compilación nativa |
| **Escritorio** | Windows (MSI) · macOS (DMG) · Linux (AppImage/deb/rpm) |

### Soporte de plataformas

| Plataforma | Arquitectura |
|------------|-------------|
| Windows | x86_64, ARM64 |
| macOS | Apple Silicon (arm64), Intel (x86_64) |
| Linux | x86_64, ARM64 |
| Android | arm64-v8a, armeabi-v7a, x86_64 (emulador) |
| iOS | arm64 |

### Arquitectura Backend Rust

El backend está organizado como un workspace de Rust con **20** crates especializadas:

```
src-tauri/crates/
├── agent/            # Núcleo del agente IA (70+ archivos fuente: motor ReAct, coordinación, planificación, investigación profunda, verificación de hechos, etc.)
├── astock-data/      # Fuentes de datos de acciones A (9 fuentes de datos, 22 rutas de datos, indicadores técnicos, calendario de trading, registro de herramientas MCP)
├── core/             # Utilidades principales (85+ entidades de base de datos, 40+ repositorios, RAG, cifrado, MCP, automatización de navegador, índice AST, etc.)
├── gateway/          # Pasarela API (servidor HTTP, autenticación, rutas, interfaz compatible con OpenAI, endpoints API de acciones)
├── migration/        # Migraciones de base de datos (5 migraciones: análisis de acciones/lista de vigilancia+portfolio/programación de análisis/alertas de precio/trading)
├── npm/              # Análisis y registro de paquetes npm
├── plugins/          # Sistema de plugins (compatible con OpenClaw, instalación de paquetes npm, incluye plugin de ejemplo)
├── prompt-guard/     # Protección contra inyección de prompts (detección y defensa multinivel L1-L4, 4 detectores)
├── providers/        # Adaptadores de proveedores de modelos (OpenAI, Anthropic, Gemini, Ollama, OpenClaw, Hermes, generación de imágenes)
├── rt-dashboard/     # Sistema de plugins de panel
├── rt-messaging/     # Pasarela de mensajería (9 plataformas: DingTalk/Feishu/QQ/Slack/WeChat/WhatsApp/Telegram/Discord)
├── rt-theme/         # Motor de temas
├── rt-webhook/       # Servidor y despacho de Webhooks
├── rt-workflow/      # Motor de flujo de trabajo (orquestación DAG, 16 ejecutores de nodos, planificador, capa de caché)
├── runtime/          # Servicios de ejecución (70+ archivos fuente: gestión de sesiones, MCP, terminal, limitación de tasa, Webhooks, permisos, benchmarks, etc.)
├── runtime-core/     # Capa de abstracción de ejecución (tipos comunes, definiciones de trait, configuración, feature flags, ejecutor de permisos)
├── stock-analysis/   # Análisis de inversión inteligente (23 submódulos: pipeline, motor de decisiones, evaluación de riesgos, backtesting, selector de acciones, inversión de valor)
├── telemetry/        # Telemetría y rastreo distribuido (compatible con OpenTelemetry)
├── tools/            # Sistema de herramientas (40+ herramientas integradas, seguridad Bash, puente MCP, sistema de permisos, orquestación, auditoría)
└── trajectory/       # Sistema de aprendizaje (55+ archivos fuente: memoria, habilidades, RL, perfil de usuario, integración onírica, transferencia de estilo, coevolución)
```

#### Estructura de módulos del crate stock-analysis (23 submódulos)

```
stock-analysis/
├── backtest.rs         # Motor de backtesting de estrategias
├── data_clean.rs       # Limpieza y preprocesamiento de datos
├── decision.rs         # Motor de decisiones de inversión
├── key_levels.rs       # Identificación de niveles clave de precio
├── monitor.rs          # Monitoreo y alertas en tiempo real
├── orchestrator.rs     # Orquestación del pipeline de análisis
├── pipeline.rs         # Pipeline de análisis multi-etapa
├── plugin.rs           # Extensiones de plugins de análisis
├── portfolio_risk.rs   # Evaluación de riesgos de cartera
├── position_limits.rs  # Límites de posición y cumplimiento
├── prompts.rs          # Plantillas de prompts IA
├── quality.rs          # Verificación de calidad de datos
├── report.rs           # Generación de informes de análisis
├── review.rs           # Revisión de resultados de análisis
├── risk.rs             # Modelos de evaluación de riesgos
├── rules.rs            # Motor de reglas de trading
├── runner.rs           # Ejecutor de tareas de análisis
├── scoring.rs          # Sistema de puntuación integral
├── screener.rs         # Selector de acciones
├── signals.rs          # Generación de señales de trading
├── trading.rs          # Framework de estrategias de trading
├── value.rs            # Análisis de valor
└── value_investing.rs  # Evaluación de inversión de valor
```

#### Fuentes de datos del crate astock-data

| Fuente de datos | Identificador | Tipos de datos soportados |
|----------------|---------------|--------------------------|
| Tencent Finance | tencent | Cotizaciones en tiempo real, K-line |
| Tongdaxin | mootdx | Cotizaciones en tiempo real, K-line |
| Eastmoney | eastmoney | Cotizaciones, K-line, financiero, flujo de fondos, lista dragón-tigre, desbloqueo de acciones restringidas, financiamiento y préstamo de valores, fondos norteños, clasificación sectorial, incremento/reducción de accionistas, dividendos, informes de investigación, lista dragón-tigre de todo el mercado, flashes de Cailianshe |
| Sina Finance | sina | Cotizaciones, K-line, noticias |
| Baidu Stocks | baidu_stock | Cotizaciones, noticias, flujo de fondos, lista dragón-tigre, desbloqueo de acciones restringidas, financiamiento y préstamo de valores, fondos norteños, clasificación sectorial, incremento/reducción de accionistas, dividendos, informes de investigación, acciones populares, ranking sectorial, placas conceptuales, flujo de fondos norteños |
| THS (Tonghuashun) | ths | Cotizaciones, clasificación sectorial, EPS de consenso, placas conceptuales, acciones populares, ranking sectorial, flujo de fondos norteños |
| Iwencai | iwencai | Búsqueda de acciones, clasificación sectorial, EPS de consenso, placas conceptuales, acciones populares |
| Cninfo | cninfo | Anuncios |
| AKShare | akshare | Financiero, noticias, EPS de consenso, flashes de Cailianshe |

Cada tipo de datos configura rutas de degradación multi-fuente; cuando la fuente principal no está disponible, se conmuta automáticamente a la fuente de respaldo.

#### Módulos adicionales de astock-data

| Módulo | Funcionalidad |
|--------|--------------|
| calendar | Calendario de trading de acciones A (feriados 2025-2026 + días laborables ajustados) |
| indicators | Cálculo de indicadores técnicos (MA/MACD/RSI/Bandas de Bollinger/Tasa de desviación/Ratio de volumen/Soporte-Resistencia) |
| mcp_tools | Registro de herramientas MCP (registro de capacidades de datos de acciones como herramientas invocables por IA) |

### Arquitectura Frontend

```
src/
├── stores/                    # Gestión de estado Zustand (65 stores)
│   ├── domain/               # Estado de negocio principal (9)
│   │   ├── agentDomainStore.ts
│   │   ├── compressStore.ts
│   │   ├── conversationPreferences.ts
│   │   ├── conversationStore.ts
│   │   ├── conversationStoreEvents.ts
│   │   ├── conversationStoreSend.ts
│   │   ├── multiModelStore.ts
│   │   ├── preferenceStore.ts
│   │   └── streamStore.ts
│   ├── feature/               # Estado de módulos funcionales (46)
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
│   ├── devtools/              # Estado de herramientas de desarrollo (5)
│   │   ├── evaluatorStore.ts
│   │   ├── fineTuneStore.ts
│   │   ├── recommendationStore.ts
│   │   ├── rlStore.ts
│   │   └── tracerStore.ts
│   └── shared/                # Estado compartido (5)
│       ├── artifactStore.ts
│       ├── chatWorkspaceStore.ts
│       ├── rightPanelStore.ts
│       ├── tabStore.ts
│       └── uiStore.ts
│
├── components/                # Componentes React (25 módulos)
│   ├── chat/                # Interfaz de chat (100+ componentes: panel de ejecución de agente, comparación de ramas, automatización de navegador, ejecutor de código, panel de colaboración, investigación profunda, verificación de hechos, commit Git, generación/análisis de imágenes, recuperación de conocimientos, extracción de memoria, enrutamiento de modelos, visualización multi-modelo, gestión de permisos, mercado de plugins, panel de reflexión, creación/evolución de habilidades, pensamiento estructurado, tarjeta de sub-agente, tarjeta de llamada a herramienta, reproducción de trayectoria, llamada de voz, recuperación Wiki, progreso de flujo de trabajo, etc.)
│   ├── stock-analysis/      # Análisis de inversión inteligente (16 componentes)
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
│   │   ├── StockAnalysisSettingsModal.tsx
│   │   └── StockAnalysisChatIndicator.tsx
│   ├── workflow/            # Editor de flujo de trabajo (16 tipos de nodos + 16 paneles de propiedades + panel IA + plantillas + depuración)
│   ├── gateway/             # UI Pasarela API (resumen/claves/métricas/monitoreo/configuración/plantillas/diagnóstico)
│   ├── settings/            # Paneles de configuración (50+ componentes: proveedores/modelos/MCP/conocimiento/memoria/proxy/atajos/temas/herramientas/Webhook/Cron/configuración de análisis de acciones, etc.)
│   ├── terminal/            # UI Terminal (terminal integrada/Docker/SSH/selección de backend/completación de rutas/completación con barra)
│   ├── skill/               # Editor y renderizador de habilidades (edición de cadena de acciones/editor frontend/contenedor sandbox/verificación de dependencias/panel de estadísticas)
│   ├── benchmark/           # Panel de benchmarks (configuración/informe/selector/lista de tareas/resultados)
│   ├── files/               # Página de gestión de archivos
│   ├── fine-tune/           # Configuración de ajuste fino LoRA (dataset/tareas de entrenamiento/configuración LoRA)
│   ├── link/                # Gestión de enlaces externos (resumen/modelos/estrategias/habilidades/detalle de estrategia)
│   ├── llm-wiki/            # Editor LLM Wiki (puntuación de calidad/estado de sincronización)
│   ├── proactive/           # Sistema de sugerencias proactivas (predicción de contexto/indicador de prefetch/barra de sugerencias/lista de recordatorios)
│   ├── wiki/                # Gestión Wiki (enlaces inversos/vista de grafo/ingesta/lint de código/timeline de operaciones/agregación de etiquetas/historial de versiones)
│   ├── devtools/            # Timeline Trace/Span (gráfico de costos/gráfico de duración/detalles/filtros/lista)
│   ├── decomposition/       # Descomposición de habilidades (vista previa de descomposición/dependencias de herramientas/generación de herramientas/instalación de herramientas)
│   ├── recommendation/      # Panel de recomendación de herramientas
│   ├── style/               # Transferencia de estilo de código (muestras/sliders de ajuste/comparación/panel de vista previa)
│   ├── layout/              # Componentes de diseño (barra de título/barra lateral/paleta de comandos/copia global/limite de errores/barra de estado/campana de notificaciones/modal de perfil de usuario)
│   ├── help/                # Panel de ayuda
│   ├── notification/        # Centro de notificaciones
│   ├── search/              # Búsqueda de sesiones
│   ├── onboarding/          # Asistente de incorporación (tutorial interactivo/asistente de bienvenida)
│   ├── common/              # Componentes comunes (copia/iconos/slider de parámetros de modelo/pegado)
│   └── shared/              # Componentes compartidos (edición de avatar/modales/renderizado de gráficos/iconos dinámicos/selección de modelo de embedding/selección de Emoji/icono de base de conocimientos/icono MCP/selección de modelo/editor Monaco/icono de namespace/icono de proveedor de búsqueda)
│
├── pages/                    # Componentes de página (22 páginas)
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
├── lib/                      # Funciones utilitarias (33 módulos + Web Worker)
│   ├── workers/            # Web Worker (heavy.worker.ts)
│   ├── actionRouter.ts     # Enrutamiento de acciones
│   ├── artifactRenderer.ts # Renderizado de artefactos
│   ├── chartGenerator.ts   # Generación de gráficos
│   ├── chatMarkdown.ts     # Renderizado Markdown
│   ├── codeExecutor.ts     # Ejecución de código
│   ├── invoke.ts           # Wrapper IPC de Tauri
│   ├── skillActionExecutor.ts  # Ejecución de acciones de habilidades
│   ├── skillEventBus.ts    # Bus de eventos de habilidades
│   ├── skillLifecycle.ts   # Ciclo de vida de habilidades
│   ├── skillPermissions.ts # Permisos de habilidades
│   ├── storeRegistry.ts    # Registro de stores
│   ├── tokenEstimator.ts   # Estimación de tokens
│   ├── workflowLayout.ts   # Layout de flujo de trabajo
│   └── ...                 # Otros módulos utilitarios
│
├── types/                    # Definiciones de tipos TypeScript (22)
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
├── sdk/                      # SDK (incluyendo SDK Python)
│   ├── index.ts
│   ├── types.ts
│   ├── rpcBridge.ts
│   ├── sandboxTemplate.ts
│   └── python/              # SDK Python
│       ├── setup.py
│       └── axagent_sdk/__init__.py
│
└── i18n/                     # Traducciones en 11 idiomas
```

## Inicio rápido

### Descargar versiones preconstruidas

Visite la página [Releases](https://github.com/polite0803/AxAgent/releases) para descargar el instalador de su plataforma.

### Compilar desde el código fuente

#### Requisitos previos

- [Node.js](https://nodejs.org/) 20+
- [Rust](https://www.rust-lang.org/) 1.75+
- [npm](https://www.npmjs.com/) 10+
- Windows: [Visual Studio Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/) + objetivos Rust MSVC

#### Pasos de compilación

```bash
# Clonar el repositorio
git clone https://github.com/polite0803/AxAgent.git
cd AxAgent

# Instalar dependencias
npm install

# Modo desarrollo
npm run tauri dev

# Compilar solo el frontend
npm run build

# Compilar la aplicación de escritorio
npm run tauri build
```

Los artefactos de compilación están en `src-tauri/target/release/`.

### Pruebas

```bash
# Pruebas unitarias
npm run test          # Vitest watch
npm run test:run      # Vitest ejecución única

# Pruebas E2E
npm run test:e2e      # Playwright
npm run test:e2e:ui   # Playwright modo UI

# Pruebas backend Rust
cd src-tauri && cargo nextest run   # cargo-nextest (2-3x más rápido)
cd src-tauri && cargo test          # Pruebas estándar

# Verificación de tipos
npm run typecheck     # TypeScript
cd src-tauri && cargo clippy -- -D warnings  # Rust

# Formateo de código
npm run format        # dprint
cd src-tauri && cargo fmt

# Verificación CI completa
npm run ci:check
```

---

## Estructura del proyecto

```
AxInvest/
├── src/                         # Código fuente frontend (React + TypeScript)
│   ├── components/              # Componentes React (25 módulos)
│   │   ├── chat/               # Interfaz de chat (100+ componentes)
│   │   ├── stock-analysis/     # Análisis de inversión inteligente (16 componentes)
│   │   ├── workflow/           # Editor de flujo de trabajo (16 tipos de nodos + paneles de propiedades + panel IA)
│   │   ├── gateway/            # Componentes de la pasarela API
│   │   ├── settings/           # Paneles de configuración (50+ componentes)
│   │   ├── terminal/           # Componentes de terminal
│   │   ├── skill/              # Editor y renderizador de habilidades
│   │   ├── benchmark/          # Benchmarks
│   │   ├── files/              # Gestión de archivos
│   │   ├── fine-tune/          # Ajuste fino LoRA
│   │   ├── link/               # Enlaces externos
│   │   ├── llm-wiki/           # LLM Wiki
│   │   ├── proactive/          # Sugerencias proactivas
│   │   ├── wiki/               # Gestión Wiki
│   │   ├── devtools/           # Herramientas de desarrollo
│   │   ├── decomposition/      # Descomposición de habilidades
│   │   ├── recommendation/     # Recomendación de herramientas
│   │   ├── style/              # Estilo de código
│   │   ├── layout/             # Componentes de diseño
│   │   ├── help/               # Panel de ayuda
│   │   ├── notification/       # Centro de notificaciones
│   │   ├── search/             # Búsqueda de sesiones
│   │   ├── onboarding/         # Asistente de incorporación
│   │   ├── common/             # Componentes comunes
│   │   └── shared/             # Componentes compartidos
│   ├── pages/                   # Componentes de página (22 páginas)
│   ├── stores/                  # Gestión de estado Zustand (65 stores)
│   │   ├── domain/            # Estado de negocio principal (9)
│   │   ├── feature/           # Estado de módulos funcionales (46)
│   │   ├── devtools/          # Estado de herramientas de desarrollo (5)
│   │   └── shared/            # Estado compartido (5)
│   ├── hooks/                   # React hooks (12)
│   ├── lib/                     # Funciones utilitarias (33 módulos + Web Worker)
│   ├── types/                   # Definiciones de tipos TypeScript (22)
│   ├── sdk/                     # SDK (TypeScript + Python)
│   └── i18n/                    # Traducciones en 11 idiomas
│
├── src-tauri/                    # Código fuente backend (Rust)
│   ├── crates/                  # Workspace Rust (20 crates)
│   │   ├── agent/             # Núcleo del agente IA (70+ archivos fuente)
│   │   ├── astock-data/       # Fuentes de datos de acciones A (9 fuentes de datos, 22 rutas de datos, indicadores técnicos, calendario de trading)
│   │   ├── core/              # Utilidades principales (85+ entidades, 40+ repositorios, RAG, cifrado, MCP)
│   │   ├── gateway/           # Pasarela API (incluye endpoints API de acciones)
│   │   ├── migration/         # Migraciones de base de datos (5 migraciones)
│   │   ├── npm/               # Análisis de paquetes npm
│   │   ├── plugins/           # Sistema de plugins
│   │   ├── prompt-guard/      # Protección contra inyección de prompts
│   │   ├── providers/         # Adaptadores de proveedores de modelos
│   │   ├── rt-dashboard/      # Plugin de panel
│   │   ├── rt-messaging/      # Pasarela de mensajería (9 plataformas)
│   │   ├── rt-theme/          # Motor de temas
│   │   ├── rt-webhook/        # Servidor Webhook
│   │   ├── rt-workflow/       # Motor de flujo de trabajo (16 ejecutores de nodos)
│   │   ├── runtime/           # Servicios de ejecución (70+ archivos fuente)
│   │   ├── runtime-core/      # Capa de abstracción de ejecución
│   │   ├── stock-analysis/    # Análisis de inversión inteligente (23 submódulos)
│   │   ├── telemetry/         # Rastreo y métricas
│   │   ├── tools/             # Sistema de herramientas (40+ herramientas integradas)
│   │   └── trajectory/        # Sistema de aprendizaje (55+ archivos fuente)
│   └── src/                    # Punto de entrada Tauri (91 módulos de comandos)
│       ├── commands/          # Módulos de comandos
│       │   ├── stock_analysis.rs        # Comandos de análisis de acciones
│       │   ├── stock_analysis_setup.rs  # Configuración de análisis de acciones
│       │   ├── stock_workflow.rs        # Comandos de flujo de trabajo de acciones
│       │   ├── agency_expert.rs         # Agente experto
│       │   ├── agent_advanced.rs        # Agente avanzado
│       │   ├── agent_analytics.rs       # Analítica de agente
│       │   ├── agent_insight.rs         # Insight de agente
│       │   ├── agent_nudge.rs           # Nudge de agente
│       │   ├── agent_profile.rs         # Perfil de agente
│       │   ├── agent_role.rs            # Rol de agente
│       │   ├── background_tasks.rs      # Tareas en segundo plano
│       │   ├── browser.rs              # Automatización de navegador
│       │   ├── chart_generator.rs       # Generación de gráficos
│       │   ├── cloud_workspace.rs       # Workspace en la nube
│       │   ├── computer_control.rs      # Control informático
│       │   ├── context_breakdown.rs     # Desglose de contexto
│       │   ├── conversation_categories.rs  # Categorías de conversación
│       │   ├── conversations_search.rs  # Búsqueda de conversaciones
│       │   ├── crash_report.rs          # Informe de fallos
│       │   ├── dream.rs                # Integración onírica
│       │   ├── evolution.rs            # Evolución de habilidades
│       │   ├── fine_tune.rs            # Ajuste fino LoRA
│       │   ├── gateway.rs              # Pasarela API
│       │   ├── gateway_link.rs         # Enlaces externos
│       │   ├── generated_tool.rs        # Herramientas generadas
│       │   ├── image_gen.rs            # Generación de imágenes
│       │   ├── knowledge.rs            # Base de conocimientos
│       │   ├── llm_wiki.rs             # LLM Wiki
│       │   ├── local_models.rs         # Modelos locales
│       │   ├── mcp.rs                  # Protocolo MCP
│       │   ├── memory.rs              # Sistema de memoria
│       │   ├── message_continuation.rs  # Continuación de mensajes
│       │   ├── onboarding.rs           # Asistente de incorporación
│       │   ├── parallel_execution.rs    # Ejecución paralela
│       │   ├── plan.rs                 # Gestión de planes
│       │   ├── platform_integration.rs  # Integración de plataforma
│       │   ├── plugin.rs               # Gestión de plugins
│       │   ├── proactive.rs            # Sugerencias proactivas
│       │   ├── prompt_templates.rs      # Plantillas de prompts
│       │   ├── providers.rs            # Proveedores de modelos
│       │   ├── quickbar.rs             # QuickBar
│       │   ├── reflection.rs           # Reflexión
│       │   ├── research.rs             # Investigación profunda
│       │   ├── rl.rs                   # Aprendizaje por refuerzo
│       │   ├── sandbox.rs              # Sandbox
│       │   ├── scheduled_task.rs        # Tareas programadas
│       │   ├── screen_vision.rs        # Visión de pantalla
│       │   ├── search.rs               # Búsqueda
│       │   ├── session_share.rs         # Compartir sesión
│       │   ├── shell.rs                # Shell
│       │   ├── skill_decomposition.rs   # Descomposición de habilidades
│       │   ├── skills_hub.rs           # Hub de habilidades
│       │   ├── tool_recommender.rs      # Recomendación de herramientas
│       │   ├── tracer.rs               # Rastreo
│       │   ├── user_profile.rs          # Perfil de usuario
│       │   ├── webdav.rs               # WebDAV
│       │   ├── webhook.rs              # Webhook
│       │   ├── wiki.rs                 # Wiki
│       │   ├── work_engine.rs          # Motor de trabajo
│       │   ├── workflow_ai.rs          # Flujo de trabajo IA
│       │   ├── workflow_template.rs     # Plantillas de flujo de trabajo
│       │   └── ...                     # Otros comandos
│       ├── init/              # Módulos de inicialización
│       ├── stock_scheduler.rs # Planificador de acciones
│       └── ...                # Otros módulos principales
│
├── extension/                  # Extensión de navegador (Wiki Clipper: popup/content/background)
├── e2e/                        # Pruebas E2E Playwright (9 suites de pruebas)
├── scripts/                    # Scripts de build y herramientas
└── website/                    # Sitio web del proyecto (VitePress, documentación en 11 idiomas)
```

## Directorio de datos

```
~/.axinvest/                     # Directorio de configuración
├── axinvest.db                  # Base de datos SQLite
├── master.key                   # Clave maestra AES-256
├── vector_db/                   # Base de datos vectorial (sqlite-vec)
└── ssl/                         # Certificados SSL

~/Documents/axinvest/           # Directorio de archivos de usuario
├── images/                     # Imágenes adjuntas
├── files/                      # Archivos adjuntos
└── backups/                    # Archivos de copia de seguridad
```

---

## Preguntas frecuentes

### macOS: «La app está dañada» o «No se puede verificar al desarrollador»

Como la aplicación no está firmada por Apple:

**1. Permitir apps de «Cualquier origen»**
```bash
sudo spctl --master-disable
```

Luego ve a **Configuración del sistema → Privacidad y seguridad → Seguridad** y selecciona **Cualquier origen**.

**2. Eliminar el atributo de cuarentena**
```bash
sudo xattr -dr com.apple.quarantine /Applications/AxInvest.app
```

**3. Paso adicional para macOS Ventura+**
Ve a **Configuración del sistema → Privacidad y seguridad** y haz clic en **Abrir igualmente**.

---

## Comunidad

- [LinuxDO](https://linux.do)

## Licencia

Este proyecto está bajo la licencia [AGPL-3.0](LICENSE).
