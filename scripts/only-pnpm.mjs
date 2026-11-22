// preinstall guard (HD-01): this project uses pnpm, and pnpm-lock.yaml is the only lockfile.
const agent = process.env.npm_config_user_agent ?? "";
if (!agent.startsWith("pnpm/")) {
  console.error("\nThis project uses pnpm. Run `pnpm install` (see packageManager in package.json); npm and yarn would create a second lockfile.\n");
  process.exit(1);
}
