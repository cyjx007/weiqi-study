# weiqi-study — 本地围棋学习工具

[![ci](https://github.com/cyjx007/weiqi-study/actions/workflows/ci.yml/badge.svg)](https://github.com/cyjx007/weiqi-study/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

**在自己的笔记本上，用 KataGo 复盘自己的棋。**

[English](README.md) · 中文

两个前端共用一个规则/分析内核：

- `go-review` —— 喂它一盘 SGF，吐出每一手的胜率/目差变化，以及"你这几手亏在哪"的人话讲评
- `weiqi-tui` —— 终端棋盘：前后翻手、试下、看当前局面的胜率和候选点、整盘复盘后一键跳到失误手

全程本地：不注册、不上传、不用云端引擎。

## 复盘长什么样

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

## 依赖

- **KataGo** + 一个神经网络（`*.bin.gz`）。任何 KataGo 1.15+ 的二进制都行。
  仓库默认指向 Steam 棋类合集里自带的那张网，但**代码不依赖它**——
  把 `KATAGO_MODEL` 指向你自己的网络即可。游戏目录只读引用，不会被改动。
- 显卡可选。KataGo 的 OpenCL 后端让它快得舒服，CPU 也能跑，只是慢。

## 构建

```sh
# 用 Nix（推荐，不往系统里装 Rust）
nix shell nixpkgs#cargo nixpkgs#rustc -c cargo build --release

# 或者用仓库里的包装脚本
./dev.sh build --release
```

产物在 `target/release/`。

## 用法

```sh
# 复盘
./target/release/go-review 棋谱.sgf --visits 120

# 终端学习界面
./target/release/weiqi-tui 棋谱.sgf
```

配置走环境变量：

| 变量 | 默认值 |
| --- | --- |
| `KATAGO_BIN` | `~/go-study/katago/bin/katago` |
| `KATAGO_MODEL` | Steam 合集 `NinOneQi` 目录里的 b18c384nbt |
| `KATAGO_CFG` | `~/go-study/analysis.cfg` |

`--visits N` 用时间换精度：60 看趋势，200+ 抠细节。

### TUI 按键

| 按键 | 作用 |
| --- | --- |
| `hjkl` / 方向键 | 移动光标 |
| `Enter` | 在光标处落子 |
| `u` | 撤回一手 |
| `n` / `p` | 下一手 / 上一手 |
| `g` / `G` | 回到开局 / 跳到末尾 |
| `a` | 分析当前局面 |
| `A` | 全盘复盘 |
| `b` | 跳到下一个失误 |
| `?` | 帮助 |
| `q` / `Esc` | 退出 |

## 架构

```
crates/goban     规则：提子、自杀、位置超级劫、面积数子（无 IO）
crates/sgf       SGF 读取（跟随主线，跳过变化）
crates/engine    KataGo 进程 + analysis JSON → 黑方视角数值
crates/review    确定性中文讲评模板
crates/app       与界面无关的状态（导航、落子/撤回、分析缓存）
crates/cli       go-review
crates/tui       weiqi-tui（ratatui）
```

两条硬约定：

- 胜率/目差内部**一律按黑方视角存**，只在渲染时翻转一次
- SGF/GTP 坐标字符串只出现在 `sgf` 和 `engine`，内核只认 `Point { x, y }`

完整设计与 GTK4 计划见 [DESIGN.md](DESIGN.md)。

## 已知限制

- SGF 只跟主线，变化图不读
- 用 `AB`/`AW` 摆出来的局面按让子棋处理（白先）
- 提交给引擎的规则固定为中国规则；日本规则棋谱也能复盘，只是贴目语义略有差异
- 自带的面积数子不判死活，只做粗略显示；真正的形势判断一律用引擎 `scoreLead`
- 每次分析都会新起一个 KataGo 进程，有约 1.5 秒的模型加载开销

## License

MIT
