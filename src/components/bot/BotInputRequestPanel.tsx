import type { BotInputRequest } from "@/types";
import { useMemo } from "react";
import { BotInputRequestCard } from "./BotInputRequestCard";

export function BotInputRequestPanel({
  botId,
  requests,
}: {
  botId: string | null;
  requests: BotInputRequest[];
}) {
  const visible = useMemo(() => {
    if (!botId) return [];
    return requests.filter((r) => r.botId === botId);
  }, [botId, requests]);

  if (!botId || visible.length === 0) return null;

  return (
    <div
      data-testid="bot-input-request-panel"
      style={{ display: "flex", flexDirection: "column", gap: 8, padding: "8px 16px 0" }}
    >
      {visible.map((request) => <BotInputRequestCard key={request.id} request={request} />)}
    </div>
  );
}
