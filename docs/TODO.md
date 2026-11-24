# Roadmap / TODO

Deferred work and known gaps. None of these block normal 9×9 / 13×13 / 19×19 study or
human-vs-engine play. There is no timeline; items are ordered by expected value, not promise.

## Official scoring (next up)

Two consecutive passes now enter a **scoring phase** (`GamePhase::Scoring`) that blocks normal
moves, stops automatic replies and shows no winner. What is still missing is the confirmation
itself:

- Mark dead stones per connected chain (click a chain to toggle; a partly marked chain is not
  allowed) without touching the recorded move history.
- Area scoring: live stones + single-owned empty points + komi; shared empty points (dame) go to
  neither side. Territory scoring: territory + own captures (including marked dead stones) + komi.
- Show both totals, the difference and draws; never declare a winner before confirmation.
- Only a confirmed result writes `RE`. Decision already taken: **persistence is `RE` only** — dead
  stone marks are scoring-time state and are not serialized (documented in `DESIGN.md`).
- Verify against real positions; do not ship an approximation (e.g. using `area_score()` for
  Japanese/Korean rules) as an official result.

## SGF variation trees

Today `sgf` follows the main line only and ignores variation trees; the copy saved into the game
store therefore loses branches. The original file is never modified (copy-on-open).

| Level | Scope | Rough size |
| --- | --- | --- |
| L0 | Keep branches when saving (write the whole tree, not just the main line) | ~half a day |
| L1 | Read-only tree + switch between branches (show the node path) | 1–2 days |
| L2 | Play moves on any branch (create a new branch) | 2–3 days |
| L3 | Edit the main line (promote/delete nodes, edit comments) | larger; not planned yet |

Touch points: `sgf` tree model, `app`'s `moves + pos` becoming a node path, `write_game`
writing the full tree, TUI/GUI branch switching. Risk: save format, conflict copies, and the
analysis/review caches are keyed by linear move number; L1+ needs a new cache key.

## Product features

- Batch analysis (queue several games; bound concurrency / GPU memory)
- Report export (PDF/HTML/Markdown with diagrams)
- Tags / favourites / search
- Side-by-side comparison (two games or two branches)
- Interactive commentary (step-by-step questions/hints)
- Mistake book generated from review blunders (needs a definition of "mistake")
- Life-and-death / joseki libraries (needs a problem format and source)
- Strength statistics over time (win rate, blunder rate)

## Test gaps

- **Real KataGo / humanv0**: `plays_against_real_engine` and `human_model_returns_a_policy` are
  `#[ignore]` because CI has no model/GPU. Run them locally with
  `cargo test -p app -- --ignored` (needs a KataGo binary, a network, and humanv0), or review a
  real game and check that `humanPolicy` has `size²+1` entries and the engine replies.
- **`.sgf.gz`**: only UTF-8 plain text is read. Decide the semantics first (read-only
  decompression, keep gzip when saving, or save plain text), and handle corrupt archives and
  conflict-copy naming.
- Already automated: GUI file-dialog import (`gui-smoke.sh --import`), TUI import + 9×9 game,
  kitty/sixel encoders (`tui-image-smoke.py`). Real-terminal image rendering was checked
  manually (kitty graphics and sixel).

## Small UX items

1. Make the GUI game list a dialog (it is a drop-down plus rename/delete buttons today).
2. Label the four thinking-time buttons with their values (60/120/300/600).
3. `--game N` is CLI-only; TUI/GUI only open the first game of a collection.
4. `area_score` does not judge life and death and only applies to area scoring.
5. Switch the interface language at runtime in the TUI (only `WEIQI_LANG` at startup today).
6. Add an `i` info overlay to the GUI (the side panel already shows the same information).
7. ~~Official scoring after game over~~: the phase split is done (see above); the remaining work is
   dead-stone marking plus the two scoring algorithms.
