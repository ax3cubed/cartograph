import type { NextConfig } from "next";

const config: NextConfig = {
  // The dev server only trusts localhost. Opening the app at 127.0.0.1 would otherwise block
  // its dev scripts, so the page renders but nothing on it responds.
  allowedDevOrigins: ["127.0.0.1"],
};

export default config;
