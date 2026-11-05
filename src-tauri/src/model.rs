use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Media {
    pub request_id: u64,
    pub title: String,
    pub duration: Option<f64>,
    pub thumbnail: Option<String>,
    pub uploader: Option<String>,
    pub heights: Vec<u32>,
    pub has_audio: bool,
    pub has_video: bool,
    #[serde(default)]
    pub subtitle_languages: Vec<String>,
    #[serde(default)]
    pub entries: Vec<MediaEntry>,
    #[serde(default)]
    pub capabilities: crate::providers::Capabilities,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaEntry {
    pub id: String,
    pub title: String,
    pub url: String,
    pub thumbnail: Option<String>,
    pub duration: Option<f64>,
}

#[derive(Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    Mp4,
    Mp3,
    M4a,
    Opus,
    Source,
    #[serde(rename = "audio")]
    AudioOriginal,
}

impl Format {
    pub fn is_video(self) -> bool {
        matches!(self, Self::Mp4 | Self::Source)
    }
}

#[cfg(test)]
#[derive(Clone, PartialEq, Eq)]
pub struct DownloadKey {
    pub url: String,
    pub folder: std::path::PathBuf,
    pub format: Format,
    pub height: Option<u32>,
    pub audio_bitrate: u16,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub request_id: u64,
    pub phase: String,
    pub percent: Option<f64>,
    pub speed: Option<f64>,
    pub downloaded: Option<f64>,
    pub total: Option<f64>,
}

pub fn validate_url(input: &str) -> Result<String, String> {
    let parsed = url::Url::parse(input.trim())
        .map_err(|_| "Paste a complete http:// or https:// video URL.")?;
    if !matches!(parsed.scheme(), "https" | "http")
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return Err("Use a public HTTP or HTTPS video URL without credentials.".into());
    }
    let host = parsed.host_str().unwrap_or_default();
    let local_error = "Local or reserved network URLs are not supported.";
    match parsed.host() {
        Some(url::Host::Ipv4(ip)) if !crate::providers::is_public_ip(ip.into()) => {
            return Err(local_error.into());
        }
        Some(url::Host::Ipv6(ip)) if !crate::providers::is_public_ip(ip.into()) => {
            return Err(local_error.into());
        }
        Some(url::Host::Domain(_)) => {
            let host = host.trim_end_matches('.');
            if !host.contains('.')
                || [
                    "localhost",
                    "local",
                    "internal",
                    "lan",
                    "home",
                    "invalid",
                    "test",
                    "example",
                ]
                .iter()
                .any(|suffix| host == *suffix || host.ends_with(&format!(".{suffix}")))
            {
                return Err(local_error.into());
            }
        }
        _ => {}
    }
    Ok(parsed.to_string())
}

fn thumbnail(value: &Value) -> Option<String> {
    value
        .as_str()
        .filter(|s| s.starts_with("https://"))
        .and_then(|s| validate_url(s).ok())
}

fn duration(value: &Value) -> Option<f64> {
    value.as_f64().filter(|v| v.is_finite() && *v > 0.0)
}

pub fn valid_subtitle_language(language: &str) -> bool {
    if language.len() > 24 || matches!(language.to_ascii_lowercase().as_str(), "all" | "live_chat")
    {
        return false;
    }
    let mut parts = language.split(['-', '_']);
    let first = parts.next().unwrap_or_default();
    (2..=3).contains(&first.len())
        && first.bytes().all(|c| c.is_ascii_alphabetic())
        && parts.all(|part| !part.is_empty() && part.bytes().all(|c| c.is_ascii_alphanumeric()))
}

