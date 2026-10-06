// Answers POST /binding with the assets binding's answers to the [method, path]
// requests in its body, and any other request with its method and path.
export default {
  async fetch(request, env) {
    const url = new URL(request.url);
    if (url.pathname !== "/binding") {
      return new Response(`Worker ${request.method} ${url.pathname}`);
    }
    const answers = [];
    for (const [method, path] of await request.json()) {
      const response = await env.STATIC.fetch(new URL(path, url), { method });
      answers.push({ status: response.status, statusText: response.statusText, body: await response.text() });
    }
    return Response.json(answers);
  },
};
