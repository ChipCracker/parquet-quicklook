#!/bin/bash
# Builds the Rust static library as a universal binary (arm64 + x86_64) for the Quick Look extension.
# Prefers the rustup toolchain (its stdlib is built for older macOS versions;
# Homebrew's `rust` is only compiled for the host macOS).
set -euo pipefail
export PATH="/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:/opt/homebrew/bin:/usr/local/bin:$PATH"
cd "$(dirname "$0")/../rust"
export SDKROOT="$(xcrun --sdk macosx --show-sdk-path)"
export MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-13.0}"

TARGETS=(aarch64-apple-darwin x86_64-apple-darwin)
LIBS=()
for t in "${TARGETS[@]}"; do
  cargo build --release --lib --target "$t"
  LIBS+=("target/$t/release/libparquet_ql.a")
done
mkdir -p target/universal
lipo -create "${LIBS[@]}" -output target/universal/libparquet_ql.a.tmp
mv target/universal/libparquet_ql.a.tmp target/universal/libparquet_ql.a
