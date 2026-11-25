// SPDX-License-Identifier: AGPL-3.0-only
/**
 * G2 模拟观察组合（Paper Trading Portfolio）Zustand store
 *
 * 负责：
 * - 列表 / 详情的 IPC 调用与缓存
 * - 创建 / 关闭 / 归档 / 添加持仓 / 平仓等操作
 * - loading / error 状态管理
 *
 * 命令清单（宿主 `paper_portfolio_*` Tauri 命令已退役，现由 stock 插件 worker 声明，
 * 经 `stock.analysis` 接缝调 analysis-engine 的同一份权威实现）：
 * - paper_portfolio_create / list / get / close / archive
 * - paper_portfolio_add_position / close_position / close_all_positions
 * - paper_portfolio_list_active_details
 */

import { invokePluginCommand } from "@/lib/pluginCommand";
import type {
  AddPositionInput,
  ClosePositionInput,
  CreatePortfolioInput,
  PaperPortfolio,
  PortfolioDetail,
} from "@/types/paper-portfolio";
import { create } from "zustand";

interface PaperPortfolioState {
  // ── 数据 ──
  /** active 组合详情列表（Dashboard 用） */
  activeDetails: PortfolioDetail[];
  /** 全量组合列表（按状态过滤后） */
  portfolios: PaperPortfolio[];
  /** 当前选中的组合详情 */
  currentDetail: PortfolioDetail | null;

  // ── 状态 ──
  loadingList: boolean;
  loadingDetail: boolean;
  submitting: boolean;
  error: string | null;

  // ── Actions ──
  /** 列出所有 active 组合的详情（Dashboard 用，含持仓 + 实时盈亏） */
  fetchActiveDetails: () => Promise<void>;
  /** 列出所有组合（按状态过滤，None = 全部） */
  fetchPortfolios: (status?: string) => Promise<void>;
  /** 获取单个组合详情 */
  fetchPortfolioDetail: (portfolioId: string) => Promise<void>;
  /** 创建模拟组合 */
  createPortfolio: (input: CreatePortfolioInput) => Promise<PaperPortfolio>;
  /** 关闭组合 */
  closePortfolio: (portfolioId: string) => Promise<void>;
  /** 归档组合 */
  archivePortfolio: (portfolioId: string) => Promise<void>;
  /** 添加虚拟持仓 */
  addPosition: (input: AddPositionInput) => Promise<void>;
  /** 平仓单个持仓 */
  closePosition: (input: ClosePositionInput) => Promise<void>;
  /** 批量平仓 */
  closeAllPositions: (
    portfolioId: string,
    exitPrice: number,
    exitDate: string,
  ) => Promise<number>;
  /** 清空当前详情 */
  clearCurrentDetail: () => void;
  /** 清空错误 */
  clearError: () => void;
}

export const usePaperPortfolioStore = create<PaperPortfolioState>((set, get) => ({
  activeDetails: [],
  portfolios: [],
  currentDetail: null,

  loadingList: false,
  loadingDetail: false,
  submitting: false,
  error: null,

  fetchActiveDetails: async () => {
    set({ loadingList: true, error: null });
    try {
      const data = await invokePluginCommand<PortfolioDetail[]>("paper_portfolio_list_active_details");
      set({ activeDetails: data, loadingList: false });
    } catch (e) {
      set({ loadingList: false, error: String(e) });
    }
  },

  fetchPortfolios: async (status?: string) => {
    set({ loadingList: true, error: null });
    try {
      const data = await invokePluginCommand<PaperPortfolio[]>("paper_portfolio_list", {
        status: status ?? null,
      });
      set({ portfolios: data, loadingList: false });
    } catch (e) {
      set({ loadingList: false, error: String(e) });
    }
  },

  fetchPortfolioDetail: async (portfolioId: string) => {
    set({ loadingDetail: true, error: null });
    try {
      const data = await invokePluginCommand<PortfolioDetail | null>("paper_portfolio_get", {
        portfolioId,
      });
      set({ currentDetail: data, loadingDetail: false });
    } catch (e) {
      set({ loadingDetail: false, error: String(e) });
    }
  },

  createPortfolio: async (input: CreatePortfolioInput) => {
    set({ submitting: true, error: null });
    try {
      const created = await invokePluginCommand<PaperPortfolio>("paper_portfolio_create", { input });
      set({ submitting: false });
      // 创建后刷新 active 列表
      await get().fetchActiveDetails();
      return created;
    } catch (e) {
      set({ submitting: false, error: String(e) });
      throw e;
    }
  },

  closePortfolio: async (portfolioId: string) => {
    set({ submitting: true, error: null });
    try {
      await invokePluginCommand<PaperPortfolio>("paper_portfolio_close", { portfolioId });
      set({ submitting: false });
      await get().fetchActiveDetails();
    } catch (e) {
      set({ submitting: false, error: String(e) });
      throw e;
    }
  },

  archivePortfolio: async (portfolioId: string) => {
    set({ submitting: true, error: null });
    try {
      await invokePluginCommand<PaperPortfolio>("paper_portfolio_archive", { portfolioId });
      set({ submitting: false });
      await get().fetchActiveDetails();
    } catch (e) {
      set({ submitting: false, error: String(e) });
      throw e;
    }
  },

  addPosition: async (input: AddPositionInput) => {
    set({ submitting: true, error: null });
    try {
      await invokePluginCommand("paper_portfolio_add_position", { input });
      set({ submitting: false });
      // 添加后刷新当前详情
      if (input.portfolioId) {
        await get().fetchPortfolioDetail(input.portfolioId);
      }
      await get().fetchActiveDetails();
    } catch (e) {
      set({ submitting: false, error: String(e) });
      throw e;
    }
  },

  closePosition: async (input: ClosePositionInput) => {
    set({ submitting: true, error: null });
    try {
      await invokePluginCommand("paper_portfolio_close_position", { input });
      set({ submitting: false });
      // 平仓后刷新当前详情（如有）
      const cur = get().currentDetail;
      if (cur) {
        await get().fetchPortfolioDetail(cur.id);
      }
      await get().fetchActiveDetails();
    } catch (e) {
      set({ submitting: false, error: String(e) });
      throw e;
    }
  },

  closeAllPositions: async (portfolioId: string, exitPrice: number, exitDate: string) => {
    set({ submitting: true, error: null });
    try {
      const affected = await invokePluginCommand<number>("paper_portfolio_close_all_positions", {
        portfolioId,
        exitPrice,
        exitDate,
      });
      set({ submitting: false });
      await get().fetchPortfolioDetail(portfolioId);
      await get().fetchActiveDetails();
      return affected;
    } catch (e) {
      set({ submitting: false, error: String(e) });
      throw e;
    }
  },

  clearCurrentDetail: () => set({ currentDetail: null }),
  clearError: () => set({ error: null }),
}));
