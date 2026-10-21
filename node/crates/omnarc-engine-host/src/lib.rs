//! The engine host: runs libomnarc in its own process, `omnarc-engine`, and talks to it over the process's stdin and
//! stdout with the bulk data in shared memory (PLAN.md §4). If the engine crashes, the agent keeps running and sees
//! the failure as an I/O error on the next request. The protocol is specified in `engine/HOST.md`.
//!
//! Layers are numbered as in the engine: the repeating layers are `0..layer_count` and the output head is layer
//! `layer_count`, so a whole model is the window `0..layer_count + 1`.

mod shm;

use std::fmt;
use std::io::{self, BufReader, Read, Write};
use std::ops::Range;
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::time::{Duration, Instant};

pub use omnarc_protocol::frame::DType;
pub use omnarc_protocol::v1::Backend;

use shm::SharedMemory;

/// Status codes of the engine (`om_status` in `engine/include/omnarc/omnarc.h`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    InvalidArgument,
    NotImplemented,
    OutOfMemory,
    Backend,
    Io,
    /// The KV cache or the request slots of a shard are full.
    Capacity,
    /// The model or the requested backend cannot be used this way.
    Unsupported,
    Unknown(u16),
}

impl Status {
    fn from_code(code: u16) -> Self {
        match code {
            1 => Self::InvalidArgument,
            2 => Self::NotImplemented,
            3 => Self::OutOfMemory,
            4 => Self::Backend,
            5 => Self::Io,
            6 => Self::Capacity,
            7 => Self::Unsupported,
            other => Self::Unknown(other),
        }
    }
}

/// An error from the engine or from talking to it.
#[derive(Debug)]
pub enum Error {
    /// The engine process could not be started, or it died: the pipe or the shared memory failed.
    Io(io::Error),
    /// The engine refused or failed a request.
    Engine { status: Status, message: String },
    /// The engine sent something this host does not understand.
    Protocol(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "engine process: {e}"),
            Self::Engine { status, message } => write!(f, "engine: {status:?}: {message}"),
            Self::Protocol(m) => write!(f, "engine protocol: {m}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// Model metadata read from the GGUF header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelInfo {
    pub architecture: String,
    /// Repeating layers; the output head is layer `layer_count`.
    pub layer_count: u32,
    /// Width of the hidden state passed between shards.
    pub hidden_dim: u32,
    pub vocab_size: u32,
    pub context_length: u32,
    pub file_size: u64,
    /// Whether the architecture can be split into windows.
    pub splittable: bool,
}

/// A GPU the engine can run layers on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    pub backend: Backend,
    /// As its driver names it, e.g. `NVIDIA GeForce GTX 1050`.
    pub name: String,
    /// Its memory as its backend reports it; 0 if unknown.
    pub memory_bytes: u64,
    /// Its memory free when it was listed; 0 if unknown or the engine is older than ABI 7.
    pub free_bytes: u64,
    /// It shares the processor's memory (an integrated GPU); false from an engine older than ABI 7.
    pub integrated: bool,
}

/// The one GPU a shard runs on, and how much of the shard (ABI 7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Gpu {
    /// Its index among [`Engine::devices`].
    pub index: u32,
    /// The window's repeating layers on it, the last ones of the window; the output head goes along when the window
    /// holds it. 0: all of them.
    pub layers: u32,
}

/// What [`Engine::bench`] measured on a GPU.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bench {
    /// Bytes of weights it reads per second, as one token's pass does.
    pub bandwidth_bytes_per_s: f64,
    /// Its matrix throughput, as a prefill uses it.
    pub flops: f64,
}

/// An open shard: a window of layers loaded in the engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShardInfo {
    pub id: ShardId,
    pub model: ModelInfo,
    pub layers: Range<u32>,
    pub backend: Backend,
}

impl ShardInfo {
    /// The shard starts at layer 0 and takes token ids.
    #[must_use]
    pub fn takes_tokens(&self) -> bool {
        self.layers.start == 0
    }

