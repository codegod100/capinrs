#!/bin/bash

# Build WASM module
cd wasm
wasm-pack build --target web --out-dir ../typescript/pkg

# Install dependencies
cd ../typescript
pnpm install

echo "WASM build complete!"