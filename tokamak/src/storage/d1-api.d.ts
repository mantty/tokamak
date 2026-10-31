// workerd's D1 binding, vendored from src/cloudflare/internal/d1-api.ts.

/** The D1 service a binding sends its queries to. */
export interface D1Fetcher {
  fetch(input: string, init?: RequestInit): Promise<Response>;
}

/** The binding of the database `env.fetcher` answers for. */
export default function makeBinding(env: { fetcher: D1Fetcher }): D1Database;
