//! Capability-gated binary frames for payload-bearing operations.
//!
//! Control stays JSON: a bulk frame only ever carries the bytes of one chunk and
//! the identifiers needed to place them. Nothing inside a bulk frame is JSON, so
//! a 256 KiB chunk costs 256 KiB plus a fixed header instead of a base64 string
//! (+33 percent) inside a serialized object.
//!
//! Layout, all integers big-endian:
//!
//! | offset | size | field |
//! | --- | --- | --- |
//! | 0 | 1 | magic `I` |
//! | 1 | 1 | version (1) |
//! | 2 | 1 | kind (0 upload_chunk, 1 download_chunk, 2 terminal_data, 3 reserved) |
//! | 3 | 1 | flags (0 today) |
//! | 4 | 16 | transfer or terminal id as raw UUID bytes |
//! | 20 | 8 | offset |
//! | 28 | 4 | payload length |
//! | 32 | n | payload |
//!
//! The header is 32 bytes: the four one-byte fields, the 16-byte UUID, the u64
//! offset and the u32 length. (The improvement plan calls it a "16-byte fixed
//! header"; the field list it gives sums to 32, and the field list is what both
//! sides implement.)
//!
//! Both peers only use these frames after the agent advertised
//! [`TRANSFER_BINARY_CAPABILITY`] / [`TERMINAL_BINARY_CAPABILITY`] and the
//! gateway acknowledged them in [`crate::HelloAck::binary_frames`]. A binary
//! frame on any other connection is a protocol error.

use thiserror::Error;
use uuid::Uuid;

/// Advertised by an agent that can carry transfer chunks in bulk frames.
pub const TRANSFER_BINARY_CAPABILITY: &str = "transfer.binary_v1";
/// Advertised by an agent that can carry terminal input in bulk frames.
pub const TERMINAL_BINARY_CAPABILITY: &str = "terminal.binary_v1";

pub const BULK_FRAME_MAGIC: u8 = b'I';
pub const BULK_FRAME_VERSION: u8 = 1;
/// magic + version + kind + flags + 16-byte id + u64 offset + u32 length.
pub const BULK_FRAME_HEADER_LEN: usize = 32;
/// Largest payload one frame may carry. Matches the agent's transfer chunk cap;
/// raising it is a protocol change, not a tuning knob.
pub const MAX_BULK_PAYLOAD: usize = 256 * 1024;

/// What a bulk frame carries. The numeric values are the wire values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum BulkKind {
    UploadChunk = 0,
    DownloadChunk = 1,
    TerminalData = 2,
}

impl BulkKind {
    pub fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::UploadChunk),
            1 => Some(Self::DownloadChunk),
            2 => Some(Self::TerminalData),
            _ => None,
        }
    }

    pub fn as_u8(self) -> u8 {
        self as u8
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum BulkFrameError {
    #[error("bulk frame is {len} bytes, shorter than the 32-byte header")]
    TooShort { len: usize },
    #[error("bulk frame magic is {0:#04x}, not {BULK_FRAME_MAGIC:#04x}")]
    BadMagic(u8),
    #[error("bulk frame version {0} is not supported")]
    UnsupportedVersion(u8),
    #[error("bulk frame kind {0} is not defined")]
    UnknownKind(u8),
    #[error("bulk frame declares {declared} payload bytes but carries {actual}")]
    LengthMismatch { declared: usize, actual: usize },
    #[error("bulk frame payload of {len} bytes exceeds the 262144-byte cap")]
    PayloadTooLarge { len: usize },
}

/// A borrowed bulk frame: the header fields plus a slice of the received buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BulkFrame<'a> {
    pub kind: BulkKind,
    pub flags: u8,
    pub id: Uuid,
    pub offset: u64,
    pub payload: &'a [u8],
}

