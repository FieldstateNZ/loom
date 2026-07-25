import type { Event } from '@oasp/schemas';

/**
 * The transcript-seeding payload `createSession` accepts to realize
 * `migrate`'s Stage 2 (transcript-seed with a suppression marker). The
 * server is responsible for producing a flat, already-non-compounded
 * `events` array (see `docs/spec/interactions.md`'s non-compounding
 * rule); the adapter's obligation begins once this is handed to it —
 * see `docs/spec/adapters.md` § the `seed` transport for the full
 * normative treatment of what a conformant adapter must and must not
 * do with it.
 */
export interface SeedTranscript {
  /** The flattened, ordered transcript to seed into the newly created session, treated as already exchanged. */
  readonly events: readonly Event[];
}
