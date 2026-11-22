#!/usr/bin/env bash
# 开发用：把 cargo 包在 nix shell 里，避免本机装 Rust 工具链。
#   ./dev.sh test
#   ./dev.sh run --release -p weiqi-review-cli -- samples/demo.sgf --visits 60
set -euo pipefail
exec nix shell nixpkgs#cargo nixpkgs#rustc -c cargo "$@"