impl<'a> BulkFrame<'a> {
    pub fn new(kind: BulkKind, id: Uuid, offset: u64, payload: &'a [u8]) -> Self {
        Self {
            kind,
            flags: 0,
            id,
            offset,
            payload,
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(BULK_FRAME_HEADER_LEN + self.payload.len());
        self.encode_into(&mut out);
        out
    }

    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.reserve(BULK_FRAME_HEADER_LEN + self.payload.len());
        out.push(BULK_FRAME_MAGIC);
        out.push(BULK_FRAME_VERSION);
        out.push(self.kind.as_u8());
        out.push(self.flags);
        out.extend_from_slice(self.id.as_bytes());
        out.extend_from_slice(&self.offset.to_be_bytes());
        out.extend_from_slice(&(self.payload.len() as u32).to_be_bytes());
        out.extend_from_slice(self.payload);
    }

    pub fn decode(bytes: &'a [u8]) -> Result<Self, BulkFrameError> {
        if bytes.len() < BULK_FRAME_HEADER_LEN {
            return Err(BulkFrameError::TooShort { len: bytes.len() });
        }
        if bytes[0] != BULK_FRAME_MAGIC {
            return Err(BulkFrameError::BadMagic(bytes[0]));
        }
        if bytes[1] != BULK_FRAME_VERSION {
            return Err(BulkFrameError::UnsupportedVersion(bytes[1]));
        }
        let kind = BulkKind::from_u8(bytes[2]).ok_or(BulkFrameError::UnknownKind(bytes[2]))?;
        let flags = bytes[3];
        let id = Uuid::from_bytes(
            <[u8; 16]>::try_from(&bytes[4..20]).expect("header length was checked"),
        );
        let offset = u64::from_be_bytes(
            <[u8; 8]>::try_from(&bytes[20..28]).expect("header length was checked"),
        );
        let declared = u32::from_be_bytes(
            <[u8; 4]>::try_from(&bytes[28..32]).expect("header length was checked"),
        ) as usize;
        let payload = &bytes[BULK_FRAME_HEADER_LEN..];
        if declared != payload.len() {
            return Err(BulkFrameError::LengthMismatch {
                declared,
                actual: payload.len(),
            });
        }
        if declared > MAX_BULK_PAYLOAD {
            return Err(BulkFrameError::PayloadTooLarge { len: declared });
        }
        Ok(Self {
            kind,
            flags,
            id,
            offset,
            payload,
        })
    }

    pub fn to_owned(&self) -> OwnedBulkFrame {
        OwnedBulkFrame {
            kind: self.kind,
            flags: self.flags,
            id: self.id,
            offset: self.offset,
            payload: self.payload.to_vec(),
        }
    }
}

/// A bulk frame that owns its payload, for handing across tasks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnedBulkFrame {
    pub kind: BulkKind,
    pub flags: u8,
    pub id: Uuid,
    pub offset: u64,
    pub payload: Vec<u8>,
}

impl OwnedBulkFrame {
    pub fn new(kind: BulkKind, id: Uuid, offset: u64, payload: Vec<u8>) -> Self {
        Self {
            kind,
            flags: 0,
            id,
            offset,
            payload,
        }
    }

