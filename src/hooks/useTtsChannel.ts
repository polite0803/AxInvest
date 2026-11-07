// SPDX-License-Identifier: AGPL-3.0-only

import { useTTSChannelStore } from "@/stores/feature/ttsChannelStore";

/**
 * TTS 通道读面（开关 + 进度简报 + 消息列表）。
 *
 * 抽成 hook 是为了让 chat（VoiceCall）组件不直接依赖 ttsChannelStore
 * （组件中立性，见 scripts/check-component-neutrality.mjs），统一从这里取。
 */
export function useTtsChannel() {
  const enabled = useTTSChannelStore((s) => s.enabled);
  const setEnabled = useTTSChannelStore((s) => s.setEnabled);
  const handleProgressBrief = useTTSChannelStore((s) => s.handleProgressBrief);
  const messages = useTTSChannelStore((s) => s.messages);
  return { enabled, setEnabled, handleProgressBrief, messages };
}
