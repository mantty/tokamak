import { defineConfig } from "astro/config";
import tailwindcss from "@tailwindcss/vite";
import { tokamak } from "@tokamakdev/tok/astro";

export default defineConfig({
  output: "server",
  adapter: tokamak({
    name: "Astro Tokamak",
    version: "1.0.0",
    // Bundles the sound files in sounds/ for the `sound` field and channels.
    notifications: { sounds: "sounds" },
    // Declares location at all times, updates off screen, and the reason for the temporary
    // precise location the Location card asks for.
    location: {
      "background-always": true,
      "background-updates": true,
      "accuracy-purposes": { Directions: "Shows directions from where you are" },
    },
  }),
  vite: {
    plugins: [tailwindcss()],
  },
});
