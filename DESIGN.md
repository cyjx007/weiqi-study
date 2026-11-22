# weiqi-study 设计文档

> 2026-11-22 · 本地围棋学习工具（TUI + GTK4 双前端）
> 目标：纯本地、不依赖浏览器、单人自学；引擎用本机 KataGo + 游戏目录里的网络。

## 一、技术选型

### 结论：Rust 单一语言，一个 core，两个前端

| 维度 | Rust（选定） | Python（备选） |
|---|---|---|
| 规则正确性 | 强类型 + 模式匹配，劫/禁入点这类状态机不容易漏 | 也能写，但重构时靠自觉 |
| 两端一致性 | 两个前端链接**同一个** core crate，不可能漂移 | core 是包，也能共享，但打包/环境更麻烦 |
| 分发 | 单二进制，`rustPlatform.buildRustPackage` 一行 | 依赖 GTK4/PyGObject 运行时，要 wrapGAppsHook |
| 启动/内存 | 快、小 | Python + GTK 起步 ~80 MB |
| 迭代速度 | 慢一点（编译） | 快 |
| 本机工具链 | `nix shell nixpkgs#{cargo,rustc}`（1.99） | python3 3.14 已有 |

选 Rust 的决定性理由：**规则引擎必须只有一个**。TUI 和 GUI 各自实现提子/劫争，迟早出现"同一个 SGF 两边结果不同"，那对学棋是灾难。

反悔条件（满足其一就换 Python）：一周内写不出可用的 M1（TUI），或者你发现自己主要想改 UI 而不是写规则。

### 依赖版本（已确认在 nixpkgs）

```
gtk4              4.24.0
libadwaita        1.10.0
rustc / cargo     1.99.1
```

Rust crates 走 crates.io：`ratatui`、`crossterm`、`gtk4`、`libadwaita`、`glib`、`anyhow`、`thiserror`、`serde`、`serde_json`。

## 二、架构

```
                 ┌─────────────────────────────┐
                 │           crates/           │
                 │                             │
  输入事件 ──►   │  tui (ratatui)  gui (gtk4)  │  ◄── 两个前端只做「画 + 输入」
                 │        │             │      │
                 │        └──── 命令 ────┘      │
                 │              │              │
                 │        ┌─────▼─────┐        │
                 │        │  app-core │        │  ◄── AppState + Controller，纯逻辑 + 后台线程
                 │        └─────┬─────┘        │
                 │    ┌─────────┼─────────┐    │
                 │    ▼         ▼         ▼    │
                 │  goban     sgf      review  │  ◄── 纯函数，无 IO
                 │  (规则)   (读写)    (讲评)   │
                 │              │              │
                 │        ┌─────▼─────┐        │
                 │        │  engine   │        │  ◄── 唯一碰进程/IO 的地方
                 │        └─────┬─────┘        │
                 └──────────────┼──────────────┘
                                ▼
                        KataGo (OpenCL)
                     GTP 长驻 / analysis 批处理
```

### 目录

```
weiqi-study/               # 实际放在 weiqi-study/
├─ Cargo.toml                # workspace
├─ crates/
│  ├─ goban/                 # 规则：Board/Position/Game、提子、劫、禁入点、面积数子
│  ├─ sgf/                   # SGF 子集：SZ/KM/RU/B/W/AB/AW/C，只读主线
│  ├─ engine/                # KataGo 进程管理 + 协议
│  ├─ review/                # 讲评模板（纯函数）
│  ├─ app/                   # AppState（导航/落子/分析缓存）+ Controller（spawn/tick），M1 已实现
│  ├─ cli/   [bin go-review] # 批量复盘（M0 已实现）
│  ├─ tui/   [bin weiqi-tui] # ratatui 前端
│  └─ gui/   [bin weiqi-gui] # gtk4 + libadwaita 前端
```

### crate 边界（评审重点：越界就是 bug）

| crate | 允许 | 禁止 |
|---|---|---|
| `goban` | 纯计算、无分配竞赛、可 `no_std` 化 | 任何 IO、任何字符串坐标转换、任何 UI 概念 |
| `sgf` | 文件内容字符串 ↔ 结构体 | 知道引擎存在 |
| `engine` | spawn 进程、管道、JSON、超时、重启 | 知道 UI/规则细节（只收发坐标与胜率） |
| `review` | 输入 `TurnEval` + 规则，输出中文句子 | IO、时间、随机数（保证快照测试稳定） |
| `app` | 状态机：当前局面、选中的手、分析结果缓存；`Controller` 起后台线程、收事件、推进 AI | 直接画图、直接管理进程/GTP（交给 `engine`） |
| `tui`/`gui` | 渲染、输入映射成 `Controller` 调用 | 自己实现规则、自己算胜率、自己 spawn 分析 |

