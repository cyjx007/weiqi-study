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

const USAGE: &str = "用法：go-review <棋谱.sgf> [--visits N] [--min-loss P] [--komi K] \
                     [--all] [--no-engine] [--strict] [--game N]";

/// `--visits` 与 App / GUI 的思考量范围保持一致。
const VISITS_MIN: u32 = 10;
const VISITS_MAX: u32 = 5000;

#[derive(Debug)]
enum Parsed {
    Help,
    Version,
    Run(Opts),
}

/// 命令行选项；解析阶段不碰文件系统，也不读 engine 配置。
#[derive(Debug)]
struct Opts {
    path: Option<String>,
    visits: Option<u32>,
    komi: Option<f32>,
    min_loss: f32,
    show_all: bool,
    no_engine: bool,
    strict: bool,
    game_no: usize,
}

fn parse_args<I: Iterator<Item = String>>(mut args: I) -> Result<Parsed, String> {
    let mut opts = Opts {
        path: None,
        visits: None,
        komi: None,
        min_loss: 5.0,
        show_all: false,
        no_engine: false,
        strict: false,
        game_no: 1,
    };

    while let Some(a) = args.next() {
        match a.as_str() {
            "-h" | "--help" => return Ok(Parsed::Help),
            "-V" | "--version" => return Ok(Parsed::Version),
            "--visits" => {
                let raw = args.next().ok_or("--visits 需要数字")?;
                let v: u32 = raw
                    .parse()
                    .map_err(|_| format!("--visits 不是非负整数：{raw}"))?;
                if !(VISITS_MIN..=VISITS_MAX).contains(&v) {
                    return Err(format!(
                        "--visits 必须在 {VISITS_MIN}–{VISITS_MAX} 之间，收到 {v}"
                    ));
                }
                opts.visits = Some(v);
            }
            "--min-loss" => {
                let raw = args.next().ok_or("--min-loss 需要数字")?;
                let v: f32 = raw
                    .parse()
                    .map_err(|_| format!("--min-loss 不是数字：{raw}"))?;
                if !v.is_finite() || v < 0.0 {
                    return Err(format!("--min-loss 必须是有限的非负数，收到 {raw}"));
                }
                opts.min_loss = v;
            }
            "--komi" => {
                let raw = args.next().ok_or("--komi 需要数字")?;
                let v: f32 = raw.parse().map_err(|_| format!("--komi 不是数字：{raw}"))?;
                // 让子/反贴目可以是负数，只拒绝 NaN/Infinity
                if !v.is_finite() {
                    return Err(format!("--komi 必须是有限数值，收到 {raw}"));
                }
                opts.komi = Some(v);
            }
            "--all" => opts.show_all = true,
            "--no-engine" => opts.no_engine = true,
            "--strict" => opts.strict = true,
            "--game" => {
                let raw = args.next().ok_or("--game 需要序号")?;
                let n: usize = raw.parse().map_err(|_| format!("--game 不是数字：{raw}"))?;
                if n == 0 {
                    return Err("--game 从 1 开始".to_string());
                }
                opts.game_no = n;
            }
            other if other.starts_with('-') => {
                return Err(format!("未知参数 {other}\n{USAGE}"));
            }
            other => {
                if opts.path.is_some() {
                    return Err(format!(
                        "只接受一个棋谱文件，多余的位置参数：{other}\n{USAGE}"
                    ));
                }
                opts.path = Some(other.to_string());
            }
        }
    }

    Ok(Parsed::Run(opts))
}

