use crate::{
    captions::{self, Segment},
    library::LibraryItem,
};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

const MAX_SEGMENTS: usize = 100_000;
const MAX_TEXT: usize = 2 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum CaptionEditOptions {
    Segment {
        index: usize,
        start: f64,
        end: f64,
        text: String,
    },
    Replace {
        find: String,
        replace: String,
    },
    Shift {
        seconds: f64,
    },
    Sync {
        source_start: f64,
        target_start: f64,
        source_end: f64,
        target_end: f64,
    },
}

fn valid_text(text: &str) -> bool {
    !text.trim().is_empty()
        && !text
            .chars()
            .any(|character| character.is_control() && character != '\n' && character != '\t')
        && !text.lines().any(|line| line.trim().is_empty())
}
fn validate_segments(segments: &[Segment]) -> Result<(), String> {
    if segments.is_empty() || segments.len() > MAX_SEGMENTS {
        return Err("Edit 1–100,000 subtitle segments.".into());
    }
    let mut size = 0usize;
    for segment in segments {
        if !segment.start.is_finite()
            || !segment.end.is_finite()
            || segment.start < 0.0
            || segment.end <= segment.start
            || segment.end > 365.0 * 24.0 * 60.0 * 60.0
        {
            return Err("Subtitle times must be finite, non-negative, within one year, and end after their start.".into());
        }
        if !valid_text(&segment.text) {
            return Err(
                "Use non-empty plain subtitle text without blank lines or control characters."
                    .into(),
            );
        }
        size = size
            .checked_add(segment.text.len())
            .ok_or("The subtitle text is too large.")?;
        if size > MAX_TEXT {
            return Err("Edit at most 2 MiB of subtitle text per track.".into());
        }
    }
    Ok(())
}

pub fn transform_segments(
    segments: &[Segment],
    options: &CaptionEditOptions,
) -> Result<Vec<Segment>, String> {
    validate_segments(segments)?;
    let mut result = segments.to_vec();
    match options {
        CaptionEditOptions::Segment {
            index,
            start,
            end,
            text,
        } => {
            let cue = result
                .get_mut(*index)
                .ok_or("Choose an existing subtitle segment.")?;
            if text.len() > MAX_TEXT {
                return Err("Edit at most 2 MiB of subtitle text.".into());
            }
            *cue = Segment {
                start: *start,
                end: *end,
                text: text.replace("\r\n", "\n").replace('\r', "\n"),
            };
        }
        CaptionEditOptions::Replace { find, replace } => {
            if find.is_empty() || find.len() > 4096 || replace.len() > 4096 {
                return Err("Enter a literal search value of 1–4096 bytes and a replacement up to 4096 bytes.".into());
            }
            let mut total = 0usize;
            for cue in &mut result {
                let matches = cue.text.matches(find).count();
                let size = cue
                    .text
                    .len()
                    .checked_sub(
                        matches
                            .checked_mul(find.len())
                            .ok_or("The replacement is too large.")?,
                    )
                    .and_then(|size| size.checked_add(matches.checked_mul(replace.len())?))
                    .ok_or("The replacement is too large.")?;
                total = total
                    .checked_add(size)
                    .ok_or("The replacement is too large.")?;
                if total > MAX_TEXT {
                    return Err("This replacement would exceed the 2 MiB track limit.".into());
                }
                cue.text = cue.text.replace(find, replace);
            }
        }
        CaptionEditOptions::Shift { seconds } => {
            if !seconds.is_finite() {
                return Err("Choose a finite time offset in seconds.".into());
            }
            for cue in &mut result {
                cue.start += seconds;
                cue.end += seconds;
            }
        }
        CaptionEditOptions::Sync {
            source_start,
            target_start,
            source_end,
            target_end,
        } => {
            if ![source_start, target_start, source_end, target_end]
                .iter()
                .all(|value| value.is_finite() && **value >= 0.0)
                || source_end <= source_start
                || target_end <= target_start
            {
                return Err("Choose two increasing source points and two increasing target points, in seconds.".into());
            }
            let scale = (target_end - target_start) / (source_end - source_start);
            for cue in &mut result {
                cue.start = target_start + (cue.start - source_start) * scale;
                cue.end = target_start + (cue.end - source_start) * scale;
            }
        }
    }
    validate_segments(&result)?;
    Ok(result)
}

