# weiqi-study

[![ci](https://github.com/cyjx007/weiqi-study/actions/workflows/ci.yml/badge.svg)](https://github.com/cyjx007/weiqi-study/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

**Study Go on your own machine: play the engine, see why each move was good or bad.**

English · [中文](README.zh-CN.md)

weiqi-study is a small toolkit for learning Go locally. It drives a local KataGo engine and
turns its numbers into things you can read: how much a move cost, what the engine would have
played instead, and what the follow-up would look like.

It ships two interactive front-ends (TUI and GTK4) plus a CLI. `go-review` takes an SGF and prints a per-move table plus a written
review of your worst moves. `weiqi-tui` is a terminal board for playing against the engine:
after every move it shows the evaluation, the top candidate moves, and a one-paragraph
explanation of the move just played.

It does **not** include the engine or the neural network. You point it at a KataGo binary and
a `*.bin.gz` network; nothing is downloaded, uploaded, or sent to a server.

## Requirements

- Linux (developed on NixOS; any distribution with a Rust toolchain works)
- Rust 1.92+ (or Nix, which brings its own toolchain)
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

Or install all three binaries into a profile:

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
weiqi-gui --version            # all three binaries can self-report their version
```

`weiqi-gui` takes no game path: it opens the most recent readable game in the store (or an
empty board). The **Import** button (or the `o` key) opens an SGF via a file dialog and copies it into the library.
`-h/--help` and `-V/--version` are handled before GTK starts, so they work without a display.

The GUI uses the mouse: click an intersection to play, hover to preview. Buttons mirror the TUI
keys (`New`, `Analyse`, `Review`, `Undo`), the game dropdown opens any saved game, and the
`Rename` / `Delete` buttons manage the selected file. A strength card sets visits and toggles the
humanv0 net/rank; `Variation` previews the engine line and `Use` commits it. `EN/中文` switches
the interface language. Board navigation and the main actions use the same keys as the TUI where available; the GUI
adds buttons for importing an SGF, passing (`Pass` / `P`), continuing from a two-pass position
(`Study on`), the game list, rename/delete and AI settings, and `?` opens a shortcut cheat-sheet.

The Nix package installs `packaging/weiqi-study.desktop`, so `weiqi-gui` appears in the
application launcher.

Every game is saved automatically as SGF under `$XDG_DATA_HOME/weiqi-study/games` (usually
`~/.local/share/weiqi-study/games`). Press `L` in the TUI to reopen any of them, review, or
continue playing. Deleting a game moves it to `games/.trash/` instead of erasing it, so a
mis-click can be recovered by hand.

`m` switches the engine to the humanv0 human-like net (if `KATAGO_HUMAN_MODEL` or the Steam
copy is found). The AI then samples its move from KataGo's `humanPolicy` instead of taking the
top search move; `[`/`]` select ranks from 20k to 9d. This is separate from `--visits`: visits
control how much normal search the analysis does, while the human net controls the style.

TUI keys:

| Key | Action |
| --- | --- |
| `hjkl` / arrows | move the cursor |
| `Enter` | play at the cursor |
| `t` | start a human-vs-engine game (keeps your side) |
| `s` | swap sides (you play Black / White) |
| `T` | start a new game and pick 9×9 / 13×13 / 19×19 |
| `o` | import an SGF by typing a path |
| `P` | pass (stop for this turn) |
| `R` | keep studying from a two-pass position (passes are kept) |
| `d` / `Enter` | scoring phase: mark / unmark a whole dead chain |
| `X` | clear all dead marks |
| `Y` | confirm the score and write `RE` |
| `v` | toggle automatic engine replies |
| `u` | undo one move |
| `n` / `p` | next / previous move |
| `g` / `G` | jump to start / end |
| `a` | analyse the current position |
| `A` | full-game review (builds a blunder list) |
| `b` | jump to the next blunder |
| `V` | preview the engine's top variation (then `n`/`p` step, `c` use, `Esc` exit) |
| `+` / `-` | change analysis strength (visits) |
| `m` | toggle the humanv0 human-like net |
| `[` / `]` | cycle the human rank (when the human net is on) |
| `L` | game list: open a saved game (review or continue); `d` delete to trash, `r` rename |
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
| `KATAGO_HUMAN_MODEL` | auto-detected `*humanv0*.bin.gz` | Human SL network for `m`; set it if auto-detection misses |
| `WEIQI_LANG` | from `LANG` | `zh` or `en` for the interface **and the commentary** |

Command-line flags (`--help` is authoritative):

| Flag | Default | Applies to | Meaning |
| --- | --- | --- | --- |
| `--visits N` | `120` | both | Visits per position, 10–5000: more is slower and stronger |
| `--mode auto\|image\|text` | `auto` | `weiqi-tui` | Board renderer: `auto` probes kitty/sixel/iTerm2 and falls back to the text grid; `image` errors if unsupported; `text` forces the dot grid |
| `--min-loss P` | `5` | `go-review` | Only show moves that lost at least P winrate points (finite, ≥ 0) |
| `--all` | off | `go-review` | Show every move in the table |
| `--komi K` | from SGF (`7.5` fallback) | `go-review` | Override komi (negative values allowed) |
| `--no-engine` | off | `go-review` | Parse the SGF only, skip the engine |
| `--strict` | off | `go-review` | Treat any SGF warning as an error |
| `--game N` | `1` | `go-review` | Pick the N-th game when the file is an SGF collection |
| `-V`, `--version` | — | both | Print the version (useful after `nix profile upgrade`) |

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
crates/goban     rules only: captures, suicide, RuleSet (ko + scoring), area scoring
crates/sgf       SGF reader: follows the main line, skips variations
crates/engine    KataGo process management and analysis JSON parsing
crates/review    commentary templates (pure, deterministic)
crates/app       UI-independent state + shared Controller (analysis threads, AI pump)
crates/cli       go-review binary
crates/tui       weiqi-tui binary (ratatui)
crates/gui       weiqi-gui binary (GTK4 + libadwaita, Cairo board)
packaging/       desktop entry for the GUI
docs/            TODO.md: deferred work and known gaps
scripts/         gui-smoke.sh: headless GUI smoke test used by CI
                 tui-smoke.py: pty-driven TUI smoke test used by CI
                 tui-image-smoke.py: kitty/sixel encoder smoke (no visual check)
nix/package.nix  Nix package building all three binaries
flake.nix        flake outputs: package and dev shell
samples/         demo SGF opened in memory by the TUI fallback (never copied into the library)
```

## Development

```bash
cargo test --workspace              # unit tests: no GPU, no engine needed
cargo test -p app -- --ignored      # end-to-end: needs a local KataGo + network
cargo build --release
nix develop                         # GTK4 / libadwaita / pkg-config for the GUI
nix build .#default                 # builds all binaries via Nix

scripts/gui-smoke.sh                # GUI smoke test under Xvfb (needs Xvfb, optional xdotool)
scripts/gui-smoke.sh --switch --import  # also switch games and import an SGF via the dialog
WEIQI_TUI_BIN=target/release/weiqi-tui \
  python3 scripts/tui-smoke.py       # TUI: start → list → analyse (no engine) → help → import → 9×9 game
WEIQI_TUI_BIN=target/release/weiqi-tui \
  python3 scripts/tui-image-smoke.py # image encoders: force kitty / sixel in a pty, check escape sequences
```

CI installs GTK4 and libadwaita, runs `cargo fmt`, `cargo clippy`, `cargo test --workspace`,
`cargo build --release --workspace`, then starts the GUI under Xvfb with
`scripts/gui-smoke.sh --switch --import` and fails if it crashes, panics, idles above 80% CPU, or the
file-dialog import does not land in the library; it then runs `scripts/tui-smoke.py` in a real pty
(start → games list → analyse with a missing engine → help → import → 9×9 game) and
`scripts/tui-image-smoke.py` (kitty/sixel encoders, not a visual test). A separate `msrv (1.92)`
job runs `cargo check --workspace --all-targets` on the declared minimum toolchain.

Deferred work and known gaps are tracked in [docs/TODO.md](docs/TODO.md).

### Real-terminal image check (manual)

`tui-image-smoke.py` only proves the encoder emits graphics escapes. Look at the real thing once in
each terminal family:

```bash
kitty weiqi-tui --mode image samples/demo.sgf        # kitty graphics
nix shell nixpkgs#foot --command foot weiqi-tui --mode image samples/demo.sgf   # sixel
```

Check: the board is a real image (wood texture, round stones), `n`/`p` and `V` redraw, resizing
does not blur the stones, and `--mode text` still gives the dot grid. Run `--mode image` in a
non-image terminal (e.g. `xterm`) and confirm it fails with a clear error instead of a blank board.

## Notes

- SGF: supports `SZ` `KM` `RU` `PL` `HA` `RE` `AB` `AW` `AE` `B` `W`; the rest is metadata. `AE` clears setup stones (mid-game `AE` is rejected). Common metadata (`GM/FF/CA/PB/PW/DT/C/LB`...) is read and ignored silently; genuinely unknown properties produce one summarised warning.
- SGF: main line only; variation trees in the file are ignored. `V` previews the engine PV, not SGF branches. Root `PL` (explicit next player) is honoured; handicap setups default to White to move.
- Because only the main line is kept, the copy the tool saves into `games/` loses any extra branches. The original file is never modified (copy-on-open), so keep it if you need the variations; `go-review` likewise analyses the main line only.
- SGF collections (several game trees in one file) are parsed in full: the TUI/GUI open the first game with a visible warning, `go-review --game N` picks another. Leading/trailing junk is a parse error with a character position, never silently ignored.
- Illegal SGF/GTP coordinates are parse errors, never silently turned into passes. Two consecutive passes are legal and end the game.
- Malformed `KM`, out-of-turn moves and unrecognised `PL` are reported as warnings (status bar / stderr); `go-review --strict` turns them into errors.
- Engine queries follow the SGF `RU` through an explicit rule set: Chinese, Japanese, Korean,
  AGA and Ing are passed to KataGo verbatim (`chinese` / `japanese` / `korean` / `aga` / `ing`).
  Unknown values, and New Zealand (self-capture is not supported), fall back to Chinese and
  produce a visible warning — never a silent downgrade. Ing is approximated with situational
  superko; its cycle/special-end rules are not fully implemented.
- SGF board sizes 2–25 are accepted and passed to KataGo unchanged. KataGo's official builds
  default to `MAX_LEN=19`: 20×20 and larger need a custom build compiled with larger-board
  support, and the official networks were not trained on those sizes. If the engine rejects the
  size, its message is shown as-is with that hint in the status line.
- Two consecutive passes only end the **normal playing phase** and enter *scoring* (awaiting
  life-and-death confirmation): moves and passes are rejected, automatic replies stop, but the
  engine may still fill in one estimate. No winner is shown and no `RE` is written until a result
  is confirmed.
- In the scoring phase you mark **whole chains** dead or alive (TUI `d` / `Enter`, GUI click on a
  stone; `X` clears). Marks never modify the move history, and unmarked stones count as alive.
  Area rules score "live stones + single-owned empty points + komi"; Japanese/Korean score
  "territory + own captures + confirmed dead stones + komi". Empty points touching both colours
  (dame) belong to neither side.
- `Y` / the GUI **Confirm result** button turns the preview into `B+3.5` / `W+0.5` / `0` (draw)
  and writes it as the game's `RE`. Dead-stone marks themselves are not saved — reopening a game
  only shows the `RE`.
- **Study on** (TUI `R`, GUI button) keeps both passes and continues after them; the first new
  move voids any confirmed result and the marks. Don't simulate this by undoing a pass — that
  deletes the pass history and creates a different branch.
- The result line follows the rule set: area rules (Chinese / AGA / Ing) show area scoring and the
  point difference; Japanese / Korean show territory scoring and the score difference. Both show
  totals, komi, draws and the number of marked dead stones. The engine's `scoreLead` is always
  labelled "engine estimate (Black's view)" — not an AI lead, and not an official result.
- Known limits: strict Japanese life-and-death (seki territory, dispute resumption) is not
  implemented and scoring trusts your marks; AGA / Ing remain area approximations, and Ing cycles
  are not fully handled.
- You choose the side you play (Black or White). The choice is kept when you start a new game,
  and switching sides during a human-vs-engine game makes the engine take its move immediately
  (switching to White on an empty board means the engine opens). The GUI exposes the same choice
  as the "You play" row in AI settings.
- Opening a game from the library puts the board into study mode: AI auto-replies are switched
  off (`ai_enabled=false`) so the game is not re-analysed under you; the human-model preference
  is kept for the next game you start.
- The built-in area scorer does not judge life and death — it is a rough display only; real
  evaluation comes from KataGo's `scoreLead`.
- The first analysis starts KataGo and keeps it alive for the rest of the session; later requests
  skip the model load. Closing the TUI or the GUI ends the engine process.
- The interface and the written commentary are English or Chinese (`WEIQI_LANG`, default from `LANG`); `go-review` follows the same setting.
- Games are saved automatically after every move; nothing is deleted by the tool. An untouched new
  game is not written at all, so quitting right after `t` does not leave an unreadable file.
- On startup the GUI opens the most recent **parseable** game in the library, skipping corrupted
  entries (the reason is shown in the status line if none can be opened).
- Saves are atomic: a uniquely named temp file (`create_new`), `fsync`, rename, then a directory
  `fsync`. If the file changed on disk since it was opened (another process edited it), the tool
  does not overwrite it — it writes a `*.conflict-<nanos>-<pid>.sgf` copy and switches to it.

## License

MIT — see [LICENSE](LICENSE).
