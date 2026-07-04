// App — the console's root. Resolves the LoomClient (live HTTP client when a
// base URL is configured, otherwise the frozen mock) and provides it to the
// shell. The shell itself lives in console-screen.tsx.
import { useMemo } from "react";
import { LoomProvider, resolveLoomClient } from "./api/context.tsx";
import { ConsoleScreen } from "./console-screen.tsx";

/** Application root: wires the resolved client into the provider + shell. */
export function App() {
  const client = useMemo(() => resolveLoomClient(), []);
  return (
    <LoomProvider client={client}>
      <ConsoleScreen client={client} />
    </LoomProvider>
  );
}
