async function echo(request) {
  const bodyIsStream = request.body instanceof ReadableStream;
  const body = new Uint8Array(await request.arrayBuffer());
  return Response.json({
    method: request.method,
    bodyLength: body.length,
    bodyIsStream,
    contentType: request.headers.get("content-type"),
  });
}

function stream() {
  const encoder = new TextEncoder();
  const body = new ReadableStream({
    async start(controller) {
      for (const chunk of ["alpha-", "beta-", "gamma"]) {
        controller.enqueue(encoder.encode(chunk));
        await new Promise(resolve => setTimeout(resolve, 5));
      }
      controller.close();
    },
  });
  return new Response(body, { headers: { "content-type": "text/event-stream" } });
}

async function egress(port) {
  const response = await fetch(`http://127.0.0.1:${port}/upstream`);
  return Response.json({
    status: response.status,
    body: await response.text(),
    cookies: response.headers.getSetCookie(),
  });
}

export default {
  async fetch(request, env) {
    const url = new URL(request.url);
    if (url.pathname === "/echo") return echo(request);
    if (url.pathname === "/stream") return stream();
    if (url.pathname === "/egress") return egress(env.UPSTREAM_PORT);
    return new Response("not found", { status: 404 });
  },
};
