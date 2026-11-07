use serde::{Deserialize, Serialize};
use std::net::IpAddr;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProviderId {
    Youtube,
    Twitter,
    Instagram,
    Reddit,
    Tiktok,
    Vimeo,
    Twitch,
    Pinterest,
    Facebook,
    Bilibili,
    Dailymotion,
    Bluesky,
    Threads,
    Soundcloud,
    Bandcamp,
    Gallery,
    Generic,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    pub video: bool,
    pub audio: bool,
    pub images: bool,
    pub batches: bool,
    pub subtitles: bool,
    pub verified: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderInfo {
    pub id: ProviderId,
    pub name: String,
    pub host: String,
    pub capabilities: Capabilities,
}

pub fn course_page(input:&str)->bool{crate::courses::browser::is_course_page(input)||url::Url::parse(input).ok().is_some_and(|u|{let h=u.host_str().unwrap_or_default();(h=="udemy.com"||h.ends_with(".udemy.com"))&&u.path().starts_with("/course/")||(h=="hotmart.com"||h.ends_with(".hotmart.com")||h=="kiwify.com"||h.ends_with(".kiwify.com"))&&!u.path().contains("download")})}

pub fn identify(input: &str) -> Result<ProviderInfo, String> {
    let normalized = crate::model::validate_url(input)?;
    let parsed = url::Url::parse(&normalized).map_err(|_| "Invalid URL.")?;
    let host = parsed.host_str().unwrap_or_default().trim_end_matches('.');
    let matches = |domain: &str| host == domain || host.ends_with(&format!(".{domain}"));
    let (id, name) =
        if matches("youtube.com") || matches("youtube-nocookie.com") || host == "youtu.be" {
            (ProviderId::Youtube, "YouTube")
        } else if matches("x.com") || matches("twitter.com") {
            (ProviderId::Twitter, "X / Twitter")
        } else if matches("instagram.com") {
            (ProviderId::Instagram, "Instagram")
        } else if matches("reddit.com") || host == "redd.it" || host == "v.redd.it" {
            (ProviderId::Reddit, "Reddit")
        } else if matches("tiktok.com") {
            (ProviderId::Tiktok, "TikTok")
        } else if matches("vimeo.com") {
            (ProviderId::Vimeo, "Vimeo")
        } else if matches("twitch.tv") || host == "clips.twitch.tv" {
            (ProviderId::Twitch, "Twitch")
        } else if matches("pinterest.com") || host == "pin.it" {
            (ProviderId::Pinterest, "Pinterest")
        } else if matches("facebook.com") || host == "fb.watch" {
            (ProviderId::Facebook, "Facebook")
        } else if matches("bilibili.com") || host == "b23.tv" {
            (ProviderId::Bilibili, "Bilibili")
        } else if matches("dailymotion.com") || host == "dai.ly" {
            (ProviderId::Dailymotion, "Dailymotion")
        } else if matches("bsky.app") {
            (ProviderId::Bluesky, "Bluesky")
        } else if matches("threads.net") || matches("threads.com") {
            (ProviderId::Threads, "Threads")
        } else if matches("soundcloud.com") {
            (ProviderId::Soundcloud, "SoundCloud")
        } else if matches("bandcamp.com") {
            (ProviderId::Bandcamp, "Bandcamp")
        } else if crate::galleries::supported(input) {
            (ProviderId::Gallery, "Gallery")
        } else {
            (ProviderId::Generic, "HTTP")
        };
    Ok(ProviderInfo {
        id,
        name: name.into(),
        host: host.into(),
        capabilities: Capabilities {
            video: true,
            audio: true,
            batches: true,
            // Capabilities describe the engine, not a platform certification or this URL's formats.
            subtitles: true,
            images: false,
            verified: false,
        },
    })
}

pub fn scope(input: &str) -> Result<String, String> {
    let provider = identify(input)?;
    Ok(if provider.id == ProviderId::Generic {
        provider.host
    } else {
        format!("{:?}", provider.id)
    })
}

pub(crate) fn valid_scope_key(key: &str) -> bool {
    if key == "*" {
        return true;
    }
    if matches!(
        key,
        "Youtube"
            | "Twitter"
            | "Instagram"
            | "Reddit"
            | "Tiktok"
            | "Vimeo"
            | "Twitch"
            | "Pinterest"
            | "Facebook"
            | "Bilibili"
            | "Dailymotion"
            | "Bluesky"
            | "Threads"
            | "Soundcloud"
            | "Bandcamp"
            | "Gallery"
    ) {
        return true;
    }
    if key.is_empty()
        || key.len() > 253
        || !key
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b".-:[]".contains(&byte))
    {
        return false;
    }
    identify(&format!("https://{key}"))
        .is_ok_and(|provider| provider.id == ProviderId::Generic && provider.host == key)
}

pub(crate) fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let [a, b, c, _] = ip.octets();
            !(matches!(a, 0 | 10 | 127)
                || a >= 224
                || (a == 100 && (64..=127).contains(&b))
                || (a == 169 && b == 254)
                || (a == 172 && (16..=31).contains(&b))
                || (a == 192 && (b == 0 || b == 168 || (b == 88 && c == 99)))
                || (a == 198 && (b == 18 || b == 19 || (b == 51 && c == 100)))
                || (a == 203 && b == 0 && c == 113))
        }
        IpAddr::V6(ip) => {
            let segments = ip.segments();
            // Conservatively allow global unicast, excluding reserved/documentation and 6to4.
            (segments[0] & 0xe000) == 0x2000
                && !(segments[0] == 0x2001 && segments[1] <= 0x01ff)
                && !(segments[0] == 0x2001 && segments[1] == 0x0db8)
                && segments[0] != 0x2002
                && !(segments[0] == 0x3fff && segments[1] <= 0x0fff)
        }
    }
}

