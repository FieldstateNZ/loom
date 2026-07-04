// Per-conversation totals for a transcript: the real cost + token figures from
// the usage rollup, falling back to summing per-turn token usage (cost 0) when
// the rollup has no row for the conversation.
import { usageRollup, type RequestFn } from "./http-request.ts";
import type { Transcript, TranscriptTurn } from "./transcript.ts";

/** Resolves a transcript's rolled-up totals for the header strip. */
export async function transcriptTotals(
  request: RequestFn,
  apiKey: string,
  conversationId: string,
  turns: readonly TranscriptTurn[],
): Promise<Transcript["totals"]> {
  const rows = await usageRollup(request, "/v1/usage", apiKey, { group_by: "conversation" });
  const row = rows.find((r) => r.group === conversationId);
  if (row) {
    return {
      cost: row.cost,
      inTok: row.input_tokens,
      outTok: row.output_tokens,
      cacheRead: row.cache_read_tokens,
      cacheWrite: row.cache_write_tokens,
    };
  }
  return turns.reduce(
    (a, t) => ({
      cost: a.cost,
      inTok: a.inTok + (t.usage?.inTok ?? 0),
      outTok: a.outTok + (t.usage?.outTok ?? 0),
      cacheRead: a.cacheRead + (t.usage?.cacheRead ?? 0),
      cacheWrite: a.cacheWrite + (t.usage?.cacheWrite ?? 0),
    }),
    { cost: 0, inTok: 0, outTok: 0, cacheRead: 0, cacheWrite: 0 },
  );
}
