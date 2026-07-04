/** {@link TurnInput} — the shorthand forms a caller can pass as a user turn. */

import type { ContentPart } from "./models/content-part.js";
import type { Message } from "./models/message.js";

/**
 * A user turn expressed as one of three conveniences:
 * - a plain `string` (the common case — wrapped as a single text part),
 * - a ready-made `ContentPart[]` (mixed text/image/tool content), or
 * - a full {@link Message} (whose `content` is used).
 *
 * {@link toContent} normalises any of these into the `content` array the API
 * expects.
 */
export type TurnInput = string | readonly ContentPart[] | Message;
