//! Wire frames on a data channel (docs/SHARING.md §2.2).
//!
//! Browsers cap a data channel message (Safari ~64 KiB, Chromium 256 KiB) and big messages
//! stall the SCTP stream for everyone else, while collab frames reach 16 MiB (snapshots)
//! and media chunks are 1 MiB. So every [`WireFrame`] is cut into fragments of at most
//! [`DC_FRAGMENT_BYTES`]: each data channel message is `[flags u8][bytes]`, always sent as
//! a binary message, with `flags` bit 0 = last fragment of the frame and bit 1 = the frame
//! is binary (else UTF-8 text). The channel is ordered and reliable, so fragments of one
//! frame are contiguous. A reassembled frame larger than
//! [`crate::wire::MAX_COLLAB_MESSAGE_BYTES`] is an error (the link is closed).

use crate::wire::{MAX_COLLAB_MESSAGE_BYTES, WireFrame};

/// Largest data channel message (header included): interoperable everywhere.
pub const DC_FRAGMENT_BYTES: usize = 16 * 1024;
/// The data channel's label (negotiated by the joiner, who creates it).
pub const DC_LABEL: &str = "ethereal-collab/1";

const LAST: u8 = 1;
const BINARY: u8 = 2;

/// Cut `frame` into data channel messages.
pub fn fragment(frame: &WireFrame) -> Vec<Vec<u8>> {
    let (bytes, kind) = match frame {
        WireFrame::Text(t) => (t.as_bytes(), 0),
        WireFrame::Binary(b) => (b.as_slice(), BINARY),
    };
    let body = DC_FRAGMENT_BYTES - 1;
    let mut out = Vec::with_capacity(bytes.len() / body + 1);
    let mut chunks = bytes.chunks(body).peekable();
    if chunks.peek().is_none() {
        return vec![vec![kind | LAST]];
    }
    while let Some(chunk) = chunks.next() {
        let flags = kind | if chunks.peek().is_none() { LAST } else { 0 };
        let mut m = Vec::with_capacity(chunk.len() + 1);
        m.push(flags);
        m.extend_from_slice(chunk);
        out.push(m);
    }
    out
}

/// Reassembles fragments into frames (one per direction of a channel).
#[derive(Default)]
pub struct Reassembler {
    buf: Vec<u8>,
    kind: Option<u8>,
}

impl Reassembler {
    /// Feed one data channel message; a frame when it was the last fragment.
    pub fn push(&mut self, message: &[u8]) -> Result<Option<WireFrame>, String> {
        let (&flags, bytes) = message
            .split_first()
            .ok_or_else(|| "empty data channel message".to_string())?;
        if flags & !(LAST | BINARY) != 0 {
            return Err(format!("unknown fragment flags {flags:#x}"));
        }
        let kind = flags & BINARY;
        if self.kind.is_some_and(|k| k != kind) {
            return Err("fragment kind changed inside a frame".into());
        }
        if self.buf.len() + bytes.len() > MAX_COLLAB_MESSAGE_BYTES {
            return Err("frame too large".into());
        }
        self.kind = Some(kind);
        self.buf.extend_from_slice(bytes);
        if flags & LAST == 0 {
            return Ok(None);
        }
        self.kind = None;
        let bytes = std::mem::take(&mut self.buf);
        Ok(Some(if kind == BINARY {
            WireFrame::Binary(bytes)
        } else {
            WireFrame::Text(String::from_utf8(bytes).map_err(|_| "text frame is not UTF-8")?)
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(frame: WireFrame) {
        let parts = fragment(&frame);
        assert!(parts.iter().all(|p| p.len() <= DC_FRAGMENT_BYTES));
        let mut r = Reassembler::default();
        let (last, init) = parts.split_last().unwrap();
        for p in init {
            assert_eq!(r.push(p).unwrap(), None);
        }
        assert_eq!(r.push(last).unwrap(), Some(frame));
    }

    #[test]
    fn frames_survive_fragmentation() {
        roundtrip(WireFrame::Text(String::new()));
        roundtrip(WireFrame::Text(
            "{\"type\":\"Leave\",\"site\":\"1\"}".into(),
        ));
        roundtrip(WireFrame::Text("é".repeat(DC_FRAGMENT_BYTES)));
        roundtrip(WireFrame::Binary(vec![]));
        roundtrip(WireFrame::Binary(vec![7; DC_FRAGMENT_BYTES - 1]));
        roundtrip(WireFrame::Binary(
            (0..3 * DC_FRAGMENT_BYTES + 5).map(|i| i as u8).collect(),
        ));
    }

    #[test]
    fn bad_fragments_are_errors() {
        let mut r = Reassembler::default();
        assert!(r.push(&[]).is_err());
        assert!(r.push(&[0x80]).is_err());
        let mut r = Reassembler::default();
        assert_eq!(r.push(&[0, b'a']).unwrap(), None);
        assert!(
            r.push(&[BINARY | LAST, 1]).is_err(),
            "kind changed mid-frame"
        );
        let mut r = Reassembler::default();
        assert!(r.push(&[LAST, 0xff]).is_err(), "text must be UTF-8");
    }
}
