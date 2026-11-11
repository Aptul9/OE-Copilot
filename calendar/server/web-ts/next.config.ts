import type { NextConfig } from "next";

// OE_ESPORTA=1: solo le pagine app/**/*.esporta.tsx, esportate come file statici per il gateway in Rust (agente-rs/gateway, pagine.rs)
const esporta = process.env.OE_ESPORTA === "1";

const nextConfig: NextConfig = esporta
  ? { output: "export", poweredByHeader: false, pageExtensions: ["esporta.tsx"], images: { unoptimized: true } }
  : { output: "standalone", poweredByHeader: false, serverExternalPackages: ["better-sqlite3"] };

export default nextConfig;
