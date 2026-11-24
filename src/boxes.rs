#![allow(dead_code)]
//! ISO BMFF / QuickTime box parsing that tolerates damage, plus a small box writer.
//!
//! Tolerated damage: size 0 (box runs to end of file), size 1 (64-bit largesize), a largesize
//! that is the Android MPEG4Writer placeholder ("????????") or larger than the file (clamped),
//! a declared size larger than the available bytes (clamped and flagged), garbage after the last box.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SizeKind {
    Normal,
    /// 32-bit size field was 0: the box extends to the end of the file.
    ToEof,
    /// 32-bit size 1 with a 64-bit largesize.
    Large,
    /// size 1 with a largesize of 0x3F3F3F3F3F3F3F3F ('????????') or 0: never patched by the writer.
    Placeholder,
}

#[derive(Clone, Debug)]
pub struct BoxHdr {
    pub typ: [u8; 4],
    pub offset: u64,
    pub header: u32,
    /// Size as declared in the header (for ToEof: 0).
    pub declared: u64,
    /// Size clamped to the limit.
    pub size: u64,
    pub kind: SizeKind,
    /// The declared size runs past the limit.
    pub truncated: bool,
}

impl BoxHdr {
    pub fn end(&self) -> u64 {
        self.offset + self.size
    }
    pub fn payload_start(&self) -> u64 {
        self.offset + self.header as u64
    }
    pub fn type_str(&self) -> String {
        fourcc_str(&self.typ)
    }
}

pub fn fourcc_str(t: &[u8]) -> String {
    t.iter()
        .map(|&b| if (0x20..0x7f).contains(&b) { b as char } else { '?' })
        .collect()
}

pub fn valid_type(t: &[u8]) -> bool {
    t.len() == 4 && t.iter().all(|&b| (0x20..=0x7e).contains(&b) || b == 0xA9)
}

/// Parse a box header from `d` (bytes starting at file offset `offset`); `limit` is the end of
/// the enclosing space (file length for top-level boxes).
pub fn parse_header(d: &[u8], offset: u64, limit: u64) -> Option<BoxHdr> {
    if d.len() < 8 || offset + 8 > limit {
        return None;
    }
    let size32 = u32::from_be_bytes([d[0], d[1], d[2], d[3]]) as u64;
    let mut typ = [0u8; 4];
    typ.copy_from_slice(&d[4..8]);
    if !valid_type(&typ) {
        return None;
    }
    let avail = limit - offset;
    let (header, declared, kind) = match size32 {
        0 => (8u32, 0u64, SizeKind::ToEof),
        1 => {
            if d.len() < 16 {
                return None;
            }
            let ls = u64::from_be_bytes([d[8], d[9], d[10], d[11], d[12], d[13], d[14], d[15]]);
            if ls == 0x3F3F3F3F3F3F3F3F || ls == 0 {
                (16, ls, SizeKind::Placeholder)
            } else if ls < 16 {
                return None;
            } else {
                (16, ls, SizeKind::Large)
            }
        }
        s if s < 8 => return None,
        s => (8, s, SizeKind::Normal),
    };
    let (size, truncated) = match kind {
        SizeKind::ToEof | SizeKind::Placeholder => (avail, false),
        _ => {
            if declared > avail {
                (avail, true)
            } else {
                (declared, false)
            }
        }
    };
    if size < header as u64 {
        return None;
    }
    Some(BoxHdr { typ, offset, header, declared, size, kind, truncated })
}

/// A child box inside an in-memory buffer. Offsets are relative to the buffer.
#[derive(Clone, Copy, Debug)]
pub struct Child {
    pub typ: [u8; 4],
    pub start: usize,
    pub payload: usize,
    pub end: usize,
    pub truncated: bool,
}

/// Iterate the boxes in `d` (tolerant: a box running past the end is clamped and flagged).
pub fn children(d: &[u8]) -> Vec<Child> {
    let mut out = vec![];
    let mut off = 0usize;
    while off + 8 <= d.len() {
        let h = match parse_header(&d[off..], 0, (d.len() - off) as u64) {
            Some(h) => h,
            None => break,
        };
        let end = off + h.size as usize;
        out.push(Child { typ: h.typ, start: off, payload: off + h.header as usize, end, truncated: h.truncated });
        if h.size == 0 {
            break;
        }
        off = end;
    }
    out
}

pub fn child<'a>(d: &'a [u8], typ: &[u8; 4]) -> Option<&'a [u8]> {
    children(d).into_iter().find(|c| &c.typ == typ).map(|c| &d[c.payload..c.end])
}

/// Full box bytes (header included) of the first child of the given type.
pub fn child_raw<'a>(d: &'a [u8], typ: &[u8; 4]) -> Option<&'a [u8]> {
    children(d).into_iter().find(|c| &c.typ == typ).map(|c| &d[c.start..c.end])
}

pub fn path<'a>(d: &'a [u8], p: &[&[u8; 4]]) -> Option<&'a [u8]> {
    let mut cur = d;
    for t in p {
        cur = child(cur, t)?;
    }
    Some(cur)
}

