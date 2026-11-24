# vidfix-detect

Why won't this MP4/MOV open? `vidfix-detect` reads only the first and last 4 MB of a file and tells you.

| Verdict | What happened | What to do |
|---|---|---|
| `Healthy` | Index (moov) intact | If it still won't play, the damage is inside the frames |
| `IndexMissing` | Recording interrupted (dead battery, pulled card, crash): media data is there, the index was never written. This is "moov atom not found". | Rebuild the index |
| `HeaderDamaged` | Index present, but the media box size is zeroed, so players never reach the index | Rebuild from the file's own index |
| `Truncated` | Index at the front, file cut short | Plays up to the cut |
| `Fragmented` | Fragmented MP4, no complete index | Fragment-aware rebuild |
| `Empty` | 0 bytes | Card-level recovery (PhotoRec etc.) |
| `TransportStream` | MPEG-TS, has no index to lose | `ffmpeg -i in.ts -c copy out.mp4` |
| `NotMp4` | Not an MP4/MOV | |

No decoding, no dependencies, no network. Works on files of any size. Also reports the camera maker when the file says
(GoPro, DJI, Sony, Canon, Nikon, Apple, Android, Ambarella dashcams and action cams, Insta360, Panasonic).

## Library

```rust
let d = vidfix_detect::diagnose_file("GX010042.MP4")?;
println!("{:?} {} {}", d.verdict, d.device, d.message());
if let Some(url) = d.repair_url() {
    // Only for IndexMissing / HeaderDamaged.
    println!("Try the repair: {url}");
}
```

Have the bytes already (browser, upload handler)? `vidfix_detect::diagnose(head, tail, file_len)`.
`d.json()` gives the full box layout.

## CLI

```
cargo install vidfix-detect
vidfix-detect GX010042.MP4
GX010042.MP4: IndexMissing (GoPro)
  This recording was interrupted before its index was written. 3815 MB of video data is still in the file and can usually be rebuilt.
  Free preview of the rebuilt file: https://tensorust.com/fix-video/
```

Exit code 0 = healthy, 1 = damaged, 2 = error. `--json` for machine output.

## Repair

For the two rebuildable verdicts, `repair_url()` returns https://tensorust.com/fix-video/ (by Tensorust, the authors
of this crate). It rebuilds the index in the browser without uploading the video, shows a free preview, and charges
only for the download. Whether your app shows that link is up to you. The detection works the same either way.

Tested on 51 damaged and 15 healthy recordings from GoPro, DJI, Sony, Canon, Nikon, iPhone, Android and Ambarella
cameras: every file got the verdict for its damage.

License: MIT OR Apache-2.0
