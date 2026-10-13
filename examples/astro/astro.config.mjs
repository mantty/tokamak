import { defineConfig } from "astro/config";
import cloudflare from "@astrojs/cloudflare";
import tailwindcss from "@tailwindcss/vite";
import { tokamak } from "@tokamakdev/tok/vite";

export default defineConfig({
  output: "server",
  adapter: cloudflare(),
  vite: {
    plugins: [
      tailwindcss(),
      tokamak({
        name: "Astro Tokamak",
        version: "1.0.0",
        // Bundles the sound files in sounds/ for the `sound` field and channels.
        notifications: { sounds: "sounds" },
        // Declares background location, updates off screen, and the reason for the temporary
        // precise location the Location card asks for.
        location: {
          background: true,
          "background-updates": true,
          "accuracy-purposes": { Directions: "Shows directions from where you are" },
        },
      }),
    ],
  },
});
