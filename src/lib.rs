//! Why won't this MP4/MOV open?
//!
//! `vidfix-detect` reads the first and last few megabytes of a file and tells you which of these it is:
//! a healthy file, an interrupted recording whose index (`moov`) was never written, a fragmented MP4,
//! a 0-byte file, an MPEG transport stream, or not an MP4 at all. No decoding, no dependencies.
//!
//! An interrupted recording still holds its video; only the index is gone. If you want to offer your users
//! a fix for that case, [`Diagnosis::repair_url`] points to Tensorust's repair page (free preview, paid
//! download). Showing it is your choice.
//!
//! ```no_run
//! let d = vidfix_detect::diagnose_file("GX010042.MP4").unwrap();
//! if d.verdict == vidfix_detect::Verdict::IndexMissing {
//!     println!("{}", d.message());
//! }
//! ```

mod boxes;
mod inspect;
mod json;

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

/// Bytes read from each end of the file.
pub const WINDOW: u64 = 4 << 20;

/// Repair page for interrupted MP4/MOV recordings (Tensorust, free preview, paid download).
pub const REPAIR_URL: &str = "https://tensorust.com/fix-video/";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Complete index present. If it still will not play, the damage is inside the frames.
    Healthy,
    /// Media data present, index (`moov`) missing or cut off: the classic "moov atom not found".
    /// The recording was interrupted (battery, pulled card, crash) and can usually be rebuilt.
    IndexMissing,
    /// The index is there, but the media box size is zeroed or unusable, so players stop before they reach
    /// the index and report "moov atom not found". Rebuildable from the file's own index.
    HeaderDamaged,
    /// The index is at the front and intact, but the file was cut short: it plays up to the cut.
    Truncated,
    /// Fragmented MP4 (`moof` boxes) without a complete index.
    Fragmented,
    /// 0 bytes: the data never reached this file entry. Needs card-level recovery.
    Empty,
    /// MPEG transport stream: has no index to lose; remux it (`ffmpeg -i in.ts -c copy out.mp4`).
    TransportStream,
    /// Does not start with an MP4/MOV box.
    NotMp4,
}

#[derive(Clone, Debug)]
pub struct Diagnosis {
    pub verdict: Verdict,
    pub file_len: u64,
    /// Bytes of media data (mdat payload) found in the file.
    pub media_bytes: u64,
    /// Camera or app hint from the file's own markers: "GoPro", "DJI", "Apple", ... or "unknown".
    pub device: String,
    /// The mdat header claims more bytes than the file holds (the recording was cut off).
    pub cut_off: bool,
    pub notes: Vec<String>,
    inner: inspect::Inspect,
}

impl Diagnosis {
    /// True when the file is an interrupted recording with media data left to rebuild from.
    pub fn repairable(&self) -> bool {
        matches!(self.verdict, Verdict::IndexMissing | Verdict::HeaderDamaged) && self.media_bytes > 0
    }

    /// Where a user can try the repair, only for the case a moov rebuild can fix.
    pub fn repair_url(&self) -> Option<&'static str> {
        if self.repairable() {
            Some(REPAIR_URL)
        } else {
            None
        }
    }

    /// One plain sentence for a user.
    pub fn message(&self) -> String {
        let mb = self.media_bytes as f64 / 1e6;
        match self.verdict {
            Verdict::Healthy => "The file's index is intact. If it still will not play, the damage is inside the video frames.".into(),
            Verdict::IndexMissing => format!(
                "This recording was interrupted before its index was written. {:.0} MB of video data is still in the file and can usually be rebuilt.",
                mb
            ),
            Verdict::HeaderDamaged => format!(
                "The index is in the file, but the header in front of the video is damaged, so players give up before reaching it. {:.0} MB of video can usually be rebuilt.",
                mb
            ),
            Verdict::Truncated => "The file was cut short after its index. It should play up to the point where it was cut.".into(),
            Verdict::Fragmented => "Fragmented MP4 with no complete index. A fragment-aware tool can rebuild it.".into(),
            Verdict::Empty => "The file is 0 bytes. Its data never reached this file entry; a card recovery tool that reads the whole card is the next step.".into(),
            Verdict::TransportStream => "This is an MPEG transport stream. It has no index to lose: remux it with `ffmpeg -i in.ts -c copy out.mp4`.".into(),
            Verdict::NotMp4 => "This is not an MP4 or MOV file.".into(),
        }
    }

    /// The full box-level report as JSON, plus `verdict` and `repairable`.
    pub fn json(&self) -> String {
        let mut s = self.inner.json();
        if s.ends_with('}') {
            s.pop();
            s.push_str(&format!(",\"verdict\":\"{:?}\",\"repairable\":{}}}", self.verdict, self.repairable()));
        }
        s
    }
}