// ISO 639-2/T first, followed by B and 639-1 aliases; factual codes from the LOC table.
const ISO_639_2_CODES:&str="
aar,aa abk,ab ace ach ada ady afa afh afr,af ain aka,ak akk sqi,alb,sq ale alg alt amh,am ang anp
apa ara,ar arc arg,an hye,arm,hy arn arp art arw asm,as ast ath aus ava,av ave,ae awa aym,ay aze,az
bad bai bak,ba bal bam,bm ban eus,baq,eu bas bat bej bel,be bem ben,bn ber bho bih bik bin bis,bi
bla bnt bos,bs bra bre,br btk bua bug bul,bg mya,bur,my byn cad cai car cat,ca cau ceb cel cha,ch
chb che,ce chg zho,chi,zh chk chm chn cho chp chr chu,cu chv,cv chy cmc cnr cop cor,kw cos,co
cpe cpf cpp cre,cr crh crp csb cus ces,cze,cs dak dan,da dar day del den dgr din div,dv doi dra
dsb dua dum nld,dut,nl dyu dzo,dz efi egy eka elx eng,en enm epo,eo est,et ewe,ee ewo fan fao,fo fat
fij,fj fil fin,fi fiu fon fra,fre,fr frm fro frr frs fry,fy ful,ff fur gaa gay gba gem kat,geo,ka
deu,ger,de gez gil gla,gd gle,ga glg,gl glv,gv gmh goh gon gor got grb grc ell,gre,el grn,gn gsw
guj,gu gwi hai hat,ht hau,ha haw heb,he her,hz hil him hin,hi hit hmn hmo,ho hrv,hr hsb hun,hu hup
iba ibo,ig isl,ice,is ido,io iii,ii ijo iku,iu ile,ie ilo ina,ia inc ind,id ine inh ipk,ik ira iro ita,it
jav,jv jbo jpn,ja jpr jrb kaa kab kac kal,kl kam kan,kn kar kas,ks kau,kr kaw kaz,kk kbd kha khi
khm,km kho kik,ki kin,rw kir,ky kmb kok kom,kv kon,kg kor,ko kos kpe krc krl kro kru kua,kj kum kur,ku
kut lad lah lam lao,lo lat,la lav,lv lez lim,li lin,ln lit,lt lol loz ltz,lb lua lub,lu lug,lg lui lun
luo lus mkd,mac,mk mad mag mah,mh mai mak mal,ml man mri,mao,mi map mar,mr mas msa,may,ms mdf mdr men mga
mic min mkh mlg,mg mlt,mt mnc mni mno moh mon,mn mos mun mus mwl mwr myn myv nah nai nap nau,na nav,nv
nbl,nr nde,nd ndo,ng nds nep,ne new nia nic niu nno,nn nob,nb nog non nor,no nqo nso nub nwc nya,ny nym
nyn nyo nzi oci,oc oji,oj ori,or orm,om osa oss,os ota oto paa pag pal pam pan,pa pap pau peo fas,per,fa
phi phn pli,pi pol,pl pon por,pt pra pro pus,ps que,qu raj rap rar roa roh,rm rom ron,rum,ro run,rn rup
rus,ru sad sag,sg sah sai sal sam san,sa sas sat scn sco sel sem sga sgn shn sid sin,si sio sit sla
slk,slo,sk slv,sl sma sme,se smi smj smn smo,sm sms sna,sn snd,sd snk sog som,so son sot,st spa,es srd,sc
srn srp,sr srr ssa ssw,ss suk sun,su sus sux swa,sw swe,sv syc syr tah,ty tai tam,ta tat,tt tel,te tem
ter tet tgk,tg tgl,tl tha,th bod,tib,bo tig tir,ti tiv tkl tlh tli tmh tog ton,to tpi tsi tsn,tn tso,ts
tuk,tk tum tup tur,tr tut tvl twi,tw tyv udm uga uig,ug ukr,uk umb urd,ur uzb,uz vai ven,ve vie,vi vol,vo
vot wak wal war was cym,wel,cy wen wln,wa wol,wo xal xho,xh yao yap yid,yi yor,yo ypk zap zbl zen zgh
zha,za znd zul,zu zun zza
";

pub fn normalized_subtitle_language(language: &str) -> Option<&'static str> {
    if !valid_subtitle_language(language) {
        return None;
    }
    let primary = language.split(['-', '_']).next()?.to_ascii_lowercase();
    ISO_639_2_CODES.split_ascii_whitespace().find_map(|entry| {
        let mut codes = entry.split(',');
        let canonical = codes.next()?;
        (canonical == primary || codes.any(|alias| alias == primary)).then_some(canonical)
    })
}

