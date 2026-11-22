#!/usr/bin/env bash
# 开发用：把 cargo 包进 flake 的 devShell，避免本机装 Rust 工具链。
# devShell 里有 cargo/rustc/katago/pkg-config/gtk4/libadwaita，
# 所以 `./dev.sh test --workspace` 能直接编 GUI（裸 nix shell 没有 pkg-config 路径）。
#   ./dev.sh test
#   ./dev.sh run --release -p weiqi-review-cli -- samples/demo.sgf --visits 60
set -euo pipefail
exec nix develop --command cargo "$@"
