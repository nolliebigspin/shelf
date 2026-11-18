//! ffmpeg / ffprobe: locating the binaries, reading metadata and grabbing
//! thumbnails. Media files are only ever read.

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// How to install ffmpeg, shown when it's missing.
const INSTALL_HINT: &str = if cfg!(target_os = "macos") {
    "brew install ffmpeg"
} else if cfg!(target_os = "windows") {
    "winget install Gyan.FFmpeg (then restart Shelf)"
} else {
    "sudo apt install ffmpeg (or your distribution's package manager)"
};

/// Where to look for ffmpeg. An app started from Finder or a desktop launcher
/// doesn't always get the shell's PATH, so the usual install locations come first.
fn search_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if cfg!(target_os = "macos") {
        // Homebrew: Apple Silicon, then Intel.
        dirs.extend(["/opt/homebrew/bin", "/usr/local/bin"].map(PathBuf::from));
    } else if cfg!(target_os = "windows") {
        let env = |k: &str| std::env::var_os(k).map(PathBuf::from);
        dirs.extend(env("LOCALAPPDATA").map(|d| d.join("Microsoft").join("WinGet").join("Links")));
        dirs.extend(env("USERPROFILE").map(|d| d.join("scoop").join("shims")));
        dirs.extend(env("ProgramData").map(|d| d.join("chocolatey").join("bin")));
        dirs.push(PathBuf::from("C:\\ffmpeg\\bin"));
    } else {
        dirs.extend(["/usr/bin", "/usr/local/bin", "/snap/bin"].map(PathBuf::from));
    }
    if let Some(path) = std::env::var_os("PATH") {
        dirs.extend(std::env::split_paths(&path));
    }
    dirs
}

#[derive(Clone, Debug)]
pub struct Tools {
    pub ffmpeg: PathBuf,
    pub ffprobe: PathBuf,
}

fn find(name: &str) -> Option<PathBuf> {
    let file = if cfg!(target_os = "windows") {
        format!("{name}.exe")
    } else {
        name.to_owned()
    };
    search_dirs().into_iter().map(|dir| dir.join(&file)).find(|p| p.is_file())
}

pub fn detect() -> Result<Tools, String> {
    match (find("ffmpeg"), find("ffprobe")) {
        (Some(ffmpeg), Some(ffprobe)) => Ok(Tools { ffmpeg, ffprobe }),
        (ffmpeg, ffprobe) => {
            let mut missing = Vec::new();
            if ffmpeg.is_none() {
                missing.push("ffmpeg");
            }
            if ffprobe.is_none() {
                missing.push("ffprobe");
            }
            Err(format!(
                "{} not found. Install it with: {INSTALL_HINT}",
                missing.join(" and ")
            ))
        }
    }
}

#[derive(Debug, Default, PartialEq)]
pub struct Meta {
    pub duration_ms: Option<i64>,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub codec: Option<String>,
    /// Capture date (unix seconds) from the container metadata, if present.
    pub created_at: Option<i64>,
}

