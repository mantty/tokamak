import { defineConfig } from "vite";
import { cloudflare } from "@cloudflare/vite-plugin";
import { tokamak } from "@tokamakdev/tok/vite";

export default defineConfig({
  plugins: [cloudflare(), tokamak({ name: "Wasm Example", version: "1.0.0" })],
});
