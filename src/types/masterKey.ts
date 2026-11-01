// SPDX-License-Identifier: AGPL-3.0-only

// 主密钥（master.key）相关类型 —— 与后端 DTO（src-tauri/src/commands/master_key.rs）
// 逐字段对齐（Rust 侧 snake_case + serde rename_all = "camelCase"，TS 侧消费 camelCase）。

/** 当前主密钥概要（只有指纹，不含钥匙本体） */
export interface MasterKeyInfo {
  fingerprint: string;
}

/** 主密钥导出结果（keyHex 即钥匙本体，64 位 hex） */
export interface MasterKeyExport {
  fingerprint: string;
  keyHex: string;
}

/** 主密钥导入结果报告 */
export interface MasterKeyImportReport {
  /** 导入前本机旧钥匙的指纹；本机原本没有 master.key 时为 null */
  oldFingerprint: string | null;
  /** 导入后的新钥匙指纹 */
  newFingerprint: string;
  /** 本次导入产生的备份文件路径；同指纹 no-op 导入为 null */
  backupPath: string | null;
  /** 是否重加密了 db_config.json 的 pgPasswordEnc */
  dbConfigReencrypted: boolean;
}
