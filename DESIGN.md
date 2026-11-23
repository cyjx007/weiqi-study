# weiqi-study 设计文档

> 版本以 `Cargo.toml` 为准· 本地围棋学习工具（CLI + TUI + GTK4）
> 目标：纯本地、不依赖浏览器、单人自学；引擎用本机 KataGo + 游戏目录里的网络。
>
> 本文只描述**当前代码真实做了什么**。已实现 / 有限支持 / 尚未实现 在每节显式标注；
> 路线图不写进“功能”里。

## 一、技术选型

### 结论：Rust 单一语言，一个 core，三个前端

| 维度 | Rust（选定） | Python（备选） |
|---|---|---|
| 规则正确性 | 强类型 + 模式匹配，劫/禁入点这类状态机不容易漏 | 也能写，但重构时靠自觉 |
| 多端一致性 | CLI/TUI/GUI 链接**同一个** core crate，不可能漂移 | core 是包，也能共享，但打包/环境更麻烦 |
| 分发 | 单二进制，`rustPlatform.buildRustPackage` 一行 | 依赖 GTK4/PyGObject 运行时，要 wrapGAppsHook |
| 启动/内存 | 快、小 | Python + GTK 起步 ~80 MB |
| 本机工具链 | nixpkgs 的 Rust 1.99.1 | python3 3.14 已有 |

决定性理由：**规则引擎必须只有一个**。多个前端各自实现提子/劫争，迟早出现
“同一个 SGF 两边结果不同”，那对学棋是灾难。

### 依赖版本（本机实测）

```
gtk4              4.24.0
libadwaita        1.10.0
rustc / cargo     1.99.1
```

Rust crates：`ratatui`、`crossterm`、`ratatui-image`、`gtk4`、`libadwaita`、`glib`、
`serde`、`serde_json`、`chrono`。

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
                 │        │    app    │        │  ◄── App 状态 + Controller（后台线程）
                 │        └─────┬─────┘        │
                 │    ┌─────────┼─────────┐    │
                 │    ▼         ▼         ▼    │
                 │  goban     sgf      review  │  ◄── 纯计算，无 IO
                 │  (规则)   (读写)    (讲评)   │
                 │              │              │
                 │        ┌─────▼─────┐        │
                 │        │  engine   │        │  ◄── 唯一碰进程/IO 的地方
                 │        └─────┬─────┘        │
                 └──────────────┼──────────────┘
                                ▼
                        KataGo (`analysis` 长驻)
```

### 目录（当前实现）

```
weiqi-study/
├─ Cargo.toml                  # workspace（版本见文件内 version）
├─ crates/
│  ├─ goban/                   # Board/Position/Game、提子、劫、禁入点、RuleSet、面积数子
│  ├─ sgf/                     # SGF 解析/写回（含 collection）
│  ├─ engine/                  # KataGo analysis 进程、协议、超时、恢复
│  ├─ review/                  # 讲评模板（纯函数、确定性）
│  ├─ app/                     # App 状态 + Controller + storage + i18n + preview
│  ├─ cli/   [bin go-review]   # 逐手复盘 / --strict / --game N
│  ├─ tui/   [bin weiqi-tui]   # ratatui 前端（文本 / 图片棋盘）
│  └─ gui/   [bin weiqi-gui]   # gtk4 + libadwaita 前端
├─ crates/app/tests/           # 端到端 fixtures（SGF → App → Goban → 写回 → 再读）
└─ scripts/                    # gui-smoke.sh / tui-smoke.py（CI 用）
```

`app/src/lib.rs` 目前 900 行上下；测试在 `tests.rs`、变化预览在 `preview.rs`。

### crate 边界（越界就是 bug）

| crate | 允许 | 禁止 |
|---|---|---|
| `goban` | 纯计算、盘面/历史、规则语义 | 任何 IO、字符串坐标、UI 概念 |
| `sgf` | 文件内容字符串 ↔ 结构体、规则名映射 | 知道引擎存在 |
| `engine` | spawn 进程、管道、JSON、超时、重建会话 | 知道 UI/规则细节（只收发坐标与胜率） |
| `review` | 输入 `TurnEval` + 规则，输出句子 | IO、时间、随机数（保证快照稳定） |
| `app` | 状态机、分析缓存、存储、后台任务编排 | 画图、直接管理 GTP/进程（交给 `engine`） |
| `tui`/`gui` | 渲染、输入映射成 `Controller` 调用 | 自己实现规则、算胜率、spawn 分析 |

## 三、核心数据结构（当前实现）

```rust
// goban
pub enum Color { Black, White }
pub struct Point { pub x: u8, pub y: u8 }          // 0-based，左上原点