fn command(bin: &Path) -> Command {
    let mut cmd = Command::new(bin);
    cmd.stdin(Stdio::null());
    #[cfg(target_os = "windows")]
    {
        // Don't flash a console window for every probe / thumbnail.
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// `ffprobe -v error -print_format json -show_format -show_streams <file>`.
/// Opens the media file read-only.
pub fn probe(tools: &Tools, file: &Path, is_video: bool) -> Result<Meta, String> {
    let out = command(&tools.ffprobe)
        .args(["-v", "error", "-print_format", "json", "-show_format", "-show_streams"])
        .arg(file)
        .output()
        .map_err(|e| format!("cannot run ffprobe: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_owned());
    }
    let json: Value = serde_json::from_slice(&out.stdout).map_err(|e| e.to_string())?;
    Ok(parse_probe(&json, is_video))
}

pub fn parse_probe(json: &Value, is_video: bool) -> Meta {
    let empty = Vec::new();
    let streams = json["streams"].as_array().unwrap_or(&empty);
    // First real video stream; embedded cover art doesn't count.
    let video = streams.iter().find(|s| {
        s["codec_type"] == "video" && s["disposition"]["attached_pic"].as_i64().unwrap_or(0) == 0
    });
    let format = &json["format"];
    let seconds = |v: &Value| v.as_str().and_then(|s| s.parse::<f64>().ok());

    let mut meta = Meta::default();
    if let Some(v) = video {
        meta.width = v["width"].as_i64();
        meta.height = v["height"].as_i64();
        meta.codec = v["codec_name"].as_str().map(str::to_owned);
        // Phone footage is often stored sideways with a rotation flag.
        let rotation = v["side_data_list"]
            .as_array()
            .and_then(|l| l.iter().find_map(|d| d["rotation"].as_f64()))
            .unwrap_or(0.0);
        if (rotation.abs() as i64) % 180 == 90 {
            std::mem::swap(&mut meta.width, &mut meta.height);
        }
    }
    if is_video {
        meta.duration_ms = seconds(&format["duration"])
            .or_else(|| video.and_then(|v| seconds(&v["duration"])))
            .map(|s| (s * 1000.0).round() as i64);
    }
    meta.created_at = [&format["tags"]["creation_time"], &format["tags"]["com.apple.quicktime.creationdate"]]
        .into_iter()
        .chain(video.map(|v| &v["tags"]["creation_time"]))
        .find_map(|v| v.as_str().and_then(parse_iso_utc));
    meta
}

/// "2024-05-01T12:34:56.000000Z" → unix seconds. Honors a trailing ±hh:mm / ±hhmm offset.
pub fn parse_iso_utc(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() < 19 || !s.is_char_boundary(19) {
        return None;
    }
    let num = |r: std::ops::Range<usize>| s.get(r)?.parse::<i64>().ok();
    let (y, m, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (hh, mm, ss) = (num(11..13)?, num(14..16)?, num(17..19)?);
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    // Days since 1970-01-01 (Howard Hinnant's days_from_civil).
    let y2 = if m <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2.rem_euclid(400);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;

    let tail = &s[19..];
    let offset = tail.rfind(['+', '-']).and_then(|i| {
        let sign = if tail.as_bytes()[i] == b'-' { -1 } else { 1 };
        let digits: String = tail[i + 1..].chars().filter(char::is_ascii_digit).collect();
        if digits.len() != 4 {
            return None;
        }
        Some(sign * (digits[..2].parse::<i64>().ok()? * 3600 + digits[2..].parse::<i64>().ok()? * 60))
    });
    Some(days * 86400 + hh * 3600 + mm * 60 + ss - offset.unwrap_or(0))
}

/// One 400px-wide JPEG frame, written to `out` (always inside the app data folder).
/// Videos are grabbed at 1 s, or at 0 s for clips shorter than that.
pub fn thumbnail(
    tools: &Tools,
    file: &Path,
    out: &Path,
    is_video: bool,
    duration_ms: Option<i64>,
) -> Result<(), String> {
    let run = |seek: Option<&str>| -> Result<(), String> {
        let mut cmd = command(&tools.ffmpeg);
        cmd.args(["-v", "error", "-y"]);
        if let Some(s) = seek {
            cmd.args(["-ss", s]);
        }
        cmd.arg("-i").arg(file).args([
            "-an", "-sn", "-frames:v", "1", "-update", "1", "-vf", "scale=400:-2", "-q:v", "5",
        ]);
        let o = cmd.arg(out).output().map_err(|e| format!("cannot run ffmpeg: {e}"))?;
        let written = std::fs::metadata(out).map(|m| m.len() > 0).unwrap_or(false);
        if o.status.success() && written {
            Ok(())
        } else {
            let _ = std::fs::remove_file(out);
            Err(String::from_utf8_lossy(&o.stderr).trim().to_owned())
        }
    };
    if !is_video {
        return run(None);
    }
    if duration_ms.is_some_and(|d| d < 1000) {
        return run(Some("0"));
    }
    // Unknown or wrong duration: fall back to the first frame.
    run(Some("1")).or_else(|_| run(Some("0")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_dates() {
        assert_eq!(parse_iso_utc("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_iso_utc("2024-05-01T12:34:56.000000Z"), Some(1714566896));
        assert_eq!(parse_iso_utc("2024-05-01T14:34:56+0200"), Some(1714566896));
        assert_eq!(parse_iso_utc("2024-05-01T10:34:56-02:00"), Some(1714566896));
        assert_eq!(parse_iso_utc("garbage"), None);
    }

    #[test]
    fn probe_json_to_meta() {
        let json: Value = serde_json::from_str(
            r#"{"streams":[
                 {"codec_type":"audio","codec_name":"aac"},
                 {"codec_type":"video","codec_name":"hevc","width":3840,"height":2160,
                  "side_data_list":[{"rotation":-90}]}],
                "format":{"duration":"12.345000","tags":{"creation_time":"2024-05-01T12:34:56.000000Z"}}}"#,
        )
        .unwrap();
        assert_eq!(
            parse_probe(&json, true),
            Meta {
                duration_ms: Some(12345),
                width: Some(2160),
                height: Some(3840),
                codec: Some("hevc".into()),
                created_at: Some(1714566896),
            }
        );
        assert_eq!(parse_probe(&json, false).duration_ms, None);
    }

    /// Needs Homebrew ffmpeg. Generates its own sample clips in the temp folder.
    #[test]
    fn real_ffmpeg_probe_and_thumbnail() {
        let Ok(tools) = detect() else { return };
        let dir = std::env::temp_dir().join(format!("asset-manager-media-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let make = |name: &str, args: &[&str]| {
            let out = dir.join(name);
            let ok = Command::new(&tools.ffmpeg)
                .args(["-v", "error", "-y", "-f", "lavfi", "-i", "testsrc=size=640x360:rate=25"])
                .args(args)
                .arg(&out)
                .status()
                .unwrap()
                .success();
            assert!(ok, "could not generate {name}");
            out
        };
        let long = make("long.mov", &["-t", "3", "-pix_fmt", "yuv420p"]);
        let short = make("short.mp4", &["-t", "0.4", "-pix_fmt", "yuv420p"]);
        let still = make("still.png", &["-frames:v", "1"]);

        for (file, is_video) in [(&long, true), (&short, true), (&still, false)] {
            let before = std::fs::metadata(file).unwrap();
            let meta = probe(&tools, file, is_video).unwrap();
            assert_eq!((meta.width, meta.height), (Some(640), Some(360)));
            assert!(meta.codec.is_some());
            assert_eq!(meta.duration_ms.is_some(), is_video);
            let thumb = dir.join("thumb.jpg");
            thumbnail(&tools, file, &thumb, is_video, meta.duration_ms).unwrap();
            let t = probe(&tools, &thumb, false).unwrap();
            assert_eq!((t.width, t.height), (Some(400), Some(226)));
            // The source file is untouched.
            let after = std::fs::metadata(file).unwrap();
            assert_eq!((before.len(), before.modified().unwrap()), (after.len(), after.modified().unwrap()));
        }
        assert_eq!(probe(&tools, &long, true).unwrap().duration_ms, Some(3000));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