    pub fn frame(&self) -> BulkFrame<'_> {
        BulkFrame {
            kind: self.kind,
            flags: self.flags,
            id: self.id,
            offset: self.offset,
            payload: &self.payload,
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        self.frame().encode()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(kind: BulkKind, offset: u64, len: usize) -> OwnedBulkFrame {
        OwnedBulkFrame::new(
            kind,
            Uuid::from_u128(0x0011_2233_4455_6677_8899_aabb_ccdd_eeff),
            offset,
            (0..len).map(|index| (index % 251) as u8).collect(),
        )
    }

    #[test]
    fn header_layout_is_the_documented_32_bytes() {
        let frame = sample(BulkKind::UploadChunk, 0x0102_0304_0506_0708, 4);
        let encoded = frame.encode();
        assert_eq!(BULK_FRAME_HEADER_LEN, 32);
        assert_eq!(encoded.len(), BULK_FRAME_HEADER_LEN + 4);
        assert_eq!(encoded[0], b'I');
        assert_eq!(encoded[1], 1);
        assert_eq!(encoded[2], 0);
        assert_eq!(encoded[3], 0);
        assert_eq!(&encoded[4..20], frame.id.as_bytes());
        assert_eq!(&encoded[20..28], &[1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(&encoded[28..32], &[0, 0, 0, 4]);
        assert_eq!(&encoded[32..], &frame.payload[..]);
    }

    #[test]
    fn roundtrip_preserves_kind_id_offset_and_payload() {
        for kind in [
            BulkKind::UploadChunk,
            BulkKind::DownloadChunk,
            BulkKind::TerminalData,
        ] {
            for offset in [0, 1, 262_144, 1 << 40, u64::MAX] {
                for len in [0, 1, 4096, MAX_BULK_PAYLOAD] {
                    let frame = sample(kind, offset, len);
                    let encoded = frame.encode();
                    let decoded = BulkFrame::decode(&encoded).expect("decode");
                    assert_eq!(decoded.kind, kind);
                    assert_eq!(decoded.id, frame.id);
                    assert_eq!(decoded.offset, offset);
                    assert_eq!(decoded.payload, frame.payload.as_slice());
                    assert_eq!(decoded.to_owned(), frame);
                }
            }
        }
    }

    #[test]
    fn offsets_are_big_endian_and_survive_the_full_u64_range() {
        let frame = sample(BulkKind::DownloadChunk, u64::MAX - 1, 8);
        let encoded = frame.encode();
        assert_eq!(&encoded[20..28], &(u64::MAX - 1).to_be_bytes());
        assert_eq!(BulkFrame::decode(&encoded).unwrap().offset, u64::MAX - 1);
    }

    #[test]
    fn truncated_and_malformed_headers_are_rejected() {
        let encoded = sample(BulkKind::UploadChunk, 7, 16).encode();
        assert_eq!(
            BulkFrame::decode(&encoded[..BULK_FRAME_HEADER_LEN - 1]),
            Err(BulkFrameError::TooShort { len: 31 })
        );
        assert_eq!(
            BulkFrame::decode(&[]),
            Err(BulkFrameError::TooShort { len: 0 })
        );

        let mut bad_magic = encoded.clone();
        bad_magic[0] = b'X';
        assert_eq!(
            BulkFrame::decode(&bad_magic),
            Err(BulkFrameError::BadMagic(b'X'))
        );

        let mut bad_version = encoded.clone();
        bad_version[1] = 2;
        assert_eq!(
            BulkFrame::decode(&bad_version),
            Err(BulkFrameError::UnsupportedVersion(2))
        );

        let mut reserved_kind = encoded.clone();
        reserved_kind[2] = 3;
        assert_eq!(
            BulkFrame::decode(&reserved_kind),
            Err(BulkFrameError::UnknownKind(3))
        );
    }

    #[test]
    fn declared_length_must_match_the_carried_bytes() {
        let encoded = sample(BulkKind::UploadChunk, 0, 16).encode();
        let mut short = encoded.clone();
        short.truncate(encoded.len() - 1);
        assert_eq!(
            BulkFrame::decode(&short),
            Err(BulkFrameError::LengthMismatch {
                declared: 16,
                actual: 15
            })
        );
        let mut long = encoded.clone();
        long.push(0);
        assert_eq!(
            BulkFrame::decode(&long),
            Err(BulkFrameError::LengthMismatch {
                declared: 16,
                actual: 17
            })
        );
    }

    #[test]
    fn payloads_above_the_chunk_cap_are_rejected() {
        let oversized = sample(BulkKind::DownloadChunk, 0, MAX_BULK_PAYLOAD + 1);
        assert_eq!(
            BulkFrame::decode(&oversized.encode()),
            Err(BulkFrameError::PayloadTooLarge {
                len: MAX_BULK_PAYLOAD + 1
            })
        );
    }

    #[test]
    fn kind_wire_values_are_stable() {
        assert_eq!(BulkKind::UploadChunk.as_u8(), 0);
        assert_eq!(BulkKind::DownloadChunk.as_u8(), 1);
        assert_eq!(BulkKind::TerminalData.as_u8(), 2);
        assert_eq!(BulkKind::from_u8(3), None);
        assert_eq!(BulkKind::from_u8(255), None);
    }
}
