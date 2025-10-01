# Agent Configuration

This project uses pnpm as the package manager.

## Package Management

- Use `pnpm install` to install dependencies
- Use `pnpm add <package>` to add new dependencies
- Use `pnpm run <script>` to run npm scripts

## Why pnpm?

pnpm uses a content-addressable filesystem to store packages, which means:

- Faster installations
- Better disk space usage
- Strict dependency resolution
- Improved security

## Common Commands

```bash
# Install dependencies
pnpm install

# Add a dependency
pnpm add <package-name>

# Add a dev dependency
pnpm add -D <package-name>

# Run a script
pnpm run build
pnpm run dev
pnpm run test

# Update dependencies
pnpm update
```