pub struct Position {                               // 可落子的局面
    pub board: Board,
    pub to_move: Color,
    pub captures: [u16; 2],
    pub last_move: Option<Move>,
}

pub enum KoRule { Simple, PositionalSuperko, SituationalSuperko }
pub enum Scoring { Area, Territory }

pub struct RuleSet {                                // SGF RU 的语义载体
    pub ko: KoRule,
    pub scoring: Scoring,
    pub name: &'static str,                         // Chinese / Japanese / Korean / AGA / Ing
}

pub struct Game {                                   // 历史 + 规则 + 先手
    rule: RuleSet,                                  // 默认 Chinese（PSK + 数子）
    first: Color,                                   // SGF PL / 首手颜色 / HA>=2，不靠摆子猜
    positions: Vec<Position>,
    board_hashes: Vec<u64>,                         // 只哈希盘面（PSK）
    situational_hashes: Vec<u64>,                   // 盘面 + 行棋方（SSK）
    moves: Vec<Move>,
}

impl Game {
    pub fn play(&mut self, mv: Move) -> Result<(), IllegalMove>; // Pass 不受 superko 限制
    pub fn color_of_move(&self, turn: u16) -> Color;            // 1-based
    pub fn to_move_at(&self, turn: u16) -> Color;               // 历史内读 Position::to_move
    pub fn rules(&self) -> RuleSet;
    pub fn area_score(&self) -> AreaScore;                      // 仅 Area，不判死活
}
```

```rust
// engine
pub struct Candidate {
    pub mv: Move,
    pub winrate_black: f32,      // 统一按黑方存，显示时再换算
    pub score_black: f32,
    pub pv: Vec<Move>,
    pub visits: u32,
}

pub struct TurnEval {
    pub turn: u16,
    pub to_move: Color,
    pub winrate_black: f32,
    pub score_black: f32,
    pub candidates: Vec<Candidate>,
    pub human_policy: Option<Vec<f32>>,   // humanv0 的 row-major 概率，长度 size*size+1，最后一位 pass，-1 为非法点
}

