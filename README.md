# weiqi-study

[![ci](https://github.com/cyjx007/weiqi-study/actions/workflows/ci.yml/badge.svg)](https://github.com/cyjx007/weiqi-study/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

**Study Go on your own machine: play the engine, see why each move was good or bad.**

English · [中文](README.zh-CN.md)

weiqi-study is a small toolkit for learning Go locally. It drives a local KataGo engine and
turns its numbers into things you can read: how much a move cost, what the engine would have
played instead, and what the follow-up would look like.

It ships two front-ends. `go-review` takes an SGF and prints a per-move table plus a written
review of your worst moves. `weiqi-tui` is a terminal board for playing against the engine:
after every move it shows the evaluation, the top candidate moves, and a one-paragraph
explanation of the move just played.

It does **not** include the engine or the neural network. You point it at a KataGo binary and
a `*.bin.gz` network; nothing is downloaded, uploaded, or sent to a server.

## Requirements

- Linux (developed on NixOS; any distribution with a Rust toolchain works)
- Rust 1.85+ (or Nix, which brings its own toolchain)
- KataGo 1.15+ with any backend (OpenCL, CUDA, or Eigen) and one `*.bin.gz` network
- A terminal with 256-color support for the TUI
- GTK4 and libadwaita for the native window (the Nix package provides them)

## Install

Try it without installing anything:

```bash
nix build github:cyjx007/weiqi-study
./result/bin/go-review --help
./result/bin/weiqi-gui        # native window
```

Or install both binaries into a profile:

```bash
nix profile install github:cyjx007/weiqi-study
```

From a checkout:

```bash
git clone https://github.com/cyjx007/weiqi-study
cd weiqi-study
nix develop          # cargo, rustc, katago
cargo build --release
```

## Usage

Point the tools at your engine and network once per shell:

```bash
export KATAGO_BIN=/opt/katago/katago
export KATAGO_MODEL=/opt/katago/networks/model.bin.gz
```

Then:

```bash
# review a recorded game; prints a table and the worst-move explanations
go-review game.sgf --visits 120

# open a game in the study board
weiqi-tui game.sgf

# choose the board renderer (auto probes kitty/sixel/iTerm2, text fallback)
weiqi-tui game.sgf --mode auto

# or start from an empty board and press t to play the engine
weiqi-tui

# native GTK4 window (also available from the desktop menu)
weiqi-gui
```

The GUI uses the mouse: click an intersection to play, hover to preview. Buttons mirror the TUI
keys (`New`, `Analyse`, `Review`, `Undo`), the game dropdown opens any saved game, and `EN/中文`
switches the interface language. Keyboard shortcuts are the same as the TUI.

The Nix package installs `packaging/weiqi-study.desktop`, so `weiqi-gui` appears in the
application launcher.

Every game is saved automatically as SGF under `$XDG_DATA_HOME/weiqi-study/games` (usually
`~/.local/share/weiqi-study/games`). Press `L` in the TUI to reopen any of them, review, or
continue playing.

TUI keys:

| Key | Action |
| --- | --- |
| `hjkl` / arrows | move the cursor |
| `Enter` | play at the cursor |
| `t` | start a human-vs-engine game (you are Black) |
| `v` | toggle automatic engine replies |
| `u` | undo one move |
| `n` / `p` | next / previous move |
| `g` / `G` | jump to start / end |
| `a` | analyse the current position |
| `A` | full-game review (builds a blunder list) |
| `b` | jump to the next blunder |
| `L` | game list: open a saved game (review or continue) |
| `i` | toggle the info panel (useful when the window is narrow) |
| `?` | help |
| `q` / `Esc` | quit |

First run and daily use differ only in setup:

| Task | First run | Afterwards |
| --- | --- | --- |
| Review a game | set `KATAGO_BIN` / `KATAGO_MODEL`, then `go-review game.sgf` | `go-review game.sgf` |
| Study board | same env vars, then `weiqi-tui game.sgf` | `weiqi-tui game.sgf` |
| Play from scratch | `weiqi-tui`, press `t` | same |

## Configuration

Environment variables:

| Option | Default | Meaning |
| --- | --- | --- |
| `KATAGO_BIN` | `katago` from `PATH` | Engine executable |
| `KATAGO_MODEL` | auto-detected `*.bin.gz` | Neural network to load |
| `KATAGO_CFG` | `$XDG_CONFIG_HOME/weiqi-study/analysis.cfg` | Analysis config; generated on first use |
| `WEIQI_LANG` | from `LANG` | `zh` or `en` for the interface **and the commentary** |

Command-line flags (`--help` is authoritative):

| Flag | Default | Meaning |
| --- | --- | --- |
| `--visits N` | `120` | Visits per position: more is slower and stronger |
| `--mode auto\|image\|text` | `auto` | Board renderer: `auto` probes kitty/sixel/iTerm2 and falls back to the text grid; `image` errors if unsupported; `text` forces the dot grid |
| `--min-loss P` | `5` | Only show moves that lost at least P winrate points |
| `--all` | off | Show every move in the table |
| `--komi K` | from SGF (`7.5` fallback) | Override komi |
| `--no-engine` | off | Parse the SGF only, skip the engine |

Auto-detection looks for `katago` on `PATH` and for a network shipped with a Steam
board-game collection; set the variables explicitly if either is missing.

## How it works

```text
SGF ──> sgf (main line, setup stones)
          │
          ▼
        goban (rules: captures, suicide, positional superko)
          │
          ▼
        engine (KataGo analysis JSON, batched)
          │   winrate/score → always stored from Black's point of view
          ▼
        review (deterministic commentary templates)
          │
          ├──> go-review   (text table + explanations)
          └──> app (Controller) + tui/gui  (board, prediction, commentary, engine replies)
```

Key invariants:

- Winrates and score leads are stored **from Black's point of view** and flipped exactly once,
  at render time.
- SGF and GTP coordinate strings exist only in `sgf` and `engine`; the core works on
  `Point { x, y }`.
- `review` is deterministic: no clock, no randomness, so its output can be snapshot-tested.
- Explanations are derived, not generated: tactical facts come from the board, consequences and
  recommendations come from the engine's principal variation.
- Failure modes are explicit: a missing engine, network, or config produces an error; an
  analysis error is shown in the status line and is not retried automatically.

## Project layout

```text
crates/goban     rules only: captures, suicide, positional superko, area scoring
crates/sgf       SGF reader: follows the main line, skips variations
crates/engine    KataGo process management and analysis JSON parsing
crates/review    commentary templates (pure, deterministic)
crates/app       UI-independent state + shared Controller (analysis threads, AI pump)
crates/cli       go-review binary
crates/tui       weiqi-tui binary (ratatui)
crates/gui       weiqi-gui binary (GTK4 + libadwaita, Cairo board)
packaging/       desktop entry for the GUI
nix/package.nix  Nix package building both binaries
flake.nix        flake outputs: package and dev shell
samples/         demo SGF used by the TUI fallback
```

## Development

```bash
cargo test --workspace              # unit tests: no GPU, no engine needed
cargo test -p app -- --ignored      # end-to-end: needs a local KataGo + network
cargo build --release
nix develop                         # GTK4 / libadwaita / pkg-config for the GUI
nix build .#default                 # builds all binaries via Nix
```

CI installs GTK4 and libadwaita, then runs `cargo test --workspace` and
`cargo build --release --workspace` on every push.

## Notes

- SGF: main line only; variation trees are ignored.
- Positions set up with `AB`/`AW` are treated as handicap games (White moves first).
- Engine queries use Chinese rules; Japanese-rules records still review, but komi semantics differ.
- The built-in area scorer does not judge life and death — it is a rough display only; real
  evaluation comes from KataGo's `scoreLead`.
- The first analysis starts KataGo and keeps it alive for the rest of the session; later requests
  skip the model load. Closing the TUI or the GUI ends the engine process.
- The interface and the written commentary are English or Chinese (`WEIQI_LANG`, default from `LANG`); `go-review` follows the same setting.
- Games are saved automatically after every move; nothing is deleted by the tool.

## License

MIT — see [LICENSE](LICENSE).
