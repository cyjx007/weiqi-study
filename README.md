# weiqi-study — local Go study tools

[![ci](https://github.com/cyjx007/weiqi-study/actions/workflows/ci.yml/badge.svg)](https://github.com/cyjx007/weiqi-study/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

**Review your own games with KataGo, offline, on a laptop GPU.**

English · [中文](README.zh-CN.md)

Two front-ends share one rules/analysis core:

- `go-review` — feed it an SGF, get a per-move winrate/score table and a plain-language review
  of your worst moves.
- `weiqi-tui` — a terminal board: navigate the game, try variations, ask for the current
  position's evaluation, run a full-game review and jump straight to blunders.

Everything runs locally. No accounts, no uploads, no cloud engine.

## What a review looks like

```
19 路 · 贴目 7.5 · 共 40 手

手数  执  落点   胜率变化                    目差变化
─────────────────────────────────────────────────────────────
  15  黑  R17    38.6% →   4.6%  (-34.0)   -1.0 → -6.9
  12  白  R10    65.5% →  55.7%   (-9.7)   +1.3 → +0.3
  ...

整体：平均每手损失 4.8 个百分点。
布局（第 1–50 手）：平均 4.8，最差 -34.0（第 15 手）
最大失误：第 15 手 黑 R17（-34.0）

──── 失误详情 ────
第 15 手 黑 R17：胜率 38.6% → 4.6%（-34.0）。这手是明显失误。
    当时该下 R11（38.9%），之后大致 R11 O3 M3 J17。
    目差 -1.0 → -6.9。
```

The narrative is currently Chinese-only; the numbers are language-neutral.

## Requirements

- **KataGo** with a neural net (`*.bin.gz`). Any KataGo 1.15+ binary works.
  This repo defaults to a net that ships with a Steam board-game collection, but nothing
  depends on it — point `KATAGO_MODEL` at any net you have.
- A GPU is optional. KataGo's OpenCL backend is what makes it pleasant; CPU works too,
  just slower.

## Build

```sh
# with Nix
nix shell nixpkgs#cargo nixpkgs#rustc -c cargo build --release

# or use the wrapper
./dev.sh build --release
```

Binaries land in `target/release/`.

## Run

```sh
# review a game
./target/release/go-review game.sgf --visits 120

# terminal study board
./target/release/weiqi-tui game.sgf
```

Configuration is by environment variable:

| Variable | Default |
| --- | --- |
| `KATAGO_BIN` | `~/go-study/katago/bin/katago` |
| `KATAGO_MODEL` | net from the Steam collection's `NinOneQi` directory |
| `KATAGO_CFG` | `~/go-study/analysis.cfg` |

`--visits N` trades time for accuracy: 60 is fine for spotting trends, 200+ for serious review.

### TUI keys

| Key | Action |
| --- | --- |
| `hjkl` / arrows | move cursor |
| `Enter` | play at cursor |
| `u` | undo one move |
| `n` / `p` | next / previous move |
| `g` / `G` | jump to start / end |
| `a` | analyse current position |
| `A` | full-game review |
| `b` | jump to next blunder |
| `?` | help |
| `q` / `Esc` | quit |

## Architecture

```
crates/goban     rules: captures, suicide, positional superko, area scoring (no IO)
crates/sgf       SGF reader (follows the main line, skips variations)
crates/engine    KataGo process + analysis JSON -> black-perspective values
crates/review    deterministic Chinese commentary templates
crates/app       UI-independent state (navigation, play/undo, cached analysis)
crates/cli       go-review binary
crates/tui       weiqi-tui binary (ratatui)
```

Two invariants keep the code honest:

- Winrates and score leads are stored **from Black's point of view** and flipped exactly once,
  at render time.
- Coordinate strings (SGF/GTP) exist only in `sgf` and `engine`; the core works on
  `Point { x, y }`.

See [DESIGN.md](DESIGN.md) (Chinese) for the full design and the GTK4 plan.

## Known limits

- SGF: main line only; variation trees are ignored.
- Analysis queries use Chinese rules; Japanese-rules games still review fine, komi semantics differ slightly.
- The built-in area scorer does not judge life and death — it is a rough display only. Real
  evaluation always comes from KataGo's `scoreLead`.
- Each analysis spawns a fresh KataGo process, so there is a ~1.5 s model-load cost each time.

## License

MIT
