//! 棋谱持久化：`$XDG_DATA_HOME/weiqi-study/games/*.sgf`。
//!
//! 写入规则：先写同目录的**唯一**临时文件（`create_new`），`fsync` 后原子 rename，
//! 再 `fsync` 父目录。保存过程用 `<文件名>.lock` 上的 advisory 排它锁（`File::lock`）
//! 把「比较版本 → rename」包成一个临界区，进程退出/关闭句柄时自动释放。
//!
//! 版本用内容指纹（修改时间 + 长度 + FNV-1a）判断，而不是只看 mtime；
//! 目标被别的进程改过就另存 `*.conflict-*` 副本（`create_new`，绝不覆盖已有副本）。
//!
//! 边界：advisory 锁只能约束同样遵守它的 `weiqi-study` 进程，挡不住任意编辑器；
//! 也不保证所有文件系统上都是真正的 CAS。
//!
//! 删除不真删：移到 `games/.trash/`，文件名带时间戳，可手动找回。

use std::fs;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{Local, TimeZone};
use goban::{Color, Move, Point, RuleSet};

#[derive(Clone, Debug)]
pub struct GameEntry {
    pub path: PathBuf,
    pub label: String,
}

/// 文件版本指纹：比单独 mtime 可靠（同一秒内写入、或 mtime 被外部工具保留都能区分）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileStamp {
    pub mtime: SystemTime,
    pub len: u64,
    /// 内容 FNV-1a；SGF 只有几 KB，读一遍代价可以忽略
    pub hash: u64,
}

impl FileStamp {
    /// 读文件并计算指纹；文件不存在返回 `None`。
    pub fn of(path: &Path) -> Option<FileStamp> {
        let meta = fs::metadata(path).ok()?;
        let mtime = meta.modified().ok()?;
        let bytes = fs::read(path).ok()?;
        Some(FileStamp {
            mtime,
            len: bytes.len() as u64,
            hash: fnv1a(&bytes),
        })
    }
}

/// FNV-1a 64：够用、无依赖、几十行就能解释清楚。
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// 写 SGF 需要的规则参数（打包传，避免 save 参数过多）。
#[derive(Clone, Copy, Debug)]
pub struct WriteOptions {
    /// 完整规则集：写回 SGF 时用它的规范名（`RU[...]`）
    pub rules: RuleSet,
    pub first: Color,
    /// 上次读/写这个文件时看到的指纹；`None` = 新文件，不做冲突检查。
    pub expected: Option<FileStamp>,
}

/// 一次保存的结果。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SaveOutcome {
    /// 正常覆盖目标文件。
    Written,
    /// 目标被外部改过：没有覆盖，内容写到了这个副本。
    Conflict(PathBuf),
}

#[derive(Clone, Debug)]
pub struct GameStore {
    dir: PathBuf,
}

impl Default for GameStore {
    fn default() -> Self {
        Self::open()
    }
}

impl GameStore {
    pub fn open() -> Self {
        Self::at(data_home().join("weiqi-study/games"))
    }

