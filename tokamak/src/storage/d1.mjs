import { d1Query } from "tokamak:storage";
import makeBinding from "./d1-api.mjs";

const BOOKMARK_HEADER = "x-cf-d1-session-commit-token";

// The D1 service a binding sends its queries to, answered on the device.
class D1Service {
  #binding;

  constructor(binding) {
    this.#binding = binding;
  }

  async fetch(input, init = {}) {
    const url = new URL(String(input));
    if (url.pathname !== "/query" && url.pathname !== "/execute") return new Response(null, { status: 404 });
    const format = url.searchParams.get("resultsFormat") ?? "";
    const { body, bookmark } = await d1Query(this.#binding, format, String(init.body ?? ""));
    return new Response(body, { headers: { "content-type": "application/json", [BOOKMARK_HEADER]: bookmark } });
  }
}

export function createD1Database(binding) {
  return makeBinding({ fetcher: new D1Service(binding) });
}
