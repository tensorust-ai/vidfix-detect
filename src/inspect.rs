#![allow(dead_code)]
//! Top-level layout diagnosis from the head and tail of a file.

use crate::boxes::{self, parse_header, BoxHdr, SizeKind};
use crate::json::{esc, Obj};

#[derive(Clone, Debug, Default)]
pub struct MdatInfo {
    pub offset: u64,
    pub header: u32,
    pub payload_start: u64,
    pub declared_size: u64,
    pub size_field: &'static str,
    /// Where the payload really ends (EOF, or the start of boxes found after it).
    pub actual_end: u64,
    pub exceeds_eof_by: u64,
}

#[derive(Clone, Debug, Default)]
pub struct Inspect {
    pub file_len: u64,
    pub format: &'static str,
    pub boxes: Vec<BoxHdr>,
    pub ftyp: Option<Vec<u8>>,
    pub major_brand: String,
    pub compatible: Vec<String>,
    pub moov: Option<(u64, u64, bool)>,
    pub mdat: Option<MdatInfo>,
    pub fragmented: bool,
    pub device: String,
    pub needs_reference: bool,
    pub notes: Vec<String>,
}

fn read_at<'a>(head: &'a [u8], tail: &'a [u8], file_len: u64, off: u64, n: usize) -> Option<&'a [u8]> {
    if off + n as u64 <= head.len() as u64 {
        return Some(&head[off as usize..off as usize + n]);
    }
    let tail_start = file_len - tail.len() as u64;
    if off >= tail_start && off + n as u64 <= file_len {
        let s = (off - tail_start) as usize;
        return Some(&tail[s..s + n]);
    }
    // header may straddle: take what is available (min 8 bytes)
    if n > 8 {
        return read_at(head, tail, file_len, off, 8);
    }
    None
}

fn contains(h: &[u8], pat: &[u8]) -> bool {
    h.windows(pat.len()).any(|w| w == pat)
}

fn device_hint(major: &str, compat: &[String], head: &[u8], tail: &[u8]) -> String {
    let hay = |p: &[u8]| contains(head, p) || contains(tail, p);
    let all: Vec<&str> = std::iter::once(major).chain(compat.iter().map(|s| s.as_str())).collect();
    if hay(b"GoPro") || hay(b"GPRO") {
        return "GoPro".into();
    }
    if hay(b"DJI") {
        return "DJI".into();
    }
    if all.contains(&"XAVC") || hay(b"SONY") || hay(b"Sony") {
        return "Sony".into();
    }
    if all.contains(&"CAEP") || hay(b"Canon") {
        return "Canon".into();
    }
    if all.contains(&"niko") || hay(b"NIKON") || hay(b"Nikon") {
        return "Nikon".into();
    }
    if hay(b"Insta360") {
        return "Insta360".into();
    }
    if hay(b"Panasonic") {
        return "Panasonic".into();
    }
    if hay(b"Apple") || hay(b"iPhone") || hay(b"com.apple") {
        return "Apple".into();
    }
    if hay(b"Android") || hay(b"android") || hay(b"MOTO") || hay(b"SAMSUNG") {
        return "Android phone".into();
    }
    if hay(b"Ambarella") || hay(b"AMBA") {
        return "Ambarella (action cam / dashcam)".into();
    }
    if major == "qt  " {
        return "QuickTime (unknown maker)".into();
    }
    "unknown".into()
}

/// Find top-level boxes of known types at the end of the file that chain exactly to EOF.
fn trailing_boxes(tail: &[u8], file_len: u64, after: u64) -> Option<Vec<BoxHdr>> {
    let tail_start = file_len - tail.len() as u64;
    const TRAILING: [&[u8; 4]; 7] = [b"moov", b"free", b"skip", b"meta", b"uuid", b"udta", b"wide"];
    let mut i = 4usize;
    while i + 4 <= tail.len() {
        if TRAILING.iter().any(|t| &tail[i..i + 4] == *t) {
            let start = i - 4;
            let mut chain = vec![];
            let mut off = start;
            let mut ok = true;
            while off < tail.len() {
                match parse_header(&tail[off..], tail_start + off as u64, file_len) {
                    Some(h) if !h.truncated && h.kind != SizeKind::ToEof && TRAILING.iter().any(|t| &h.typ == *t) => {
                        off += h.size as usize;
                        chain.push(h);
                    }
                    _ => {
                        ok = false;
                        break;
                    }
                }
            }
            if ok && off == tail.len() && tail_start + start as u64 >= after {
                return Some(chain);
            }
        }
        i += 1;
    }
    None
}

