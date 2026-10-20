//! Tensor frames: a fixed 32-byte little-endian header followed by the payload.
//!
//! The layout is specified in `protocol/FRAME.md`. Decoding validates every field, so a
//! receiver can trust a header before it touches the payload.

use std::fmt;

/// Size of the frame header in bytes.
pub const HEADER_LEN: usize = 32;
/// Magic bytes at the start of every frame.
pub const MAGIC: [u8; 4] = *b"OMT1";
/// Frame format version implemented here.
pub const VERSION: u8 = 1;

/// Element type of the hidden state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum DType {
    F32 = 0,
    F16 = 1,
    Bf16 = 2,
    /// 8-bit float (E4M3) with one f32 scale per token.
    F8E4M3 = 3,
    /// 8-bit integer with one f32 scale per token.
    I8 = 4,
}

impl DType {
    /// Bytes per element of the hidden state.
    #[must_use]
    pub const fn element_size(self) -> usize {
        match self {
            Self::F32 => 4,
            Self::F16 | Self::Bf16 => 2,
            Self::F8E4M3 | Self::I8 => 1,
        }
    }

    /// Whether the payload starts with one f32 scale per token.
    #[must_use]
    pub const fn has_scales(self) -> bool {
        matches!(self, Self::F8E4M3 | Self::I8)
    }

    const fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::F32),
            1 => Some(Self::F16),
            2 => Some(Self::Bf16),
            3 => Some(Self::F8E4M3),
            4 => Some(Self::I8),
            _ => None,
        }
    }
}

/// Frame flags (bit field).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Flags(u16);

impl Flags {
    /// The tokens are speculative drafts that still need verification.
    pub const DRAFT: Self = Self(1 << 0);
    /// The frame is addressed to the stage that produces logits.
    pub const LAST_STAGE: Self = Self(1 << 1);
    /// Resent after a failover; the receiver may already hold this step.
    pub const REPLAY: Self = Self(1 << 2);
    /// A copy for the replica of these layers: it runs the frame to keep its KV cache in step with the
    /// device that is answering, and forwards nothing. The replica can then carry the answer on from the
    /// same token if that device leaves (EN-17).
    pub const MIRROR: Self = Self(1 << 3);
    /// The payload is `token_count` little-endian i32 token ids, not a hidden state. Only the window that
    /// holds layer 0 takes tokens, so only its replica is sent them; `hidden_dim` still carries the model's
    /// hidden size and takes no part in the payload size.
    pub const TOKENS: Self = Self(1 << 4);
    const KNOWN: u16 = 0b11111;

    /// No flags set.
    #[must_use]
    pub const fn empty() -> Self {
        Self(0)
    }

    /// Raw bits.
    #[must_use]
    pub const fn bits(self) -> u16 {
        self.0
    }

    /// Whether every flag in `other` is set.
    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// Both sets of flags combined.
    #[must_use]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

/// The 32-byte header of a tensor frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FrameHeader {
    pub dtype: DType,
    pub flags: Flags,
    pub request_id: u64,
    /// Decoding step within the request (0 = prefill).
    pub step: u32,
    /// Position of the first token in the frame.
    pub position_start: u32,
    pub token_count: u16,
    pub hidden_dim: u16,
    pub payload_len: u32,
}

/// Why a header was rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameError {
    /// Fewer than [`HEADER_LEN`] bytes were given.
    TooShort(usize),
    BadMagic([u8; 4]),
    UnsupportedVersion(u8),
    UnknownDType(u8),
    ReservedFlags(u16),
    EmptyTensor,
    /// `payload_len` disagrees with `token_count`, `hidden_dim` and `dtype`.
    PayloadMismatch {
        declared: u32,
        expected: u64,
    },
}

impl fmt::Display for FrameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooShort(n) => write!(f, "frame header needs {HEADER_LEN} bytes, got {n}"),
            Self::BadMagic(m) => write!(f, "bad magic {m:02x?}"),
            Self::UnsupportedVersion(v) => write!(f, "unsupported frame version {v}"),
            Self::UnknownDType(d) => write!(f, "unknown dtype {d}"),
            Self::ReservedFlags(bits) => write!(f, "reserved flag bits set: {bits:#06x}"),
            Self::EmptyTensor => f.write_str("token_count and hidden_dim must be at least 1"),
            Self::PayloadMismatch { declared, expected } => {
                write!(f, "payload_len is {declared}, expected {expected}")
            }
        }
    }
}