    /// The shard has the output head and gives logits.
    #[must_use]
    pub fn gives_logits(&self) -> bool {
        self.layers.end > self.model.layer_count
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ShardId(u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SamplerId(u32);

/// What a shard runs on.
#[derive(Debug, Clone, Copy)]
pub enum Input<'a> {
    /// Token ids, for a shard that starts at layer 0.
    Tokens(&'a [i32]),
    /// `count` tokens of hidden states as little-endian `F32`, `F16` or `Bf16` values: a tensor frame's payload as
    /// it arrives.
    Hidden { dtype: DType, count: u32, data: &'a [u8] },
}

/// One step of one request through a shard.
#[derive(Debug, Clone, Copy)]
pub struct Step<'a> {
    /// A request keeps its KV cache in the shard until [`Engine::drop_request`].
    pub request: u64,
    /// Position of the first token.
    pub position: u32,
    pub input: Input<'a>,
    /// For a shard with the output head: logits of every token instead of the last one only.
    pub all_logits: bool,
    /// For a shard without the output head: the type of the hidden states it gives.
    pub output: DType,
}

/// The result of a run, in the engine's output area.
#[derive(Debug, Clone, Copy)]
pub struct Output<'a> {
    pub rows: u32,
    /// Values per row: the hidden size, or the vocabulary size for logits.
    pub row_len: u32,
    /// `F32` for logits; the requested type for hidden states.
    pub dtype: DType,
    /// Little-endian values.
    pub data: &'a [u8],
}

impl Output<'_> {
    /// The values as `f32`; only for `F32` output (logits, or hidden states requested as `F32`).
    #[must_use]
    pub fn to_f32(&self) -> Vec<f32> {
        debug_assert_eq!(self.dtype, DType::F32);
        self.data.as_chunks::<4>().0.iter().map(|b| f32::from_le_bytes(*b)).collect()
    }
}

/// The element type of a shard's KV cache (EN-16).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u32)]
pub enum KvType {
    #[default]
    F16 = 0,
    /// 8 bits and a scale per 32 values: about half of f16.
    Q8_0 = 1,
    /// 4 bits and a scale per 32 values: about a quarter.
    Q4_0 = 2,
}

impl KvType {
    /// Bytes one cached value takes.
    #[must_use]
    pub fn bytes_per_value(self) -> f64 {
        match self {
            Self::F16 => 2.0,
            Self::Q8_0 => 34.0 / 32.0,
            Self::Q4_0 => 18.0 / 32.0,
        }
    }

    /// The name used in plans and requests: empty or `f16`, `q8_0`, `q4_0`.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "" | "f16" => Some(Self::F16),
            "q8_0" | "q8" => Some(Self::Q8_0),
            "q4_0" | "q4" => Some(Self::Q4_0),
            _ => None,
        }
    }

    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::F16 => "f16",
            Self::Q8_0 => "q8_0",
            Self::Q4_0 => "q4_0",
        }
    }
}

/// Sampling settings; a temperature of 0 is greedy.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SamplerParams {
    pub temperature: f32,
    pub top_k: u32,
    pub top_p: f32,
    pub min_p: f32,
    pub seed: u64,
}

impl Default for SamplerParams {
    fn default() -> Self {
        Self { temperature: 0.0, top_k: 0, top_p: 1.0, min_p: 0.0, seed: 0 }
    }
}

mod op {
    pub const HELLO: u16 = 1;
    pub const MODEL_INFO: u16 = 2;
    pub const OPEN: u16 = 3;
    pub const CLOSE: u16 = 4;
    pub const RUN: u16 = 5;
    pub const DROP: u16 = 6;
    pub const TRUNCATE: u16 = 7;
    pub const PREFETCH: u16 = 8;
    pub const THREADS: u16 = 9;
    pub const TOKENIZE: u16 = 10;
    pub const TOKEN_TEXT: u16 = 11;
    pub const SAMPLER_OPEN: u16 = 12;
    pub const SAMPLE: u16 = 13;
    pub const SAMPLER_CLOSE: u16 = 14;
    pub const QUIT: u16 = 15;
    pub const DEVICES: u16 = 16;
    pub const KV_SIZE: u16 = 17;
    pub const KV_READ: u16 = 18;
    pub const KV_WRITE: u16 = 19;
    pub const CHAT_PROMPT: u16 = 20;
    pub const BENCH: u16 = 21;
}

const EL_TOKENS: u8 = 0;

fn invalid(message: &str) -> Error {
    Error::Engine { status: Status::InvalidArgument, message: message.into() }
}

fn capacity(message: &str) -> Error {
    Error::Engine { status: Status::Capacity, message: message.into() }
}

