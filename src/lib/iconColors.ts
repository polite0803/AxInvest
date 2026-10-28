// SPDX-License-Identifier: AGPL-3.0-only

import { type CategoricalHue, hueVar } from "@/theme/categoricalColors";

/**
 * Semantic color mapping for lucide-react icons.
 * Each key matches a lucide icon name; the value is the color applied via the `color` prop.
 * Colors are chosen to be visually distinctive while remaining harmonious.
 */

// ── Navigation / Sidebar icons ──────────────────────────────────────
export const NAV_ICON_COLORS: Record<string, CategoricalHue> = {
  MessageSquare: "blue", // blue-500  — Chat
  Sparkles: "orange", // amber-500 — Skills
  BookOpen: "emerald", // emerald-500 — Knowledge
  Library: "teal", // teal-500 — Wiki
  Brain: "violet", // violet-500 — Memory
  Link2: "cyan", // cyan-500 — Link
  Router: "volcano", // orange-500 — Gateway
  FolderOpen: "gray", // slate-500 — Files
};

// ── Settings sidebar icons ──────────────────────────────────────────
export const SETTINGS_ICON_COLORS: Record<string, CategoricalHue> = {
  Cloud: "blue", // blue-500  — Providers
  MessageSquare: "blue", // blue-500  — Conversation
  Bot: "green", // green-500 — Default Model
  Settings: "gray", // slate-500 — General
  Palette: "pink", // pink-500  — Display
  Globe: "cyan", // cyan-500  — Proxy
  Zap: "yellow", // yellow-500 — Shortcuts
  Database: "violet", // violet-500 — Data
  HardDrive: "gray", // slate-500 — Storage
  Info: "geekblue", // indigo-500 — About
  Search: "sky", // sky-500   — Search Providers
  Wrench: "volcano", // orange-500 — Local Tools
  Plug: "purple", // purple-500 — MCP Servers
  Clock: "teal", // teal-500  — Scheduler
  CloudUpload: "blue", // blue-500  — Backup
  Workflow: "violet", // violet-500 — Workflow
  Send: "blue", // blue-500  — Message Channels
  FileText: "blue", // blue-500  — Prompt Templates
  Network: "cyan", // cyan-500  — ACP
  Puzzle: "purple", // purple-500 — Plugins
  ShoppingBag: "orange", // amber-500 — Skills Hub
  LayoutDashboard: "violet", // violet-500 — Dashboard Plugins
  BookOpen: "emerald", // emerald-500 — Knowledge Settings
};

// ── Title bar icons ─────────────────────────────────────────────────
export const TITLEBAR_ICON_COLORS: Record<string, CategoricalHue> = {
  Pin: "blue",
  PinOff: "gray",
  Monitor: "gray",
  Sun: "orange",
  Moon: "geekblue",
  Globe: "cyan",
  CloudUpload: "blue",
  GitFork: "gray",
  ArrowDownCircle: "green",
  RotateCcw: "gray",
  Settings: "gray",
  XCircle: "red",
  MessageSquarePlus: "green",
  Bug: "red",
  Star: "yellow",
};

