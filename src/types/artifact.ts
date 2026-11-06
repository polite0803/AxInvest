// SPDX-License-Identifier: AGPL-3.0-only

/**
 * 取值域与后端注释一致（`axagent-core/crates/harness/src/types/rag_voice_etc.rs:631` 的
 * `kind: String, // draft | note | report | snippet | checklist`）。
 *
 * #192 裁定：**不给后端加枚举、也不加运行时校验** —— 后端该字段是 `String` 不校验，加枚举要附带
 * migration 与历史值兼容，而这一层只影响编译期与展示；前端收窄的作用是让新增取值在编译期被发现。
 */
export type ArtifactKind =
  | "draft"
  | "note"
  | "report"
  | "snippet"
  | "checklist";
export type ArtifactFormat =
  | "markdown"
  | "text"
  | "json"
  | "html"
  | "css"
  | "javascript"
  | "typescript"
  | "jsx"
  | "tsx"
  | "python"
  | "svg"
  | "mermaid"
  | "d2";

export type ArtifactLanguage = ArtifactFormat;

export type ArtifactPreviewMode = "split" | "preview" | "code";

export type Artifact = {
  id: string;
  conversationId: string;
  kind: ArtifactKind;
  title: string;
  content: string;
  format: ArtifactFormat;
  pinned: boolean;
  updatedAt: string;
};

export type CreateArtifactInput = {
  conversationId: string;
  sourceMessageId?: string;
  kind: ArtifactKind;
  title: string;
  content: string;
  format: ArtifactFormat;
};

export type UpdateArtifactInput = {
  title?: string;
  content?: string;
  format?: ArtifactFormat;
  pinned?: boolean;
};