impl std::error::Error for FrameError {}

impl FrameHeader {
    /// Builds a header and computes `payload_len` from the tensor shape.
    ///
    /// # Errors
    /// Returns [`FrameError::EmptyTensor`] for an empty shape and
    /// [`FrameError::PayloadMismatch`] when the payload would not fit in 32 bits.
    pub fn new(
        dtype: DType,
        flags: Flags,
        request_id: u64,
        step: u32,
        position_start: u32,
        token_count: u16,
        hidden_dim: u16,
    ) -> Result<Self, FrameError> {
        if token_count == 0 || hidden_dim == 0 {
            return Err(FrameError::EmptyTensor);
        }
        let expected = expected_payload_len(dtype, flags, token_count, hidden_dim);
        let payload_len =
            u32::try_from(expected).map_err(|_| FrameError::PayloadMismatch { declared: u32::MAX, expected })?;
        Ok(Self { dtype, flags, request_id, step, position_start, token_count, hidden_dim, payload_len })
    }

    /// Serializes the header.
    #[must_use]
    pub fn encode(&self) -> [u8; HEADER_LEN] {
        let mut out = [0u8; HEADER_LEN];
        out[0..4].copy_from_slice(&MAGIC);
        out[4] = VERSION;
        out[5] = self.dtype as u8;
        out[6..8].copy_from_slice(&self.flags.bits().to_le_bytes());
        out[8..16].copy_from_slice(&self.request_id.to_le_bytes());
        out[16..20].copy_from_slice(&self.step.to_le_bytes());
        out[20..24].copy_from_slice(&self.position_start.to_le_bytes());
        out[24..26].copy_from_slice(&self.token_count.to_le_bytes());
        out[26..28].copy_from_slice(&self.hidden_dim.to_le_bytes());
        out[28..32].copy_from_slice(&self.payload_len.to_le_bytes());
        out
    }

    /// Parses and validates a header from the first [`HEADER_LEN`] bytes of `bytes`.
    ///
    /// # Errors
    /// Returns a [`FrameError`] describing the first field that is invalid.
    pub fn decode(bytes: &[u8]) -> Result<Self, FrameError> {
        let h: &[u8; HEADER_LEN] =
            bytes.get(..HEADER_LEN).and_then(|s| s.try_into().ok()).ok_or(FrameError::TooShort(bytes.len()))?;
        let magic = [h[0], h[1], h[2], h[3]];
        if magic != MAGIC {
            return Err(FrameError::BadMagic(magic));
        }
        if h[4] != VERSION {
            return Err(FrameError::UnsupportedVersion(h[4]));
        }
        let dtype = DType::from_u8(h[5]).ok_or(FrameError::UnknownDType(h[5]))?;
        let flags = u16::from_le_bytes([h[6], h[7]]);
        if flags & !Flags::KNOWN != 0 {
            return Err(FrameError::ReservedFlags(flags));
        }
        let token_count = u16::from_le_bytes([h[24], h[25]]);
        let hidden_dim = u16::from_le_bytes([h[26], h[27]]);
        if token_count == 0 || hidden_dim == 0 {
            return Err(FrameError::EmptyTensor);
        }
        let payload_len = u32::from_le_bytes([h[28], h[29], h[30], h[31]]);
        let expected = expected_payload_len(dtype, Flags(flags), token_count, hidden_dim);
        if u64::from(payload_len) != expected {
            return Err(FrameError::PayloadMismatch { declared: payload_len, expected });
        }
        Ok(Self {
            dtype,
            flags: Flags(flags),
            request_id: u64::from_le_bytes([h[8], h[9], h[10], h[11], h[12], h[13], h[14], h[15]]),
            step: u32::from_le_bytes([h[16], h[17], h[18], h[19]]),
            position_start: u32::from_le_bytes([h[20], h[21], h[22], h[23]]),
            token_count,
            hidden_dim,
            payload_len,
        })
    }
}