fn element_code(dtype: DType) -> Result<u8> {
    match dtype {
        DType::F32 => Ok(1),
        DType::F16 => Ok(2),
        DType::Bf16 => Ok(3),
        other => Err(Error::Protocol(format!("the engine takes hidden states as f32, f16 or bf16, not {other:?}"))),
    }
}

/// A running `omnarc-engine` process.
#[derive(Debug)]
pub struct Engine {
    child: Child,
    /// Closing it tells the engine to exit.
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    shm: SharedMemory,
    abi_version: u32,
    version: String,
}

impl Engine {
    /// Starts the engine with a shared-memory region of `region_size` bytes; each half must hold the largest input
    /// and output of a run.
    ///
    /// # Errors
    /// The executable cannot be started, or it does not answer the handshake.
    pub fn spawn(executable: &Path, region_size: usize) -> Result<Self> {
        let shm = SharedMemory::create(region_size)?;
        let mut child = Command::new(executable)
            .arg("--shm")
            .arg(shm.name())
            .arg("--shm-size")
            .arg(shm.len().to_string())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()?;
        let stdin = child.stdin.take().ok_or_else(|| Error::Protocol("no stdin".into()))?;
        let stdout = BufReader::new(child.stdout.take().ok_or_else(|| Error::Protocol("no stdout".into()))?);
        let mut engine = Self { child, stdin: Some(stdin), stdout, shm, abi_version: 0, version: String::new() };

        let mut r = engine.request(op::HELLO, &[])?;
        engine.abi_version = r.u32()?;
        engine.version = r.string()?;
        let size = r.u64()?;
        // the engine has the region open now, so its name can go
        engine.shm.unlink();
        if size != engine.shm.len() as u64 {
            return Err(Error::Protocol(format!("the engine mapped {size} bytes of {}", engine.shm.len())));
        }
        if engine.abi_version < 2 {
            return Err(Error::Protocol(format!("engine ABI {} is older than 2", engine.abi_version)));
        }
        Ok(engine)
    }

    #[must_use]
    pub fn abi_version(&self) -> u32 {
        self.abi_version
    }

    /// The engine's version, e.g. "0.1.0".
    #[must_use]
    pub fn version(&self) -> &str {
        &self.version
    }

    /// Size of the input and of the output area in bytes.
    #[must_use]
    pub fn area_size(&self) -> usize {
        self.shm.half()
    }

    /// The GPUs this build of the engine can run layers on; none means it computes on the processor. An engine older
    /// than ABI 3 names none.
    ///
    /// # Errors
    /// The engine is gone, or it answers something this host does not understand.
    pub fn devices(&mut self) -> Result<Vec<Device>> {
        if self.abi_version < 3 {
            return Ok(Vec::new());
        }
        let mut r = self.request(op::DEVICES, &[])?;
        let count = r.u32()?;
        (0..count)
            .map(|_| {
                let backend = Backend::try_from(i32::try_from(r.u32()?).unwrap_or(0)).unwrap_or(Backend::Unspecified);
                let memory_bytes = r.u64()?;
                let name = r.string()?;
                let (integrated, free_bytes) =
                    if self.abi_version >= 7 { (r.u32()? != 0, r.u64()?) } else { (false, 0) };
                Ok(Device { backend, name, memory_bytes, free_bytes, integrated })
            })
            .collect()
    }

    /// Measures GPU `index` of [`devices`](Self::devices) for about `time` (NO-40).
    ///
    /// # Errors
    /// There is no such GPU, it fails the test, or the engine is older than ABI 7.
    pub fn bench(&mut self, index: u32, time: std::time::Duration) -> Result<Bench> {
        if self.abi_version < 7 {
            return Err(Error::Protocol(format!("engine ABI {} cannot measure a GPU", self.abi_version)));
        }
        let mut payload = index.to_le_bytes().to_vec();
        payload.extend_from_slice(&u32::try_from(time.as_millis()).unwrap_or(u32::MAX).to_le_bytes());
        let mut r = self.request(op::BENCH, &payload)?;
        Ok(Bench { bandwidth_bytes_per_s: f64::from_bits(r.u64()?), flops: f64::from_bits(r.u64()?) })
    }

    /// Reads a model's metadata without loading its weights.
    ///
    /// # Errors
    /// The file is missing or not a GGUF model, or the engine is gone.
    pub fn model_info(&mut self, path: &Path) -> Result<ModelInfo> {
        let mut payload = Vec::new();
        put_str(&mut payload, &path.to_string_lossy());
        let mut r = self.request(op::MODEL_INFO, &payload)?;
        r.model_info()
    }

