// SPDX-License-Identifier: AGPL-3.0-only

import fs from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";

function readSource(...segments: string[]) {
  return fs.readFileSync(path.resolve(process.cwd(), ...segments), "utf8");
}

describe("Phase B category template regressions", () => {
  it("keeps the category contract to model selection only", () => {
    const typeSource = readSource("src/types/index.ts");
    const rustTypeSource = readSource(
      "src-tauri/crates/harness/src/types/conversation.rs",
    );
    const entitySource = readSource(
      "src-tauri/crates/entities/src/conversation_categories.rs",
    );

    // 只截分类自己的声明段 —— AppSettings 仍合法保留 defaultTemperature 等全局默认字段，
    // 对整份文件做 not.toMatch 会把那条合法定义误判成违规。
    const categoryBlock = typeSource.slice(
      typeSource.indexOf("export interface ConversationCategory"),
      typeSource.indexOf("export interface Conversation {"),
    );
    expect(categoryBlock).toMatch(/defaultModel: NullableModelRef;/);
    expect(categoryBlock).not.toMatch(/defaultTemperature/);
    expect(categoryBlock).not.toMatch(/defaultMaxTokens/);
    expect(categoryBlock).not.toMatch(/defaultTopP/);
    expect(categoryBlock).not.toMatch(/defaultFrequencyPenalty/);

    const rustCategoryBlock = rustTypeSource.slice(
      rustTypeSource.indexOf("pub struct ConversationCategory"),
      rustTypeSource.indexOf("pub struct CreateConversationCategoryInput"),
    );
    expect(rustCategoryBlock).toMatch(/pub default_provider_id: Option<String>/);
    expect(rustCategoryBlock).toMatch(/pub default_model_id: Option<String>/);
    expect(rustCategoryBlock).not.toMatch(/pub default_temperature/);
    expect(rustCategoryBlock).not.toMatch(/pub default_max_tokens/);
    expect(rustCategoryBlock).not.toMatch(/pub default_top_p/);
    expect(rustCategoryBlock).not.toMatch(/pub default_frequency_penalty/);

    expect(entitySource).toMatch(/pub default_provider_id: Option<String>/);
    expect(entitySource).toMatch(/pub default_model_id: Option<String>/);
    expect(entitySource).not.toMatch(/pub default_max_tokens/);
  });

  it("lets the category editor configure a default model but no sampling params", () => {
    const modalSource = readSource("src/components/chat/CategoryEditModal.tsx");

    expect(modalSource).toContain("ModelSelect");
    expect(modalSource).toContain("defaultProviderId");
    expect(modalSource).toContain("defaultModelId");
    expect(modalSource).not.toContain("ModelParamSliders");
    expect(modalSource).not.toContain("defaultTemperature");
    expect(modalSource).not.toContain("defaultMaxTokens");
    expect(modalSource).not.toContain("defaultTopP");
    expect(modalSource).not.toContain("defaultFrequencyPenalty");
  });

  it("provides a new conversation action scoped to workspace groups", () => {
    const sidebarSource = readSource("src/components/chat/ChatSidebar.tsx");

    expect(sidebarSource).toContain("handleNewConversation");
    expect(sidebarSource).toContain("new-conversation-btn");
    expect(sidebarSource).toContain("MessageSquarePlus");
  });
});
