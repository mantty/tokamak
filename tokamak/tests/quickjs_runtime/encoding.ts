import { cases } from "./encoding-cases.ts";

export default {
  fetch(request) {
    const item = cases[Number(new URL(request.url).searchParams.get("case"))];
    let body: BodyInit | null = item.body === "null" ? null : item.body === "empty" ? "" : "payload";
    if (item.body === "stream") body = new ReadableStream({ async start(controller) {
      controller.enqueue(new TextEncoder().encode("pay"));
      await new Promise(resolve => setTimeout(resolve, 1));
      controller.enqueue(new TextEncoder().encode("load"));
      controller.close();
    } });
    const response = new Response(body, { headers: { "content-encoding": item.encoding }, encodeBody: item.mode });
    if (item.copy === "clone") return response.clone();
    if (item.copy === "init") return new Response(response.body, response);
    return response;
  },
} satisfies ExportedHandler;
