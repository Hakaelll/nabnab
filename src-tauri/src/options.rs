use crate::{
    model::Format,
    scheduling::{Priority, Schedule},
    store::Repository,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Component, Path, PathBuf},
};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SubtitleMode {
    #[default]
    None,
    Sidecar,
    Embed,
    Only,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Clip {
    pub start: f64,
    pub end: f64,
    pub precise: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct WorkOptions {
    pub format: Format,
    pub height: Option<u32>,
    pub audio_bitrate: Option<u16>,
    pub additional_audio: Option<Format>,
    pub subtitle_languages: Vec<String>,
    pub subtitle_mode: SubtitleMode,
    pub automatic_subtitles: bool,
    pub relative_folder: String,
    pub filename_template: String,
    pub collection: Option<String>,
    pub tags: Vec<String>,
    pub clip: Option<Clip>,
    pub priority: Priority,
    pub schedule: Schedule,
}
impl Default for WorkOptions {
    fn default() -> Self {
        Self {
            format: Format::Mp4,
            height: None,
            audio_bitrate: None,
            additional_audio: None,
            subtitle_languages: vec![],
            subtitle_mode: SubtitleMode::None,
            automatic_subtitles: false,
            relative_folder: String::new(),
            filename_template: "{titulo} [{id}]".into(),
            collection: None,
            tags: vec![],
            clip: None,
            priority: Priority::default(),
            schedule: Schedule::default(),
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Preset {
    pub id: String,
    pub name: String,
    pub revision: u32,
    pub options: WorkOptions,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedOptions {
    pub version: u32,
    pub preset_id: Option<String>,
    pub preset_revision: Option<u32>,
    pub values: WorkOptions,
    pub origins: BTreeMap<String, String>,
    #[serde(default)]
    pub context: Context,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Context {
    pub url: String,
    pub source_url: String,
    pub title: String,
    pub creator: String,
    pub list: String,
    pub content_id: String,
    pub platform: String,
    pub content_type: String,
    pub tags: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreparedDownload {
    pub url: String,
    pub title: String,
    pub content_id: String,
    pub options: ResolvedOptions,
    pub source_id: Option<String>,
    pub group_id: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Conditions {
    pub platform: String,
    pub source: String,
    pub content_type: String,
    pub title: String,
    pub tag: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Rule {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub conditions: Conditions,
    pub preset_id: Option<String>,
    pub relative_folder: Option<String>,
    pub collection: Option<String>,
    pub tags: Vec<String>,
}

pub fn content_identity(input: &str) -> Result<String, String> {
    let validated = crate::model::validate_url(input)?;
    let parsed = url::Url::parse(&validated).map_err(|e| e.to_string())?;
    let host = parsed.host_str().unwrap_or_default();
    if host == "youtu.be" || host.ends_with(".youtu.be") {
        if let Some(id) = parsed
            .path_segments()
            .and_then(|mut s| s.next())
            .filter(|s| !s.is_empty())
        {
            return Ok(format!("youtube:{id}"));
        }
    }
    if host == "youtube.com" || host.ends_with(".youtube.com") {
        if let Some((_, id)) = parsed.query_pairs().find(|(key, _)| key == "v") {
            return Ok(format!("youtube:{id}"));
        }
        let parts: Vec<_> = parsed.path_segments().into_iter().flatten().collect();
        if parts.len() > 1 && matches!(parts[0], "shorts" | "embed" | "live") {
            return Ok(format!("youtube:{}", parts[1]));
        }
    }
    use sha2::{Digest, Sha256};
    Ok(format!("url:{:x}", Sha256::digest(validated.as_bytes())))
}
pub fn display_url(input: &str) -> String {
    let Ok(url) = url::Url::parse(input) else {
        return input.chars().take(240).collect();
    };
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return format!("{}:", url.scheme());
    }
    let host = url.host_str().unwrap_or_default();
    let public_id = |id: &str| {
        !id.is_empty()
            && id.len() <= 256
            && id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"-_".contains(&byte))
    };
    if host == "youtube.com" || host.ends_with(".youtube.com") || host == "youtu.be" {
        if let Ok(identity) = content_identity(input) {
            if let Some(id) = identity.strip_prefix("youtube:").filter(|id| public_id(id)) {
                return format!("https://www.youtube.com/watch?v={id}");
            }
        }
        if url.path() == "/playlist" {
            if let Some((_, id)) = url
                .query_pairs()
                .find(|(key, id)| key == "list" && public_id(id))
            {
                return format!("https://www.youtube.com/playlist?list={id}");
            }
        }
        let parts: Vec<_> = url.path_segments().into_iter().flatten().collect();
        if (2..=3).contains(&parts.len())
            && matches!(parts[0], "channel" | "c" | "user")
            && public_id(parts[1])
            && (parts.len() == 2
                || matches!(parts[2], "videos" | "shorts" | "streams" | "featured"))
        {
            return format!("https://www.youtube.com{}", url.path());
        }
        if !parts.is_empty()
            && parts[0].starts_with('@')
            && parts.len() <= 2
            && parts[0]
                .chars()
                .all(|ch| ch.is_alphanumeric() || "@-_%.".contains(ch))
            && (parts.len() == 1
                || matches!(
                    parts[1],
                    "videos" | "shorts" | "playlists" | "streams" | "featured"
                ))
        {
            return format!("https://www.youtube.com{}", url.path());
        }
    }
    url.origin().ascii_serialization()
}

pub fn examples() -> Vec<Preset> {
    let video = WorkOptions {
        relative_folder: "{creador}".into(),
        ..WorkOptions::default()
    };
    vec![
        Preset {
            id: "compatible".into(),
            name: "Vídeo compatible".into(),
            revision: 1,
            options: video,
        },
        Preset {
            id: "audio".into(),
            name: "Audio".into(),
            revision: 1,
            options: WorkOptions {
                format: Format::AudioOriginal,
                relative_folder: "{plataforma}".into(),
                ..WorkOptions::default()
            },
        },
        Preset {
            id: "reference".into(),
            name: "Referencia".into(),
            revision: 1,
            options: WorkOptions {
                collection: Some("Referencias".into()),
                ..WorkOptions::default()
            },
        },
    ]
}

fn matches(rule: &Rule, context: &Context) -> bool {
    let c = &rule.conditions;
    rule.enabled
        && (c.platform.is_empty() || c.platform.eq_ignore_ascii_case(&context.platform))
        && (c.source.is_empty()
            || c.source == context.url
            || c.source == context.source_url
            || c.source == context.list)
        && (c.content_type.is_empty() || c.content_type == context.content_type)
        && (c.title.is_empty()
            || context
                .title
                .to_lowercase()
                .contains(&c.title.to_lowercase()))
        && (c.tag.is_empty()
            || context
                .tags
                .iter()
                .any(|tag| tag.eq_ignore_ascii_case(&c.tag)))
}
fn merge_tags(target: &mut Vec<String>, values: &[String]) {
    for tag in values {
        let tag = tag.trim();
        if !tag.is_empty() && !target.iter().any(|v| v.eq_ignore_ascii_case(tag)) {
            target.push(tag.into());
        }
    }
}
fn record_values(origins: &mut BTreeMap<String, String>, values: &WorkOptions, label: &str) {
    if let Ok(serde_json::Value::Object(fields)) = serde_json::to_value(values) {
        for field in fields.keys().filter(|field| field.as_str() != "tags") {
            origins.insert(field.clone(), label.into());
        }
    }
    record_tags(origins, &values.tags, label);
}
fn record_tags(origins: &mut BTreeMap<String, String>, tags: &[String], label: &str) {
    for tag in tags {
        origins
            .entry(format!("tag:{}", tag.to_lowercase()))
            .or_insert_with(|| label.into());
    }
}

pub fn resolve(
    repo: &Repository,
    context: &Context,
    preset_id: Option<&str>,
    manual: Option<&WorkOptions>,
) -> Result<ResolvedOptions, String> {
    let mut values = WorkOptions::default();
    let mut origins = BTreeMap::new();
    record_values(&mut origins, &values, "General");
    let mut selected = None;
    if let Some(rule) = repo.data.rules.iter().find(|r| matches(r, context)) {
        let label = format!("Regla: {}", rule.name);
        if let Some(id) = &rule.preset_id {
            let preset = repo
                .data
                .presets
                .iter()
                .find(|p| p.id == *id)
                .ok_or("The rule's preset no longer exists.")?;
            values = preset.options.clone();
            selected = Some(preset);
            record_values(&mut origins, &values, &label);
        }
        if let Some(folder) = &rule.relative_folder {
            values.relative_folder = folder.clone();
            origins.insert("relativeFolder".into(), label.clone());
        }
        if let Some(collection) = &rule.collection {
            values.collection = (!collection.is_empty()).then(|| collection.clone());
            origins.insert("collection".into(), label.clone());
        }
        merge_tags(&mut values.tags, &rule.tags);
        record_tags(&mut origins, &rule.tags, &label);
    }
    if let Some(id) = preset_id {
        let preset = repo
            .data
            .presets
            .iter()
            .find(|p| p.id == id)
            .ok_or("Choose an existing preset.")?;
        let tags = values.tags.clone();
        values = preset.options.clone();
        merge_tags(&mut values.tags, &tags);
        selected = Some(preset);
        record_values(
            &mut origins,
            &values,
            &format!("Ajuste: {} · v{}", preset.name, preset.revision),
        );
    }
    if let Some(manual) = manual {
        let tags = values.tags.clone();
        values = manual.clone();
        merge_tags(&mut values.tags, &tags);
        record_values(&mut origins, &values, "Cambios del elemento o lote");
    }
    merge_tags(&mut values.tags, &context.tags);
    record_tags(&mut origins, &context.tags, "Metadatos del elemento");
    let mut tag_origins = Vec::new();
    for tag in &values.tags {
        if let Some(origin) = origins.get(&format!("tag:{}", tag.to_lowercase())) {
            if !tag_origins.contains(origin) {
                tag_origins.push(origin.clone());
            }
        }
    }
    origins.insert("tags".into(), tag_origins.join(" + "));
    validate(&values)?;
    let mut context = context.clone();
    context.url = display_url(&context.url);
    context.source_url = display_url(&context.source_url);
    Ok(ResolvedOptions {
        version: 1,
        preset_id: selected.map(|p| p.id.clone()),
        preset_revision: selected.map(|p| p.revision),
        values,
        origins,
        context,
    })
}

const VARIABLES: &[&str] = &["plataforma", "creador", "lista", "fecha", "titulo", "id"];
fn validate_template(template: &str) -> Result<(), String> {
    if template.len() > 1024 || template.contains(['%', '\0']) {
        return Err("Use a template of at most 1024 characters with named variables, without engine expressions.".into());
    }
    let mut rest = template;
    while let Some((before, after)) = rest.split_once('{') {
        if before.contains('}') {
            return Err("Unbalanced template braces.".into());
        }
        let (key, suffix) = after.split_once('}').ok_or("Unbalanced template braces.")?;
        if !VARIABLES.contains(&key) {
            return Err(format!("Unknown template variable: {key}"));
        }
        rest = suffix;
    }
    if rest.contains('}') {
        return Err("Unbalanced template braces.".into());
    }
    Ok(())
}
pub fn validate(options: &WorkOptions) -> Result<(), String> {
    if options
        .height
        .is_some_and(|v| !options.format.is_video() || v == 0 || v > 16384)
    {
        return Err("Choose a valid video height.".into());
    }
    if options
        .audio_bitrate
        .is_some_and(|v| options.format != Format::Mp3 || !matches!(v, 128 | 192 | 256 | 320))
    {
        return Err("Choose a valid MP3 bitrate.".into());
    }
    if options.additional_audio.is_some_and(Format::is_video)
        || (options.additional_audio.is_some() && !options.format.is_video())
    {
        return Err("An additional audio output requires a retained video.".into());
    }
    if options.subtitle_mode == SubtitleMode::Only && options.additional_audio.is_some() {
        return Err("Subtitles-only work cannot retain an additional media output.".into());
    }
    if options.subtitle_mode != SubtitleMode::None && options.subtitle_languages.is_empty() {
        return Err("Choose at least one available subtitle language.".into());
    }
    if options.subtitle_languages.len() > 10
        || options
            .subtitle_languages
            .iter()
            .any(|s| !crate::model::valid_subtitle_language(s))
    {
        return Err("Choose valid subtitle language codes.".into());
    }
    if options.subtitle_mode == SubtitleMode::Embed && !options.format.is_video() {
        return Err("Only a video container can embed subtitles.".into());
    }
    if options.tags.len() > 50
        || options.tags.iter().any(|t| t.len() > 100)
        || options.collection.as_ref().is_some_and(|c| c.len() > 100)
    {
        return Err("Use at most 50 tags and labels of at most 100 characters.".into());
    }
    if options.clip.as_ref().is_some_and(|c| {
        !c.start.is_finite() || !c.end.is_finite() || c.start < 0.0 || c.end <= c.start
    }) {
        return Err("Choose a valid start and end time.".into());
    }
    validate_template(&options.relative_folder)?;
    validate_template(&options.filename_template)?;
    validate_relative(&options.relative_folder)?;
    if options.filename_template.trim().is_empty()
        || options.filename_template.contains(['/', '\\'])
    {
        return Err("The filename template must be one nonempty path segment.".into());
    }
    options.schedule.validate()?;
    Ok(())
}
fn validate_relative(relative: &str) -> Result<(), String> {
    if relative.starts_with(['/', '\\'])
        || relative.contains(':')
        || relative
            .split(['/', '\\'])
            .any(|part| part == ".." || part == ".")
        || Path::new(relative)
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
            && !relative.is_empty()
    {
        return Err("Choose a relative destination inside the authorized folder.".into());
    }
    Ok(())
}
pub fn safe_segment(value: &str) -> String {
    let mut output: String = value
        .chars()
        .map(|c| {
            if c.is_control() || "<>:\"/\\|?*%".contains(c) {
                '_'
            } else {
                c
            }
        })
        .take(120)
        .collect();
    output = output.trim().trim_end_matches(['.', ' ']).to_string();
    if output.is_empty()
        || matches!(
            output
                .split('.')
                .next()
                .unwrap_or_default()
                .to_ascii_uppercase()
                .as_str(),
            "CON"
                | "PRN"
                | "AUX"
                | "NUL"
                | "COM1"
                | "COM2"
                | "COM3"
                | "COM4"
                | "COM5"
                | "COM6"
                | "COM7"
                | "COM8"
                | "COM9"
                | "LPT1"
                | "LPT2"
                | "LPT3"
                | "LPT4"
                | "LPT5"
                | "LPT6"
                | "LPT7"
                | "LPT8"
                | "LPT9"
        )
    {
        output.insert(0, '_');
    }
    output
}
pub fn expand(template: &str, context: &Context) -> Result<String, String> {
    validate_template(template)?;
    let mut result = template.to_owned();
    let date = date_string(crate::queue::now());
    for (key, value) in [
        ("plataforma", &context.platform),
        ("creador", &context.creator),
        ("lista", &context.list),
        ("fecha", &date),
        ("titulo", &context.title),
        ("id", &context.content_id),
    ] {
        result = result.replace(&format!("{{{key}}}"), &safe_segment(value));
    }
    Ok(result)
}
pub fn preview_path(
    base: &Path,
    context: &Context,
    options: &WorkOptions,
) -> Result<PathBuf, String> {
    validate(options)?;
    let relative = expand(&options.relative_folder, context)?;
    validate_relative(&relative)?;
    let mut output = base.to_owned();
    for part in relative.split(['/', '\\']).filter(|s| !s.is_empty()) {
        output.push(safe_segment(part));
    }
    output.push(safe_segment(&expand(&options.filename_template, context)?));
    Ok(output)
}
pub fn create_destination(base: &Path, destination: &Path) -> Result<PathBuf, String> {
    std::fs::create_dir_all(base).map_err(|e| e.to_string())?;
    let root = crate::library::authorized_path(base)?;
    let relative = destination
        .strip_prefix(base)
        .map_err(|_| "The destination must stay in the authorized folder.")?;
    let mut current = root.clone();
    for part in relative.components() {
        let Component::Normal(part) = part else {
            return Err("Invalid relative destination.".into());
        };
        current.push(part);
        match std::fs::symlink_metadata(&current) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err("Destination links are not allowed.".into())
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                std::fs::create_dir(&current).map_err(|e| e.to_string())?
            }
            Err(e) => return Err(e.to_string()),
        }
        if !current
            .canonicalize()
            .map_err(|e| e.to_string())?
            .starts_with(&root)
        {
            return Err("The destination escapes the authorized folder.".into());
        }
    }
    Ok(current)
}

fn date_string(millis: u64) -> String {
    let days = (millis / 86_400_000) as i64 + 719468;
    let era = days / 146097;
    let day_of_era = days - era * 146097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36524 - day_of_era / 146096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_part = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_part + 2) / 5 + 1;
    let month = month_part + if month_part < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

#[tauri::command]
pub fn save_preset(
    state: tauri::State<std::sync::Arc<crate::AppState>>,
    mut preset: Preset,
) -> Result<Preset, String> {
    crate::queue::validate_request_id(&preset.id)?;
    if preset.name.trim().is_empty() || preset.name.len() > 120 {
        return Err("Name the preset using at most 120 characters.".into());
    }
    validate(&preset.options)?;
    let mut guard = state.repository.lock().unwrap();
    let repo = guard.as_mut().ok_or("Open the app first.")?;
    let previous = repo.data.presets.clone();
    preset.revision = repo
        .data
        .presets
        .iter()
        .find(|p| p.id == preset.id)
        .map_or(1, |p| p.revision.saturating_add(1));
    repo.data.presets.retain(|p| p.id != preset.id);
    repo.data.presets.push(preset.clone());
    if let Err(e) = repo.save() {
        repo.data.presets = previous;
        return Err(e);
    }
    Ok(preset)
}
#[tauri::command]
pub fn delete_preset(
    state: tauri::State<std::sync::Arc<crate::AppState>>,
    id: String,
) -> Result<(), String> {
    let mut guard = state.repository.lock().unwrap();
    let repo = guard.as_mut().ok_or("Open the app first.")?;
    if repo
        .data
        .rules
        .iter()
        .any(|rule| rule.preset_id.as_deref() == Some(&id))
    {
        return Err("Change organization rules that use this preset before deleting it.".into());
    }
    if repo
        .data
        .sources
        .iter()
        .any(|source| source.settings.preset_id.as_deref() == Some(&id))
        || repo
            .data
            .drafts
            .iter()
            .any(|draft| draft.preset_id.as_deref() == Some(&id))
    {
        return Err(
            "Choose another preset in sources and saved drafts before deleting this preset.".into(),
        );
    }
    let previous = repo.data.presets.clone();
    repo.data.presets.retain(|p| p.id != id);
    if let Err(e) = repo.save() {
        repo.data.presets = previous;
        return Err(e);
    }
    Ok(())
}
#[tauri::command]
pub fn save_rules(
    state: tauri::State<std::sync::Arc<crate::AppState>>,
    rules: Vec<Rule>,
) -> Result<Vec<Rule>, String> {
    let mut guard = state.repository.lock().unwrap();
    let repo = guard.as_mut().ok_or("Open the app first.")?;
    if rules.len() > 200 {
        return Err("Use at most 200 organization rules.".into());
    }
    for rule in &rules {
        crate::queue::validate_request_id(&rule.id)?;
        if rule.name.is_empty() || rule.name.len() > 120 {
            return Err("Name each rule using at most 120 characters.".into());
        }
        if rule
            .preset_id
            .as_ref()
            .is_some_and(|id| !repo.data.presets.iter().any(|p| p.id == *id))
        {
            return Err("Choose an existing preset for each rule.".into());
        }
        let options = WorkOptions {
            relative_folder: rule.relative_folder.clone().unwrap_or_default(),
            collection: rule.collection.clone(),
            tags: rule.tags.clone(),
            ..WorkOptions::default()
        };
        validate(&options)?;
    }
    let old = std::mem::replace(&mut repo.data.rules, rules.clone());
    if let Err(e) = repo.save() {
        repo.data.rules = old;
        return Err(e);
    }
    Ok(rules)
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RulePreview {
    pub path: PathBuf,
    pub missing: Vec<String>,
    pub collision: bool,
    pub matched: bool,
    pub resolved: ResolvedOptions,
}
#[tauri::command]
pub fn test_rule(
    state: tauri::State<std::sync::Arc<crate::AppState>>,
    context: Context,
    rule: Rule,
) -> Result<RulePreview, String> {
    let mut guard = state.repository.lock().unwrap();
    let repo = guard.as_mut().ok_or("Open the app first.")?;
    let previous = std::mem::replace(&mut repo.data.rules, vec![rule.clone()]);
    let result = resolve(repo, &context, None, None);
    repo.data.rules = previous;
    let resolved = result?;
    let path = preview_path(&repo.data.settings.folder, &context, &resolved.values)?;
    let template = format!(
        "{} {}",
        resolved.values.relative_folder, resolved.values.filename_template
    );
    let missing = [
        ("creador", &context.creator),
        ("lista", &context.list),
        ("titulo", &context.title),
        ("id", &context.content_id),
        ("plataforma", &context.platform),
    ]
    .into_iter()
    .filter(|(key, value)| value.is_empty() && template.contains(&format!("{{{key}}}")))
    .map(|(key, _)| key.into())
    .collect();
    let collision = path
        .parent()
        .and_then(|p| std::fs::read_dir(p).ok())
        .is_some_and(|entries| {
            entries
                .flatten()
                .any(|entry| entry.path().file_stem() == path.file_name())
        });
    Ok(RulePreview {
        path,
        missing,
        collision,
        matched: matches(&rule, &context),
        resolved,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn templates_keep_destinations_inside_authorized_root() {
        let context = Context {
            title: "../CON.txt".into(),
            creator: "../../escape".into(),
            content_id: "x".into(),
            ..Context::default()
        };
        let base = std::env::temp_dir();
        let options = WorkOptions {
            relative_folder: "Tutoriales/{creador}".into(),
            ..WorkOptions::default()
        };
        let path = preview_path(&base, &context, &options).unwrap();
        assert!(path.starts_with(&base));
        assert_eq!(path.parent().unwrap().file_name().unwrap(), ".._.._escape");
        for relative in [
            "../outside",
            "C:\\outside",
            "\\\\server\\share",
            "{unknown}",
            "%(title)s",
            "a/../b",
        ] {
            assert!(
                validate(&WorkOptions {
                    relative_folder: relative.into(),
                    ..WorkOptions::default()
                })
                .is_err(),
                "{relative}"
            );
        }
        assert_eq!(safe_segment("CON.txt"), "_CON.txt");
        assert_eq!(date_string(0), "1970-01-01");
        assert_eq!(date_string(1_799_020_800_000), "2027-01-04");
    }
    #[test]
    fn content_identity_uses_known_provider_parameters_only() {
        assert_eq!(
            content_identity("https://youtu.be/abc?t=20").unwrap(),
            "youtube:abc"
        );
        assert_eq!(
            content_identity("https://www.youtube.com/watch?v=abc&list=xyz").unwrap(),
            "youtube:abc"
        );
        assert_ne!(
            content_identity("https://media.example.org/a?id=1").unwrap(),
            content_identity("https://media.example.org/a?id=2").unwrap()
        );
    }
}
