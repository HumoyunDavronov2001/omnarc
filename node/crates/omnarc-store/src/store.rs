//! The models on this device, shared with the cluster over iroh-blobs (PLAN.md §3.8).
//!
//! A model file is added where it lies — the store hashes it with BLAKE3 and refers to it, so 40 GB are not copied.
//! Another device asks for the header, works out which bytes its window of layers needs ([`Layout`]) and fetches
//! only those; every chunk is checked against the BLAKE3 tree before it is kept, and a fetch that breaks off resumes
//! with the chunks still missing. The engine then gets a file of the model's full size that holds the window's
//! bytes and holes elsewhere ([`ModelStore::export`]); it never reads the holes.

use std::fmt;
use std::io::SeekFrom;
use std::ops::Range;
use std::path::{Path, PathBuf};

use iroh_blobs::api::blobs::{AddPathOptions, ImportMode};
use iroh_blobs::api::proto::ExportRangesItem;
use iroh_blobs::api::remote::GetProgressItem;
use iroh_blobs::protocol::{ChunkRanges, ChunkRangesExt, GetRequest};
use iroh_blobs::store::fs::FsStore;
use iroh_blobs::{BlobFormat, BlobsProtocol};
use n0_future::StreamExt;
use omnarc_network::Connection;
use tokio::io::{AsyncSeekExt, AsyncWriteExt};

pub use iroh_blobs::{ALPN as BLOBS_ALPN, Hash};

use crate::gguf::{self, Header};
use crate::layers::{Layout, Window};
use crate::sparse;

/// The first piece of a header asked for; it doubles until the header is whole, so at most the header's own
/// length of tensor bytes comes along with it.
const FIRST_HEADER_BYTES: u64 = 64 << 10;

/// The size of the writes of an export.
const WRITE_BYTES: usize = 8 << 20;

/// Tags of models in the store: `model/<name>`.
const TAG_PREFIX: &str = "model/";

/// An error of the model store.
#[derive(Debug)]
pub struct Error(String);

