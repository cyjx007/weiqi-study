//! go-review：把一盘 SGF 交给本地 KataGo，输出中文讲评。

use std::time::Instant;

use engine::EngineConfig;
use goban::Game;

fn main() {
    if let Err(e) = run() {
        eprintln!("错误：{e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let mut path: Option<String> = None;
    let mut cfg = EngineConfig::default();
    let mut komi: Option<f32> = None;
    let mut min_loss = 5.0f32;
    let mut show_all = false;
    let mut no_engine = false;
    let mut strict = false;
    let mut game_no = 1usize;

    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "-h" | "--help" => {
                print_help();
                return Ok(());
            }
            "-V" | "--version" => {
                println!("go-review {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            "--visits" => {
                cfg.visits = args
                    .next()
                    .ok_or("--visits 需要数字")?
                    .parse()
                    .map_err(|_| "--visits 不是数字")?;
            }
            "--min-loss" => {
                min_loss = args
                    .next()
                    .ok_or("--min-loss 需要数字")?
                    .parse()
                    .map_err(|_| "--min-loss 不是数字")?;
            }
            "--komi" => {
                komi = Some(
                    args.next()
                        .ok_or("--komi 需要数字")?
                        .parse()
                        .map_err(|_| "--komi 不是数字")?,
                );
            }
            "--all" => show_all = true,
            "--no-engine" => no_engine = true,
            "--strict" => strict = true,
            "--game" => {
                game_no = args
                    .next()
                    .ok_or("--game 需要序号")?
                    .parse()
                    .map_err(|_| "--game 不是数字")?;
                if game_no == 0 {
                    return Err("--game 从 1 开始".to_string());
                }
            }
            other if other.starts_with('-') => return Err(format!("未知参数 {other}")),
            other => path = Some(other.to_string()),
        }
    }

    let path = path.ok_or(
        "用法：go-review <棋谱.sgf> [--visits N] [--min-loss P] [--all] [--strict] [--game N]",
    )?;
    let text = std::fs::read_to_string(&path).map_err(|e| format!("读不了 {path}：{e}"))?;
    // SGF collection：默认第一盘，--game N 选其它盘；不静默丢后面的棋谱
    let games = sgf::parse_collection(&text)?;
    if game_no > games.len() {
        return Err(format!(
            "文件只有 {} 盘棋，--game {game_no} 超出范围",
            games.len()
        ));
    }
    if games.len() > 1 {
        eprintln!("文件含 {} 盘棋，已选第 {game_no} 盘", games.len());
    }
    let sg = &games[game_no - 1];
    if strict {
        if let Some(w) = sg.warnings.first() {
            return Err(format!("严格模式：{w}（共 {} 条警告）", sg.warnings.len()));
        }
    }
    for w in &sg.warnings {
        eprintln!("警告：{w}");
    }
    let komi = komi.or(sg.komi).unwrap_or(7.5);
    let rules = sgf::rule_set_from_rules(sg.rules.as_deref());
    let mut game =
        Game::with_setup_rules_first(sg.size, komi, sg.setup.clone(), rules, sg.first_to_move());
    for (i, mv) in sg.moves.iter().enumerate() {
        game.play(*mv).map_err(|e| {
            let coord = match mv {
                goban::Move::Play(p) => engine::gtp_coord(*p, sg.size),
                goban::Move::Pass => "停一手".to_string(),
            };
            format!("第 {} 手 {coord} 不合法：{e}", i + 1)
        })?;
    }

    if no_engine {
        println!(
            "{} 路 · 贴目 {:.1} · {} 手（未调用引擎）",
            sg.size,
            komi,
            sg.moves.len()
        );
        return Ok(());
    }

    eprintln!("分析 {} 手（每手 {} visits）…", sg.moves.len(), cfg.visits);
    let started = Instant::now();
    let engine = engine::Engine::new(cfg);
    let evals = engine.analyze_game(&game, |done, total| {
        eprint!("\r  进度 {done}/{total}");
    })?;
    eprintln!("\r  完成，用时 {:.1}s", started.elapsed().as_secs_f32());

    let english = match std::env::var("WEIQI_LANG") {
        Ok(v) => v.to_ascii_lowercase().starts_with("en"),
        Err(_) => !std::env::var("LANG")
            .map(|l| l.to_ascii_lowercase().starts_with("zh"))
            .unwrap_or(false),
    };
    let r = review::build(english, &game, &evals, min_loss);
    print!("{}", r.render(min_loss, show_all));
    Ok(())
}

fn print_help() {
    println!(
        "go-review — 本地 KataGo 中文复盘

用法：
  go-review <棋谱.sgf> [选项]

选项：
  --visits N      每手推演次数（默认 120，越大越准越慢）
  --min-loss P    只显示亏损 ≥ P 个百分点的手（默认 5）
  --all           显示每一手
  --komi K        覆盖贴目
  --no-engine     只解析棋谱，不调用引擎（自检用）
  --strict        棋谱有任何 warning 就报错（默认只警告）
  --game N        多盘棋（SGF collection）里选第 N 盘，默认 1
  -h, --help      显示帮助
  -V, --version   显示版本

环境变量：
  KATAGO_BIN      默认 ~/go-study/katago/bin/katago
  KATAGO_MODEL    默认 Steam 游戏 NinOneQi 里的 b18c384nbt 网络
  KATAGO_CFG      默认 ~/go-study/analysis.cfg
  WEIQI_LANG      输出语言 zh|en（默认跟随 LANG）"
    );
}
