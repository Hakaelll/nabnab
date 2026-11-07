pub mod commands;
pub mod component;

use crate::{
    captions::{self, Cue},
    engine, library, portable_zip, sessions, store,
    workspaces::Workspace,
};
use component::Pair;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    sync::OnceLock,
};
use tokio::sync::watch;

const MAX_CUES: usize = 10_000;
const MAX_TEXT: usize = 2 * 1024 * 1024;
const MAX_CUE: usize = 16 * 1024;
const MAX_DRAFT: u64 = 8 * 1024 * 1024;
const MAX_DRAFT_STORAGE: u64 = 64 * 1024 * 1024;
const MAX_DRAFT_COUNT: usize = 256;
const ENGINE: &str = "Bergamot 0.4.9";

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Input {
    pub library_id: String,
    pub track_id: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Selection {
    pub input: Input,
    pub pair: Pair,
    pub source_hash: String,
}

impl Selection {
    pub fn validate(&self) -> Result<(), String> {
        if self.input.library_id.len() != 64
            || !self
                .input
                .library_id
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
            || self.input.track_id.is_empty()
            || self.input.track_id.len() > 256
            || self.source_hash.len() != 64
            || !self
                .source_hash
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            return Err("Choose a valid local subtitle track.".into());
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    Srt,
    Vtt,
}

impl Format {
    fn extension(self) -> &'static str {
        match self {
            Self::Srt => "srt",
            Self::Vtt => "vtt",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Export {
    pub draft_id: String,
    pub revision: u64,
    pub format: Format,
    pub bilingual: bool,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Draft {
    pub id: String,
    pub revision: u64,
    pub name: String,
    pub selection: Selection,
    pub original: Vec<Cue>,
    pub translated: Vec<String>,
    pub contextual_blocks: usize,
    pub edited: BTreeSet<usize>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Edit {
    pub index: usize,
    pub text: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Reference {
    pub draft_id: String,
    pub revision: u64,
    pub pair: Pair,
    pub bilingual: bool,
    pub engine: String,
}

fn plain_markup(value: &str, output: &mut String) {
    let decoded = value
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#x27;", "'")
        .replace("&nbsp;", " ")
        .replace("&amp;", "&");
    for character in decoded.chars() {
        match character {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            '\t' => output.push(' '),
            '\n' => output.push('\n'),
            control if control.is_control() => {}
            value => output.push(value),
        }
    }
}

pub fn markup(value: &str) -> String {
    static TAG: OnceLock<regex::Regex> = OnceLock::new();
    let tag =
        TAG.get_or_init(|| regex::Regex::new(r"(?i)<(/?)([a-z0-9][a-z0-9.]*)([^<>\n]*)>").unwrap());
    let mut result = String::new();
    let mut active = Vec::<&str>::new();
    let mut end = 0;
    for capture in tag.captures_iter(value) {
        let matched = capture.get(0).unwrap();
        plain_markup(&value[end..matched.start()], &mut result);
        end = matched.end();
        let name = match capture[2].to_ascii_lowercase().as_str() {
            "i" => "i",
            "b" => "b",
            "u" => "u",
            "br" => {
                result.push('\n');
                continue;
            }
            _ => continue,
        };
        if !capture[3].trim().is_empty() {
            continue;
        }
        if capture[1].is_empty() && active.len() < 8 {
            active.push(name);
            result.push_str(&format!("<{name}>"));
        } else if !capture[1].is_empty() && active.last().copied() == Some(name) {
            active.pop();
            result.push_str(&format!("</{name}>"));
        }
    }
    plain_markup(&value[end..], &mut result);
    for name in active.into_iter().rev() {
        result.push_str(&format!("</{name}>"));
    }
    result
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn validate_text<'a>(values: impl IntoIterator<Item = &'a str>) -> Result<(), String> {
    let mut bytes = 0;
    for value in values {
        if value.len() > MAX_CUE || captions::readable_text(value).is_empty() {
            return Err(
                "Each subtitle cue needs readable text and can contain at most 16 KiB.".into(),
            );
        }
        bytes += value.len();
        if bytes > MAX_TEXT {
            return Err("Subtitle text exceeds the 2 MiB translation limit.".into());
        }
    }
    Ok(())
}

pub fn source(item: &library::LibraryItem, input: &Input) -> Result<(Vec<Cue>, String), String> {
    if item.id != input.library_id {
        return Err("Choose the original subtitle source.".into());
    }
    let track = item
        .details
        .tracks
        .iter()
        .find(|track| track.id == input.track_id)
        .ok_or("Choose a local subtitle track from Library.")?;
    let mut cues = if let Some(file) = &track.path {
        let file = library::authorized_path(file)?;
        if portable_zip::regular_file(&file)? > 16 * 1024 * 1024
            || !file.extension().is_some_and(|extension| {
                extension.eq_ignore_ascii_case("vtt") || extension.eq_ignore_ascii_case("srt")
            })
        {
            return Err("Choose a local SRT or VTT file of at most 16 MiB.".into());
        }
        captions::parse_cues(
            &fs::read_to_string(file).map_err(|error| error.to_string())?,
            true,
        )?
    } else {
        track
            .segments
            .iter()
            .cloned()
            .map(|segment| Cue {
                identifier: None,
                segment,
            })
            .collect()
    };
    if cues.is_empty() || cues.len() > MAX_CUES {
        return Err("Translate a track with 1–10,000 subtitle cues.".into());
    }
    for cue in &mut cues {
        let segment = &mut cue.segment;
        if !segment.start.is_finite()
            || !segment.end.is_finite()
            || segment.start < 0.0
            || segment.end <= segment.start
            || segment.end > 365.0 * 86_400.0
        {
            return Err("The source contains invalid subtitle times.".into());
        }
        segment.text = markup(&segment.text);
    }
    validate_text(cues.iter().map(|cue| cue.segment.text.as_str()))?;
    let hash = source_hash(&cues)?;
    Ok((cues, hash))
}

fn source_hash(cues: &[Cue]) -> Result<String, String> {
    let bytes = serde_json::to_vec(cues).map_err(|error| error.to_string())?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

fn blocks(cues: &[Cue]) -> Vec<Vec<&str>> {
    let mut result = Vec::new();
    let mut current = Vec::new();
    let mut size = 0;
    let mut last_end = 0.0;
    for cue in cues {
        let segment = &cue.segment;
        if !current.is_empty()
            && (current.len() == 6
                || size + segment.text.len() > 480
                || segment.start - last_end > 2.0
                || segment.start < last_end)
        {
            result.push(std::mem::take(&mut current));
            size = 0;
        }
        size += segment.text.len();
        last_end = segment.end;
        current.push(segment.text.as_str());
    }
    if !current.is_empty() {
        result.push(current);
    }
    result
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Translated {
    cues: Vec<String>,
    contextual_blocks: usize,
}

fn local_arg(path: &Path) -> Result<String, String> {
    let path = path
        .to_str()
        .ok_or("The local translation path is not Unicode.")?;
    #[cfg(windows)]
    let path = path.strip_prefix(r"\\?\").unwrap_or(path);
    if path.contains(',') || !Path::new(path).is_absolute() {
        return Err("Local translation requires a private data path without commas.".into());
    }
    Ok(path.into())
}

pub async fn translate(
    engines: &Path,
    component_root: &Path,
    root: &Path,
    selection: &Selection,
    cues: &[Cue],
    cancel: watch::Receiver<bool>,
) -> Result<(Vec<String>, usize), String> {
    let (runtime, model) = selection.pair.enabled(component_root)?;
    component::verify(&runtime, &model, selection.pair, cancel.clone()).await?;
    let workspace = Workspace::new(root)?;
    let script = workspace.path.join("translate.mjs");
    let input = workspace.path.join("input.json");
    let (from, to) = selection.pair.languages();
    let (model_file, shortlist_file) = selection.pair.files();
    let groups = blocks(cues);
    let batch = serde_json::json!({
        "pair": { "from": from, "to": to },
        "modelFile": model_file,
        "shortlistFile": shortlist_file,
        "blocks": groups,
    });
    fs::write(&script, include_str!("translation/worker.mjs"))
        .map_err(|error| error.to_string())?;
    fs::write(
        &input,
        serde_json::to_vec(&batch).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    let (runtime, model, scratch) = (
        local_arg(&runtime)?,
        local_arg(&model)?,
        local_arg(&workspace.path)?,
    );
    let args = [
        "run".into(),
        "--no-config".into(),
        "--no-lock".into(),
        "--no-npm".into(),
        "--no-remote".into(),
        "--cached-only".into(),
        "--no-prompt".into(),
        "--deny-net".into(),
        "--deny-run".into(),
        "--deny-write".into(),
        "--deny-env".into(),
        "--deny-ffi".into(),
        format!("--allow-read={runtime},{model},{scratch}"),
        local_arg(&script)?,
        runtime,
        model,
        local_arg(&input)?,
    ];
    let mut command = tokio::process::Command::new(engines.join("deno.exe"));
    command
        .args(args)
        .env("DENO_DIR", workspace.path.join("deno"));
    let raw = engine::run_configured_process(command, cancel, 0, true, |_| {}).await?;
    let translated: Translated =
        serde_json::from_str(&raw).map_err(|_| "Bergamot returned invalid subtitle data.")?;
    if translated.cues.len() != cues.len() || translated.contextual_blocks > groups.len() {
        return Err("The engine could not match translated cues to the original track.".into());
    }
    let translated_cues: Vec<_> = translated.cues.iter().map(|cue| markup(cue)).collect();
    validate_text(translated_cues.iter().map(String::as_str))?;
    Ok((translated_cues, translated.contextual_blocks))
}

pub fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 80 && id.bytes().all(|byte| byte.is_ascii_digit() || byte == b'-')
}

pub fn directory(root: &Path) -> Result<PathBuf, String> {
    sessions::secure_module_directory(root, "subtitle-translations")
}

fn draft_path(root: &Path, id: &str) -> Result<PathBuf, String> {
    if !valid_id(id) {
        return Err("Choose a valid subtitle translation.".into());
    }
    Ok(directory(root)?.join(format!("{id}.json")))
}

pub fn read(root: &Path, id: &str) -> Result<Draft, String> {
    let file = library::authorized_path(&draft_path(root, id)?)
        .map_err(|_| "This translation draft is unavailable.")?;
    if portable_zip::regular_file(&file)? > MAX_DRAFT {
        return Err("This translation draft exceeds its size limit.".into());
    }
    let draft: Draft =
        serde_json::from_reader(fs::File::open(file).map_err(|error| error.to_string())?)
            .map_err(|_| "This translation draft is invalid.")?;
    draft.selection.validate()?;
    if draft.id != id
        || draft.revision == 0
        || draft.original.is_empty()
        || draft.original.len() > MAX_CUES
        || draft.original.len() != draft.translated.len()
        || source_hash(&draft.original)? != draft.selection.source_hash
    {
        return Err("This translation draft is invalid.".into());
    }
    Ok(draft)
}

pub fn existing(root: &Path, id: &str) -> Result<bool, String> {
    Ok(fs::symlink_metadata(draft_path(root, id)?).is_ok())
}

pub fn write(root: &Path, draft: &Draft) -> Result<(), String> {
    let path = draft_path(root, &draft.id)?;
    let directory = path.parent().ok_or("Invalid translation folder.")?;
    let mut bytes = 0;
    let mut count = 0;
    for entry in fs::read_dir(directory).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        if entry.path() != path
            && entry
                .path()
                .file_stem()
                .and_then(|stem| stem.to_str())
                .is_some_and(valid_id)
            && entry
                .path()
                .extension()
                .is_some_and(|extension| extension == "json")
        {
            bytes += portable_zip::regular_file(&entry.path())?;
            count += 1;
        }
    }
    let content = serde_json::to_vec(draft).map_err(|error| error.to_string())?;
    if count >= MAX_DRAFT_COUNT
        || content.len() as u64 > MAX_DRAFT
        || bytes + content.len() as u64 > MAX_DRAFT_STORAGE
    {
        return Err(
            "Translation drafts are at their storage limit. Remove an old draft before saving."
                .into(),
        );
    }
    if fs::symlink_metadata(&path).is_ok() {
        library::authorized_path(&path)?;
    }
    let temporary = store::temporary_path(directory, "translation");
    let result = store::write_temporary(&temporary, &content)
        .and_then(|_| store::replace_file(&temporary, &path));
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

pub fn remove(root: &Path, id: &str) -> Result<(), String> {
    let path = library::authorized_path(&draft_path(root, id)?)?;
    fs::remove_file(path).map_err(|error| error.to_string())
}

pub fn edit(draft: &mut Draft, revision: u64, edits: Vec<Edit>) -> Result<(), String> {
    if draft.revision != revision || edits.is_empty() || edits.len() > MAX_CUES {
        return Err("Reload the current translation before editing it.".into());
    }
    let mut changed = false;
    for edit in edits {
        let current = draft
            .translated
            .get_mut(edit.index)
            .ok_or("Choose an existing subtitle cue.")?;
        if edit.text.len() > MAX_CUE {
            return Err("This subtitle cue is too large.".into());
        }
        let text = markup(&edit.text);
        if *current != text {
            *current = text;
            draft.edited.insert(edit.index);
            changed = true;
        }
    }
    validate_text(draft.translated.iter().map(String::as_str))?;
    if changed {
        draft.revision += 1;
    }
    Ok(())
}

pub fn export(
    draft: &Draft,
    options: &Export,
    staging: &Path,
) -> Result<(PathBuf, Reference), String> {
    if options.draft_id != draft.id || options.revision != draft.revision {
        return Err("The translation changed after export was queued. Review it and queue the current version.".into());
    }
    let mut output = if options.format == Format::Vtt {
        "WEBVTT\n\n".to_owned()
    } else {
        String::new()
    };
    let separator = if options.format == Format::Vtt {
        '.'
    } else {
        ','
    };
    for (index, (original, translated)) in draft.original.iter().zip(&draft.translated).enumerate()
    {
        let identifier = match options.format {
            Format::Vtt => original.identifier.as_deref().map(str::to_owned),
            Format::Srt => Some(
                original
                    .identifier
                    .as_deref()
                    .filter(|id| id.bytes().all(|byte| byte.is_ascii_digit()))
                    .map_or_else(|| (index + 1).to_string(), str::to_owned),
            ),
        };
        if let Some(identifier) = identifier {
            output.push_str(&identifier);
            output.push('\n');
        }
        let cue = &original.segment;
        output.push_str(&format!(
            "{} --> {}\n",
            captions::time(cue.start, separator),
            captions::time(cue.end, separator)
        ));
        if options.bilingual {
            output.push_str(&markup(&cue.text));
            output.push('\n');
        }
        output.push_str(&markup(translated));
        output.push_str("\n\n");
    }
    let verified = captions::parse_cues(&output, true)?;
    if verified.len() != draft.original.len()
        || verified
            .iter()
            .zip(&draft.original)
            .any(|(actual, expected)| {
                (actual.segment.start - expected.segment.start).abs() > 0.000_501
                    || (actual.segment.end - expected.segment.end).abs() > 0.000_501
                    || options.format == Format::Vtt && actual.identifier != expected.identifier
            })
    {
        return Err("The subtitle export could not preserve cue correspondence.".into());
    }
    let (from, to) = draft.selection.pair.languages();
    let label = Path::new(&draft.name)
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy();
    let label: String = label
        .chars()
        .filter(|c| !c.is_control() && !r#"<>:"/\|?*"#.contains(*c))
        .take(72)
        .collect();
    let mode = if options.bilingual {
        format!("{from}-{to}")
    } else {
        to.into()
    };
    let file = staging.join(format!(
        "{label}.translated.{mode}.{}",
        options.format.extension()
    ));
    fs::write(&file, output).map_err(|error| error.to_string())?;
    Ok((
        file,
        Reference {
            draft_id: draft.id.clone(),
            revision: draft.revision,
            pair: draft.selection.pair,
            bilingual: options.bilingual,
            engine: ENGINE.into(),
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translated_export_preserves_cue_order_and_markup_after_manual_edit() {
        let root = std::env::temp_dir().join(format!(
            "nabnab-translation-export-{}-{}",
            std::process::id(),
            crate::queue::now()
        ));
        fs::create_dir(&root).unwrap();
        let mut original = captions::parse_cues(
            "WEBVTT\n\nsecond\n00:00:02.110 --> 00:00:03.200\n<i>Adiós</i>\n\nfirst\n00:00:00.120 --> 00:00:01.200\nHola<img src=x onerror=bad>\n", true
        ).unwrap();
        for cue in &mut original {
            cue.segment.text = markup(&cue.segment.text);
        }
        let mut draft = Draft {
            id: "1-0".into(),
            revision: 1,
            name: "Fixture.vtt".into(),
            selection: Selection {
                input: Input {
                    library_id: "a".repeat(64),
                    track_id: "fixture".into(),
                },
                pair: Pair::EsEn,
                source_hash: source_hash(&original).unwrap(),
            },
            original,
            translated: vec!["<i>Goodbye</i>".into(), "Hi".into()],
            contextual_blocks: 0,
            edited: BTreeSet::new(),
        };
        edit(
            &mut draft,
            1,
            vec![Edit {
                index: 1,
                text: "<b>Hello</b><img src=x onerror=bad>".into(),
            }],
        )
        .unwrap();
        let mut options = Export {
            draft_id: draft.id.clone(),
            revision: 1,
            format: Format::Vtt,
            bilingual: true,
        };
        assert!(export(&draft, &options, &root)
            .unwrap_err()
            .contains("changed"));
        options.revision = 2;
        let (file, reference) = export(&draft, &options, &root).unwrap();
        assert_eq!(fs::read_to_string(file).unwrap(),
            "WEBVTT\n\nsecond\n00:00:02.110 --> 00:00:03.200\n<i>Adiós</i>\n<i>Goodbye</i>\n\nfirst\n00:00:00.120 --> 00:00:01.200\nHola\n<b>Hello</b>\n\n");
        assert_eq!(reference.revision, 2);
        fs::remove_dir_all(root).unwrap();
    }
}