pub fn inspect(head: &[u8], tail: &[u8], file_len: u64) -> Inspect {
    let tail = if tail.len() as u64 > file_len { &tail[tail.len() - file_len as usize..] } else { tail };
    let mut r = Inspect { file_len, format: "iso-bmff", ..Default::default() };
    if file_len == 0 {
        r.format = "empty";
        r.notes.push("the file is 0 bytes: the data never reached the file system entry".into());
        r.needs_reference = true;
        return r;
    }
    let mut off = 0u64;
    while off + 8 <= file_len {
        let d = match read_at(head, tail, file_len, off, 16) {
            Some(d) => d,
            None => {
                r.notes.push(format!("box headers at offset {} are outside the provided head/tail", off));
                break;
            }
        };
        let h = match parse_header(d, off, file_len) {
            Some(h) => h,
            None => {
                if off == 0 {
                    if head.len() > 188 * 2 && head[0] == 0x47 && head[188] == 0x47 && head[376] == 0x47 {
                        r.format = "mpeg-ts";
                        r.notes.push("MPEG transport stream: no moov needed, remux instead".into());
                    } else {
                        r.format = "unknown";
                        r.notes.push("the file does not start with an MP4/MOV box".into());
                    }
                } else {
                    r.notes.push(format!("unrecognized bytes after the last box at offset {}", off));
                }
                break;
            }
        };
        let is_mdat = &h.typ == b"mdat";
        let open_ended = matches!(h.kind, SizeKind::ToEof | SizeKind::Placeholder) || h.truncated;
        r.boxes.push(h.clone());
        if &h.typ == b"moof" {
            r.fragmented = true;
        }
        if is_mdat && open_ended {
            if let Some(chain) = trailing_boxes(tail, file_len, h.payload_start()) {
                let end = chain[0].offset;
                let last = r.boxes.last_mut().unwrap();
                last.size = end - last.offset;
                r.boxes.extend(chain);
                r.notes.push("mdat size field is unusable, but boxes after the media data were found".into());
                break;
            }
        }
        if h.size == 0 {
            break;
        }
        off = h.end();
    }
    for b in &r.boxes {
        match &b.typ {
            b"ftyp" => {
                if let Some(d) = read_at(head, tail, file_len, b.offset, b.size.min(256) as usize) {
                    if d.len() >= 16 {
                        r.major_brand = boxes::fourcc_str(&d[8..12]);
                        r.compatible = d[16..].chunks(4).filter(|c| c.len() == 4).map(boxes::fourcc_str).collect();
                        r.ftyp = Some(d.to_vec());
                    }
                }
            }
            b"moov" => {
                if r.moov.is_none() {
                    r.moov = Some((b.offset, b.size, !b.truncated));
                }
            }
            b"mdat" => {
                let pick = match &r.mdat {
                    None => true,
                    Some(m) => b.size > m.actual_end - m.offset,
                };
                if pick {
                    let declared_end = match b.kind {
                        SizeKind::Normal | SizeKind::Large => b.offset + b.declared,
                        _ => file_len,
                    };
                    r.mdat = Some(MdatInfo {
                        offset: b.offset,
                        header: b.header,
                        payload_start: b.payload_start(),
                        declared_size: b.declared,
                        size_field: match b.kind {
                            SizeKind::Normal => "32-bit",
                            SizeKind::ToEof => "zero (to end of file)",
                            SizeKind::Large => "64-bit",
                            SizeKind::Placeholder => "64-bit placeholder (never patched)",
                        },
                        actual_end: b.end(),
                        exceeds_eof_by: declared_end.saturating_sub(file_len),
                    });
                }
            }
            _ => {}
        }
    }
    if r.mdat.is_none() && r.format == "iso-bmff" && !r.boxes.is_empty() {
        // headers stop before any mdat: treat everything after the last box as media data
        let start = r.boxes.last().map(|b| b.end()).unwrap_or(0);
        if start < file_len {
            r.notes.push("no mdat header found; scanning the bytes after the last box as media data".into());
            r.mdat = Some(MdatInfo {
                offset: start,
                header: 0,
                payload_start: start,
                declared_size: 0,
                size_field: "missing",
                actual_end: file_len,
                exceeds_eof_by: 0,
            });
        }
    }
    // Only look for maker strings in box metadata (before the media data, plus GoPro's GPRO block just
    // after the mdat header, and the moov when it sits in the tail), never in compressed frames.
    let meta_end = r.mdat.as_ref().map(|m| m.payload_start + (64 << 10)).unwrap_or(1 << 20);
    let ht = &head[..head.len().min(meta_end as usize)];
    let tail_start = file_len - tail.len() as u64;
    let tt: &[u8] = match r.moov {
        Some((off, size, _)) if off >= tail_start => {
            let s = (off - tail_start) as usize;
            &tail[s..tail.len().min(s + size as usize)]
        }
        _ => &[],
    };
    r.device = device_hint(&r.major_brand, &r.compatible, ht, tt);
    let moov_ok = matches!(r.moov, Some((_, _, true)));
    r.needs_reference = !moov_ok;
    if let Some(m) = &r.mdat {
        if m.exceeds_eof_by > 0 {
            r.notes.push(format!("the mdat header claims {} more bytes than the file holds (recording was cut off)", m.exceeds_eof_by));
        }
    }
    if r.fragmented {
        r.notes.push("fragmented MP4 (moof boxes): needs a fragment scan, not a moov rebuild".into());
    }
    r
}

