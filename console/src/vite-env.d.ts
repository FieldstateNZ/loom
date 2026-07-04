/// <reference types="vite/client" />

// Live-gateway configuration read by src/api/context.tsx (resolveLoomConfig).
interface ImportMetaEnv {
  /** Base URL of a running Loom gateway, e.g. https://gateway.example.com. */
  readonly VITE_LOOM_BASE_URL?: string;
  /** Root admin token for the /admin surface (key/tenant provisioning). */
  readonly VITE_LOOM_ADMIN_TOKEN?: string;
  /** A tenant virtual key (loom_…) for the tenant-scoped /v1 surface. */
  readonly VITE_LOOM_API_KEY?: string;
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}
