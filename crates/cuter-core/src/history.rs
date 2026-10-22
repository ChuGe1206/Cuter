//! 截图历史：原图 + 缩略图的本地缓存。
//!
//! ## 为什么原图也要落盘
//!
//! `docs/HANDOFF.md` 第 8 节给过三个现实约束，这里逐条对应：
//!
//! 1. **只存缩略图不行**：面板里的「再复制一次 / 再贴一次」会拿到一张缩小的图——
//!    等于把用户的图悄悄降质了，这是不能接受的；
//! 2. **只存内存不行**：30 张 4K 位图就是 ~1 GB，直接违反本项目的硬约束；
//! 3. **存原图又会让用户惊讶磁盘占用** → 所以面板里必须把**张数、总字节数**摆出来，
//!    并且「清空历史」和「打开历史目录」都必须一键可达。
//!
//! 另外：历史里**只会有你自己的截图**，不会进剪贴板里贴进来的图（那是另一条路径），
//! 这一点写在面板的说明里。
//!
//! ## 目录布局（`Config::data_dir()/history`）
//!
//! ```text
//! index.toml        条目清单（id / 时间 / 尺寸 / 字节数），原子写入
//! images/<id>.png   原图，无损 PNG
//! thumbs/<id>.png   缩略图，宽不超过 `thumb_width`
//! ```
//!
//! 清单与文件不一致时**以文件为准**：清单里指向的文件已经不在了，就把这条丢掉
//! （用户手动删过目录、或者上次写到一半断电）。反过来，目录里有孤儿文件也不去扫——
//! 那会让"打开历史面板"变成一次目录遍历。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::frame::{Frame, ImageFormat};
use crate::geom::Size;

/// 清单文件名。
pub const INDEX_FILE: &str = "index.toml";
/// 原图目录。
pub const IMAGES_DIR: &str = "images";
/// 缩略图目录。
pub const THUMBS_DIR: &str = "thumbs";

/// 一条历史记录。**只存元数据**：图在磁盘上，按 id 找。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub id: u64,
    /// Unix 秒。排序与清理用它，与时区无关。
    pub created: u64,
    /// 本地时间的显示文本（`YYYY-MM-DD HH:MM:SS`）。存下来是为了让"这张图是什么时候截的"
    /// 在换了时区之后仍然显示当时的时间，也省得 UI 层再引一套时间格式化。
    pub created_text: String,
    pub width: u32,
    pub height: u32,
    /// 原图字节数（面板显示占用用，不必每次去问文件系统）。
    pub bytes: u64,
}

impl HistoryEntry {
    /// 原图路径。
    #[must_use]
    pub fn image_path(&self, root: &Path) -> PathBuf {
        root.join(IMAGES_DIR).join(format!("{}.png", self.id))
    }

    /// 缩略图路径。
    #[must_use]
    pub fn thumb_path(&self, root: &Path) -> PathBuf {
        root.join(THUMBS_DIR).join(format!("{}.png", self.id))
    }

    /// 人类可读的尺寸文本（`1920×1080`）。
    #[must_use]
    pub fn size_text(&self) -> String {
        format!("{}×{}", self.width, self.height)
    }

    /// 人类可读的占用文本（`1.2 MB` / `340 KB`）。
    #[must_use]
    pub fn bytes_text(&self) -> String {
        human_bytes(self.bytes)
    }
}

/// 把字节数写成好读的文本。
#[must_use]
pub fn human_bytes(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    let value = bytes as f64;
    if value < KB {
        format!("{bytes} B")
    } else if value < KB * KB {
        format!("{:.0} KB", value / KB)
    } else {
        format!("{:.1} MB", value / (KB * KB))
    }
}

/// 磁盘上的历史缓存。
#[derive(Debug, Clone)]
pub struct HistoryStore {
    root: PathBuf,
    /// 新的在前。
    entries: Vec<HistoryEntry>,
}

/// 清单文件的内容。
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct Index {
    version: u32,
    entries: Vec<HistoryEntry>,
}

/// 当前清单版本。将来改格式时用它做迁移，而不是"读不动就当空的"。
const INDEX_VERSION: u32 = 1;

