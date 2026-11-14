import fs from "node:fs";
import path from "node:path";
import type { NextConfig } from "next";

// Paths a status page may not take (src/lib/status-pages.ts): every top-level entry of src/app and public at build time
const reserved = ["src/app", "public"].flatMap((d) => fs.readdirSync(path.join(__dirname, d)));

// TR_EXPORT=1: only the pages app/**/*.export.tsx, exported as static files for the Rust web app (webapp-rs, pages.rs)
const exported = process.env.TR_EXPORT === "1";

const nextConfig: NextConfig = exported
  ? { output: "export", poweredByHeader: false, pageExtensions: ["export.tsx"], images: { unoptimized: true } }
  : {
      output: "standalone",
      poweredByHeader: false,
      serverExternalPackages: ["better-sqlite3"],
      env: { STATUS_RESERVED: reserved.join(",") },
    };

export default nextConfig;
