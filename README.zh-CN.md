# weiqi-study

[![ci](https://github.com/cyjx007/weiqi-study/actions/workflows/ci.yml/badge.svg)](https://github.com/cyjx007/weiqi-study/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

**本地学围棋：和引擎下，每步告诉你为什么好、为什么亏。**

[English](README.md) · 中文

weiqi-study 是一套本地围棋学习工具。它驱动本机的 KataGo，把引擎的数字翻译成能读的东西：
这一手亏了多少、引擎会下哪里、之后大概走成什么样。

包含两个前端：`go-review` 读一盘 SGF，输出逐手表格和你失误手的讲评；`weiqi-tui` 是终端
棋盘，可以和引擎对下；每落一子就显示胜率变化、候选点，以及这一手的讲解——由棋盘上的
事实（几路、是否接触、提子、打吃）加上引擎的评估和预演变化拼出来。

它**不含**引擎和神经网络。你需要自己提供 KataGo 二进制和 `*.bin.gz` 网络；不下载、不上传、
不连服务器。

## 依赖

- Linux（在 NixOS 上开发；任何有 Rust 工具链的发行版都能编）
- Rust 1.85+（或用 Nix，工具链由它提供）
- KataGo 1.15+，任一后端（OpenCL / CUDA / Eigen），以及一个 `*.bin.gz` 网络
- 能显示 256 色的终端（TUI 用）
- GTK4 + libadwaita（原生窗口用；Nix 包会带上）

## 安装

不安装直接试：

```bash
nix build github:cyjx007/weiqi-study
./result/bin/go-review --help
./result/bin/weiqi-gui        # 原生窗口
```

或者把两个二进制装进 profile：

```bash
nix profile install github:cyjx007/weiqi-study
```

从源码构建：

```bash
git clone https://github.com/cyjx007/weiqi-study
cd weiqi-study
nix develop          # cargo、rustc、katago
cargo build --release
```

## 用法

每个 shell 先指向你的引擎和网络：

```bash
export KATAGO_BIN=/opt/katago/katago
export KATAGO_MODEL=/opt/katago/networks/model.bin.gz
```

然后：

```bash
# 复盘一盘棋：逐手表格 + 失误讲评
go-review game.sgf --visits 120

# 在学习界面里打开棋谱
weiqi-tui game.sgf

# 或者从空盘开始，按 t 和引擎对下
weiqi-tui

# GTK4 原生窗口（也会出现在应用菜单里）
weiqi-gui
```

图形界面用鼠标：点交叉点落子，悬停预览；按钮对应 TUI 的快捷键（新局 / 分析 / 复盘 / 撤回），
下拉框打开任意棋谱，`EN/中文` 切语言；键盘快捷键和 TUI 一致。

Nix 包装会安装 `packaging/weiqi-study.desktop`，所以 `weiqi-gui` 会出现在启动器里。

每盘棋都会自动存成 SGF，放在 `$XDG_DATA_HOME/weiqi-study/games`（通常是
`~/.local/share/weiqi-study/games`）。在 TUI 里按 `L` 可以随时打开、复盘或继续走。

棋盘保持正方形：格距和行距成对缩放（2×1、4×2、6×3），整体居中；窗口变窄时侧栏自动
隐藏，按 `i` 可以临时覆盖显示。

TUI 按键：

| 按键 | 作用 |
| --- | --- |
| `hjkl` / 方向键 | 移动光标 |
| `Enter` | 在光标处落子 |
| `t` | 开始人机对局（你执黑） |
| `v` | 开关 AI 自动应手 |
| `u` | 撤回一手 |
| `n` / `p` | 下一手 / 上一手 |
| `g` / `G` | 回到开局 / 跳到末尾 |
| `a` | 分析当前局面 |
| `A` | 全盘复盘（生成失误清单） |
| `b` | 跳到下一个失误 |
| `L` | 棋谱列表（打开、复盘或继续） |
| `i` | 开关信息侧栏（窗口窄时用） |
| `?` | 帮助 |
| `q` / `Esc` | 退出 |

首次运行和日常使用只差一次环境配置：

| 任务 | 首次运行 | 之后 |
| --- | --- | --- |
| 复盘 | 设 `KATAGO_BIN` / `KATAGO_MODEL`，再 `go-review game.sgf` | `go-review game.sgf` |
| 学习界面 | 同样的环境变量，再 `weiqi-tui game.sgf` | `weiqi-tui game.sgf` |
| 空盘对局 | `weiqi-tui`，按 `t` | 同上 |

## 配置

环境变量：

| 选项 | 默认值 | 说明 |
| --- | --- | --- |
| `KATAGO_BIN` | `PATH` 里的 `katago` | 引擎可执行文件 |
| `KATAGO_MODEL` | 自动探测 `*.bin.gz` | 要加载的神经网络 |
| `KATAGO_CFG` | `$XDG_CONFIG_HOME/weiqi-study/analysis.cfg` | 分析配置；首次运行时生成 |
| `WEIQI_LANG` | 跟随 `LANG` | `zh` / `en`，界面**和讲评**的语言 |

命令行参数（以 `--help` 为准）：

| 参数 | 默认值 | 说明 |
| --- | --- | --- |
| `--visits N` | `120` | 每个局面推演次数，越大越准越慢 |
| `--min-loss P` | `5` | 只显示亏损 ≥ P 个胜率点的手 |
| `--all` | 关 | 表格里显示每一手 |
| `--komi K` | 取自 SGF（缺省 `7.5`） | 覆盖贴目 |
| `--no-engine` | 关 | 只解析 SGF，不调引擎 |

自动探测会找 `PATH` 里的 `katago`，以及 Steam 棋类合集自带的网络；找不到就显式设置上面
两个变量。

## 工作原理

```text
SGF ──> sgf（主线、摆子）
          │
          ▼
        goban（规则：提子、自杀、位置超级劫）
          │
          ▼
        engine（KataGo analysis JSON，分批提交）
          │   胜率/目差一律存成黑方视角
          ▼
        review（确定性的讲评模板）
          │
          ├──> go-review   文本表格 + 讲评
          └──> app（Controller）+ tui/gui  棋盘、预测、讲解、引擎应手
```

几条硬约定：

- 胜率/目差内部**一律按黑方视角存**，只在渲染时翻转一次
- SGF/GTP 坐标字符串只出现在 `sgf` 和 `engine`，内核只认 `Point { x, y }`
- `review` 是确定性的：不读时钟、不随机，输出可以做快照测试
- 讲解是"推导"出来的，不是模型生成：战术事实来自棋盘，后果和推荐来自引擎的主变化
- 失败模式明确：缺引擎/网络/配置会直接报错；单次分析失败只显示在状态栏，不会自动重试

## 目录结构

```text
crates/goban     规则：提子、自杀、位置超级劫、面积数子
crates/sgf       SGF 读取：跟随主线，跳过变化
crates/engine    KataGo 进程管理与 analysis JSON 解析
crates/review    讲评模板（纯函数、确定性）
crates/app       与界面无关的状态 + 共用 Controller（分析线程、AI 推进）
crates/cli       go-review
crates/tui       weiqi-tui（ratatui）
crates/gui       weiqi-gui（GTK4 + libadwaita，Cairo 棋盘）
packaging/       图形界面的桌面入口
nix/package.nix  构建两个二进制的 Nix 包
flake.nix        flake 输出：package 与 dev shell
samples/         TUI 无参数时打开的示例棋谱
```

## 开发

```bash
cargo test --workspace              # 单元测试：不需要 GPU 和引擎
cargo test -p app -- --ignored      # 端到端：需要本机 KataGo + 网络
cargo build --release
nix develop                         # 图形界面需要 GTK4 / libadwaita / pkg-config
nix build .#default                 # 用 Nix 构建全部二进制
```

CI 会先装 GTK4 和 libadwaita，再跑 `cargo test --workspace` 和
`cargo build --release --workspace`。

## 注意

- SGF 只跟主线，变化图不读
- 用 `AB`/`AW` 摆出来的局面按让子棋处理（白先）
- 提交给引擎的规则固定为中国规则；日本规则棋谱也能复盘，只是贴目语义略有差异
- 自带的面积数子不判死活，只做粗略显示；真正的形势判断用引擎的 `scoreLead`
- 第一次分析时启动 KataGo，之后整个会话复用同一个进程（不再重复加载模型）；
  关掉 TUI 或窗口时引擎随之退出
- 界面和讲评都支持中/英（`WEIQI_LANG`，默认跟随 `LANG`）；`go-review` 同样跟随
- 每落一子自动保存棋谱，工具本身不删除任何棋谱

## 许可

MIT — 见 [LICENSE](LICENSE)。
