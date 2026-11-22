import type { NextConfig } from "next";

const config: NextConfig = {
  // A static export in out/, embedded in the binary and served on loopback (A1, A2, FE-01). No trailingSlash,
  // so links and share links keep their shape.
  output: "export",
  // The dev server only trusts localhost. Opening the app at 127.0.0.1 would otherwise block
  // its dev scripts, so the page renders but nothing on it responds.
  allowedDevOrigins: ["127.0.0.1"],
};

export default config;