impl Inspect {
    pub fn json(&self) -> String {
        let mut j = Obj::new();
        j.num("file_len", self.file_len as f64);
        j.str("format", self.format);
        let bx: Vec<String> = self
            .boxes
            .iter()
            .map(|b| {
                let mut o = Obj::new();
                o.str("type", &b.type_str());
                o.num("offset", b.offset as f64);
                o.num("size", b.size as f64);
                o.num("header", b.header as f64);
                o.num("declared_size", b.declared as f64);
                o.bool("truncated", b.truncated);
                o.finish()
            })
            .collect();
        j.raw("boxes", &format!("[{}]", bx.join(",")));
        j.str("major_brand", &self.major_brand);
        j.raw("compatible_brands", &format!("[{}]", self.compatible.iter().map(|s| esc(s)).collect::<Vec<_>>().join(",")));
        match self.moov {
            Some((o, s, c)) => {
                let mut m = Obj::new();
                m.bool("present", true);
                m.num("offset", o as f64);
                m.num("size", s as f64);
                m.bool("complete", c);
                j.raw("moov", &m.finish());
            }
            None => j.raw("moov", "{\"present\":false}"),
        }
        match &self.mdat {
            Some(m) => {
                let mut o = Obj::new();
                o.num("offset", m.offset as f64);
                o.num("header", m.header as f64);
                o.num("payload_start", m.payload_start as f64);
                o.num("declared_size", m.declared_size as f64);
                o.str("size_field", m.size_field);
                o.num("actual_end", m.actual_end as f64);
                o.num("exceeds_eof_by", m.exceeds_eof_by as f64);
                j.raw("mdat", &o.finish());
            }
            None => j.null("mdat"),
        }
        j.bool("fragmented", self.fragmented);
        j.str("device_hint", &self.device);
        j.bool("needs_reference", self.needs_reference);
        j.str(
            "reference_free",
            if self.needs_reference { "possible only if the stream repeats its parameter sets (scan to find out)" } else { "not needed: the file keeps its own moov" },
        );
        j.strs("notes", &self.notes);
        j.finish()
    }
}