impl HistoryStore {
    /// 打开（必要时创建）历史目录并读回清单。
    ///
    /// **永不失败**：读不动清单就当空的（图还在磁盘上，但不再显示），
    /// 建不了目录就返回一个"什么都存不下"的空 store，让调用方照常截图。
    #[must_use]
    pub fn open(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        let mut entries = match Self::read_index(&root) {
            Ok(entries) => entries,
            Err(err) => {
                log::warn!("读取历史清单失败（{err}），这次先当作空历史");
                Vec::new()
            }
        };
        // 以文件为准：清单里指向的文件没了就把这条丢掉。
        let before = entries.len();
        entries.retain(|entry| entry.image_path(&root).is_file());
        if entries.len() != before {
            log::info!(
                "历史清单里有 {} 条的图已经不在磁盘上，已丢弃",
                before - entries.len()
            );
        }
        entries.sort_by_key(|entry| std::cmp::Reverse(entry.id));
        Self { root, entries }
    }

    /// 历史目录。
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// 全部条目（新的在前）。
    #[must_use]
    pub fn entries(&self) -> &[HistoryEntry] {
        &self.entries
    }

    /// 张数。
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// 占用字节总数。
    #[must_use]
    pub fn total_bytes(&self) -> u64 {
        self.entries.iter().map(|entry| entry.bytes).sum()
    }

    /// 按 id 找条目。
    #[must_use]
    pub fn find(&self, id: u64) -> Option<&HistoryEntry> {
        self.entries.iter().find(|entry| entry.id == id)
    }

    /// 追加一条：写原图 + 缩略图 + 清单，并按 `limit` 淘汰最旧的。
    ///
    /// `limit == 0` 表示不保留历史，此时什么都不写（调用方本来就该先看配置）。
    pub fn add(
        &mut self,
        frame: &Frame,
        limit: usize,
        thumb_width: u32,
        created: u64,
        created_text: String,
    ) -> Result<HistoryEntry> {
        if limit == 0 || frame.is_empty() {
            return Err(Error::Invalid("历史已关闭或图片为空".to_owned()));
        }
        std::fs::create_dir_all(self.root.join(IMAGES_DIR)).map_err(|e| Error::io(&self.root, e))?;
        std::fs::create_dir_all(self.root.join(THUMBS_DIR)).map_err(|e| Error::io(&self.root, e))?;

        let id = self.entries.iter().map(|e| e.id).max().unwrap_or(0) + 1;
        let entry = HistoryEntry {
            id,
            created,
            created_text,
            width: frame.width(),
            height: frame.height(),
            bytes: 0, // 写完才知道（PNG 是压缩的）
        };

        let image_path = entry.image_path(&self.root);
        frame.save_as(&image_path, ImageFormat::Png)?;
        let bytes = std::fs::metadata(&image_path).map(|m| m.len()).unwrap_or(0);

        // 缩略图失败不该让整条记录失败：面板大不了显示占位符，
        // 但"再复制一次"用的是原图，功能不受影响。
        let thumb = frame.resized(thumb_size(frame.size(), thumb_width), image::imageops::FilterType::Triangle);
        if let Err(err) = thumb.save_as(entry.thumb_path(&self.root), ImageFormat::Png) {
            log::warn!("写历史缩略图失败（面板会退回占位符）：{err}");
        }

        let entry = HistoryEntry { bytes, ..entry };
        self.entries.insert(0, entry.clone());
        self.prune(limit);
        self.save_index()?;
        Ok(entry)
    }

    /// 删除一条（连同磁盘上的两个文件）。
    pub fn remove(&mut self, id: u64) -> Result<()> {
        let Some(entry) = self.find(id).cloned() else {
            return Ok(());
        };
        remove_file(&entry.image_path(&self.root));
        remove_file(&entry.thumb_path(&self.root));
        self.entries.retain(|e| e.id != id);
        self.save_index()
    }

    /// 清空历史（整个目录，包括孤儿文件——这是"一键清空"该有的语义）。
    pub fn clear(&mut self) -> Result<()> {
        self.entries.clear();
        if self.root.exists() {
            std::fs::remove_dir_all(&self.root).map_err(|e| Error::io(&self.root, e))?;
        }
        // 目录整个删掉之后清单也没了；下次 add 会重建。
        Ok(())
    }

    /// 读回原图。
    pub fn load_image(&self, id: u64) -> Result<Frame> {
        let entry = self
            .find(id)
            .ok_or_else(|| Error::Invalid(format!("历史里没有 id={id} 这一条")))?;
        let path = entry.image_path(&self.root);
        let bytes = std::fs::read(&path).map_err(|e| Error::io(&path, e))?;
        Frame::decode(&bytes)
    }

