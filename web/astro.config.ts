import { defineConfig } from "astro/config";
import cloudflare from "@astrojs/cloudflare";

// https://astro.build/config
export default defineConfig({
  adapter: cloudflare(),
  server: { port: 4321 },
  vite: {
    server: {
      proxy: {
        "/api": "http://localhost:3000",
        "/xrpc": "http://localhost:3000",
        "/login": "http://localhost:3000",
        "/oauth": "http://localhost:3000",
        "/client-metadata.json": "http://localhost:3000",
        "/jwks.json": "http://localhost:3000",
      },
    },
  },
});