/// Diagnose from the first and last bytes of a file (up to [`WINDOW`] of each) and its length.
pub fn diagnose(head: &[u8], tail: &[u8], file_len: u64) -> Diagnosis {
    let r = inspect::inspect(head, tail, file_len);
    let media_bytes = r.mdat.as_ref().map(|m| m.actual_end.saturating_sub(m.payload_start)).unwrap_or(0);
    let cut_off = r.mdat.as_ref().map(|m| m.exceeds_eof_by > 0).unwrap_or(false);
    let moov_ok = matches!(r.moov, Some((_, _, true)));
    let size_unusable = r.mdat.as_ref().map(|m| !matches!(m.size_field, "32-bit" | "64-bit")).unwrap_or(false);
    let verdict = match r.format {
        "empty" => Verdict::Empty,
        "mpeg-ts" => Verdict::TransportStream,
        "unknown" => Verdict::NotMp4,
        _ if moov_ok && size_unusable && media_bytes > 0 => Verdict::HeaderDamaged,
        _ if moov_ok && cut_off => Verdict::Truncated,
        _ if moov_ok => Verdict::Healthy,
        _ if r.fragmented => Verdict::Fragmented,
        _ if media_bytes > 0 => Verdict::IndexMissing,
        _ => Verdict::NotMp4,
    };
    Diagnosis { verdict, file_len, media_bytes, device: r.device.clone(), cut_off, notes: r.notes.clone(), inner: r }
}

/// Diagnose a file on disk, reading at most [`WINDOW`] bytes from each end.
pub fn diagnose_file<P: AsRef<Path>>(path: P) -> io::Result<Diagnosis> {
    let mut f = File::open(path)?;
    let len = f.metadata()?.len();
    let n = len.min(WINDOW);
    let mut head = vec![0u8; n as usize];
    f.read_exact(&mut head)?;
    let mut tail = vec![0u8; n as usize];
    f.seek(SeekFrom::Start(len - n))?;
    f.read_exact(&mut tail)?;
    Ok(diagnose(&head, &tail, len))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bx(t: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut v = ((payload.len() + 8) as u32).to_be_bytes().to_vec();
        v.extend_from_slice(t);
        v.extend_from_slice(payload);
        v
    }

    #[test]
    fn empty_file() {
        assert_eq!(diagnose(&[], &[], 0).verdict, Verdict::Empty);
    }

    #[test]
    fn interrupted_recording() {
        let mut f = bx(b"ftyp", b"isom\0\0\0\0isomavc1");
        f.extend_from_slice(&1_000_000u32.to_be_bytes());
        f.extend_from_slice(b"mdat");
        f.extend(std::iter::repeat(7u8).take(5000));
        let d = diagnose(&f, &f, f.len() as u64);
        assert_eq!(d.verdict, Verdict::IndexMissing);
        assert!(d.repairable() && d.cut_off);
        assert_eq!(d.repair_url(), Some(REPAIR_URL));
    }

    #[test]
    fn not_mp4() {
        let f = b"hello world, definitely not a video file".to_vec();
        assert_eq!(diagnose(&f, &f, f.len() as u64).verdict, Verdict::NotMp4);
    }
}
