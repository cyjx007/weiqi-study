# weiqi-study

本地围棋学习工具（Rust）。M0 已可用：**给一盘 SGF，用本机 KataGo 出中文讲评**。

引擎和网络都不下载——直接用 Steam 上那个棋类合集里自带的 KataGo 和 b18c384nbt 权重。

## 快速开始

```bash
cd ~/go-study

# 构建（Rust 工具链走 nix shell，不装进系统）
./dev.sh build --release

# 复盘一盘棋
./target/release/go-review samples/demo.sgf --visits 60

# 看所有手、只标大失误
./target/release/go-review 你的棋谱.sgf --visits 200 --min-loss 8 --all
```

输出长这样：

```
19 路 · 贴目 7.5 · 共 40 手

手数  执  落点   胜率变化                    目差变化
─────────────────────────────────────────────────────────────
  15  黑  R17    38.6% →   4.6%  (-34.0)   -1.0 → -6.9
  ...

整体：平均每手损失 4.8 个百分点。
布局（第 1–50 手）：平均 4.8，最差 -34.0（第 15 手）
最大失误：第 15 手 黑 R17（-34.0）

──── 失误详情 ────
第 15 手 黑 R17：胜率 38.6% → 4.6%（-34.0）。这手是明显失误。
    当时该下 R11（38.9%），之后大致 R11 O3 M3 J17。
    目差 -1.0 → -6.9。
```

## 参数

| 参数 | 作用 |
| --- | --- |
| `--visits N` | 每手推演次数，默认 120；60 够看趋势，200+ 适合细抠 |
| `--min-loss P` | 只显示亏损 ≥ P 个百分点的手，默认 5 |
| `--all` | 显示每一手（含下得好的） |
| `--komi K` | 覆盖棋谱里的贴目 |
| `--no-engine` | 只解析棋谱，不调引擎 |

环境变量：`KATAGO_BIN`、`KATAGO_MODEL`、`KATAGO_CFG`，默认值见 `crates/engine/src/lib.rs`。

## 原理

```
SGF → goban（规则/提子/超级劫） → 逐手 query
      → KataGo analysis（OpenCL，1650 Ti） → review（模板讲评）
```

- 每手 60 visits 时约 **140 局面/分钟**；250 手整盘、120 visits 大约 3–4 分钟
- 胜负/目差内部统一按**黑方视角**存，显示时按落子方翻转
- 阶段划分默认按 19 路缩放：布局 1–50、中盘 51–150、官子 151+

## 目录

```
crates/goban    规则（纯计算，无 IO）
crates/sgf      SGF 解析（跟随主线，跳过分支）
crates/engine   KataGo 进程 + analysis JSON
crates/review   中文讲评模板（确定性输出）
crates/cli      go-review 命令
samples/        示例棋谱
DESIGN.md       架构与设计约束
```

## 已知限制

- SGF 只跟**主线**，不读变化图
- 贴目/规则写死中国规则提交给引擎（日本规则棋盘也能复盘，只是贴目语义略有差异）
- 自带的面积数子（`goban::area_score`）不判死活，仅作粗略显示；真正的形势判断用引擎的 `scoreLead`
- 目前只做了复盘；TUI / GTK4 前端在 `DESIGN.md` 的 M1 / M2

## 下一步

- M1：TUI（摆子、落子、实时胜率、候选点、点失误跳转）
- M2：GTK4 学习窗口
- M3：拟人网对照、错题本导出