// ── Chat / common action icons ──────────────────────────────────────
export const CHAT_ICON_COLORS: Record<string, CategoricalHue> = {
  Pencil: "blue",
  Share2: "cyan",
  Copy: "gray",
  Check: "green",
  Trash2: "red",
  RefreshCw: "blue",
  Search: "sky",
  Bot: "green",
  Brain: "violet",
  Lightbulb: "orange",
  Code: "geekblue",
  Terminal: "green",
  Wrench: "volcano",
  BookOpen: "emerald",
  Globe: "cyan",
  Database: "violet",
  Shield: "green",
  Key: "yellow",
  Zap: "yellow",
  Plug: "purple",
  Mic: "red",
  Paperclip: "gray",
  Download: "blue",
  Upload: "blue",
  Eye: "geekblue",
  GitBranch: "volcano",
  Activity: "green",
  Clock: "teal",
  Sparkles: "orange",
  User: "blue",
  X: "red",
  ChevronDown: "gray",
  ChevronRight: "gray",
  ChevronLeft: "gray",
  AlertCircle: "orange",
  AlertTriangle: "orange",
  Info: "geekblue",
  CheckCircle2: "green",
  XCircle: "red",
  Loader2: "blue",
  ExternalLink: "cyan",
  FolderOpen: "gray",
  FileText: "gray",
  Plus: "green",
  Undo2: "gray",
  Layers: "violet",
  Route: "volcano",
  MessageSquare: "blue",
  HardDrive: "gray",
  Cloud: "blue",
  Maximize2: "gray",
  Minimize2: "gray",
  Expand: "gray",
  Shrink: "gray",
  Heart: "red",
  Power: "red",
  PowerOff: "gray",
  SquarePen: "blue",
  ListChecks: "green",
  CircleHelp: "gray",
  GripVertical: "gray",
  MoreHorizontal: "gray",
  GripHorizontal: "gray",
  Scissors: "volcano",
  Eraser: "red",
  SlidersHorizontal: "gray",
  ArrowUp: "green",
  ArrowDown: "gray",
  ArrowLeftRight: "gray",
  Columns2: "gray",
  LayoutList: "gray",
  Rows3: "gray",
  PlayCircle: "green",
  Pause: "orange",
  Play: "green",
  SkipForward: "blue",
  Archive: "gray",
  PinOff: "gray",
  Loader: "blue",
  Share: "cyan",
  ArrowLeft: "gray",
  FileImage: "violet",
  FileCode: "geekblue",
  FileType: "gray",
  FolderPlus: "green",
  MessageSquareText: "blue",
  MessageCircleQuestion: "blue",
  Send: "blue",
  Link: "cyan",
  CheckCircle: "green",
  ZoomIn: "gray",
  ZoomOut: "gray",
  Palette: "pink",
  Image: "violet",
  FolderEdit: "volcano",
  RotateCcw: "gray",
  Radio: "red",
  CloudUpload: "blue",
  BarChart3: "blue",
  Gauge: "volcano",
  ScrollText: "gray",
  GitCompareArrows: "gray",
  ShieldCheck: "green",
  ShieldAlert: "orange",
  ShieldX: "red",
  ZapOff: "gray",
  Coins: "yellow",
  Timer: "teal",
  ChartNoAxesColumn: "blue",
  ArrowUpRight: "green",
  ArrowDownRight: "gray",
  Bell: "orange",
  Atom: "violet",
  Square: "gray",
  Minus: "gray",
  TextCursorInput: "gray",
  Bug: "red",
  ClipboardPaste: "gray",
  BoxSelect: "gray",
  Puzzle: "purple", // purple-500
  Code2: "geekblue",
  PanelLeftClose: "gray",
  Network: "cyan",
  FileSearch: "sky",
  FileEdit: "volcano",
  Save: "green",
  Smile: "orange",
  Grid2x2: "gray",
  Trash: "red",
  Web: "cyan",
  Phone: "green",
  Volume2: "blue",
  MicOff: "gray",
  CircleOff: "gray",
  SignalLow: "red",
  SignalMedium: "orange",
  SignalHigh: "green",
  Signal: "gray",
  Film: "violet",
  Music: "pink",
  File: "gray",
  ChevronsDownUp: "gray",
  ChevronsUpDown: "gray",
};

/**
 * Get the semantic color for a given icon name.
 * Falls back to `undefined` (which means the icon uses its inherited CSS color).
 */
export function getIconColor(
  iconName: string,
  context?: "nav" | "settings" | "titlebar",
): string | undefined {
  const hue = getIconHue(iconName, context);
  return hue ? hueVar(hue) : undefined;
}

/** 取图标对应的类目色相（未登记时返回 undefined，由调用方继承文字色）。 */
export function getIconHue(
  iconName: string,
  context?: "nav" | "settings" | "titlebar",
): CategoricalHue | undefined {
  if (context === "nav") {
    return NAV_ICON_COLORS[iconName];
  }
  if (context === "settings") {
    return SETTINGS_ICON_COLORS[iconName];
  }
  if (context === "titlebar") {
    return TITLEBAR_ICON_COLORS[iconName];
  }
  return CHAT_ICON_COLORS[iconName];
}
