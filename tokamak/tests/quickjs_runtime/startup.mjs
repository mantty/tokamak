const decoder = new TextDecoder();
const ready = decoder.decode(new TextEncoder().encode("ready"));
const constructors = [TextDecoder, TextEncoder, Response, ReadableStream, process];

export default {
  async fetch(request, env, ctx) {
    if (ready !== "ready") throw new Error("Worker text encoding failed");
    const { run } = await import("./contracts.mjs");
    return new Response(JSON.stringify(await run(env, ctx, constructors)));
  },
};