/// Payload size for a tensor of `token_count × hidden_dim` elements of `dtype`,
/// including the per-token scales of the 8-bit types.
#[must_use]
pub fn expected_payload_len(dtype: DType, flags: Flags, token_count: u16, hidden_dim: u16) -> u64 {
    let tokens = u64::from(token_count);
    if flags.contains(Flags::TOKENS) {
        return tokens * 4;
    }
    let elements = tokens * u64::from(hidden_dim) * dtype.element_size() as u64;
    if dtype.has_scales() { elements + tokens * 4 } else { elements }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> FrameHeader {
        FrameHeader::new(DType::F16, Flags::DRAFT.union(Flags::LAST_STAGE), 42, 7, 1024, 3, 8192).unwrap()
    }

    #[test]
    fn layout_matches_the_specification() {
        let bytes = sample().encode();
        assert_eq!(&bytes[0..4], b"OMT1");
        assert_eq!(bytes[4], 1);
        assert_eq!(bytes[5], 1, "f16");
        assert_eq!(u16::from_le_bytes([bytes[6], bytes[7]]), 0b11, "DRAFT | LAST_STAGE");
        assert_eq!(u64::from_le_bytes(bytes[8..16].try_into().unwrap()), 42);
        assert_eq!(u32::from_le_bytes(bytes[16..20].try_into().unwrap()), 7);
        assert_eq!(u32::from_le_bytes(bytes[20..24].try_into().unwrap()), 1024);
        assert_eq!(u16::from_le_bytes([bytes[24], bytes[25]]), 3);
        assert_eq!(u16::from_le_bytes([bytes[26], bytes[27]]), 8192);
        assert_eq!(u32::from_le_bytes(bytes[28..32].try_into().unwrap()), 3 * 8192 * 2);
    }

    #[test]
    fn eight_bit_payloads_carry_one_scale_per_token() {
        assert_eq!(expected_payload_len(DType::F8E4M3, Flags::empty(), 4, 4096), 4 * 4 + 4 * 4096);
        assert_eq!(expected_payload_len(DType::I8, Flags::empty(), 1, 16), 4 + 16);
        assert_eq!(expected_payload_len(DType::F32, Flags::empty(), 2, 10), 80);
        // a token payload is four bytes each, whatever the hidden size says
        assert_eq!(expected_payload_len(DType::F16, Flags::TOKENS, 7, 4096), 28);
    }

    #[test]
    fn decode_rejects_invalid_headers() {
        let good = sample().encode();
        assert_eq!(FrameHeader::decode(&good[..31]), Err(FrameError::TooShort(31)));

        let mut bad = good;
        bad[0] = b'X';
        assert!(matches!(FrameHeader::decode(&bad), Err(FrameError::BadMagic(_))));

        let mut bad = good;
        bad[4] = 2;
        assert_eq!(FrameHeader::decode(&bad), Err(FrameError::UnsupportedVersion(2)));

        let mut bad = good;
        bad[5] = 9;
        assert_eq!(FrameHeader::decode(&bad), Err(FrameError::UnknownDType(9)));

        let mut bad = good;
        bad[7] = 0x80;
        assert!(matches!(FrameHeader::decode(&bad), Err(FrameError::ReservedFlags(_))));

        let mut bad = good;
        bad[24..26].copy_from_slice(&0u16.to_le_bytes());
        assert_eq!(FrameHeader::decode(&bad), Err(FrameError::EmptyTensor));

        let mut bad = good;
        bad[28..32].copy_from_slice(&1u32.to_le_bytes());
        assert!(matches!(FrameHeader::decode(&bad), Err(FrameError::PayloadMismatch { declared: 1, .. })));
    }

    #[test]
    fn new_rejects_an_empty_tensor() {
        assert_eq!(FrameHeader::new(DType::F32, Flags::empty(), 0, 0, 0, 0, 16), Err(FrameError::EmptyTensor));
    }
}