pub fn parse_media(raw: &str, request_id: u64) -> Result<Media, String> {
    let value: Value = serde_json::from_str(raw)
        .map_err(|_| "The source returned unreadable video information.")?;
    if value["is_live"] == true || value["live_status"] == "is_upcoming" {
        return Err("Choose a finished video. Live and upcoming streams are not supported.".into());
    }
    if value["has_drm"] == true {
        return Err("Protected media cannot be downloaded by nabnab.".into());
    }
    let entries: Vec<MediaEntry> = value["entries"]
        .as_array()
        .into_iter()
        .flatten()
        .take(200)
        .filter(|entry| {
            entry["has_drm"] != true
                && entry["is_live"] != true
                && entry["live_status"] != "is_upcoming"
        })
        .filter_map(|entry| {
            let url = ["webpage_url", "original_url", "url"]
                .into_iter()
                .filter_map(|field| entry[field].as_str())
                .find_map(|url| validate_url(url).ok())?;
            Some(MediaEntry {
                id: entry["id"]
                    .as_str()
                    .unwrap_or_default()
                    .chars()
                    .take(256)
                    .collect(),
                title: entry["title"]
                    .as_str()
                    .unwrap_or("Untitled item")
                    .chars()
                    .take(1024)
                    .collect(),
                url,
                thumbnail: thumbnail(&entry["thumbnail"]).or_else(|| {
                    entry["thumbnails"].as_array().and_then(|images| {
                        images
                            .iter()
                            .rev()
                            .find_map(|image| thumbnail(&image["url"]))
                    })
                }),
                duration: duration(&entry["duration"]),
            })
        })
        .collect();
    let batch = value.get("entries").is_some() || value["_type"] == "playlist";
    if batch && entries.is_empty() {
        return Err("This list has no supported public items. Private, live and protected entries are excluded.".into());
    }
    let empty = Vec::new();
    let formats = value["formats"].as_array().unwrap_or(&empty);
    let mut heights = BTreeSet::new();
    let mut has_audio = false;
    let mut has_video = false;
    for f in formats {
        if f["has_drm"] == true {
            continue;
        }
        let video = f["vcodec"].as_str().map_or_else(
            || f["video_ext"].as_str().is_some_and(|ext| ext != "none"),
            |codec| codec != "none",
        );
        // Direct media links can omit codecs; let the engine probe their audio.
        let audio = f["acodec"].as_str().map_or_else(
            || video || f["audio_ext"].as_str().is_some_and(|ext| ext != "none"),
            |codec| codec != "none",
        );
        has_video |= video;
        has_audio |= audio;
        if video {
            if let Some(height) = f["height"].as_u64().filter(|h| *h > 0 && *h <= 16384) {
                heights.insert(height as u32);
            }
        }
    }
    if !batch && !has_audio && !has_video {
        return Err("This source has no supported public formats.".into());
    }
    let subtitle_languages: Vec<String> = if batch {
        Vec::new()
    } else {
        value["subtitles"]
            .as_object()
            .into_iter()
            .flatten()
            .filter(|(language, formats)| {
                normalized_subtitle_language(language).is_some()
                    && formats.as_array().is_some_and(|formats| {
                        formats
                            .iter()
                            .any(|f| f.is_object() && f["has_drm"] != true)
                    })
            })
            .map(|(language, _)| language.clone())
            .take(50)
            .collect()
    };
    let has_subtitles = !subtitle_languages.is_empty();
    Ok(Media {
        request_id,
        title: value["title"]
            .as_str()
            .unwrap_or("Untitled video")
            .chars()
            .take(1024)
            .collect(),
        duration: duration(&value["duration"]),
        thumbnail: thumbnail(&value["thumbnail"]),
        uploader: value["uploader"]
            .as_str()
            .map(|s| s.chars().take(512).collect()),
        heights: heights.into_iter().rev().collect(),
        has_audio,
        has_video,
        subtitle_languages,
        capabilities: crate::providers::Capabilities {
            video: has_video,
            audio: has_audio,
            batches: batch,
            subtitles: has_subtitles,
            images: false,
            verified: false,
        },
        entries,
    })
}