fn run() -> Result<(), String> {
    let opts = match parse_args(std::env::args().skip(1))? {
        Parsed::Help => {
            print_help();
            return Ok(());
        }
        Parsed::Version => {
            println!("go-review {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        Parsed::Run(opts) => opts,
    };

    let path = opts.path.ok_or_else(|| format!("缺少棋谱文件\n{USAGE}"))?;
    let mut cfg = EngineConfig::default();
    if let Some(v) = opts.visits {
        cfg.visits = v;
    }
    let komi = opts.komi;
    let min_loss = opts.min_loss;
    let show_all = opts.show_all;
    let no_engine = opts.no_engine;
    let strict = opts.strict;
    let game_no = opts.game_no;
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
    let r = review::build(english, &game, &evals, min_loss)?;
    print!("{}", r.render(min_loss, show_all));
    Ok(())
}

fn print_help() {
    println!(
        "go-review — 本地 KataGo 中文复盘

用法：
  go-review <棋谱.sgf> [选项]

选项：
  --visits N      每手推演次数（10–5000，默认 120；越大越准越慢）
  --min-loss P    只显示亏损 ≥ P 个百分点的手（默认 5，必须非负）
  --all           显示每一手
  --komi K        覆盖贴目（可为负）
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

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Parsed, String> {
        parse_args(args.iter().map(|s| s.to_string()))
    }

    fn opts(args: &[&str]) -> Opts {
        match parse(args).expect("参数应当合法") {
            Parsed::Run(o) => o,
            other => panic!("应当解析成运行参数，实际 {other:?}"),
        }
    }

    #[test]
    fn visits_must_stay_within_the_app_range() {
        assert_eq!(opts(&["a.sgf", "--visits", "10"]).visits, Some(10));
        assert_eq!(opts(&["a.sgf", "--visits", "5000"]).visits, Some(5000));
        for bad in ["0", "9", "5001", "-1", "abc", "1.5"] {
            let err = parse(&["a.sgf", "--visits", bad]).unwrap_err();
            assert!(err.contains("--visits"), "{bad} 应被拒绝：{err}");
        }
    }

    #[test]
    fn min_loss_must_be_finite_and_non_negative() {
        assert_eq!(opts(&["a.sgf", "--min-loss", "0"]).min_loss, 0.0);
        assert_eq!(opts(&["a.sgf", "--min-loss", "5.5"]).min_loss, 5.5);
        for bad in ["-1", "-0.1", "NaN", "nan", "inf", "-inf", "x"] {
            let err = parse(&["a.sgf", "--min-loss", bad]).unwrap_err();
            assert!(err.contains("--min-loss"), "{bad} 应被拒绝：{err}");
        }
    }

    #[test]
    fn komi_must_be_finite_but_may_be_negative() {
        assert_eq!(opts(&["a.sgf", "--komi", "7.5"]).komi, Some(7.5));
        assert_eq!(
            opts(&["a.sgf", "--komi", "-7.5"]).komi,
            Some(-7.5),
            "让子/反贴目允许负贴目"
        );
        for bad in ["NaN", "inf", "-inf", "x"] {
            let err = parse(&["a.sgf", "--komi", bad]).unwrap_err();
            assert!(err.contains("--komi"), "{bad} 应被拒绝：{err}");
        }
    }

    #[test]
    fn only_one_positional_file_is_allowed() {
        let err = parse(&["a.sgf", "b.sgf"]).unwrap_err();
        assert!(err.contains("只接受一个棋谱文件"), "{err}");
        assert!(err.contains("b.sgf"), "错误里要带多余参数：{err}");
        assert!(err.contains("用法"), "错误里要带用法：{err}");
        assert!(opts(&["a.sgf"]).path.as_deref() == Some("a.sgf"));
    }

    #[test]
    fn game_number_and_missing_values_are_rejected() {
        assert_eq!(opts(&["a.sgf", "--game", "3"]).game_no, 3);
        for bad in ["0", "-1", "abc"] {
            let err = parse(&["a.sgf", "--game", bad]).unwrap_err();
            assert!(err.contains("--game"), "{bad}: {err}");
        }
        for flag in ["--visits", "--min-loss", "--komi", "--game"] {
            let err = parse(&["a.sgf", flag]).unwrap_err();
            assert!(err.contains(flag), "{flag} 缺值要报错：{err}");
        }
    }

    #[test]
    fn flags_and_help_short_circuit() {
        let o = opts(&["a.sgf", "--all", "--strict", "--no-engine"]);
        assert!(o.show_all && o.strict && o.no_engine);
        assert!(matches!(parse(&["-h"]).unwrap(), Parsed::Help));
        assert!(matches!(parse(&["--help"]).unwrap(), Parsed::Help));
        assert!(matches!(parse(&["-V"]).unwrap(), Parsed::Version));
        assert!(matches!(parse(&["--version"]).unwrap(), Parsed::Version));
    }

    #[test]
    fn unknown_flags_are_rejected_and_path_is_optional_at_parse_time() {
        let err = parse(&["--wat"]).unwrap_err();
        assert!(err.contains("未知参数"), "{err}");
        // 缺文件由 run() 报错；解析阶段只负责把选项收齐
        assert!(opts(&["--all"]).path.is_none());
    }
}