## 三、核心数据结构

```rust
// goban
pub enum Color { Black, White }
pub struct Point { pub x: u8, pub y: u8 }          // 0-based，左上原点

pub struct Board {                                  // 只描述静态盘面
    pub size: u8,
    cells: Vec<Option<Color>>,
}

pub struct Position {                               // 可落子的局面
    pub board: Board,
    pub to_move: Color,
    pub captures: [u16; 2],
    pub ko: Option<Point>,                          // 简单劫
}

pub struct Game {                                   // 整盘棋：历史 + 超级劫
    initial: Position,
    moves: Vec<Move>,                               // Move::Play(Point) | Move::Pass
    history: Vec<u64>,                              // Zobrist，位置超级劫
}

impl Game {
    pub fn play(&self, mv: Move) -> Result<Game, IllegalMove>;
    pub fn undo(&self) -> Option<Game>;
    pub fn score_area(&self) -> AreaScore;          // 中国规则，死子由调用方传入
}
```

```rust
// engine
pub struct Candidate {
    pub mv: Move,
    pub winrate_black: f32,      // 统一按黑方存，显示时再换算
    pub score_lead_black: f32,
    pub pv: Vec<Move>,
    pub visits: u32,
}

pub struct TurnEval {
    pub turn: u16,
    pub to_move: Color,
    pub candidates: Vec<Candidate>,
}
```

```rust
// review
pub struct Blunder {
    pub turn: u16,
    pub played: Move,
    pub best: Move,
    pub loss: f32,               // 百分点
    pub text: String,            // 给人看的一句话
}

pub struct Review {
    pub header: String,
    pub phases: [PhaseStat; 3],  // 布局 / 中盘 / 官子
    pub blunders: Vec<Blunder>,  // 按严重程度降序
    pub turns: Vec<TurnEval>,
}
```

**约定（写进 crate 文档，评审时按这个查）：**

1. 胜率/目差内部一律**黑方视角**，只在渲染层按"你是黑还是白"翻转一次。
2. SGF 字母 ↔ GTP 字母（跳过 I）的转换只允许出现在 `sgf` 和 `engine`，core 里不许出现 `'Q'`、`"Q16"` 这种字符串。
3. `review` 输出必须**确定性**：同样的输入两次运行必须逐字节一致（不给随机、不读时钟），否则快照测试没法用。

## 四、引擎接入

两种模式，分别服务不同场景：

| 模式 | 协议 | 用途 | 生命周期 |
|---|---|---|---|
| `GtpSession` | 长驻 `katago gtp`，行协议 | TUI/GUI 实时分析、人机对弈 | 应用启动时拉起，退出时杀 |
| `AnalysisBatch` | 一次性 `katago analysis`，stdin 一批 query | 复盘整盘、`go-review` CLI | 每次调用 |

- 进程管理：崩溃自动重启（指数退避，最多 3 次）；启动时 `kata-analyze` 的 `interval` 用于限流。
- 线程模型：**专用线程 + mpsc**，不用 async。理由：GTP 是行协议，async 只会增加复杂度。分析线程和 AI 推进统一由 `app::Controller` 管理（TUI/GUI 共用）；GUI 侧用 `glib::timeout_add_local` 每 50 ms 调一次 `Controller::tick`，只有状态变化才重画。
- 复盘分批：一次 query 最多 30 个 `analyzeTurns`，避免长棋谱内存峰值；TUI/GUI 显示进度条。

## 五、前端

### TUI（ratatui + crossterm）

```
┌ 对局 ─────────────────────────────┐ ┌ 分析 ────────────┐
│    A B C D E F G H J K L M N O P Q R S T │ 胜率 ████░░ 54% │
│ 19 · · · · · · · · · · · · · · · · · · · │ 目差 黑 +2.3     │
│ 18 · · · · · · · · · · · · · · · · · · · │ 推荐 R14 52.8%   │
│ ...                                      │ 变化 R14 Q4 ...  │
│  1 · · · · · · · · · · · · · · · · · · · │                  │
└──────────────────────────────────────────┘ └──────────────────┘
```