    /// 读回缩略图的 RGBA（面板用）。
    ///
    /// 返回 `None` 表示这张缩略图没了（或坏了）——面板画占位符，不报错。
    #[must_use]
    pub fn load_thumb_rgba(&self, id: u64) -> Option<(u32, u32, Vec<u8>)> {
        let entry = self.find(id)?;
        let bytes = std::fs::read(entry.thumb_path(&self.root)).ok()?;
        let frame = Frame::decode(&bytes).ok()?;
        Some((frame.width(), frame.height(), frame.into_data()))
    }

    /// 淘汰到 `limit` 条以内（从最旧的开始删）。
    fn prune(&mut self, limit: usize) {
        while self.entries.len() > limit {
            let Some(entry) = self.entries.pop() else {
                break;
            };
            log::debug!("历史超出上限，淘汰 id={}", entry.id);
            remove_file(&entry.image_path(&self.root));
            remove_file(&entry.thumb_path(&self.root));
        }
    }

    /// 原子写入清单（和配置文件同一套做法：临时文件 + rename）。
    fn save_index(&self) -> Result<()> {
        if self.entries.is_empty() && !self.root.exists() {
            return Ok(());
        }
        std::fs::create_dir_all(&self.root).map_err(|e| Error::io(&self.root, e))?;
        let index = Index {
            version: INDEX_VERSION,
            entries: self.entries.clone(),
        };
        let text = toml::to_string_pretty(&index)?;
        let path = self.root.join(INDEX_FILE);
        write_atomically(&path, text.as_bytes()).map_err(|e| Error::io(&path, e))
    }

    fn read_index(root: &Path) -> Result<Vec<HistoryEntry>> {
        let path = root.join(INDEX_FILE);
        let text = std::fs::read_to_string(&path).map_err(|e| Error::io(&path, e))?;
        let index: Index = toml::from_str(&text)?;
        if index.version > INDEX_VERSION {
            return Err(Error::Invalid(format!(
                "历史清单版本 {} 比本程序支持的 {INDEX_VERSION} 新",
                index.version
            )));
        }
        Ok(index.entries)
    }
}

/// 按最大宽度算缩略图尺寸（等比，至少 1 像素）。
#[must_use]
pub fn thumb_size(size: Size, max_width: u32) -> Size {
    let max_width = max_width.max(1);
    if size.w <= max_width || size.w == 0 {
        return size;
    }
    let height = ((u64::from(size.h) * u64::from(max_width)) / u64::from(size.w)).max(1);
    Size::new(max_width, height as u32)
}

fn remove_file(path: &Path) {
    if let Err(err) = std::fs::remove_file(path)
        && err.kind() != std::io::ErrorKind::NotFound
    {
        log::warn!("删除历史文件失败（{}）：{err}", path.display());
    }
}

