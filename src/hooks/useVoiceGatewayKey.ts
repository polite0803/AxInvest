// SPDX-License-Identifier: AGPL-3.0-only

import { useGatewayStore } from "@/stores/feature/gatewayStore";
import { App } from "antd";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

/**
 * 语音通话的网关 API key 接入（自 InputArea 抽出，组件中立性消键）。
 *
 * 封装 gatewayStore 的三段消费：keys 订阅、空时 fetchKeys、
 * 取首个启用 key 解密出语音鉴权 key；解密失败经 messageApi 给用户可见反馈。
 * 返回 voiceApiKey（尚未解出时为空串）。
 */
export function useVoiceGatewayKey(): string {
  const { message: messageApi } = App.useApp();
  const { t } = useTranslation();
  const gatewayKeys = useGatewayStore((s) => s.keys);
  const [voiceApiKey, setVoiceApiKey] = useState("");

  // 加载 gateway API key 用于语音通话鉴权
  useEffect(() => {
    if (gatewayKeys.length === 0) {
      useGatewayStore.getState().fetchKeys();
    }
  }, [gatewayKeys.length]);

  // 稳定 messageApi / t 引用，避免其不稳定导致解密 useEffect 反复触发
  const messageApiRef = useRef(messageApi);
  messageApiRef.current = messageApi;
  const tRef = useRef(t);
  tRef.current = t;
  // 解密尝试标记：防止 decryptKey 失败后无限重试（避免 Maximum update depth exceeded）
  const decryptAttemptedRef = useRef(false);

  useEffect(() => {
    if (gatewayKeys.length === 0 || voiceApiKey || decryptAttemptedRef.current) {
      return;
    }
    decryptAttemptedRef.current = true;
    const enabledKey = gatewayKeys.find((k) => k.enabled) || gatewayKeys[0];
    // P1-10：解密失败时给用户可见的反馈，而不是静默吞错（之前 .catch(() => {}) 会让语音入口无任何提示直接失效）
    useGatewayStore.getState().decryptKey(enabledKey.id)
      .then(setVoiceApiKey)
      .catch(() => {
        // 仅在首次失败提示；用 ref 读取最新的 messageApi/t，避免把不稳定引用放入依赖
        messageApiRef.current.error(tRef.current("voice.decryptKeyFailed"));
      });
    // 依赖只用 length 和 voiceApiKey（稳定值）；messageApi/t 通过 ref 读取
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [gatewayKeys.length, voiceApiKey]);

  return voiceApiKey;
}