    /// Loads the window `layers` of a model; `max_tokens` is the KV cache capacity shared by the shard's requests.
    ///
    /// # Errors
    /// The window is invalid, the architecture cannot be split, the backend is missing or memory runs out.
    pub fn open(&mut self, path: &Path, layers: Range<u32>, backend: Backend, max_tokens: u32) -> Result<ShardInfo> {
        self.open_with_kv(path, layers, backend, max_tokens, KvType::F16)
    }

    /// [`open`](Self::open) with the KV cache's element type (EN-16); an engine older than ABI 5 keeps f16 and
    /// refuses any other.
    ///
    /// # Errors
    /// As [`open`](Self::open), and a quantized cache on an engine that cannot hold one.
    pub fn open_with_kv(
        &mut self,
        path: &Path,
        layers: Range<u32>,
        backend: Backend,
        max_tokens: u32,
        kv: KvType,
    ) -> Result<ShardInfo> {
        self.open_on(path, layers, backend, max_tokens, kv, None)
    }

    /// [`open_with_kv`](Self::open_with_kv) on one GPU, with part of the window or all of it there (ABI 7). An older
    /// engine puts all of the window on every GPU, as before, and refuses only a part.
    ///
    /// # Errors
    /// As [`open`](Self::open), and a GPU that is not there.
    pub fn open_on(
        &mut self,
        path: &Path,
        layers: Range<u32>,
        backend: Backend,
        max_tokens: u32,
        kv: KvType,
        gpu: Option<Gpu>,
    ) -> Result<ShardInfo> {
        let mut payload = Vec::new();
        put_str(&mut payload, &path.to_string_lossy());
        payload.extend_from_slice(&layers.start.to_le_bytes());
        payload.extend_from_slice(&layers.end.to_le_bytes());
        payload.extend_from_slice(&u32::try_from(backend as i32).unwrap_or(0).to_le_bytes());
        payload.extend_from_slice(&max_tokens.to_le_bytes());
        if self.abi_version >= 5 {
            payload.extend_from_slice(&(kv as u32).to_le_bytes());
        } else if kv != KvType::F16 {
            return Err(Error::Protocol(format!("engine ABI {} holds only an f16 KV cache", self.abi_version)));
        }
        match gpu {
            Some(gpu) if self.abi_version >= 7 => {
                payload.extend_from_slice(&(gpu.index + 1).to_le_bytes());
                payload.extend_from_slice(&gpu.layers.to_le_bytes());
            }
            Some(gpu) if gpu.layers > 0 => {
                return Err(Error::Protocol(format!(
                    "engine ABI {} puts no part of a window on a GPU",
                    self.abi_version
                )));
            }
            _ => {}
        }
        let mut r = self.request(op::OPEN, &payload)?;
        let id = ShardId(r.u32()?);
        let model = r.model_info()?;
        let layers = r.u32()?..r.u32()?;
        let backend = Backend::try_from(i32::try_from(r.u32()?).unwrap_or(0)).unwrap_or(Backend::Unspecified);
        Ok(ShardInfo { id, model, layers, backend })
    }

    /// Releases a shard.
    ///
    /// # Errors
    /// The engine is gone.
    pub fn close(&mut self, shard: ShardId) -> Result<()> {
        self.request(op::CLOSE, &shard.0.to_le_bytes()).map(drop)
    }

