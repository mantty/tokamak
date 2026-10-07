import { defineConfig } from "astro/config";
import cloudflare from "@astrojs/cloudflare";
import tailwindcss from "@tailwindcss/vite";
import { tokamak } from "@tokamakdev/tok/vite";

export default defineConfig({
  output: "server",
  adapter: cloudflare(),
  vite: {
    plugins: [tailwindcss(), tokamak({ name: "Astro Tokamak", version: "1.0.0" })],
  },
});