- 棋盘两种渲染：文本点阵（`·`/`●`，窄窗口自动缩放）与 720×720 RGBA 图片
  （`--mode auto|image|text`；auto 探测 kitty/sixel/iTerm2，失败回退文本；
  图片坐标用 `font8x8` 点阵字体，按局面指纹缓存，避免每帧重编码）
- `hjkl` / 方向键移动光标，`Enter` 落子，`u` 撤销，`a` 触发分析，落子自动存 SGF
- 底部状态行：手数、轮到谁、引擎耗时
- 鼠标可选支持（crossterm 有，但先不做）

### GUI（gtk4-rs + libadwaita）

- 代码拆成 `main.rs`（入口）/ `board.rs`（Cairo 绘制与命中）/ `panels.rs`（侧栏控件与文案）/
  `controller.rs`（按键、按钮、定时器全部接到 `app::Controller`）
- 主窗口 `GtkApplicationWindow` + `AdwHeaderBar`
- 棋盘用 `gtk::DrawingArea` + Cairo：19 条线、星位、棋子（径向渐变）、最后一手标记、候选点半透明圆 + 胜率
- 侧栏是固定宽 `GtkBox` + 卡片：胜率、局面、预测/变化、讲解、操作按钮、棋谱下拉、状态 + 语言
- 快捷键与 TUI 相同（`hjkl`/Enter/`t`/`v`/`a`/`A`/`b`/`u`/`n`/`p`/`g`/`G`/`q`）；
  菜单（打开/保存/引擎设置）尚未做
- 原生 Wayland，无需 XWayland

## 六、测试策略

| 层 | 方法 |
|---|---|
| `goban` | 单测：简单劫、倒扑、禁入点、自杀、双活边界、面积数子 |
| `sgf` | golden 文件：常见 SGF（含 pass、让子、9 路）解析 → 序列化 → 再解析，结果一致 |
| `engine` | **假引擎**：一个 shell 脚本模拟 GTP/JSON 输出，CI 不需要 GPU |
| `review` | 快照测试：固定 `TurnEval` 输入 → 断言完整中文文本 |
| 集成 | `go-review` 对一盘 50 手样例跑通，断言失误数 |

## 七、打包

```nix
# devShell 需要的
nix shell nixpkgs#{cargo,rustc,pkg-config,gtk4,libadwaita,gobject-introspection}

# 发布
rustPlatform.buildRustPackage + wrapGAppsHook4
```

GUI 二进制必须 `wrapGAppsHook4`，否则 NixOS 上找不到 GSettings schema / 图标主题。

## 八、里程碑

| 阶段 | 内容 | 验收 |
|---|---|---|
| **M0** ✅ | goban + sgf + review + cli | `go-review 棋谱.sgf` 输出中文讲评，规则单测全绿（2026-11-22 完成） |
| **M1** ✅ | TUI | 能摆子、落子、撤销、看胜率/候选、跑整盘复盘（2026-11-22 完成） |
| **M2** | GUI | 打开 SGF、点棋盘落子、侧栏分析、点失误跳转 |
| **M3** | 增强 | humanv0 拟人对照、开局库、错题本导出 Markdown |

先做 M0，它是其余一切的地基；M1 和 M2 共用 app/core，不会白写。

## 九、风险与对策

| 风险 | 对策 |
|---|---|
| Rust GUI 首次编译 5–10 分钟 | 用 `nix shell` + 增量编译；GUI 放到 M2 |
| 自写数子算错死活 | 复盘只用 KataGo 的 `scoreLead`；自写数子仅用于"无引擎时的粗略显示"，并标注 |
| TUI 中文字宽 | 用 `unicode-width` 计算显示宽度，棋盘用等宽字符 |
| 引擎卡死拖住 UI | 所有引擎调用走通道 + 超时（单次 > 30s 判失败并重启） |
| 4G 显存和别的程序抢 | 引擎空闲时限制 visits；启动前检查显存 |

## 十、待你拍板

1. 语言：确认 Rust（还是 Python 快速版）？
2. 先做 M0（CLI 复盘）还是直接 M1（TUI）？
3. GUI 是否接受"只做学习模式"（不做变化树、不做棋谱库管理）？