impl Error {
    fn new(context: &str, error: impl fmt::Display) -> Self {
        Self(format!("{context}: {error}"))
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

/// A model the store holds, in whole or in part.
#[derive(Debug, Clone)]
pub struct Model {
    pub name: String,
    pub hash: Hash,
    pub header: Header,
    pub layout: Layout,
}

impl Model {
    fn new(name: String, hash: Hash, header: Header) -> Result<Self> {
        let layout = Layout::new(&header).map_err(|e| Error::new(&name, e))?;
        Ok(Self { name, hash, header, layout })
    }
}

/// The models of this device: a store of BLAKE3-verified blobs in a directory of its own.
#[derive(Debug, Clone)]
pub struct ModelStore {
    store: FsStore,
    dir: PathBuf,
}

impl ModelStore {
    /// Opens the store in `dir`, creating it the first time. Only one process can hold it open.
    ///
    /// # Errors
    /// The directory cannot be created, or another process holds the store.
    pub async fn open(dir: &Path) -> Result<Self> {
        // iroh-blobs never answers when another process holds its database, so that is asked first: the lock redb
        // takes (flock, LockFileEx) is the one File::try_lock asks for
        if let Ok(db) = std::fs::OpenOptions::new().read(true).write(true).open(dir.join("blobs.db"))
            && let Err(std::fs::TryLockError::WouldBlock) = db.try_lock()
        {
            return Err(Error("opening the model store: Database already open by another process".into()));
        }
        let store = FsStore::load(dir).await.map_err(|e| Error::new("opening the model store", e))?;
        Ok(Self { store, dir: dir.to_path_buf() })
    }

    /// The handler that serves this store's blobs to other devices (`iroh_blobs::ALPN`).
    #[must_use]
    pub fn protocol(&self) -> BlobsProtocol {
        BlobsProtocol::new(&self.store, None)
    }

    /// Adds the model file at `path`, which stays where it is and must not change afterwards. Hashing reads the
    /// whole file once; `progress` hears how many bytes are done.
    ///
    /// # Errors
    /// The file cannot be read, or it is not a model in the GGUF format.
    pub async fn add(&self, path: &Path, mut progress: impl FnMut(u64)) -> Result<Model> {
        let path = std::path::absolute(path).map_err(|e| Error::new(&path.display().to_string(), e))?;
        // a header that is not a model is refused before the long hashing
        let header = read_header_from_file(&path).await?;
        let name = path.file_stem().map_or_else(|| "model".to_owned(), |s| s.to_string_lossy().into_owned());

        let options = AddPathOptions { path: path.clone(), format: BlobFormat::Raw, mode: ImportMode::TryReference };
        let mut stream = self.store.blobs().add_path_with_opts(options).stream().await;
        let mut tag = None;
        while let Some(item) = stream.next().await {
            use iroh_blobs::api::proto::AddProgressItem as Item;
            match item {
                Item::CopyProgress(n) | Item::OutboardProgress(n) => progress(n),
                Item::Done(temp) => tag = Some(temp),
                Item::Error(e) => return Err(Error::new(&format!("adding {}", path.display()), e)),
                Item::Size(_) | Item::CopyDone => {}
            }
        }
        let temp = tag.ok_or_else(|| Error(format!("adding {} stopped before the end", path.display())))?;
        let name = self.tag(Some(name), String::new(), temp.hash()).await?;
        Model::new(name, temp.hash(), header)
    }

    /// Keeps the model `hash` under one name: `name` when given, else the name it already has, else `default`.
    /// A name another model has gets the start of the hash added. Returns the name it is kept under.
    async fn tag(&self, name: Option<String>, default: String, hash: Hash) -> Result<String> {
        let known: Vec<String> = self.list().await?.into_iter().filter(|(_, h)| *h == hash).map(|(n, _)| n).collect();
        let mut name = match (name, known.first()) {
            (Some(name), _) => name,
            (None, Some(known)) => return Ok(known.clone()),
            (None, None) => default,
        };
        let other = self.store.tags().get(format!("{TAG_PREFIX}{name}")).await.map_err(tag_error)?;
        if other.is_some_and(|t| t.hash != hash) {
            name = format!("{name}-{}", &hash.to_hex()[..8]);
        }
        self.store.tags().set(format!("{TAG_PREFIX}{name}"), hash).await.map_err(tag_error)?;
        for old in known.iter().filter(|old| **old != name) {
            self.store.tags().delete(format!("{TAG_PREFIX}{old}")).await.map_err(tag_error)?;
        }
        Ok(name)
    }

    /// The models in the store, by name, whole or in part.
    ///
    /// # Errors
    /// The store cannot be read.
    pub async fn list(&self) -> Result<Vec<(String, Hash)>> {
        let mut tags = self.store.tags().list_prefix(TAG_PREFIX).await.map_err(tag_error)?;
        let mut out = Vec::new();
        while let Some(tag) = tags.next().await {
            let tag = tag.map_err(tag_error)?;
            let name = String::from_utf8_lossy(&tag.name.0[TAG_PREFIX.len()..]).into_owned();
            out.push((name, tag.hash));
        }
        Ok(out)
    }

    /// The model with this name or hash, from the header in the store.
    ///
    /// # Errors
    /// The store has no such model, or not its header.
    pub async fn model(&self, name_or_hash: &str) -> Result<Model> {
        let list = self.list().await?;
        let found = list.into_iter().find(|(name, hash)| name == name_or_hash || hash.to_hex() == name_or_hash);
        let (name, hash) = found.ok_or_else(|| Error(format!("no model {name_or_hash} on this device")))?;
        let header = self.local_header(hash).await?;
        Model::new(name, hash, header)
    }

    /// The header of a model whose first bytes are in the store.
    async fn local_header(&self, hash: Hash) -> Result<Header> {
        let mut want = FIRST_HEADER_BYTES;
        loop {
            let bytes = self.read(hash, 0..want).await?;
            match Header::parse(&bytes) {
                Ok(header) => return Ok(header),
                Err(gguf::Error::Incomplete { needed }) if needed > want && (bytes.len() as u64) >= want => {
                    want = needed.max(want * 2).min(gguf::MAX_HEADER);
                }
                Err(e) => return Err(Error::new("reading the model header", e)),
            }
        }
    }

    /// The bytes of `range` that are in the store.
    async fn read(&self, hash: Hash, range: Range<u64>) -> Result<Vec<u8>> {
        self.store
            .blobs()
            .export_ranges(hash, range)
            .concatenate()
            .await
            .map_err(|e| Error::new("reading the store", e))
    }

    /// How many bytes of `window` are not in the store yet.
    ///
    /// # Errors
    /// The window is not a window of the model, or the store cannot be read.
    pub async fn missing(&self, model: &Model, window: Window) -> Result<u64> {
        let request = request(model, window)?;
        let local = self.store.remote().local_for_request(request).await.map_err(|e| Error::new("the store", e))?;
        if local.is_complete() {
            return Ok(0);
        }
        // the store counts whole chunks of 1 KiB, the layout exact bytes: an estimate, never 0 while incomplete
        let wanted = model.layout.bytes(window).map_err(Error)?;
        Ok(wanted.saturating_sub(local.local_bytes()).max(1))
    }

    /// Fetches `window` of the model `hash` from the device at the other end of `connection`: its header first,
    /// then only the bytes of its layers that are still missing. The model is kept under `name`, or under the name
    /// its header gives. `progress` hears how many bytes have arrived.
    ///
    /// # Errors
    /// The other device does not have them, the connection breaks or a chunk fails its BLAKE3 check. What arrived
    /// before stays in the store, and the next fetch resumes from there.
    pub async fn fetch(
        &self,
        connection: &Connection,
        name: Option<&str>,
        hash: Hash,
        window: Window,
        mut progress: impl FnMut(u64),
    ) -> Result<Model> {
        // the header, a piece at a time until it is whole
        let mut want = FIRST_HEADER_BYTES;
        let header = loop {
            let request = GetRequest::blob_ranges(hash, ChunkRanges::bytes(0..want));
            self.get(connection, request, &mut |_| {}).await?;
            let bytes = self.read(hash, 0..want).await?;
            match Header::parse(&bytes) {
                Ok(header) => break header,
                Err(gguf::Error::Incomplete { needed }) if needed > want && (bytes.len() as u64) >= want => {
                    want = needed.max(want * 2).min(gguf::MAX_HEADER);
                }
                Err(e) => return Err(Error::new("the model header from the other device", e)),
            }
        };
        // the layers land at their places in the store's file of the model: on Windows the file must be sparse before
        // they arrive, or every byte before them takes disk space
        self.make_part_sparse(hash);
        // the tag keeps a part of a model as much as a whole one
        let name = self.tag(name.map(str::to_owned), default_name(&header, hash), hash).await?;
        let model = Model::new(name, hash, header)?;

        let local = self
            .store
            .remote()
            .local_for_request(request(&model, window)?)
            .await
            .map_err(|e| Error::new("the store", e))?;
        if !local.is_complete() {
            self.get(connection, local.missing(), &mut progress).await?;
        }
        Ok(model)
    }

    /// The store's file of the model `hash` — iroh-blobs keeps a blob that outgrows its database in
    /// `data/<hash>.data` — marked sparse; its content stays as it is.
    fn make_part_sparse(&self, hash: Hash) {
        let path = self.part_path(hash);
        if let Ok(file) = std::fs::OpenOptions::new().write(true).open(&path) {
            // without sparse files the holes are written out, as they always were
            let _ = sparse::make_sparse(&file);
        }
    }

    /// Where the store keeps the bytes of the model `hash` once they outgrow its database.
    #[must_use]
    pub fn part_path(&self, hash: Hash) -> PathBuf {
        self.dir.join("data").join(format!("{}.data", hash.to_hex()))
    }

    async fn get(&self, connection: &Connection, request: GetRequest, progress: &mut impl FnMut(u64)) -> Result<()> {
        let mut stream = self.store.remote().execute_get(connection.clone(), request).stream();
        while let Some(item) = stream.next().await {
            match item {
                GetProgressItem::Progress(n) => progress(n),
                GetProgressItem::Done(_) => return Ok(()),
                GetProgressItem::Error(e) => return Err(Error::new("fetching from the other device", e)),
            }
        }
        Err(Error("the fetch stopped without an answer".into()))
    }

    /// Writes `window` of the model to `target` for the engine: a file of the model's full size with the window's
    /// bytes at their places and holes elsewhere, which take no disk space (on Windows the file is marked sparse).
    ///
    /// # Errors
    /// The store does not hold the whole window, or the file cannot be written.
    pub async fn export(&self, model: &Model, window: Window, target: &Path) -> Result<()> {
        self.export_windows(model, &[window], target).await
    }

    /// [`export`](Self::export) for several windows in one file: the head of a ring holds layer 0 and the output.
    ///
    /// # Errors
    /// As [`export`](Self::export).
    pub async fn export_windows(&self, model: &Model, windows: &[Window], target: &Path) -> Result<()> {
        let mut ranges: Vec<std::ops::Range<u64>> = Vec::new();
        for &window in windows {
            let missing = self.missing(model, window).await?;
            if missing > 0 {
                return Err(Error(format!(
                    "{missing} bytes of the layers {window} of {} are not here yet",
                    model.name
                )));
            }
            ranges.extend(model.layout.ranges(window).map_err(Error)?);
        }
        // the header and the shared tensors come with every window: each byte is written once
        let merged = merge(ranges);
        let target_name = target.display().to_string();
        // a file this model was exported to before keeps what it holds: only the bytes it lacks are written, so a new
        // plan that moves a layer or two writes those and not the whole model again (9 GB took 8 minutes on a disk)
        let written_list = target.with_extension("ranges");
        let written = match std::fs::metadata(target) {
            Ok(meta) if meta.len() == model.layout.file_len => read_ranges(&written_list),
            _ => Vec::new(),
        };
        let wanted = merged;
        let ranges = subtract(&wanted, &written);
        if ranges.is_empty() {
            return Ok(());
        }
        let fresh = written.is_empty();
        let file = if fresh {
            let _ = std::fs::remove_file(&written_list);
            let file = create_unmapped(target).await.map_err(|e| Error::new(&target_name, e))?;
            // sparse before it grows, or Windows allocates the whole length; without sparse files the holes are written
            let _ = sparse::make_sparse(&file);
            file
        } else {
            // neither truncated nor resized: Windows refuses both while the engine of the plan before still maps the
            // file, and writes into it are what it allows
            std::fs::OpenOptions::new().write(true).open(target).map_err(|e| Error::new(&target_name, e))?
        };
        let mut file = tokio::fs::File::from_std(file);
        if fresh {
            file.set_len(model.layout.file_len).await.map_err(|e| Error::new(&target_name, e))?;
        }
        for range in ranges {
            // the store hands out 1 KiB leaves; they are gathered into large writes
            let mut buffer = Vec::with_capacity(WRITE_BYTES);
            let mut at = range.start;
            file.seek(SeekFrom::Start(at)).await.map_err(|e| Error::new(&target_name, e))?;
            let mut stream = self.store.blobs().export_ranges(model.hash, range.clone()).stream();
            while let Some(item) = stream.next().await {
                match item {
                    ExportRangesItem::Size(_) => {}
                    ExportRangesItem::Data(leaf) => {
                        let end = leaf.offset + leaf.data.len() as u64;
                        if leaf.offset != at || end > range.end {
                            return Err(Error(format!("the store gave bytes {}..{end} out of order", leaf.offset)));
                        }
                        buffer.extend_from_slice(&leaf.data);
                        at = end;
                        if buffer.len() >= WRITE_BYTES {
                            file.write_all(&buffer).await.map_err(|e| Error::new(&target_name, e))?;
                            buffer.clear();
                        }
                    }
                    ExportRangesItem::Error(e) => return Err(Error::new("reading the store", e)),
                }
            }
            file.write_all(&buffer).await.map_err(|e| Error::new(&target_name, e))?;
            if at != range.end {
                return Err(Error(format!("the store ended at {at}, before {}", range.end)));
            }
        }
        file.sync_all().await.map_err(|e| Error::new(&target_name, e))?;
        // only once the bytes are on the disk does the list say so: an export cut short is written again
        let mut held = written;
        held.extend(wanted);
        let list = merge(held).iter().fold(String::new(), |mut list, r| {
            use std::fmt::Write as _;
            let _ = writeln!(list, "{}-{}", r.start, r.end);
            list
        });
        std::fs::write(&written_list, list).map_err(|e| Error::new(&written_list.display().to_string(), e))?;
        Ok(())
    }

    /// Writes what the store holds and closes it.
    pub async fn shutdown(&self) {
        let _ = self.store.shutdown().await;
    }
}

/// Creates (or truncates) `path`. On Windows a file another process still maps cannot be truncated: the engine of
/// the plan before may take a moment to exit, so it is tried again for a while.
async fn create_unmapped(path: &Path) -> std::io::Result<std::fs::File> {
    /// `ERROR_USER_MAPPED_FILE`
    const MAPPED: i32 = 1224;
    let mut tries = 0;
    loop {
        match std::fs::File::create(path) {
            Err(e) if e.raw_os_error() == Some(MAPPED) && tries < 60 => {
                tries += 1;
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
            result => return result,
        }
    }
}

/// The byte ranges a list written by [`ModelStore::export_windows`] names; none when it is missing or unreadable.
fn read_ranges(path: &Path) -> Vec<Range<u64>> {
    let Ok(text) = std::fs::read_to_string(path) else { return Vec::new() };
    let parsed: Option<Vec<Range<u64>>> = text
        .lines()
        .filter(|l| !l.is_empty())
        .map(|l| {
            let (a, b) = l.split_once('-')?;
            Some(a.parse().ok()?..b.parse().ok()?)
        })
        .collect();
    merge(parsed.unwrap_or_default())
}

/// `ranges` sorted, with those that touch or overlap made one.
fn merge(mut ranges: Vec<Range<u64>>) -> Vec<Range<u64>> {
    ranges.retain(|r| r.start < r.end);
    ranges.sort_by_key(|r| r.start);
    let mut merged: Vec<Range<u64>> = Vec::with_capacity(ranges.len());
    for range in ranges {
        match merged.last_mut() {
            Some(last) if range.start <= last.end => last.end = last.end.max(range.end),
            _ => merged.push(range),
        }
    }
    merged
}

/// The parts of `wanted` that `held` lacks; both sorted and merged.
fn subtract(wanted: &[Range<u64>], held: &[Range<u64>]) -> Vec<Range<u64>> {
    let mut out = Vec::new();
    for w in wanted {
        let mut at = w.start;
        for h in held.iter().filter(|h| h.end > w.start && h.start < w.end) {
            if h.start > at {
                out.push(at..h.start);
            }
            at = at.max(h.end);
        }
        if at < w.end {
            out.push(at..w.end);
        }
    }
    out
}

/// The request for the chunks of `window`.
fn request(model: &Model, window: Window) -> Result<GetRequest> {
    let mut chunks = ChunkRanges::empty();
    for range in model.layout.ranges(window).map_err(Error)? {
        chunks |= ChunkRanges::bytes(range);
    }
    Ok(GetRequest::blob_ranges(model.hash, chunks))
}

/// The model's own name with the characters a file name cannot hold replaced, or a name from its hash.
fn default_name(header: &Header, hash: Hash) -> String {
    let name: String = header
        .name()
        .unwrap_or_default()
        .chars()
        .map(|c| if c.is_alphanumeric() || "-_.".contains(c) { c } else { '-' })
        .collect();
    if name.trim_matches('-').is_empty() { format!("model-{}", &hash.to_hex()[..12]) } else { name }
}

fn tag_error(e: impl fmt::Display) -> Error {
    Error::new("the tags of the model store", e)
}

/// Reads the header at the start of the file at `path`.
async fn read_header_from_file(path: &PathBuf) -> Result<Header> {
    use tokio::io::AsyncReadExt;
    let name = path.display().to_string();
    let mut file = tokio::fs::File::open(path).await.map_err(|e| Error::new(&name, e))?;
    let mut bytes = Vec::new();
    let mut want = FIRST_HEADER_BYTES;
    loop {
        let have = bytes.len() as u64;
        (&mut file).take(want - have).read_to_end(&mut bytes).await.map_err(|e| Error::new(&name, e))?;
        match Header::parse(&bytes) {
            Ok(header) => return Ok(header),
            Err(gguf::Error::Incomplete { needed }) if (bytes.len() as u64) == want => {
                want = needed.max(want * 2).min(gguf::MAX_HEADER);
            }
            Err(e) => return Err(Error::new(&name, e)),
        }
    }
}

#[cfg(test)]
#[allow(clippy::single_range_in_vec_init)]
mod range_tests {
    use super::{merge, subtract};

    #[test]
    fn only_what_is_not_held_is_written_again() {
        assert_eq!(merge(vec![5..9, 0..3, 3..4, 8..12]), vec![0..4, 5..12]);
        assert_eq!(subtract(&[0..10, 20..30], &[]), vec![0..10, 20..30]);
        assert_eq!(subtract(&[0..10, 30..40], &[2..4, 6..20]), vec![0..2, 4..6, 30..40]);
        assert_eq!(subtract(&[0..4, 8..12], &[0..12]), Vec::<std::ops::Range<u64>>::new());
    }
}