    pub fn at(dir: PathBuf) -> Self {
        let _ = fs::create_dir_all(&dir);
        GameStore { dir }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// 生成一个新的棋谱路径（不创建文件）。
    ///
    /// 文件名带纳秒 + pid：两个进程同一秒创建也不会撞名（原来只精确到秒）。
    pub fn create(&self) -> Result<PathBuf, String> {
        let stamp = unix_nanos();
        let pid = std::process::id();
        for i in 0..1000u32 {
            let name = if i == 0 {
                format!("game-{stamp}-{pid}.sgf")
            } else {
                format!("game-{stamp}-{pid}-{i}.sgf")
            };
            let path = self.dir.join(name);
            if !path.exists() {
                return Ok(path);
            }
        }
        Err("无法生成棋谱文件名".to_string())
    }

    /// 原子写入：唯一临时文件 + fsync + rename + 目录 fsync。
    ///
    /// 目标在 `expected_mtime` 之后被别的进程改过时，不覆盖它，
    /// 改为写一份 `*.conflict-<纳秒>-<pid>.sgf` 副本并返回 [`SaveOutcome::Conflict`]。
    pub fn save(
        &self,
        path: &Path,
        size: u8,
        komi: f32,
        setup: &[(Color, Point)],
        moves: &[Move],
        opts: WriteOptions,
    ) -> Result<SaveOutcome, String> {
        let text = sgf::write_game(size, komi, setup, moves, opts.rules, opts.first);

        // 锁覆盖「写临时文件 → 比较版本 → rename」：另一个 weiqi-study 进程
        // 同时保存同一文件时，会在这里排队，而不是两边都看到旧指纹。
        let _lock = self.acquire_lock(path)?;

        let (tmp, mut file) = self.create_tmp(path)?;
        if let Err(e) = file.write_all(text.as_bytes()) {
            let _ = fs::remove_file(&tmp);
            return Err(format!("写入 {} 失败：{e}", tmp.display()));
        }
        if let Err(e) = file.sync_all() {
            let _ = fs::remove_file(&tmp);
            return Err(format!("同步 {} 失败：{e}", tmp.display()));
        }
        drop(file); // 先关掉，rename 才能跨平台可靠

        // 版本校验：磁盘上的文件不是我们上次看到的那个，就另存副本
        if let Some(expected) = opts.expected {
            if FileStamp::of(path) != Some(expected) {
                let alt = self.create_conflict_copy(&tmp, path)?;
                return Ok(SaveOutcome::Conflict(alt));
            }
        }

        fs::rename(&tmp, path).map_err(|e| {
            let _ = fs::remove_file(&tmp);
            format!("保存 {} 失败：{e}", path.display())
        })?;
        sync_dir(&self.dir);
        Ok(SaveOutcome::Written)
    }

    /// 在 `games/.locks/<文件名>` 上取排它 advisory 锁；返回的 File 关闭即释放。
    fn acquire_lock(&self, path: &Path) -> Result<fs::File, String> {
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("game.sgf");
        let locks = self.dir.join(".locks");
        fs::create_dir_all(&locks).map_err(|e| format!("创建锁目录失败：{e}"))?;
        let lock_path = locks.join(name);
        let file = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&lock_path)
            .map_err(|e| format!("打开锁文件 {} 失败：{e}", lock_path.display()))?;
        file.lock()
            .map_err(|e| format!("锁住 {} 失败：{e}", lock_path.display()))?;
        Ok(file)
    }