pub fn format_args(format: Format, height: Option<u32>) -> Vec<String> {
    match format {
        Format::Mp4 | Format::Source => {
            let filter = height.map(|h| format!("[height={h}]")).unwrap_or_default();
            let selection = if format == Format::Mp4 {
                format!("bv{filter}[vcodec^=avc1]+ba[acodec^=mp4a]/b{filter}[vcodec^=avc1][acodec^=mp4a]/bv{filter}+ba/b{filter}/bv{filter}")
            } else {
                format!("bv{filter}+ba/b{filter}/bv{filter}")
            };
            let mut args = vec![
                "-f".into(),
                selection,
                "--merge-output-format".into(),
                if format == Format::Mp4 { "mp4" } else { "mkv" }.into(),
            ];
            if format == Format::Mp4 {
                args.extend(["--remux-video".into(), "mp4".into()]);
            }
            args
        }
        Format::Mp3 => vec![
            "-f",
            "bestaudio/best",
            "-x",
            "--audio-format",
            "mp3",
            "--audio-quality",
            "192K",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect(),
        Format::M4a => vec![
            "-f",
            "bestaudio[acodec^=mp4a]/bestaudio[acodec=aac]/bestaudio/best",
            "-x",
            "--audio-format",
            "m4a",
            "--audio-quality",
            "0",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect(),
        Format::Opus => [
            "-f",
            "bestaudio[acodec=opus]/bestaudio/best",
            "-x",
            "--audio-format",
            "opus",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect(),
        Format::AudioOriginal => ["-f", "bestaudio/best", "-x", "--audio-format", "best"]
            .into_iter()
            .map(str::to_owned)
            .collect(),
    }
}

pub fn parse_progress(line: &str, request_id: u64) -> Option<Progress> {
    let raw = line.strip_prefix("__PROGRESS__")?;
    let p: Value = serde_json::from_str(raw).ok()?;
    let downloaded = p["downloaded_bytes"].as_f64();
    let total = p["total_bytes"]
        .as_f64()
        .or_else(|| p["total_bytes_estimate"].as_f64());
    let percent = downloaded
        .zip(total)
        .filter(|(_, t)| *t > 0.0)
        .map(|(d, t)| (100.0 * d / t).clamp(0.0, 100.0));
    Some(Progress {
        request_id,
        phase: "downloading".into(),
        percent,
        speed: p["speed"].as_f64(),
        downloaded,
        total,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn subtitle_language_tags_normalize_official_iso_aliases_without_guessing() {
        for (codes, expected) in [
            (&["en", "eng", "EN-us", "eng_GB"][..], "eng"),
            (&["es", "spa", "es-ES"][..], "spa"),
            (&["de", "deu", "ger"][..], "deu"),
            (&["fr", "fra", "fre"][..], "fra"),
            (&["zh-Hant", "zho", "chi"][..], "zho"),
            (&["fil"][..], "fil"),
            (&["frr"][..], "frr"),
        ] {
            for code in codes {
                assert_eq!(normalized_subtitle_language(code), Some(expected), "{code}");
            }
        }
        for code in [
            "und", "mul", "mis", "zxx", "xyz", "unknown", "English", "en,es", "", "qaa",
        ] {
            assert_eq!(normalized_subtitle_language(code), None, "{code}");
        }
        assert_ne!(
            normalized_subtitle_language("fil"),
            normalized_subtitle_language("fi")
        );
        assert_ne!(
            normalized_subtitle_language("frr"),
            normalized_subtitle_language("fr")
        );
    }
    #[test]
    fn subtitle_metadata_lists_only_available_manual_languages() {
        let media=parse_media(r#"{"formats":[{"vcodec":"h264","acodec":"aac"}],"subtitles":{"en":[{"ext":"vtt"}],"es":[{"ext":"srt"}],"en-US":[{"ext":"vtt"}],"all":[{"ext":"vtt"}],"live_chat":[{"ext":"json"}],"en.*":[{"ext":"vtt"}],"xyz":[{"ext":"vtt"}],"und":[{"ext":"vtt"}],"de":[],"fr":[{"ext":"vtt","has_drm":true}],"ja":[null]},"automatic_captions":{"pt":[{"ext":"vtt"}]}}"#,1).unwrap();
        assert_eq!(media.subtitle_languages, vec!["en", "en-US", "es"]);
        assert!(media.capabilities.subtitles);
        let automatic = parse_media(
            r#"{"formats":[{"vcodec":"h264"}],"automatic_captions":{"en":[{"ext":"vtt"}]}}"#,
            2,
        )
        .unwrap();
        assert!(automatic.subtitle_languages.is_empty());
        assert!(!automatic.capabilities.subtitles);
        for valid in ["en", "es", "eng", "en-US", "en-orig", "zh-Hans", "es-419"] {
            assert!(valid_subtitle_language(valid), "{valid}");
        }
        assert!(valid_subtitle_language("en-ABCDEFGHIJKLMNOPQRSTU"));
        for invalid in [
            "en--US",
            "1n",
            "123",
            "en/US",
            "en es",
            "-en",
            "all",
            "live_chat",
            "en-ABCDEFGHIJKLMNOPQRSTUV",
        ] {
            assert!(!valid_subtitle_language(invalid), "{invalid}");
        }
    }
    #[test]
    fn subtitle_metadata_is_bounded_and_legacy_media_defaults_to_no_selection() {
        let subtitles: serde_json::Map<String, Value> = (0..75)
            .map(|i| (format!("en-{i:03}"), serde_json::json!([{"ext":"vtt"}])))
            .collect();
        let raw =
            serde_json::json!({"formats":[{"vcodec":"h264"}],"subtitles":subtitles}).to_string();
        let media = parse_media(&raw, 1).unwrap();
        assert_eq!(media.subtitle_languages.len(), 50);
        let mut legacy = serde_json::to_value(media).unwrap();
        legacy.as_object_mut().unwrap().remove("subtitleLanguages");
        assert!(serde_json::from_value::<Media>(legacy)
            .unwrap()
            .subtitle_languages
            .is_empty());
        let batch=parse_media(r#"{"entries":[{"url":"https://example.org/video"}],"subtitles":{"en":[{"ext":"vtt"}]}}"#,2).unwrap();
        assert!(batch.subtitle_languages.is_empty());
        assert!(!batch.capabilities.subtitles);
    }
    #[test]
    fn url_boundary() {
        assert!(validate_url("https://www.youtube.com/watch?v=abc&list=x").is_ok());
        for u in [
            "file:///secret",
            "--exec=bad",
            "https://user:pass@host.com",
            "http://localhost/x",
            "http://192.168.1.1/a",
            "http://[::1]/a",
            "http://[::ffff:127.0.0.1]/x",
            "http://2130706433/x",
            "http://0x7f000001/x",
            "http://0177.0.0.1/x",
            "http://100.64.0.1/x",
            "http://224.0.0.1/x",
            "http://server/x",
            "https://files.internal/x",
            "https://printer.local./x",
        ] {
            assert!(validate_url(u).is_err(), "{u}");
        }
    }
    #[test]
    fn metadata_only_lists_real_non_drm_heights() {
        let m = parse_media(r#"{"title":"Test","formats":[{"vcodec":"avc1","height":720,"acodec":"none"},{"vcodec":"vp9","height":720},{"vcodec":"av1","height":2160,"has_drm":true},{"vcodec":"none","acodec":"aac"}]}"#, 7).unwrap();
        assert_eq!(m.heights, vec![720]);
        assert!(m.has_audio && m.has_video);
        assert!(parse_media(r#"{"entries":[]}"#, 1).is_err());
        assert!(parse_media(r#"{"is_live":true}"#, 1).is_err());
    }
    #[test]
    fn direct_media_without_codec_metadata_remains_usable() {
        let media = parse_media(r#"{"title":"Direct file","formats":[{"vcodec":null,"ext":"mp4","video_ext":"mp4","audio_ext":"none"}]}"#, 1).unwrap();
        assert!(media.has_video && media.has_audio);
        assert!(media.heights.is_empty());
        assert_eq!(media.duration, None);
    }
    #[test]
    fn progress_handles_unknown_size_and_clamps() {
        let p = parse_progress(
            r#"__PROGRESS__{"downloaded_bytes":50,"total_bytes":100,"speed":24}"#,
            2,
        )
        .unwrap();
        assert_eq!(p.percent, Some(50.0));
        assert_eq!(p.speed, Some(24.0));
        assert_eq!(
            parse_progress(r#"__PROGRESS__{"downloaded_bytes":50}"#, 1)
                .unwrap()
                .percent,
            None
        );
        assert_eq!(
            parse_progress(
                r#"__PROGRESS__{"downloaded_bytes":120,"total_bytes":100}"#,
                1
            )
            .unwrap()
            .percent,
            Some(100.0)
        );
        assert!(parse_progress("bad", 1).is_none());
    }
    #[test]
    fn quality_is_exact_and_audio_has_no_resolution() {
        assert!(format_args(Format::Mp4, Some(720))[1].contains("[height=720]"));
        assert!(!format_args(Format::M4a, None).join(" ").contains("height"));
        assert!(format_args(Format::Mp3, None).contains(&"mp3".to_owned()));
    }
    #[test]
    fn compatible_profiles_prefer_real_codecs_and_default_to_192k() {
        assert!(format_args(Format::Mp4, Some(1080))[1].contains("[vcodec^=avc1]"));
        assert!(format_args(Format::M4a, None)[1].contains("[acodec^=mp4a]"));
        assert!(format_args(Format::Mp3, None).contains(&"192K".to_owned()));
    }
    #[test]
    fn original_profiles_keep_codecs_and_do_not_force_mp4_or_opus() {
        let source = format_args(Format::Source, None);
        assert!(source.contains(&"mkv".into()));
        assert!(!source.contains(&"--remux-video".into()));
        assert!(!source.contains(&"mp4".into()));
        let audio = format_args(Format::AudioOriginal, None);
        assert_eq!(
            audio,
            vec!["-f", "bestaudio/best", "-x", "--audio-format", "best"]
        );
        assert_eq!(
            serde_json::to_string(&Format::AudioOriginal).unwrap(),
            "\"audio\""
        );
    }
    #[test]
    fn batch_metadata_excludes_private_live_and_protected_entries() {
        let media = parse_media(
            r#"{"_type":"playlist","title":"A list","entries":[
            {"id":"one","title":"First","url":"https://www.youtube.com/watch?v=one","duration":12},
            {"id":"two","title":"Bad","url":"file:///C:/secret"},
            {"id":"three","url":"http://127.0.0.1/secret"},
            {"id":"four","url":"https://example.org/a","has_drm":true},
            {"id":"five","url":"https://example.org/b","is_live":true},
            {"id":"six","url":"an-id-is-not-a-url"}
        ]}"#,
            9,
        )
        .unwrap();
        assert_eq!(media.entries.len(), 1);
        assert_eq!(media.entries[0].id, "one");
        assert_eq!(media.entries[0].duration, Some(12.0));
        assert!(media.capabilities.batches);
        assert!(!media.has_video && !media.has_audio);
        assert!(media.heights.is_empty());
    }
    #[test]
    fn batch_metadata_is_bounded_and_single_media_reports_observed_capabilities() {
        let entries: Vec<Value> = (0..250).map(|n| serde_json::json!({"id":n.to_string(),"url":format!("https://example.org/{n}")})).collect();
        let raw = serde_json::json!({"entries":entries}).to_string();
        assert_eq!(parse_media(&raw, 1).unwrap().entries.len(), 200);
        let media = parse_media(r#"{"formats":[{"vcodec":"none","acodec":"opus"}],"subtitles":{"en":[{"ext":"vtt"}]},"thumbnail":"https://127.0.0.1/secret"}"#, 2).unwrap();
        assert!(media.capabilities.audio && media.capabilities.subtitles);
        assert!(!media.capabilities.video && !media.capabilities.batches);
        assert_eq!(media.thumbnail, None);
        assert!(parse_media(r#"{"has_drm":true,"formats":[{"vcodec":"h264"}]}"#, 3).is_err());
        let persisted = serde_json::to_string(&media).unwrap();
        assert!(serde_json::from_str::<Media>(&persisted).is_ok());
    }
    #[test]
    fn recovery_keys_separate_sources_formats_destinations_and_qualities() {
        let key = DownloadKey {
            url: "https://example.org/a".into(),
            folder: "C:/Downloads".into(),
            format: Format::Mp3,
            height: None,
            audio_bitrate: 192,
        };
        let mut other = key.clone();
        assert!(key == other);
        other.audio_bitrate = 320;
        assert!(key != other);
        other = key.clone();
        other.url.push('b');
        assert!(key != other);
        other = key.clone();
        other.format = Format::M4a;
        assert!(key != other);
        other = key.clone();
        other.folder.push("other");
        assert!(key != other);
        other = key.clone();
        other.height = Some(720);
        assert!(key != other);
        let args = crate::engine::download_args(
            std::path::Path::new("C:/Tools with spaces"),
            Format::Mp3,
            None,
            std::path::Path::new("C:/Downloads"),
            &key.url,
        );
        assert!(args.contains(&"--continue".into()));
        assert!(!args.contains(&"--no-continue".into()));
    }
}
