// SPDX-License-Identifier: AGPL-3.0-only

/**
 * 办公室成员显示名多语言（PLAN-office-auto-provision.md 阶段 4-③）。
 *
 * 显示层解析、不动 DB：域包播种成员的 `agentSlug = profileId = "opc-<expert_key>"`，
 * 据此推导 i18n 键 `office.experts.<expert_key>`（45 键 × 11 语言，名册与
 * `capability_pack_experts.rs` 的 expert key 逐字一致）。非域包成员（手工 SubAgent /
 * OPC 组织员工）或键缺失时回退 DB 里的 `displayName`，永不显示原始键。
 */
import { useTranslation } from "react-i18next";

export function expertNameKey(agentSlug: string): string | null {
  return agentSlug.startsWith("opc-") && agentSlug.length > 4
    ? `office.experts.${agentSlug.slice(4)}`
    : null;
}

export function useExpertName(): (member: { agentSlug: string; displayName: string }) => string {
  const { t } = useTranslation();
  return (member) => {
    const key = expertNameKey(member.agentSlug);
    if (!key) {
      return member.displayName;
    }
    const translated = t(key);
    // i18next 缺键返回键名本身（未配 parseMissingKeyHandler）——据此判存在性
    return translated && translated !== key ? translated : member.displayName;
  };
}
