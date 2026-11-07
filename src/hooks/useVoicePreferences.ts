// SPDX-License-Identifier: AGPL-3.0-only

import { useVoicePreferenceStore } from "@/stores/feature/voicePreferenceStore";

/**
 * 语音偏好读面接入（自 InputArea 抽出，组件中立性消键）：
 * TTS 音色 / STT·TTS 服务商 ID 三个订阅值，供实时语音配置拼装。
 *
 * 活源是 voicePreferenceStore；抽成 hook 是为了让 chat 等跨域组件不直接依赖
 * voicePreferenceStore（组件中立性），统一从这里取。
 */
export function useVoicePreferences() {
  const ttsVoice = useVoicePreferenceStore((s) => s.ttsVoice);
  const sttProviderId = useVoicePreferenceStore((s) => s.sttProviderId);
  const ttsProviderId = useVoicePreferenceStore((s) => s.ttsProviderId);
  return { ttsVoice, sttProviderId, ttsProviderId };
}