    /// 在目标目录里 `create_new` 一个唯一临时文件；只清理自己建的那个。
    fn create_tmp(&self, path: &Path) -> Result<(PathBuf, fs::File), String> {
        let stem = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("game.sgf");
        let stamp = unix_nanos();
        let pid = std::process::id();
        for i in 0..100u32 {
            let name = format!("{stem}.tmp-{stamp}-{pid}-{i}");
            let tmp = self.dir.join(name);
            match fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&tmp)
            {
                Ok(f) => return Ok((tmp, f)),
                Err(e) if e.kind() == ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(format!("创建 {} 失败：{e}", tmp.display())),
            }
        }
        Err("无法创建唯一临时文件".to_string())
    }

    /// 把 tmp 的内容落成一份冲突副本；用 `create_new` 保证不覆盖已有副本。
    fn create_conflict_copy(&self, tmp: &Path, path: &Path) -> Result<PathBuf, String> {
        let stem = path.file_stem().and_then(|n| n.to_str()).unwrap_or("game");
        let stamp = unix_nanos();
        let pid = std::process::id();
        for i in 0..1000u32 {
            let suffix = if i == 0 {
                format!("{stamp}-{pid}")
            } else {
                format!("{stamp}-{pid}-{i}")
            };
            let alt = self.dir.join(format!("{stem}.conflict-{suffix}.sgf"));
            match fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&alt)
            {
                Ok(mut out) => {
                    let result = fs::read(tmp)
                        .and_then(|data| out.write_all(&data))
                        .and_then(|()| out.sync_all());
                    drop(out);
                    let _ = fs::remove_file(tmp);
                    result.map_err(|e| {
                        let _ = fs::remove_file(&alt);
                        format!("写冲突副本 {} 失败：{e}", alt.display())
                    })?;
                    sync_dir(&self.dir);
                    return Ok(alt);
                }
                Err(e) if e.kind() == ErrorKind::AlreadyExists => continue,
                Err(e) => {
                    let _ = fs::remove_file(tmp);
                    return Err(format!("创建冲突副本 {} 失败：{e}", alt.display()));
                }
            }
        }
        let _ = fs::remove_file(tmp);
        Err("无法生成不冲突的副本文件名".to_string())
    }

    pub fn list(&self) -> Vec<GameEntry> {
        let mut out = Vec::new();
        let Ok(entries) = fs::read_dir(&self.dir) else {
            return out;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("sgf") {
                continue;
            }
            let secs = entry
                .metadata()
                .and_then(|m| m.modified())
                .map(|t| {
                    t.duration_since(UNIX_EPOCH)
                        .map(|d| d.as_secs())
                        .unwrap_or(0)
                })
                .unwrap_or(0);
            out.push(GameEntry {
                path,
                label: format_time(secs),
            });
        }
        // 文件名带时间戳，按名字倒序即按时间倒序
        out.sort_by(|a, b| b.path.cmp(&a.path));
        out
    }

    pub fn read(&self, path: &Path) -> Result<String, String> {
        fs::read_to_string(path).map_err(|e| format!("读取 {} 失败：{e}", path.display()))
    }

    /// 重命名一盘棋；只允许改 `games/` 里的文件名，不允许换目录。
    pub fn rename(&self, path: &Path, name: &str) -> Result<PathBuf, String> {
        if !path.starts_with(&self.dir) {
            return Err("只能重命名本工具保存的棋谱".to_string());
        }
        let stem = sanitize_name(name)?;
        let target = self.dir.join(if stem.ends_with(".sgf") {
            stem
        } else {
            format!("{stem}.sgf")
        });
        if target == path {
            return Ok(target);
        }
        if target.exists() {
            return Err(format!(
                "{} 已存在",
                target.file_name().and_then(|n| n.to_str()).unwrap_or("?")
            ));
        }
        fs::rename(path, &target).map_err(|e| format!("重命名 {} 失败：{e}", path.display()))?;
        Ok(target)
    }

    /// 把棋谱移到 `games/.trash/`（带时间戳），不直接删除。
    pub fn trash(&self, path: &Path) -> Result<PathBuf, String> {
        if !path.starts_with(&self.dir) || path.starts_with(self.dir.join(".trash")) {
            return Err("只能删除本工具保存的棋谱".to_string());
        }
        let trash = self.dir.join(".trash");
        fs::create_dir_all(&trash).map_err(|e| format!("创建回收站失败：{e}"))?;
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("game.sgf");
        // 纳秒 + pid，避免同一秒删除同名棋谱时互相覆盖
        let stamp = unix_nanos();
        let pid = std::process::id();
        let mut target = trash.join(format!("{name}.{stamp}-{pid}"));
        let mut i = 1u32;
        while target.exists() {
            target = trash.join(format!("{name}.{stamp}-{pid}-{i}"));
            i += 1;
        }
        fs::rename(path, &target)
            .map_err(|e| format!("移入回收站 {} 失败：{e}", path.display()))?;
        Ok(target)
    }
}

/// 文件名只留一层：去掉路径分隔符、控制字符，限制长度。
fn sanitize_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() || name == "." || name == ".." {
        return Err("名字不能为空".to_string());
    }
    if name.contains(['/', '\\', '\0']) {
        return Err("名字不能包含路径分隔符".to_string());
    }
    let name: String = name.chars().filter(|c| !c.is_control()).take(120).collect();
    if name.is_empty() {
        return Err("名字不能为空".to_string());
    }
    Ok(name)
}

fn data_home() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".local/share")
        })
}

