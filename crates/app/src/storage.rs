//! 棋谱持久化：`$XDG_DATA_HOME/weiqi-study/games/*.sgf`。
//!
//! 写入规则：先写同目录的 `.tmp`，`fsync` 后原子 rename。
//! 删除不真删：移到 `games/.trash/`，文件名带时间戳，可手动找回。

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{Local, TimeZone};
use goban::{Color, Move, Point};

#[derive(Clone, Debug)]
pub struct GameEntry {
    pub path: PathBuf,
    pub label: String,
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
    pub fn create(&self) -> Result<PathBuf, String> {
        let now = unix_now();
        for i in 0..1000u32 {
            let name = if i == 0 {
                format!("game-{now}.sgf")
            } else {
                format!("game-{now}-{i}.sgf")
            };
            let path = self.dir.join(name);
            if !path.exists() {
                return Ok(path);
            }
        }
        Err("无法生成棋谱文件名".to_string())
    }

    /// 原子写入：临时文件 + fsync + rename。
    pub fn save(
        &self,
        path: &Path,
        size: u8,
        komi: f32,
        setup: &[(Color, Point)],
        moves: &[Move],
    ) -> Result<(), String> {
        let text = sgf::write_game(size, komi, setup, moves);
        let tmp = path.with_extension("sgf.tmp");
        {
            let mut file = fs::File::create(&tmp)
                .map_err(|e| format!("创建 {} 失败：{e}", tmp.display()))?;
            file.write_all(text.as_bytes())
                .map_err(|e| format!("写入 {} 失败：{e}", tmp.display()))?;
            file.sync_all()
                .map_err(|e| format!("同步 {} 失败：{e}", tmp.display()))?;
        }
        fs::rename(&tmp, path).map_err(|e| {
            let _ = fs::remove_file(&tmp);
            format!("保存 {} 失败：{e}", path.display())
        })
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
                .map(|t| t.duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0))
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
            return Err(format!("{} 已存在", target.file_name().and_then(|n| n.to_str()).unwrap_or("?")));
        }
        fs::rename(path, &target)
            .map_err(|e| format!("重命名 {} 失败：{e}", path.display()))?;
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
        let target = trash.join(format!("{name}.{}", unix_now()));
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
    let name: String = name
        .chars()
        .filter(|c| !c.is_control())
        .take(120)
        .collect();
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
        let dir = std::env::temp_dir().join(format!(
            "weiqi-store-test-{}-{}",
            std::process::id(),
            n
        ));
        GameStore::at(dir)
    }

    #[test]
    fn saves_lists_and_reads() {
        let store = temp_store();
        let path = store.create().unwrap();
        let moves = vec![Move::Play(Point::new(15, 3)), Move::Play(Point::new(3, 15))];
        store.save(&path, 19, 7.5, &[], &moves).unwrap();
        assert!(path.exists());
        assert!(!path.with_extension("sgf.tmp").exists());
        let listed = store.list();
        assert_eq!(listed.len(), 1);
        let text = store.read(&path).unwrap();
        let back = sgf::parse(&text).unwrap();
        assert_eq!(back.moves, moves);
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
        assert_eq!(renamed.file_name().and_then(|n| n.to_str()), Some("opening study.sgf"));
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