    /// Runs one step of a request through a shard. A shard without the output head gives hidden states in
    /// `step.output` type; a shard with it gives `f32` logits of the last token, or of every token with
    /// `step.all_logits`.
    ///
    /// # Errors
    /// The input does not match the shard or does not fit in the input area, the KV cache is full
    /// ([`Status::Capacity`]), or the engine is gone.
    pub fn run(&mut self, shard: &ShardInfo, step: &Step<'_>) -> Result<Output<'_>> {
        let area = self.shm.input_mut();
        let (count, element) = match step.input {
            Input::Tokens(tokens) => {
                if !shard.takes_tokens() {
                    return Err(invalid("this shard takes hidden states, not tokens"));
                }
                if tokens.len() * 4 > area.len() {
                    return Err(capacity("the tokens do not fit in the input area"));
                }
                for (dst, t) in area.as_chunks_mut::<4>().0.iter_mut().zip(tokens) {
                    *dst = t.to_le_bytes();
                }
                (u32::try_from(tokens.len()).map_err(|_| capacity("too many tokens"))?, EL_TOKENS)
            }
            Input::Hidden { dtype, count, data } => {
                if shard.takes_tokens() {
                    return Err(invalid("this shard takes tokens, not hidden states"));
                }
                if data.len() != count as usize * shard.model.hidden_dim as usize * dtype.element_size() {
                    return Err(invalid("the hidden states do not match the token count and the hidden size"));
                }
                if data.len() > area.len() {
                    return Err(capacity("the hidden states do not fit in the input area"));
                }
                area[..data.len()].copy_from_slice(data);
                (count, element_code(dtype)?)
            }
        };
        if count == 0 {
            return Err(invalid("a step needs at least one token"));
        }
        let logits = shard.gives_logits();
        let dtype = if logits { DType::F32 } else { step.output };

        let mut payload = Vec::with_capacity(24);
        payload.extend_from_slice(&shard.id.0.to_le_bytes());
        payload.extend_from_slice(&step.request.to_le_bytes());
        payload.extend_from_slice(&step.position.to_le_bytes());
        payload.extend_from_slice(&count.to_le_bytes());
        payload.extend_from_slice(&[element, u8::from(step.all_logits), element_code(dtype)?, 0]);
        let mut r = self.request(op::RUN, &payload)?;
        let rows = r.u32()?;
        let row_len = r.u32()?;
        let len = rows as usize * row_len as usize * dtype.element_size();
        let data = self.shm.output().get(..len).ok_or_else(|| Error::Protocol("output larger than the area".into()))?;
        Ok(Output { rows, row_len, dtype, data })
    }

    /// Forgets a request in a shard and frees its KV cache.
    ///
    /// # Errors
    /// The engine is gone.
    pub fn drop_request(&mut self, shard: ShardId, request: u64) -> Result<()> {
        let mut payload = shard.0.to_le_bytes().to_vec();
        payload.extend_from_slice(&request.to_le_bytes());
        self.request(op::DROP, &payload).map(drop)
    }

    /// Removes a request's KV cache from `position` onwards.
    ///
    /// # Errors
    /// Unknown request, or the engine is gone.
    pub fn truncate(&mut self, shard: ShardId, request: u64, position: u32) -> Result<()> {
        let mut payload = shard.0.to_le_bytes().to_vec();
        payload.extend_from_slice(&request.to_le_bytes());
        payload.extend_from_slice(&position.to_le_bytes());
        self.request(op::TRUNCATE, &payload).map(drop)
    }

    /// The bytes this request's KV cache takes right now. It grows with the answer, so it is asked again
    /// before every read.
    ///
    /// # Errors
    /// Unknown request, or the engine is gone.
    pub fn kv_size(&mut self, shard: ShardId, request: u64) -> Result<u64> {
        let mut payload = shard.0.to_le_bytes().to_vec();
        payload.extend_from_slice(&request.to_le_bytes());
        self.request(op::KV_SIZE, &payload)?.u64()
    }

    /// Copies a request's KV cache out of the engine. A replica holding the same layers writes it back with
    /// [`Self::kv_write`] and carries the answer on from the same token, instead of reading the prompt again.
    ///
    /// # Errors
    /// Unknown request, a cache larger than the shared region, or the engine is gone.
    pub fn kv_read(&mut self, shard: ShardId, request: u64) -> Result<Vec<u8>> {
        let mut payload = shard.0.to_le_bytes().to_vec();
        payload.extend_from_slice(&request.to_le_bytes());
        let mut r = self.request(op::KV_READ, &payload)?;
        let written =
            usize::try_from(r.u64()?).map_err(|_| Error::Protocol("the KV cache size does not fit".into()))?;
        let data = self
            .shm
            .output()
            .get(..written)
            .ok_or_else(|| Error::Protocol("the KV cache does not fit the shared region".into()))?;
        Ok(data.to_vec())
    }

    /// Writes a KV cache taken by [`Self::kv_read`] back under `request`, replacing what that request held.
    /// The shard must hold the same layer window the cache came from.
    ///
    /// # Errors
    /// A cache larger than the shared region, a cache from different layers, or the engine is gone.
    pub fn kv_write(&mut self, shard: ShardId, request: u64, data: &[u8]) -> Result<()> {
        if data.is_empty() {
            return Err(invalid("the KV cache is empty"));
        }
        let room = self.shm.input_mut().len();
        if data.len() > room {
            return Err(capacity("the KV cache is larger than the shared region"));
        }
        self.shm.input_mut()[..data.len()].copy_from_slice(data);
        let mut payload = shard.0.to_le_bytes().to_vec();
        payload.extend_from_slice(&request.to_le_bytes());
        payload.extend_from_slice(&(data.len() as u64).to_le_bytes());
        self.request(op::KV_WRITE, &payload).map(drop)
    }

    /// Starts reading a shard's weights from disk in the background (prima.cpp's prefetching).
    ///
    /// # Errors
    /// The engine is gone.
    pub fn prefetch(&mut self, shard: ShardId) -> Result<()> {
        self.request(op::PREFETCH, &shard.0.to_le_bytes()).map(drop)
    }

    /// CPU threads for a shard; 0 = one per physical core.
    ///
    /// # Errors
    /// The engine is gone.
    pub fn set_threads(&mut self, shard: ShardId, threads: u32) -> Result<()> {
        let mut payload = shard.0.to_le_bytes().to_vec();
        payload.extend_from_slice(&threads.to_le_bytes());
        self.request(op::THREADS, &payload).map(drop)
    }

    /// Text to token ids with the model's tokenizer; `add_special` adds the BOS token, if the model has one.
    ///
    /// # Errors
    /// The text is too long for the output area, or the engine is gone.
    pub fn tokenize(&mut self, shard: ShardId, text: &str, add_special: bool) -> Result<Vec<i32>> {
        let mut payload = shard.0.to_le_bytes().to_vec();
        payload.push(u8::from(add_special));
        put_str(&mut payload, text);
        let mut r = self.request(op::TOKENIZE, &payload)?;
        let count = r.u32()? as usize;
        let data = self.shm.output().get(..count * 4).ok_or_else(|| Error::Protocol("too many tokens".into()))?;
        Ok(data.as_chunks::<4>().0.iter().map(|b| i32::from_le_bytes(*b)).collect())
    }

    /// The text of a token, and whether it ends generation.
    ///
    /// # Errors
    /// Unknown token, or the engine is gone.
    pub fn token_text(&mut self, shard: ShardId, token: i32) -> Result<(String, bool)> {
        let mut payload = shard.0.to_le_bytes().to_vec();
        payload.extend_from_slice(&token.to_le_bytes());
        let mut r = self.request(op::TOKEN_TEXT, &payload)?;
        let text = r.string()?;
        Ok((text, r.u8()? != 0))
    }

    /// A conversation as the model's own chat template writes it, ending where the answer begins; `None` when the
    /// model has no template the engine knows, or the engine is older than ABI 6.
    ///
    /// # Errors
    /// The engine is gone.
    pub fn chat_prompt(&mut self, shard: ShardId, messages: &[(&str, &str)]) -> Result<Option<String>> {
        if self.abi_version < 6 {
            return Ok(None);
        }
        let mut payload = shard.0.to_le_bytes().to_vec();
        payload.extend_from_slice(&u32::try_from(messages.len()).unwrap_or(u32::MAX).to_le_bytes());
        for (role, text) in messages {
            put_str(&mut payload, role);
            put_str(&mut payload, text);
        }
        match self.request(op::CHAT_PROMPT, &payload) {
            Ok(mut r) => Ok(Some(r.string()?)),
            Err(Error::Engine { status: Status::Unsupported, .. }) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Creates a sampler; one per request.
    ///
    /// # Errors
    /// The engine is gone.
    pub fn sampler_open(&mut self, params: SamplerParams) -> Result<SamplerId> {
        let mut payload = Vec::with_capacity(24);
        payload.extend_from_slice(&params.temperature.to_le_bytes());
        payload.extend_from_slice(&params.top_k.to_le_bytes());
        payload.extend_from_slice(&params.top_p.to_le_bytes());
        payload.extend_from_slice(&params.min_p.to_le_bytes());
        payload.extend_from_slice(&params.seed.to_le_bytes());
        let mut r = self.request(op::SAMPLER_OPEN, &payload)?;
        Ok(SamplerId(r.u32()?))
    }

    /// Samples the next token from the logits that the last [`Engine::run`] left in the output area.
    ///
    /// # Errors
    /// Unknown sampler, or the engine is gone.
    pub fn sample(&mut self, sampler: SamplerId, vocab_size: u32) -> Result<i32> {
        let mut payload = sampler.0.to_le_bytes().to_vec();
        payload.extend_from_slice(&vocab_size.to_le_bytes());
        let mut r = self.request(op::SAMPLE, &payload)?;
        r.i32()
    }

    /// Releases a sampler.
    ///
    /// # Errors
    /// The engine is gone.
    pub fn sampler_close(&mut self, sampler: SamplerId) -> Result<()> {
        self.request(op::SAMPLER_CLOSE, &sampler.0.to_le_bytes()).map(drop)
    }

    /// Stops the engine and waits for it to exit.
    ///
    /// # Errors
    /// The engine did not answer; it is killed.
    pub fn quit(mut self) -> Result<()> {
        let result = self.request(op::QUIT, &[]).map(drop);
        self.wait_or_kill(Duration::from_secs(5));
        result
    }

    /// Whether the engine process is still running.
    pub fn is_alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    /// Kills the engine process at once (tests use it to check that a crash is reported).
    ///
    /// # Errors
    /// The process could not be killed.
    pub fn kill(&mut self) -> io::Result<()> {
        self.child.kill()?;
        self.child.wait().map(drop)
    }

    fn wait_or_kill(&mut self, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if !matches!(self.child.try_wait(), Ok(None)) {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }

    fn request(&mut self, opcode: u16, payload: &[u8]) -> Result<Reader> {
        let length = u32::try_from(4 + payload.len()).map_err(|_| Error::Protocol("request too large".into()))?;
        let mut message = Vec::with_capacity(8 + payload.len());
        message.extend_from_slice(&length.to_le_bytes());
        message.extend_from_slice(&opcode.to_le_bytes());
        message.extend_from_slice(&0u16.to_le_bytes());
        message.extend_from_slice(payload);
        let stdin = self.stdin.as_mut().ok_or_else(|| Error::Protocol("the engine was stopped".into()))?;
        stdin.write_all(&message)?;
        stdin.flush()?;

        let mut head = [0u8; 8];
        self.stdout.read_exact(&mut head)?;
        let length = u32::from_le_bytes([head[0], head[1], head[2], head[3]]) as usize;
        let status = u16::from_le_bytes([head[4], head[5]]);
        if length < 4 {
            return Err(Error::Protocol(format!("response length {length}")));
        }
        let mut body = vec![0u8; length - 4];
        self.stdout.read_exact(&mut body)?;
        let mut reader = Reader { data: body, pos: 0 };
        if status != 0 {
            let message = reader.string().unwrap_or_default();
            return Err(Error::Engine { status: Status::from_code(status), message });
        }
        Ok(reader)
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        // closing stdin makes the engine exit
        drop(self.stdin.take());
        self.wait_or_kill(Duration::from_secs(2));
    }
}

fn put_str(out: &mut Vec<u8>, s: &str) {
    out.extend_from_slice(&u32::try_from(s.len()).unwrap_or(u32::MAX).to_le_bytes());
    out.extend_from_slice(s.as_bytes());
}

/// Little-endian reader over a response payload.
struct Reader {
    data: Vec<u8>,
    pos: usize,
}

impl Reader {
    fn take<const N: usize>(&mut self) -> Result<[u8; N]> {
        let bytes =
            self.data.get(self.pos..self.pos + N).ok_or_else(|| Error::Protocol("response too short".into()))?;
        self.pos += N;
        let mut out = [0u8; N];
        out.copy_from_slice(bytes);
        Ok(out)
    }

    fn u8(&mut self) -> Result<u8> {
        Ok(self.take::<1>()?[0])
    }

    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take()?))
    }

    fn i32(&mut self) -> Result<i32> {
        Ok(i32::from_le_bytes(self.take()?))
    }

    fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(self.take()?))
    }

    fn string(&mut self) -> Result<String> {
        let len = self.u32()? as usize;
        let bytes =
            self.data.get(self.pos..self.pos + len).ok_or_else(|| Error::Protocol("string past the end".into()))?;
        self.pos += len;
        String::from_utf8(bytes.to_vec()).map_err(|_| Error::Protocol("string is not UTF-8".into()))
    }

    fn model_info(&mut self) -> Result<ModelInfo> {
        Ok(ModelInfo {
            architecture: self.string()?,
            layer_count: self.u32()?,
            hidden_dim: self.u32()?,
            vocab_size: self.u32()?,
            context_length: self.u32()?,
            file_size: self.u64()?,
            splittable: self.u8()? != 0,
        })
    }
}