/// 原子写文件（配置那边是 private 的，这里是同一套做法的第二处用处）。
fn write_atomically(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;

    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    let result = (|| -> std::io::Result<()> {
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::Size;

    fn temp_root(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "cuter-history-test-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos())
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn frame(w: u32, h: u32, shade: u8) -> Frame {
        Frame::filled(Size::new(w, h), [shade, shade, shade, 255])
    }

    fn add(store: &mut HistoryStore, w: u32, h: u32, shade: u8, limit: usize) {
        store
            .add(&frame(w, h, shade), limit, 240, 1_700_000_000, "2026-09-29 20:00:00".to_owned())
            .expect("写入历史");
    }

    #[test]
    fn add_writes_image_thumb_and_index() {
        let root = temp_root("add");
        let mut store = HistoryStore::open(&root);
        let entry = store
            .add(
                &frame(800, 600, 10),
                5,
                240,
                1_700_000_000,
                "2026-09-29 20:00:00".to_owned(),
            )
            .unwrap();

        assert!(entry.image_path(&root).is_file(), "原图必须落盘");
        assert!(entry.thumb_path(&root).is_file(), "缩略图必须落盘");
        assert!(root.join(INDEX_FILE).is_file(), "清单必须落盘");
        assert_eq!(entry.width, 800);
        assert_eq!(entry.height, 600);
        assert!(entry.bytes > 0, "字节数要真去量文件");

        // 重新打开：内容和刚才一致（清单能读回来）
        let reopened = HistoryStore::open(&root);
        assert_eq!(reopened.len(), 1);
        assert_eq!(reopened.entries()[0], entry);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn newest_first_and_pruned_to_limit() {
        let root = temp_root("prune");
        let mut store = HistoryStore::open(&root);
        for i in 0..5 {
            add(&mut store, 100, 100, i * 10, 3);
        }
        assert_eq!(store.len(), 3, "上限 3 就只能留 3 条");
        let ids: Vec<u64> = store.entries().iter().map(|e| e.id).collect();
        assert_eq!(ids, vec![5, 4, 3], "保留最新的，且新的在前");
        // 被淘汰的图必须真的删掉，否则磁盘会一直涨
        assert!(!root.join(IMAGES_DIR).join("1.png").exists());
        assert!(!root.join(IMAGES_DIR).join("2.png").exists());
        assert!(root.join(IMAGES_DIR).join("3.png").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn zero_limit_writes_nothing() {
        let root = temp_root("zero");
        let mut store = HistoryStore::open(&root);
        assert!(store.add(&frame(10, 10, 1), 0, 240, 0, String::new()).is_err());
        assert!(store.is_empty());
        assert!(!root.exists(), "关闭历史时不该建目录");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn removing_a_missing_image_drops_the_entry_on_open() {
        let root = temp_root("missing");
        let mut store = HistoryStore::open(&root);
        add(&mut store, 100, 100, 1, 5);
        add(&mut store, 100, 100, 2, 5);
        // 用户手动删掉其中一张原图
        std::fs::remove_file(root.join(IMAGES_DIR).join("1.png")).unwrap();

        let reopened = HistoryStore::open(&root);
        assert_eq!(reopened.len(), 1, "文件没了的那条要被丢掉");
        assert_eq!(reopened.entries()[0].id, 2);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn remove_and_clear_delete_files() {
        let root = temp_root("clear");
        let mut store = HistoryStore::open(&root);
        add(&mut store, 100, 100, 1, 5);
        add(&mut store, 100, 100, 2, 5);

        store.remove(1).unwrap();
        assert_eq!(store.len(), 1);
        assert!(!root.join(IMAGES_DIR).join("1.png").exists());

        store.clear().unwrap();
        assert!(store.is_empty());
        assert!(!root.exists(), "清空 = 连目录一起清掉（否则会留下孤儿文件）");
    }

    #[test]
    fn image_roundtrips_byte_for_byte() {
        let root = temp_root("roundtrip");
        let mut store = HistoryStore::open(&root);
        let original = frame(64, 32, 77);
        store
            .add(&original, 3, 240, 0, String::new())
            .unwrap();
        let back = store.load_image(1).unwrap();
        assert_eq!(back.size(), original.size());
        assert_eq!(back.data(), original.data(), "历史里的原图必须与截图逐字节一致");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn thumb_size_keeps_aspect_and_never_upscales() {
        assert_eq!(thumb_size(Size::new(1920, 1080), 240), Size::new(240, 135));
        assert_eq!(thumb_size(Size::new(100, 50), 240), Size::new(100, 50));
        // 极端长条也不能退化成 0 高
        assert_eq!(thumb_size(Size::new(4000, 3), 240).h, 1);
    }

    #[test]
    fn thumb_is_actually_smaller() {
        let root = temp_root("thumb");
        let mut store = HistoryStore::open(&root);
        let entry = store
            .add(&frame(1200, 800, 33), 5, 240, 0, String::new())
            .unwrap();
        let (w, h, rgba) = store.load_thumb_rgba(entry.id).expect("缩略图能读回");
        assert_eq!((w, h), (240, 160));
        assert_eq!(rgba.len(), (240 * 160 * 4) as usize);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn total_bytes_and_human_text() {
        let root = temp_root("bytes");
        let mut store = HistoryStore::open(&root);
        add(&mut store, 200, 200, 1, 5);
        assert!(store.total_bytes() > 0);
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(2048), "2 KB");
        assert_eq!(human_bytes(3 * 1024 * 1024), "3.0 MB");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn corrupt_index_falls_back_to_empty_history() {
        let root = temp_root("corrupt");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join(INDEX_FILE), "这不是 TOML").unwrap();
        let store = HistoryStore::open(&root);
        assert!(store.is_empty(), "清单坏了也不能让程序起不来");
        let _ = std::fs::remove_dir_all(&root);
    }
}