pub fn execute(
    input: &LibraryItem,
    track_id: &str,
    options: &CaptionEditOptions,
    staging: &Path,
) -> Result<PathBuf, String> {
    let track = input
        .details
        .tracks
        .iter()
        .find(|track| track.id == track_id)
        .ok_or("Choose an imported or generated subtitle track.")?;
    let segments = transform_segments(&track.segments, options)?;
    let staging = crate::library::authorized_path(staging)?;
    if !staging.is_dir() {
        return Err("The subtitle job staging folder is unavailable.".into());
    }
    let stem = crate::options::safe_segment(
        Path::new(&input.name)
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .as_ref(),
    );
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_nanos();
    let output = staging.join(format!("{stem}.edited-{nonce}.srt"));
    let escaped: Vec<_> = segments
        .iter()
        .map(|segment| Segment {
            start: segment.start,
            end: segment.end,
            text: segment
                .text
                .replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;"),
        })
        .collect();
    let text = captions::to_srt(&escaped);
    let parsed = captions::parse(&text)?;
    if parsed.len() != segments.len()
        || parsed.iter().zip(&segments).any(|(actual, expected)| {
            (actual.start - expected.start).abs() > 0.00051
                || (actual.end - expected.end).abs() > 0.00051
                || actual.text
                    != expected
                        .text
                        .split_whitespace()
                        .collect::<Vec<_>>()
                        .join(" ")
        })
    {
        return Err(
            "The edited track could not be represented safely as SRT. No output was published."
                .into(),
        );
    }
    let mut file=OpenOptions::new().create_new(true).write(true).open(&output).map_err(|_|"The edited subtitle output already exists or cannot be created. No original was replaced.")?;
    file.write_all(text.as_bytes())
        .and_then(|_| file.sync_all())
        .map_err(|e| e.to_string())?;
    drop(file);
    let output = crate::checked_output(&staging, &output.to_string_lossy())?;
    if fs::metadata(&output).map_err(|e| e.to_string())?.len() == 0 {
        return Err("The edited subtitle output is empty.".into());
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn cues() -> Vec<Segment> {
        vec![
            Segment {
                start: 2.0,
                end: 4.0,
                text: "Hola mundo".into(),
            },
            Segment {
                start: 6.0,
                end: 8.0,
                text: "mundo en español".into(),
            },
        ]
    }
    #[test]
    fn editing_and_literal_replacement_preserve_unselected_cues() {
        let original = cues();
        let edited = transform_segments(
            &original,
            &CaptionEditOptions::Segment {
                index: 1,
                start: 6.5,
                end: 8.5,
                text: "Texto corregido".into(),
            },
        )
        .unwrap();
        assert_eq!(
            edited,
            vec![
                Segment {
                    start: 2.0,
                    end: 4.0,
                    text: "Hola mundo".into()
                },
                Segment {
                    start: 6.5,
                    end: 8.5,
                    text: "Texto corregido".into()
                }
            ]
        );
        let replaced = transform_segments(
            &original,
            &CaptionEditOptions::Replace {
                find: "mundo".into(),
                replace: "vídeo".into(),
            },
        )
        .unwrap();
        assert_eq!(replaced[0].text, "Hola vídeo");
        assert_eq!(replaced[1].text, "vídeo en español");
        assert_eq!(original[1].text, "mundo en español");
    }
    #[test]
    fn shifting_and_two_point_sync_have_exact_expected_timing() {
        let shifted =
            transform_segments(&cues(), &CaptionEditOptions::Shift { seconds: -1.5 }).unwrap();
        assert_eq!(
            (
                shifted[0].start,
                shifted[0].end,
                shifted[1].start,
                shifted[1].end
            ),
            (0.5, 2.5, 4.5, 6.5)
        );
        let synced = transform_segments(
            &cues(),
            &CaptionEditOptions::Sync {
                source_start: 2.0,
                target_start: 3.0,
                source_end: 6.0,
                target_end: 9.0,
            },
        )
        .unwrap();
        assert_eq!(
            (
                synced[0].start,
                synced[0].end,
                synced[1].start,
                synced[1].end
            ),
            (3.0, 6.0, 9.0, 12.0)
        );
    }
    #[test]
    fn invalid_transforms_cannot_drop_cues_or_allocate_unbounded_replacement() {
        for options in [
            CaptionEditOptions::Shift { seconds: -3.0 },
            CaptionEditOptions::Shift { seconds: f64::NAN },
            CaptionEditOptions::Sync {
                source_start: 2.0,
                target_start: 3.0,
                source_end: 2.0,
                target_end: 9.0,
            },
            CaptionEditOptions::Segment {
                index: 0,
                start: 2.0,
                end: 1.0,
                text: "bad".into(),
            },
            CaptionEditOptions::Replace {
                find: "".into(),
                replace: "bad".into(),
            },
            CaptionEditOptions::Segment {
                index: 100,
                start: 0.0,
                end: 1.0,
                text: "missing".into(),
            },
            CaptionEditOptions::Segment {
                index: 0,
                start: 0.0,
                end: 1.0,
                text: "text\n\n2\n00:01 --> 00:02\ninjection".into(),
            },
        ] {
            assert!(transform_segments(&cues(), &options).is_err());
        }
        let expanded = vec![Segment {
            start: 0.0,
            end: 1.0,
            text: "a".repeat(1_000_000),
        }];
        assert!(transform_segments(
            &expanded,
            &CaptionEditOptions::Replace {
                find: "a".into(),
                replace: "b".repeat(4096)
            }
        )
        .is_err());
    }
    #[test]
    fn execute_creates_a_readable_copy_and_keeps_author_text_untouched() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let work = std::env::temp_dir().join(format!(
            "nabnab-caption-edit-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir(&work).unwrap();
        let source = work.join("author.srt");
        let original = b"1\n00:00:02,000 --> 00:00:04,000\nAuthor caption\n\n";
        fs::write(&source, original).unwrap();
        let mut item = crate::library::index(&source, None, None, None).unwrap();
        item.details.tracks.push(captions::LocalTrack {
            id: "author".into(),
            language: "es".into(),
            origin: captions::TrackOrigin::Creator,
            path: Some(source.clone()),
            segments: cues(),
            formatting_lost: false,
        });
        let stage = work.join("stage");
        fs::create_dir(&stage).unwrap();
        let output = execute(
            &item,
            "author",
            &CaptionEditOptions::Segment {
                index: 0,
                start: 2.5,
                end: 4.5,
                text: "Literal <b>plain</b> & text".into(),
            },
            &stage,
        )
        .unwrap();
        let parsed = captions::parse(&fs::read_to_string(&output).unwrap()).unwrap();
        assert_eq!(
            parsed[0],
            Segment {
                start: 2.5,
                end: 4.5,
                text: "Literal <b>plain</b> & text".into()
            }
        );
        assert_eq!(fs::read(source).unwrap(), original);
        assert_eq!(parsed[1], cues()[1]);
        assert_eq!(work.parent(), Some(std::env::temp_dir().as_path()));
        fs::remove_dir_all(work).unwrap();
    }
}