pub async fn validate_remote(input: &str) -> Result<String, String> {
    let normalized = crate::model::validate_url(input)?;
    let parsed = url::Url::parse(&normalized).map_err(|_| "Invalid URL.")?;
    let host = parsed.host_str().ok_or("The URL has no host.")?;
    let port = parsed
        .port_or_known_default()
        .ok_or("Unsupported URL scheme.")?;
    if matches!(parsed.host(), Some(url::Host::Domain(_))) {
        let addresses = tokio::time::timeout(
            std::time::Duration::from_secs(8),
            tokio::net::lookup_host((host, port)),
        )
        .await
        .map_err(|_| "The source hostname took too long to resolve. Check your connection.")?
        .map_err(|_| {
            "The source hostname could not be resolved. Check the link and your connection."
        })?;
        let mut found = false;
        for address in addresses {
            found = true;
            if !is_public_ip(address.ip()) {
                return Err("Local or reserved network destinations are not supported.".into());
            }
        }
        if !found {
            return Err("The source hostname has no usable network address.".into());
        }
    }
    // yt-dlp performs its own subsequent DNS/redirect requests. This initial check is not network isolation.
    Ok(normalized)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cooldown_scopes_share_provider_aliases_and_ignore_generic_ports() {
        for url in [
            "https://youtube.com/watch?v=1",
            "https://www.youtube.com/watch?v=2",
            "https://youtu.be/3",
            "https://youtube-nocookie.com/embed/4",
        ] {
            assert_eq!(scope(url).unwrap(), "Youtube");
        }
        assert_eq!(scope("https://x.com/a").unwrap(), "Twitter");
        assert_eq!(scope("https://twitter.com/b").unwrap(), "Twitter");
        assert_eq!(
            scope("https://CDN.example.org:8443/media").unwrap(),
            "cdn.example.org"
        );
        assert_eq!(
            scope("http://cdn.example.org:8080/other").unwrap(),
            "cdn.example.org"
        );
        assert_ne!(
            scope("https://cdn.example.org/media").unwrap(),
            scope("https://other.example.org/media").unwrap()
        );
        for key in ["Youtube", "Twitter", "cdn.example.org", "1.1.1.1", "*"] {
            assert!(valid_scope_key(key), "{key}");
        }
        for key in [
            "",
            "https://cdn.example.org",
            "cdn.example.org/path",
            "cdn.example.org?token=secret",
            "cdn.example.org:8443",
            "youtube.com",
            "127.0.0.1",
            "UPPER.example.org",
        ] {
            assert!(!valid_scope_key(key), "{key}");
        }
    }

    #[test]
    fn identifies_hosts_without_trusting_embedded_names() {
        for (url, expected) in [
            ("https://www.youtube.com/watch?v=123", ProviderId::Youtube),
            ("https://youtu.be/123", ProviderId::Youtube),
            ("https://mobile.x.com/a/status/1", ProviderId::Twitter),
            ("https://instagram.com/reel/1", ProviderId::Instagram),
            ("https://v.redd.it/a", ProviderId::Reddit),
            ("https://vm.tiktok.com/a", ProviderId::Tiktok),
            ("https://player.vimeo.com/video/1", ProviderId::Vimeo),
            ("https://clips.twitch.tv/a", ProviderId::Twitch),
            ("https://www.pinterest.com/pin/1", ProviderId::Pinterest),
            (
                "https://youtube.com.attacker.net/watch?v=1",
                ProviderId::Generic,
            ),
            ("https://attacker.net/youtube.com", ProviderId::Generic),
            ("https://pinterest.attacker.net/a", ProviderId::Generic),
            ("https://vxtwitter.com/a/status/1", ProviderId::Generic),
        ] {
            let provider = identify(url).unwrap();
            assert_eq!(provider.id, expected, "{url}");
            if expected == ProviderId::Generic {
                assert_eq!(provider.name, "HTTP");
            }
            assert!(!provider.capabilities.verified);
            assert!(!provider.capabilities.images);
        }
    }

    #[test]
    fn blocks_reserved_and_embedded_private_addresses() {
        for address in [
            "0.1.2.3",
            "10.0.0.1",
            "100.64.0.1",
            "127.0.0.1",
            "169.254.1.1",
            "172.31.0.1",
            "192.0.2.1",
            "192.168.0.1",
            "198.18.0.1",
            "198.51.100.1",
            "203.0.113.1",
            "224.0.0.1",
            "255.255.255.255",
            "::1",
            "::ffff:127.0.0.1",
            "fc00::1",
            "fe80::1",
            "2001:db8::1",
            "2002:7f00:1::",
            "3fff::1",
        ] {
            assert!(!is_public_ip(address.parse().unwrap()), "{address}");
        }
        for address in ["1.1.1.1", "8.8.8.8", "2606:4700:4700::1111"] {
            assert!(is_public_ip(address.parse().unwrap()), "{address}");
        }
    }
}
