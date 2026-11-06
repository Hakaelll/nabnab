use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Segment {
    pub start: f64,
    pub end: f64,
    pub text: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum TrackOrigin {
    Creator,
    Automatic,
    Imported,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalTrack {
    pub id: String,
    pub language: String,
    pub origin: TrackOrigin,
    pub path: Option<PathBuf>,
    pub segments: Vec<Segment>,
    #[serde(default)]
    pub formatting_lost: bool,
}

fn timestamp(value: &str) -> Option<f64> {
    let value = value.replace(',', ".");
    let parts: Vec<&str> = value.split(':').collect();
    let (hours, minutes, seconds) = match parts.as_slice() {
        [minutes, seconds] => (
            0.0,
            minutes.parse::<f64>().ok()?,
            seconds.parse::<f64>().ok()?,
        ),
        [hours, minutes, seconds] => (
            hours.parse::<f64>().ok()?,
            minutes.parse::<f64>().ok()?,
            seconds.parse::<f64>().ok()?,
        ),
        _ => return None,
    };
    (hours.is_finite()
        && minutes.is_finite()
        && seconds.is_finite()
        && hours >= 0.0
        && minutes >= 0.0
        && minutes < 60.0
        && seconds >= 0.0
        && seconds < 60.0)
        .then_some(hours * 3600.0 + minutes * 60.0 + seconds)
}

pub fn readable_text(value: &str) -> String {
    let mut text = String::new();
    let mut rest = value;
    while let Some(index) = rest.find('<') {
        text.push_str(&rest[..index]);
        let tail = &rest[index + 1..];
        if let Some(close) = tail.find('>') {
            if tail
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic() || c.is_ascii_digit() || c == '/')
            {
                rest = &tail[close + 1..];
                continue;
            }
        }
        text.push('<');
        rest = tail;
    }
    text.push_str(rest);
    while let Some(start) = text.find("{\\") {
        if let Some(end) = text[start..].find('}') {
            text.replace_range(start..start + end + 1, "");
        } else {
            break;
        }
    }
    text.replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn parse(input: &str) -> Result<Vec<Segment>, String> {
    if input.len() > 16 * 1024 * 1024 {
        return Err("Import subtitle files of at most 16 MB.".into());
    }
    let normalized = input
        .trim_start_matches('\u{feff}')
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    let lines: Vec<&str> = normalized.lines().collect();
    let mut segments = Vec::new();
    let mut cursor = 0;
    while cursor < lines.len() {
        let line = lines[cursor].trim();
        if line == "NOTE" || line.starts_with("NOTE ") || line == "STYLE" || line == "REGION" {
            while cursor < lines.len() && !lines[cursor].trim().is_empty() {
                cursor += 1;
            }
            continue;
        }
        let Some((left, right)) = line.split_once("-->") else {
            cursor += 1;
            continue;
        };
        let start = timestamp(left.trim()).ok_or("A subtitle has an invalid start time.")?;
        let end = timestamp(right.split_whitespace().next().unwrap_or_default())
            .ok_or("A subtitle has an invalid end time.")?;
        if end <= start {
            return Err("Subtitle end times must follow their start times.".into());
        }
        cursor += 1;
        let mut text = Vec::new();
        while cursor < lines.len() && !lines[cursor].trim().is_empty() {
            text.push(lines[cursor]);
            cursor += 1;
        }
        let text = readable_text(&text.join("\n"));
        if !text.is_empty() {
            segments.push(Segment { start, end, text });
        }
        if segments.len() > 100_000 {
            return Err("This subtitle track exceeds 100,000 segments.".into());
        }
    }
    if segments.is_empty() {
        return Err("This file has no readable SRT or VTT subtitle segments.".into());
    }
    segments.sort_by(|a, b| a.start.total_cmp(&b.start));
    Ok(segments)
}

pub fn rebase(segments: &[Segment], start: f64, end: f64) -> Result<Vec<Segment>, String> {
    if !start.is_finite() || !end.is_finite() || start < 0.0 || end <= start {
        return Err("Choose a valid subtitle clipping interval.".into());
    }
    Ok(segments
        .iter()
        .filter(|s| s.end > start && s.start < end)
        .map(|s| Segment {
            start: (s.start - start).max(0.0),
            end: (s.end.min(end) - start).max(0.0),
            text: s.text.clone(),
        })
        .collect())
}

fn time(value: f64, separator: char) -> String {
    let ms = (value.max(0.0) * 1000.0).round() as u64;
    format!(
        "{:02}:{:02}:{:02}{separator}{:03}",
        ms / 3_600_000,
        ms / 60_000 % 60,
        ms / 1000 % 60,
        ms % 1000
    )
}
pub fn to_srt(segments: &[Segment]) -> String {
    segments
        .iter()
        .enumerate()
        .map(|(i, s)| {
            format!(
                "{}\n{} --> {}\n{}\n\n",
                i + 1,
                time(s.start, ','),
                time(s.end, ','),
                s.text
            )
        })
        .collect()
}
pub fn to_vtt(segments: &[Segment]) -> String {
    let mut result = String::from("WEBVTT\n\n");
    for s in segments {
        result.push_str(&format!(
            "{} --> {}\n{}\n\n",
            time(s.start, '.'),
            time(s.end, '.'),
            s.text
        ));
    }
    result
}
pub fn plain_text(segments: &[Segment]) -> String {
    segments
        .iter()
        .map(|s| readable_text(&s.text))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_real_srt_and_vtt_timestamps_without_formatting() {
        let parsed = parse("WEBVTT\n\nSTYLE\n::cue { color:red; }\n\ncue-1\n02:14.500 --> 02:16.000 align:start\n<v Alice><b>Camera</b> transition &amp; light\n\n").unwrap();
        assert_eq!(
            parsed,
            vec![Segment {
                start: 134.5,
                end: 136.0,
                text: "Camera transition & light".into()
            }]
        );
        assert_eq!(parse(&to_srt(&parsed)).unwrap(), parsed);
        assert_eq!(parse(&to_vtt(&parsed)).unwrap(), parsed);
        assert_eq!(plain_text(&parsed), "Camera transition & light");
    }
    #[test]
    fn clipping_limits_both_edges_and_offsets_to_new_file() {
        let source = vec![
            Segment {
                start: 3.0,
                end: 7.0,
                text: "first".into(),
            },
            Segment {
                start: 8.0,
                end: 12.0,
                text: "last".into(),
            },
            Segment {
                start: 12.0,
                end: 15.0,
                text: "outside".into(),
            },
        ];
        assert_eq!(
            rebase(&source, 5.0, 10.0).unwrap(),
            vec![
                Segment {
                    start: 0.0,
                    end: 2.0,
                    text: "first".into()
                },
                Segment {
                    start: 3.0,
                    end: 5.0,
                    text: "last".into()
                }
            ]
        );
        assert!(rebase(&source, 10.0, 5.0).is_err());
        assert!(parse("1\n00:00:03,000 --> 00:00:02,000\nbad\n").is_err());
    }
}
