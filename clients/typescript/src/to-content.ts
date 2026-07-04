/** Normalises a {@link TurnInput} into the content array the API expects. */

import type { ContentPart } from "./models/content-part.js";
import type { Message } from "./models/message.js";
import type { TurnInput } from "./to-content.types.js";

/**
 * Converts any {@link TurnInput} shorthand into a `ContentPart[]`.
 *
 * @param input - A string, a content-part array, or a {@link Message}.
 * @returns The content parts to send as the user turn.
 */
export function toContent(input: TurnInput): readonly ContentPart[] {
  if (typeof input === "string") {
    return [{ type: "text", text: input }];
  }
  if (isMessage(input)) {
    return input.content;
  }
  return input;
}

/**
 * Distinguishes a {@link Message} from a bare `ContentPart[]`. A message is the
 * only non-array member of the union, so "not an array" is a safe test.
 */
function isMessage(input: readonly ContentPart[] | Message): input is Message {
  return !Array.isArray(input);
}
