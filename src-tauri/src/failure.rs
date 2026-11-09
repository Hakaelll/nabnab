use serde::Serialize;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Failure {
    pub code: &'static str,
    pub message: &'static str,
    pub technical: String,
    pub retryable: bool,
    pub retry_after: Option<u64>,
    pub phase: &'static str,
}

pub fn sanitize(raw: &str) -> String {
    raw.lines()
        .map(|line| {
            let lower = line.to_lowercase();
            if [
                "cookie",
                "authorization",
                "password",
                "token",
                "signature",
                "sig=",
            ]
            .iter()
            .any(|s| lower.contains(s))
            {
                return "[Sensitive diagnostic redacted]".to_owned();
            }
            line.split_whitespace()
                .map(|word| {
                    if word.contains("http://") || word.contains("https://") {
                        "[URL redacted]"
                    } else {
                        word
                    }
                })
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect::<Vec<_>>()
        .join("\n")
        .chars()
        .take(2400)
        .collect()
}

impl Failure {
    pub fn classify(raw: &str, phase: &'static str) -> Self {
        let text = raw.to_lowercase();
        let has = |items: &[&str]| items.iter().any(|s| text.contains(s));
        let (code, message, retryable) = if raw == crate::engine::CANCELLED {
            ("cancelled", "Download cancelled.", false)
        } else if raw.starts_with(crate::personal_servers::WEBDAV_INTERRUPTED) {
            ("webdav_interrupted", "The WebDAV copy stopped. Retry to continue or discard its partial file.", false)
        } else if raw.starts_with(crate::personal_servers::MOUNTED_INTERRUPTED) {
            ("mounted_interrupted", "Reconnect the mounted folder in Windows to continue the copy.", false)
        } else if raw == crate::personal_servers::MOUNTED_CHANGED {
            ("mounted_changed", "The mounted file changed. Refresh the folder and select the new version.", false)
        } else if raw == crate::personal_servers::WEBDAV_CHANGED {
            ("webdav_changed", "The WebDAV file changed. Refresh the folder and select the new version.", false)
        } else if raw == crate::personal_servers::WEBDAV_INVALID_RANGE {
            ("webdav_range", "The WebDAV server returned an incompatible byte range. Retry to start a fresh copy.", false)
        } else if raw == crate::personal_servers::SFTP_DISCONNECTED {
            ("sftp_disconnected", "The SFTP connection closed. Retry to copy from the beginning.", false)
        } else if raw == crate::personal_servers::SFTP_CHANGED {
            ("sftp_changed", "The SFTP file changed or could not be verified. Refresh the folder and review the file again.", false)
        } else if raw.starts_with("DUB_TIMING ") {
            ("dubbing_timing", "Some translated cues do not fit. Shorten the indicated cues and generate the voice again.", false)
        } else if raw == "DUB_NO_ORIGINAL" {
            ("dubbing_no_original", "This video has no original audio to mix. Choose translated voice only or select another video. The synchronized WAV is available.", false)
        } else if raw == "DUB_AUDIO_CHANGED" {
            ("dubbing_audio_changed", "The selected audio track changed. Review the video's tracks again. The synchronized WAV is available.", false)
        } else if raw == "DUB_DURATION" {
            ("dubbing_duration", "The synchronized WAV extends past the selected video. Choose the matching video. The synchronized WAV is available.", false)
        } else if text.starts_with("automation") && text.contains("budget") {
            ("budget_exceeded", "The approved automation budget was exceeded. Prepare a new plan with appropriate limits.", false)
        } else if text.starts_with("the rename operation stopped with partial results.") {
            ("partial_operation", "Some files could not be renamed. Open Review and rename → History to review, resume or undo the recorded operation.", false)
        } else if has(&["429", "too many requests", "rate_limited"]) {
            (
                "rate_limited",
                "The server is limiting requests. Wait before trying again.",
                true,
            )
        } else if has(&[
            "not a bot",
            "not bot",
            "sign in",
            "login required",
            "private video",
            "members-only",
            "age-restricted",
        ]) {
            ("authentication", "This source requires sign-in or session verification. Try a public video or try again later.", false)
        } else if has(&["403", "forbidden"]) {
            ("forbidden", "The source denied access (HTTP 403). Its formats or session requirements may have changed.", false)
        } else if has(&[
            "download tools are missing",
            "could not start",
            "no supported javascript runtime",
            "not found in path",
        ]) {
            (
                "dependency",
                "A required download tool is missing or cannot start. Repair the installation.",
                false,
            )
        } else if has(&["deno", "javascript", "challenge solver"]) {
            ("javascript", "The JavaScript challenge could not be solved. Check the engine version and runtime.", false)
        } else if has(&["requested format", "no downloadable formats"]) {
            (
                "format_unavailable",
                "This quality is no longer available. Reload the link or choose another quality.",
                false,
            )
        } else if has(&[
            "selected subtitles",
            "requested subtitle",
            "subtitle track",
            "embedded subtitle",
        ]) {
            ("subtitles", "The selected subtitles could not be embedded. Choose another manual language or download without subtitles. The media was kept for recovery.", false)
        } else if has(&[
            "ffmpeg",
            "ffprobe",
            "postprocess",
            "conversion failed",
            "generated codecs",
        ]) {
            (
                "processing",
                "The media could not be processed or verified. Recoverable files were kept.",
                false,
            )
        } else if has(&[
            "disk full",
            "no space left",
            "not enough space",
            "winerror 112",
        ]) {
            (
                "disk_full",
                "Your disk is full. Free space or choose another folder.",
                false,
            )
        } else if has(&["permission denied", "access is denied", "winerror 5"]) {
            (
                "permission",
                "The destination is not writable. Choose another folder.",
                false,
            )
        } else if has(&["this file already exists"]) {
            ("file_exists", "This file already exists. Rename it or choose another folder. The downloaded file was kept.", false)
        } else if has(&[
            "timed out",
            "timeout",
            "connection",
            "network",
            "name resolution",
            "http error 5",
        ]) {
            (
                "network",
                "Could not reach the source. Check your connection and try again.",
                true,
            )
        } else if has(&["404", "not available", "unavailable", "removed"]) {
            (
                "unavailable",
                "This video is unavailable. Check the link or try another video.",
                false,
            )
        } else {
            (
                "extraction",
                "The operation failed. Open the technical details for recovery information.",
                false,
            )
        };
        let retry_after = if code == "rate_limited" {
            let exposed = raw.lines().find_map(|line| {
                let offset = line.to_ascii_lowercase().find("retry-after")?;
                let value = line[offset + "retry-after".len()..]
                    .trim_start_matches([':', ' ', '='])
                    .trim();
                value
                    .split_whitespace()
                    .next()
                    .and_then(|n| n.parse::<u64>().ok())
                    .or_else(|| {
                        httpdate::parse_http_date(value)
                            .ok()
                            .and_then(|date| date.duration_since(std::time::SystemTime::now()).ok())
                            .map(|wait| wait.as_secs() + 1)
                    })
            });
            Some(exposed.unwrap_or(120).max(60))
        } else {
            None
        };
        Self {
            code,
            message,
            technical: sanitize(raw),
            retryable,
            retry_after,
            phase,
        }
    }
}

impl From<String> for Failure {
    fn from(value: String) -> Self {
        Self::classify(&value, "operation")
    }
}
impl From<&str> for Failure {
    fn from(value: &str) -> Self {
        Self::classify(value, "operation")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn partial_rename_requires_review_instead_of_automatic_retry() {
        let failure = Failure::classify("The rename operation stopped with partial results. A file appeared at the destination.", "processing");
        assert_eq!(failure.code, "partial_operation");
        assert!(!failure.retryable);
        assert!(failure.message.contains("History"));
    }
    #[test]
    fn classifies_causes_and_redacts_secrets() {
        for (raw, code) in [
            ("HTTP Error 429; Retry-After: 120", "rate_limited"),
            ("Sign in to confirm you're not a bot", "authentication"),
            ("HTTP Error 403: Forbidden", "forbidden"),
            ("Requested format is not available", "format_unavailable"),
            ("No supported JavaScript runtime", "dependency"),
            ("FFmpeg conversion failed", "processing"),
            ("Connection reset", "network"),
        ] {
            assert_eq!(Failure::classify(raw, "download").code, code);
        }
        assert_eq!(
            Failure::classify("HTTP Error 429\nRetry-After: 120", "download").retry_after,
            Some(120)
        );
        assert_eq!(
            Failure::classify("HTTP Error 429: Too Many Requests", "download").retry_after,
            Some(120)
        );
        assert_eq!(
            Failure::classify("HTTP Error 429\nRetry-After: 90", "download").retry_after,
            Some(90)
        );
        assert_eq!(
            Failure::classify("HTTP Error 429\nRetry-After: 5", "download").retry_after,
            Some(60)
        );
        let tomorrow = std::time::SystemTime::now() + std::time::Duration::from_secs(86400);
        let wait = Failure::classify(
            &format!(
                "HTTP Error 429\nRetry-After: {}",
                httpdate::fmt_http_date(tomorrow)
            ),
            "download",
        )
        .retry_after
        .unwrap();
        assert!((86390..=86401).contains(&wait));
        let safe = sanitize("ERROR https://host/video?secret=123\nCookie: private\nAuthorization: Bearer hidden\ntoken=secret\nconversion failed");
        assert!(
            !safe.contains("123")
                && !safe.contains("private")
                && !safe.contains("hidden")
                && !safe.contains("secret")
        );
        assert!(safe.contains("conversion failed"));
    }
}