/// 文件的修改时间；读不到就是 `None`（版本判断请用 [`FileStamp`]）。
#[allow(dead_code)]
pub fn mtime(path: &Path) -> Option<SystemTime> {
    fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// rename 之后同步父目录，保证目录项本身也落盘（失败不致命）。
fn sync_dir(dir: &Path) {
    if let Ok(f) = fs::File::open(dir) {
        let _ = f.sync_all();
    }
}

fn unix_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

#[allow(dead_code)]
fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn format_time(secs: u64) -> String {
    match Local.timestamp_opt(secs as i64, 0).single() {
        Some(t) => t.format("%m-%d %H:%M:%S").to_string(),
        None => secs.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store() -> GameStore {
        use std::sync::atomic::{AtomicU64, Ordering};
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("weiqi-store-test-{}-{}", std::process::id(), n));
        GameStore::at(dir)
    }

    #[test]
    fn saves_lists_and_reads() {
        let store = temp_store();
        let path = store.create().unwrap();
        let moves = vec![Move::Play(Point::new(15, 3)), Move::Play(Point::new(3, 15))];
        store
            .save(
                &path,
                19,
                7.5,
                &[],
                &moves,
                WriteOptions {
                    rules: RuleSet::default(),
                    first: Color::Black,
                    expected: None,
                },
            )
            .unwrap();
        assert!(path.exists());
        assert!(!path.with_extension("sgf.tmp").exists());
        let listed = store.list();
        assert_eq!(listed.len(), 1);
        let text = store.read(&path).unwrap();
        let back = sgf::parse(&text).unwrap();
        assert_eq!(back.moves, moves);
        let _ = std::fs::remove_dir_all(store.dir());
    }

    fn opts(expected: Option<FileStamp>) -> WriteOptions {
        WriteOptions {
            rules: RuleSet::default(),
            first: Color::Black,
            expected,
        }
    }

    /// 一个保证和真实文件对不上的指纹（模拟“我们手里的版本过期了”）。
    fn stale() -> Option<FileStamp> {
        Some(FileStamp {
            mtime: UNIX_EPOCH,
            len: 0,
            hash: 0,
        })
    }

    fn tmp_files(dir: &Path) -> Vec<String> {
        std::fs::read_dir(dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.contains(".tmp-"))
            .collect()
    }

    #[test]
    fn stale_version_writes_a_conflict_copy_instead_of_overwriting() {
        let store = temp_store();
        let path = store.dir().join("game-1.sgf");
        let moves = vec![Move::Play(Point::new(2, 2))];
        assert_eq!(
            store.save(&path, 9, 7.5, &[], &moves, opts(None)).unwrap(),
            SaveOutcome::Written
        );
        let original = std::fs::read_to_string(&path).unwrap();

        // 别的进程改过文件 → 我们的 expected_mtime 已经过期
        std::fs::write(&path, "(;SZ[9]KM[0.5];B[dd])").unwrap();
        let external = std::fs::read_to_string(&path).unwrap();

        let other = vec![Move::Play(Point::new(4, 4))];
        let out = store
            .save(&path, 9, 7.5, &[], &other, opts(stale()))
            .unwrap();
        let SaveOutcome::Conflict(alt) = out else {
            panic!("应当另存副本，实际 {out:?}");
        };
        assert!(alt.exists());
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            external,
            "外部修改不能被覆盖"
        );
        assert_ne!(
            std::fs::read_to_string(&alt).unwrap(),
            original,
            "副本应是本次新内容"
        );
        let back = sgf::parse(&std::fs::read_to_string(&alt).unwrap()).unwrap();
        assert_eq!(back.moves, other);
        assert!(tmp_files(store.dir()).is_empty(), "临时文件要清理干净");

        let _ = std::fs::remove_dir_all(store.dir());
    }

    #[test]
    fn concurrent_saves_do_not_corrupt_or_leak_temp_files() {
        let store = temp_store();
        let path = store.dir().join("shared.sgf");
        let mut handles = Vec::new();
        for i in 0..8u8 {
            let store = store.clone();
            let path = path.clone();
            handles.push(std::thread::spawn(move || {
                let moves = vec![Move::Play(Point::new(i, i))];
                store.save(&path, 9, 7.5, &[], &moves, opts(None)).unwrap()
            }));
        }
        for h in handles {
            assert_eq!(h.join().unwrap(), SaveOutcome::Written);
        }
        // 最终文件必须是完整 SGF，不能是两次写入拼出来的半截
        let text = std::fs::read_to_string(&path).unwrap();
        let back = sgf::parse(&text).unwrap();
        assert_eq!(back.moves.len(), 1);
        assert!(tmp_files(store.dir()).is_empty(), "临时文件要清理干净");

        let _ = std::fs::remove_dir_all(store.dir());
    }

    #[test]
    fn failed_save_leaves_the_original_readable() {
        use std::os::unix::fs::PermissionsExt;
        let store = temp_store();
        let path = store.dir().join("keep.sgf");
        let moves = vec![Move::Play(Point::new(1, 1))];
        store.save(&path, 9, 7.5, &[], &moves, opts(None)).unwrap();
        let before = std::fs::read_to_string(&path).unwrap();

        // 目录不可写 → 连临时文件都建不出来
        let mut perms = std::fs::metadata(store.dir()).unwrap().permissions();
        perms.set_mode(0o555);
        std::fs::set_permissions(store.dir(), perms).unwrap();
        let res = store.save(&path, 9, 7.5, &[], &moves, opts(None));
        let mut perms = std::fs::metadata(store.dir()).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(store.dir(), perms).unwrap();

        assert!(res.is_err(), "目录不可写时保存必须失败");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
        let _ = std::fs::remove_dir_all(store.dir());
    }

    #[test]
    fn concurrent_edits_with_the_same_stamp_never_silently_overwrite() {
        let store = temp_store();
        let path = store.dir().join("shared.sgf");
        store
            .save(
                &path,
                9,
                7.5,
                &[],
                &[Move::Play(Point::new(1, 1))],
                opts(None),
            )
            .unwrap();
        let stamp = FileStamp::of(&path).unwrap();

        // 两个进程都拿着「同一份旧版本」去保存：一个成功，另一个必须拿到副本
        let a = store
            .save(
                &path,
                9,
                7.5,
                &[],
                &[Move::Play(Point::new(2, 2))],
                opts(Some(stamp)),
            )
            .unwrap();
        let b = store
            .save(
                &path,
                9,
                7.5,
                &[],
                &[Move::Play(Point::new(3, 3))],
                opts(Some(stamp)),
            )
            .unwrap();
        assert_eq!(a, SaveOutcome::Written);
        let SaveOutcome::Conflict(alt) = b else {
            panic!("第二次保存应当另存副本，实际 {b:?}");
        };
        assert!(alt.exists());
        // 落盘的是 a 的内容，b 的内容在副本里，谁都没丢
        let main = sgf::parse(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(main.moves, vec![Move::Play(Point::new(2, 2))]);
        let copy = sgf::parse(&std::fs::read_to_string(&alt).unwrap()).unwrap();
        assert_eq!(copy.moves, vec![Move::Play(Point::new(3, 3))]);

        let _ = std::fs::remove_dir_all(store.dir());
    }

    #[test]
    fn concurrent_threads_produce_exactly_one_writer() {
        let store = temp_store();
        let path = store.dir().join("race.sgf");
        store
            .save(
                &path,
                9,
                7.5,
                &[],
                &[Move::Play(Point::new(1, 1))],
                opts(None),
            )
            .unwrap();
        let stamp = FileStamp::of(&path).unwrap();

        let mut handles = Vec::new();
        for i in 0..4u8 {
            let store = store.clone();
            let path = path.clone();
            handles.push(std::thread::spawn(move || {
                store
                    .save(
                        &path,
                        9,
                        7.5,
                        &[],
                        &[Move::Play(Point::new(i + 2, 2))],
                        opts(Some(stamp)),
                    )
                    .unwrap()
            }));
        }
        let mut written = 0;
        let mut conflicts = Vec::new();
        for h in handles {
            match h.join().unwrap() {
                SaveOutcome::Written => written += 1,
                SaveOutcome::Conflict(p) => conflicts.push(p),
            }
        }
        assert_eq!(written, 1, "只有第一个能写主文件");
        assert_eq!(conflicts.len(), 3);
        for c in &conflicts {
            assert!(c.exists(), "冲突副本必须落盘：{}", c.display());
        }
        assert_eq!(
            sgf::parse(&std::fs::read_to_string(&path).unwrap())
                .unwrap()
                .moves
                .len(),
            1
        );
        assert!(tmp_files(store.dir()).is_empty());

        let _ = std::fs::remove_dir_all(store.dir());
    }

    #[test]
    fn conflict_copies_never_overwrite_each_other() {
        let store = temp_store();
        let path = store.dir().join("multi.sgf");
        store
            .save(
                &path,
                9,
                7.5,
                &[],
                &[Move::Play(Point::new(1, 1))],
                opts(None),
            )
            .unwrap();

        let mut alts = Vec::new();
        for i in 0..2u8 {
            let out = store
                .save(
                    &path,
                    9,
                    7.5,
                    &[],
                    &[Move::Play(Point::new(i + 2, 2))],
                    opts(stale()),
                )
                .unwrap();
            let SaveOutcome::Conflict(alt) = out else {
                panic!("应当另存副本");
            };
            alts.push(alt);
        }
        assert_ne!(alts[0], alts[1], "两次冲突不能落到同一个文件名");
        assert!(alts.iter().all(|p| p.exists()));

        let _ = std::fs::remove_dir_all(store.dir());
    }

    #[test]
    fn create_never_collides() {
        let store = temp_store();
        let a = store.create().unwrap();
        std::fs::write(&a, "x").unwrap();
        let b = store.create().unwrap();
        assert_ne!(a, b);
        let _ = std::fs::remove_dir_all(store.dir());
    }

    #[test]
    fn rename_moves_file_and_rejects_bad_names() {
        let store = temp_store();
        let path = store.create().unwrap();
        std::fs::write(&path, "(;SZ[9])").unwrap();

        let renamed = store.rename(&path, "opening study").unwrap();
        assert_eq!(
            renamed.file_name().and_then(|n| n.to_str()),
            Some("opening study.sgf")
        );
        assert!(renamed.exists());
        assert!(!path.exists());
        assert_eq!(store.list().len(), 1);

        assert!(store.rename(&renamed, "../escape").is_err());
        assert!(store.rename(&renamed, "  ").is_err());

        let other = store.create().unwrap();
        std::fs::write(&other, "x").unwrap();
        assert!(store.rename(&other, "opening study").is_err(), "同名应拒绝");

        let _ = std::fs::remove_dir_all(store.dir());
    }

    #[test]
    fn trash_twice_does_not_overwrite() {
        let store = temp_store();
        let p1 = store.dir().join("same.sgf");
        std::fs::write(&p1, "(;SZ[9])").unwrap();
        let t1 = store.trash(&p1).unwrap();

        let p2 = store.dir().join("same.sgf");
        std::fs::write(&p2, "(;SZ[9])").unwrap();
        let t2 = store.trash(&p2).unwrap();

        assert_ne!(t1, t2, "同名棋谱连续删除不能落到同一个目标");
        assert!(t1.exists() && t2.exists());
        let _ = std::fs::remove_dir_all(store.dir());
    }

    #[test]
    fn trash_moves_file_out_of_list() {
        let store = temp_store();
        let path = store.create().unwrap();
        std::fs::write(&path, "(;SZ[9])").unwrap();
        assert_eq!(store.list().len(), 1);

        let trashed = store.trash(&path).unwrap();
        assert!(trashed.starts_with(store.dir().join(".trash")));
        assert!(trashed.exists());
        assert!(!path.exists());
        assert!(store.list().is_empty(), "回收站里的棋谱不应出现在列表");

        // 外部文件不能借这个接口删掉
        let outside = std::env::temp_dir().join(format!(
            "weiqi-outside-{}-{}.sgf",
            std::process::id(),
            unix_now()
        ));
        std::fs::write(&outside, "x").unwrap();
        assert!(store.trash(&outside).is_err());
        let _ = std::fs::remove_file(&outside);

        let _ = std::fs::remove_dir_all(store.dir());
    }
}