pub fn be16(d: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_be_bytes(d.get(o..o + 2)?.try_into().ok()?))
}
pub fn be32(d: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_be_bytes(d.get(o..o + 4)?.try_into().ok()?))
}
pub fn be64(d: &[u8], o: usize) -> Option<u64> {
    Some(u64::from_be_bytes(d.get(o..o + 8)?.try_into().ok()?))
}

/// Find the offset of a fourcc (preceded by a plausible 32-bit size) inside `d`.
pub fn find_fourcc(d: &[u8], typ: &[u8; 4], from: usize) -> Option<usize> {
    let mut i = from.max(4);
    while i + 4 <= d.len() {
        if &d[i..i + 4] == typ {
            return Some(i - 4);
        }
        i += 1;
    }
    None
}

/// Box writer.
#[derive(Default)]
pub struct W {
    pub b: Vec<u8>,
    stack: Vec<usize>,
}

impl W {
    pub fn new() -> Self {
        W::default()
    }
    pub fn start(&mut self, typ: &[u8; 4]) {
        self.stack.push(self.b.len());
        self.b.extend_from_slice(&[0, 0, 0, 0]);
        self.b.extend_from_slice(typ);
    }
    pub fn start_full(&mut self, typ: &[u8; 4], version: u8, flags: u32) {
        self.start(typ);
        self.u32(((version as u32) << 24) | (flags & 0xFFFFFF));
    }
    pub fn end(&mut self) {
        let s = self.stack.pop().expect("unbalanced box writer");
        let size = (self.b.len() - s) as u32;
        self.b[s..s + 4].copy_from_slice(&size.to_be_bytes());
    }
    pub fn u8(&mut self, v: u8) {
        self.b.push(v);
    }
    pub fn u16(&mut self, v: u16) {
        self.b.extend_from_slice(&v.to_be_bytes());
    }
    pub fn u32(&mut self, v: u32) {
        self.b.extend_from_slice(&v.to_be_bytes());
    }
    pub fn i32(&mut self, v: i32) {
        self.b.extend_from_slice(&v.to_be_bytes());
    }
    pub fn u64(&mut self, v: u64) {
        self.b.extend_from_slice(&v.to_be_bytes());
    }
    pub fn bytes(&mut self, v: &[u8]) {
        self.b.extend_from_slice(v);
    }
    pub fn zeros(&mut self, n: usize) {
        self.b.resize(self.b.len() + n, 0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn size_zero_runs_to_eof() {
        let d = [0, 0, 0, 0, b'm', b'd', b'a', b't', 1, 2, 3];
        let h = parse_header(&d, 100, 1000).unwrap();
        assert_eq!(h.kind, SizeKind::ToEof);
        assert_eq!(h.size, 900);
        assert_eq!(h.header, 8);
    }

    #[test]
    fn largesize_and_placeholder() {
        let mut d = vec![0, 0, 0, 1, b'm', b'd', b'a', b't'];
        d.extend_from_slice(&5000u64.to_be_bytes());
        let h = parse_header(&d, 0, 100_000).unwrap();
        assert_eq!((h.kind, h.size, h.header, h.truncated), (SizeKind::Large, 5000, 16, false));
        let h = parse_header(&d, 0, 1000).unwrap();
        assert_eq!((h.size, h.truncated), (1000, true));
        let mut p = vec![0, 0, 0, 1, b'm', b'd', b'a', b't'];
        p.extend_from_slice(b"????????");
        let h = parse_header(&p, 40, 1040).unwrap();
        assert_eq!((h.kind, h.size), (SizeKind::Placeholder, 1000));
    }

    #[test]
    fn declared_larger_than_file_is_clamped() {
        let d = [0, 0, 0x10, 0, b'm', b'd', b'a', b't'];
        let h = parse_header(&d, 0, 100).unwrap();
        assert!(h.truncated);
        assert_eq!(h.size, 100);
        assert_eq!(h.declared, 4096);
    }

    #[test]
    fn rejects_garbage_and_iterates_children() {
        assert!(parse_header(&[0xfe, 0xb3, 0xd0, 0xd6, 0x01, 0x02, 0x03, 0x04], 0, 1000).is_none());
        assert!(parse_header(&[0, 0, 0, 4, b'f', b'r', b'e', b'e'], 0, 1000).is_none());
        let mut w = W::new();
        w.start(b"moov");
        w.start_full(b"mvhd", 0, 0);
        w.u32(7);
        w.end();
        w.start(b"trak");
        w.u32(1);
        w.u32(2);
        w.end();
        w.end();
        w.bytes(&[0xde, 0xad]); // garbage after the box
        let top = children(&w.b);
        assert_eq!(top.len(), 1);
        assert_eq!(&top[0].typ, b"moov");
        let moov = child(&w.b, b"moov").unwrap();
        let kids: Vec<_> = children(moov).iter().map(|c| c.typ).collect();
        assert_eq!(kids, vec![*b"mvhd", *b"trak"]);
        assert_eq!(be32(child(moov, b"mvhd").unwrap(), 4), Some(7));
        // truncated child is clamped
        let cut = &moov[..moov.len() - 4];
        let c = children(cut);
        assert_eq!(c.len(), 2);
        assert!(c[1].truncated);
        assert_eq!(c[1].end, cut.len());
    }
}