pub struct EngineConfig {
    pub katago: PathBuf, pub model: PathBuf, pub config: PathBuf,
    pub human_model: Option<PathBuf>,
    pub visits: u32, pub chunk: usize,
    pub timeout: Duration,       // 默认 90s，超时丢弃会话
    pub extra_args: Vec<String>, // 插在 analysis 之前（测试用，生产为空）
}
```

**约定：**

1. 胜率/目差内部一律**黑方视角**，只在渲染层按“你是黑还是白”翻转一次。
2. SGF 字母 ↔ GTP 字母（跳过 I）的转换只出现在 `sgf` 和 `engine`。
3. `review` 输出必须**确定性**：同样输入两次运行逐字节一致。
4. `Game.first` 是事实（SGF 首手/`PL`/`HA`），任何重建路径都必须显式传递，不能按摆子猜。

## 四、规则支持边界

| 规则 | 劫 | 计分 | 状态 |
|---|---|---|---|
| Chinese | PSK | 数子 | ✅ 默认 |
| Japanese | 单劫 | 数目 | ✅ 声明支持（本地不判死活） |
| Korean | SSK | 数目 | ✅ 声明支持 |
| AGA | SSK | 数子 | ✅ 声明支持 |
| Ing | SSK | 数子 | ⚠️ **有限支持**：Ing 的循环/特殊终局没有完整实现，只按通用 SSK 近似 |
| New Zealand | — | — | ❌ **不支持**：NZ 允许自杀，Goban 一律禁自杀；遇到会 warning 并回退 Chinese |

- SGF `RU` 只认 Chinese/Japanese/Korean/AGA/Ing；其它值（含 NZ）在 `parse()` 时产出一条
  可见 warning，再按 Chinese 处理，不静默降级。
- 写回 SGF 用规则集规范名，`RU[AGA]`/`RU[Ing]` 不会退化成 Korean。
- 引擎请求直接使用规范名（`chinese`/`japanese`/`korean`/`aga`/`ing`）。
- `area_score()` 只是数子、把盘上所有子当活棋；**界面上的目差来自引擎 `scoreLead`**，
  不依赖它。本地数目（先标死活）尚未实现。

### 先手

- 根 `PL` > 首手 `B`/`W` 颜色 > `HA >= 2` / 有黑摆子 → 白先 > 默认黑先。
- `Game.first` 保存这个结果；`to_move_at()` 在历史范围内直接读 `Position::to_move`。
- 白先棋谱的胜率/目差按黑方视角换算（有回归测试）。

## 五、SGF 支持范围

**已实现**

- 属性：`SZ` `KM` `RU` `PL` `HA` `RE` `AB` `AW` `AE` `B` `W`。
- 规则：属性后必须至少一个 `[...]`；`B[]`/`W[]` 才是 Pass，缺值报错。
- 转义：`\]` 等按规范；反斜杠 + 换行（LF/CRLF）是行续接，从值里删掉。
- 截断输入（未闭合的 `[`、末尾孤立 `\`、括号不匹配）报字符位置，不 panic。
- 摆子：只允许开局阶段；中途 `AB`/`AW`/`AE` 显式报错。
- collection：一个文件多盘棋全部解析；`parse()` 取第一盘并给 warning，
  `go-review --game N` 选第几盘；前后多余内容报错。
  解析是**严格**的：任意一盘结构坏掉都会让整份文件打不开（变化子树仍是宽松跳过），
  这样可以保证“打开成功”等于“整份文件可读”。
- 常见元数据（`GM/FF/CA/PB/PW/DT/C/LB` 等）静默忽略；真正不认识的属性**汇总一条** warning。

**尚未实现 / 有限**

- **变化树（variation）**：只跟第一个子分支（主线），其它分支直接跳过。
  → 工具保存到 `games/` 的副本**只保留主线，分支会丢**；原文件不改（copy-on-open）。
- 编码：按 UTF-8 读取；其它编码不支持。
- 压缩 SGF（`.sgf.gz`）不支持。
- `RE`/`HA` 只作为元数据读入，不参与讲评计算；写回时不输出（让子数由 `AB` 体现）。

## 六、引擎接入

只实现了一种模式：`katago analysis` 长驻进程 + JSON lines（没有 GTP 模式）。

| 项 | 实现 |
|---|---|
| 生命周期 | 首次分析时启动，之后整个会话复用；TUI/GUI 退出时随进程结束 |
| 请求 | 一批 `analyzeTurns`（默认 30/批），带 `initialStones`/`moves`/`rules`/`komi` |
| 响应校验 | `(id, turn)` 必须同时匹配该批请求；未知 id、跨批 turn、重复 turn 都报错 |
| 字段校验 | `move`/`winrate`/`scoreLead`/`visits` 必需；越界、负数、溢出、非有限值直接拒绝 |
| 概率边界 | `winrate` 先在原始 f64 上查 `[0,1]` 再转 f32（防 `1.00000001` 舍入成 1.0）；`humanPolicy` 长度必须精确 `size²+1`，只允许精确 `-1` 哨兵或 `[0,1]`，坏数据整份丢弃但不影响候选 |
| 回合边界 | `analyze_turns` 在建会话前拒绝 `turn > moves.len()`，参数错误不再变成引擎超时 |
| 大棋盘 | 2–25 路都按原样传给 KataGo；官方构建默认 `MAX_LEN=19`，20 路以上需要自行编译，错误原文保留并附提示 |
| 超时 | `EngineConfig.timeout`（默认 90s）；超时丢掉会话 |
| 失败恢复 | 任何错误都丢弃会话（kill 子进程），下一次调用重新拉起；不会无限重试 |
| 拟人网 | `-human-model` + `includePolicy` + `humanSLProfile`，按 `humanPolicy` 抽点 |
| 强度 | `visits` 可运行时调整，下一次 query 生效；换代用 epoch，不阻塞在跑的分析 |

## 七、分析任务生命周期（重要）

- `App.generation` 是棋局版本：开新局、打开棋谱、**改走分支**、采用变化、切换拟人/规则时递增。
  在末尾续下一手**不**算换线（历史前缀没变），因此不递增，上一手迟到的分析仍然有效——
  最近一手的即时点评依赖它。
- 每批分析请求带 `generation` 和 `pos`；`App::apply` 只接受 `generation` 相同的 event，
  过期结果**不写缓存、不改状态、不复位 busy**。
- `evals` 以“手数”为键；改走分支时以**分支起点**为界清理，之后的旧分析全部作废。
- `failed_pos` 记录分析失败的位置，避免对同一局面无限重试；改走分支会清空。
- 人机执子（`App.human`）可换边：递增 generation、清 `failed_pos` 与 `last_comment`，
  但**保留 `evals`**（分析是黑方视角，与“谁是 AI”无关）。`ai_enabled` 时 AI 走另一方；
  `Controller` 换边后会 `pump()`，人执白时空盘第一手由 AI 立即接手。开新局保持当前选边。
- 打开棋谱（`open_sgf_file`）= 纯研究模式：`vs_ai` 和 `ai_enabled` 都关掉，
  `human_enabled`（拟人偏好）保留，等用户主动重新打开 AI 开关；只清 `vs_ai` 不够，
  `next_action` 看的是 `ai_enabled`。
- 引擎失败后 `busy` 复位、状态栏显示原因，用户可以再次发起分析（有回归测试）。
- 单纯 `prev/next/jump_to_turn()` 只是浏览旧棋谱，不递增 generation，前缀缓存继续可用。
- 分析取消：没有独立的取消 API；用 generation 丢弃结果，线程自然结束。
- 并发上限：`Controller` 同一时间只允许一个后台任务（分析或复盘）。任务运行期间局面又变了，
  只记 `pending`，等当前任务结束后**合并成一次**补分析，不会每落一手就新起线程排队。

## 八、存档

目录：`$XDG_DATA_HOME/weiqi-study/games`（通常 `~/.local/share/weiqi-study/games`）。

- **原子写**：同目录 `create_new` 唯一临时文件 → 写入 → `fsync` → 关闭 → 发布 → 父目录 `fsync`。
  新建（`expected: None`）用 no-replace 发布：Linux `renameat2(RENAME_NOREPLACE)`，
  文件系统不支持该 flags 时退回 `hard_link + remove`。目标已存在（哪怕是在指纹检查之后
  才被外部程序创建）一律落冲突副本，绝不覆盖。更新（`expected: Some`）仍是指纹比较后的
  普通 `rename`，外部编辑器的竞争窗口见下方「边界」。
- **目录持久性**：`rename` 同步源目录与 `games/`；`trash` 同步源目录、`games/` 和 `.trash/`。
  同步失败会返回错误并说明“内容可能已替换、只是持久性无法确认”；平台明确不支持目录 fsync 时
  按 best-effort 处理（保证较弱，已在错误语义上区分）。
- **跨进程锁**：保存全程持有 `games/.locks/<文件名>` 上的排它 advisory 锁
  （`std::fs::File::lock`），覆盖“写临时文件 → 比较版本 → rename”，关闭句柄/进程退出即释放。
  等锁有上限（默认 2s，`GameStore::with_lock_timeout` 可调），超时返回可诊断错误而不是卡住 UI；
  文件系统不支持 flock 时降级为无锁（仍保留原子写与指纹校验）。
- **版本指纹**：`FileStamp { mtime, len, FNV-1a(bytes) }`。目标文件和“上次看到的不一样”
  就另存 `*.conflict-<纳秒>-<pid>.sgf`，不覆盖别人的内容。副本优先 `hard_link`（内容与文件名
  同时原子出现），不支持硬链接时退回 `create_new` + 复制。
- 打开棋谱走 `GameStore::read_stamped()`：在锁内一次读出 `(内容, 指纹)`，并在提交前重新解析
  核对，避免“读到旧内容、记下新指纹”后在下次保存时静默覆盖。
- `SaveOutcome::Written` 会带回写入后的指纹，调用方不再重读磁盘。
- **删除**：移到 `games/.trash/`，文件名带时间戳；不真删。
- **重命名/删除也持锁**：rename 按文件名字典序锁住旧名+新名（避免互换时死锁），
  trash 锁源文件名，避免保存中的 rename 把已删文件“复活”或让同一棋谱裂成两个名字。
  **锁文件永不删除**：unlink 会让另一个已打开旧 inode、正在等锁的进程拿到“已脱链”的锁，
  两个进程各锁一个 inode，互斥直接失效。每个锁文件只有几十字节，留在 `.locks/` 复用。
- `rename` 在锁内一并返回新路径的 [`FileStamp`]，调用方不需要在锁外重读（否则会记下别人的版本）。
- **`expected: None` 表示新建**：目标已存在时同样落冲突副本，不覆盖。
- **边界**：advisory 锁只约束同样遵守它的 `weiqi-study` 进程，挡不住任意外部编辑器。
  外部编辑器不遵守锁协议时，「读指纹 → 比较 → rename」之间仍存在竞争窗口：本工具能保证
  自己的多个实例之间不互相覆盖，但**不能**承诺对任意程序都是原子 CAS。
  外部编辑后保存通常会得到冲突副本，而不是静默覆盖。
- **无锁降级**：文件系统不支持 `flock` 时（部分 FUSE/NFS）保存会降级为无锁，
  只剩内容指纹校验，保证更弱；这一点不会伪装成“已有互斥”。
  大小写不敏感文件系统（macOS/Windows）不在支持范围：`a.sgf` → `A.sgf` 会指向同一个
  锁文件，同一进程二次 `flock` 可能等到超时。

## 九、测试策略

| 层 | 方法 |
|---|---|
| `goban` | 单测：劫（三种）、倒扑、禁入点、自杀、Pass/连续 Pass、面积数子、先手 |
| `sgf` | 单测：属性值/截断/转义、PL/HA/AE、RU 映射、collection、写回往返 |
| `engine` | 假引擎：`katago=/bin/sh -c <脚本>`（不写可执行文件，避免 ETXTBSY）；协议、字段、超时、恢复 |
| `review` | 快照/语义断言：固定 `TurnEval` → 断言文本（语言显式固定）；`build()` 校验 `evals` 精确覆盖 turn `0..=n`，返回 `Result` 而不是索引 panic |
| `app` | 单测：导航、分支换线、缓存失效、存储冲突、预览、拟人策略 |
| 端到端 | `crates/app/tests/e2e_fixtures.rs` + 7 份 fixtures：PL[W]、HA+AB、Pass、变化、collection、未知 RU、AGA |
| GUI | `scripts/gui-smoke.sh`：Xvfb 启动、按键、切换棋谱、CPU 上限 |
| TUI | `scripts/tui-smoke.py`：真实 pty 120×40，启动→列表→分析失败→帮助→退出 |
| CI | fmt / clippy -D warnings / test / build / GUI smoke / TUI smoke |

## 十、打包

```nix
# devShell
nix shell nixpkgs#{cargo,rustc,pkg-config,gtk4,libadwaita,gobject-introspection}

# 发布
rustPlatform.buildRustPackage + wrapGAppsHook4
```

- `nix/package.nix` 用 `buildRustPackage` 默认的 `cargoInstallHook`，不要自己写 `installPhase`。
- GUI 必须 `wrapGAppsHook4`，否则 NixOS 上找不到 GSettings schema。
- flake 只打包 git 跟踪的文件：新增 `.nix`/`.sgf` fixture 先 `git add` 再 `nix build`。

## 十一、里程碑与未做项

| 阶段 | 内容 | 状态 |
|---|---|---|
| **M0** | goban + sgf + review + `go-review` | ✅ |
| **M1** | TUI：落子/撤销/分析/整盘复盘/图片棋盘 | ✅ |
| **M2** | GUI：打开、点击落子、侧栏、棋谱管理 | ✅ |
| **M3** | humanv0、变化预览、AI 强度、collection、RuleSet | ✅ |
| 产品功能 | 批量分析、报告导出、标签、前后对比、互动讲解、错题本、死活题、定式、棋力统计 | ❌ 未做（review 明确暂缓） |
| SGF 变化树 | 分支浏览/编辑 | ❌ 未做（只读主线，见第五节） |

## 十二、风险与对策

| 风险 | 对策 |
|---|---|
| 自写数子算错死活 | 复盘只用 KataGo `scoreLead`；`area_score` 仅标注为粗略显示 |
| 引擎卡死拖住 UI | 独立线程 + 通道 + 可配置超时；失败丢会话、下次重建 |
| 分析结果串盘 | generation + pos 双重校验；改走分支以 base 为界清缓存 |
| 多进程同时编辑同一棋谱 | 排它锁 + 内容指纹 + 冲突副本；边界写在第八节 |
| 假引擎测试竞态（ETXTBSY） | 不写可执行脚本，用 `/bin/sh -c` 内存传脚本 |
| 4G 显存和别的程序抢 | 引擎空闲时降低 visits；启动前检查显存 |

## 十三、历史决策

早期文档里“待拍板”的问题已经全部定了，保留结论以免反复讨论：

1. 语言：Rust。
2. 先做 M0（CLI 复盘），再做 M1/M2。
3. GUI 接受“只做学习模式”，不做变化树编辑与棋谱库高级管理（collection 只提供 CLI 选盘）。
