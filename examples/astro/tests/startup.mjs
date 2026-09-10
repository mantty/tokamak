const decoder = new TextDecoder();
const startup = decoder.decode(new TextEncoder().encode("ready"));

export default {
  async fetch(...args) {
    if (startup !== "ready") throw new Error("Worker text encoding failed");
    const { default: astro } = await import("../dist/server/entry.mjs");
    return astro.fetch(...args);
  },
};
