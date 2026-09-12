//! Roblox deployment metadata: mirrors, channels, and client versions.
//!
//! Launches start here: pick the fastest setup mirror, resolve the channel's
//! current `version-<hash>`, then hand off to the bootstrapper pipeline.

use serde::{Deserialize, Serialize};
use tokio::task::JoinSet;

use crate::consts::*;
use crate::error::{Error, Result};

/// Deploy information for one channel/binary pair.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientVersion {
    #[serde(rename = "version")]
    pub version: String,
    #[serde(rename = "clientVersionUpload")]
    pub version_guid: String,
    #[serde(rename = "bootstrapperVersion", default)]
    pub bootstrapper_version: String,
    /// `Last-Modified` of the package manifest, when requested.
    #[serde(default)]
    pub timestamp: Option<String>,
    /// True when this channel lags behind production.
    #[serde(default)]
    pub behind_default: bool,
}

/// True for `production` (and its `live` alias), case-insensitive.
pub fn is_default_channel(channel: &str) -> bool {
    channel.eq_ignore_ascii_case(DEFAULT_CHANNEL) || channel.eq_ignore_ascii_case(LIVE_CHANNEL_ALIAS)
}

/// Build a download URL for a deployment resource on the chosen mirror.
pub fn location(base_url: &str, channel: &str, resource: &str) -> String {
    let mut url = base_url.trim_end_matches('/').to_string();
    if !is_default_channel(channel) {
        url.push_str("/channel/common");
    }
    url.push_str(resource);
    url
}

/// Setup mirrors as (base URL, staggered start delay). The staggered start
/// prefers the primary mirror while still racing the others, so a slow
/// primary adds at most the stagger delay instead of a full timeout.
pub const BASE_URLS: &[(&str, u64)] = &[
    ("https://setup.rbxcdn.com", 0),
    ("https://setup-aws.rbxcdn.com", 2000),
    ("https://setup-ak.rbxcdn.com", 2000),
    ("https://roblox-setup.cachefly.net", 2000),
    ("https://s3.amazonaws.com/setup.roblox.com", 4000),
];

/// Race the setup mirrors and return the fastest healthy one.
///
/// Health is proven by fetching `versionStudio`, whose body is a well-known
/// constant. All losers are cancelled as soon as a winner emerges.
pub async fn initialize_connectivity(client: &reqwest::Client) -> Result<String> {
    let mut set = JoinSet::new();
    for (base, delay_ms) in BASE_URLS {
        let client = client.clone();
        let base = base.to_string();
        set.spawn(async move {
            if *delay_ms > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(*delay_ms)).await;
            }
            probe_mirror(&client, &base).await.map(|()| base)
        });
    }

    let mut last_error: Option<Error> = None;
    while let Some(outcome) = set.join_next().await {
        match outcome {
            Ok(Ok(base)) => {
                set.abort_all();
                tracing::info!("using setup mirror {base}");
                return Ok(base);
            }
            Ok(Err(e)) => {
                last_error = Some(e);
            }
            Err(e) => {
                // A join error here means the task was cancelled after we
                // already found a winner, or panicked (which cannot happen:
                // probe_mirror never panics). Record and continue regardless.
                tracing::debug!("mirror probe join outcome: {e}");
            }
        }
    }

    Err(last_error.unwrap_or_else(|| Error::Other(String::from("all setup mirrors failed"))))
}

async fn probe_mirror(client: &reqwest::Client, base: &str) -> Result<()> {
    let url = format!("{base}/versionStudio");
    let response = client.get(&url).send().await?.error_for_status()?;
    let body = response.text().await?;
    if body.trim() != VERSION_STUDIO_HASH {
        return Err(Error::Other(format!(
            "mirror {base} returned unexpected versionStudio body"
        )));
    }
    Ok(())
}

/// Resolve the current deploy for a channel/binary pair.
///
/// Falls back from the CDN host to the origin host on transport errors.
/// Non-default channels that answer 401/403/404 are reported as
/// [`Error::InvalidChannel`] instead of a generic network failure.
pub async fn get_info(
    client: &reqwest::Client,
    domain: &str,
    channel: &str,
    binary_type: &str,
    channel_token: &str,
    check_behind_default: bool,
) -> Result<ClientVersion> {
    let channel = channel.trim();
    let channel = if channel.is_empty() {
        DEFAULT_CHANNEL
    } else {
        channel
    };
    let default = is_default_channel(channel);

    let path = if default {
        format!("/v2/client-version/{binary_type}")
    } else {
        format!("/v2/client-version/{binary_type}/channel/{channel}")
    };

    let mut info = fetch_client_version(client, domain, &path, channel_token, channel, default).await?;

    if !default && check_behind_default {
        let live_path = format!("/v2/client-version/{binary_type}");
        match fetch_client_version(client, domain, &live_path, channel_token, channel, true).await {
            Ok(live) => {
                info.behind_default =
                    crate::version::compare(&info.version, &live.version)
                        == std::cmp::Ordering::Less;
            }
            Err(e) => {
                tracing::warn!("could not compare against production channel: {e}");
            }
        }
    }

    Ok(info)
}

async fn fetch_client_version(
    client: &reqwest::Client,
    domain: &str,
    path: &str,
    channel_token: &str,
    channel: &str,
    is_default: bool,
) -> Result<ClientVersion> {
    let urls = [
        format!("https://clientsettingscdn.{domain}{path}"),
        format!("https://clientsettings.{domain}{path}"),
    ];

    let mut last_error: Option<Error> = None;
    for url in urls {
        let mut request = client.get(&url);
        if !channel_token.is_empty() {
            request = request.header("Roblox-Channel-Token", channel_token);
        }
        match request.send().await {
            Ok(response) => {
                let status = response.status();
                if !is_default && matches!(status.as_u16(), 401 | 403 | 404) {
                    return Err(Error::InvalidChannel {
                        channel: channel.to_string(),
                        status: status.as_u16(),
                    });
                }
                let response = response.error_for_status()?;
                return response.json::<ClientVersion>().await.map_err(Error::Http);
            }
            Err(e) => {
                // Preserve invalid-channel signals raised by error_for_status
                // on later attempts is handled per-URL above; transport
                // errors fall through to the next host.
                tracing::warn!("clientsettings request failed for {url}: {e}");
                last_error = Some(Error::Http(e));
            }
        }
    }

    Err(last_error.unwrap_or_else(|| Error::Other(String::from("clientsettings request failed"))))
}

/// True when `channel` exists but is not publicly readable.
pub async fn is_channel_private(
    client: &reqwest::Client,
    domain: &str,
    channel: &str,
) -> Result<bool> {
    let probe = if channel.eq_ignore_ascii_case("production") {
        "live"
    } else {
        channel
    };
    let url = format!(
        "https://clientsettingscdn.{domain}/v2/client-version/{BINARY_TYPE_PLAYER}/channel/{probe}"
    );
    match client.get(&url).send().await {
        Ok(response) => {
            if response.status().is_success() {
                Ok(false)
            } else {
                Ok(matches!(response.status().as_u16(), 401 | 403 | 404))
            }
        }
        Err(e) => Err(Error::Http(e)),
    }
}

/// Best-effort `Last-Modified` timestamp of a version's package manifest.
pub async fn get_version_timestamp(
    client: &reqwest::Client,
    base_url: &str,
    version_guid: &str,
) -> Option<String> {
    let url = location(base_url, DEFAULT_CHANNEL, &format!("/{version_guid}-rbxPkgManifest.txt"));
    let response = client.get(&url).send().await.ok()?;
    let _ = response.error_for_status_ref().ok()?;
    response
        .headers()
        .get(reqwest::header::LAST_MODIFIED)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_matching() {
        assert!(is_default_channel("production"));
        assert!(is_default_channel("LIVE"));
        assert!(!is_default_channel("zekk"));
    }

    #[test]
    fn locations_are_built_correctly() {
        assert_eq!(
            location("https://setup.rbxcdn.com", "production", "/v-1-x.zip"),
            "https://setup.rbxcdn.com/v-1-x.zip"
        );
        assert_eq!(
            location("https://setup.rbxcdn.com/", "zekk", "/v-1-x.zip"),
            "https://setup.rbxcdn.com/channel/common/v-1-x.zip"
        );
    }

    #[test]
    fn client_version_deserializes() {
        let json = r#"{
            "version": "0.636.0.58101",
            "clientVersionUpload": "version-abc123",
            "bootstrapperVersion": "1, 6, 0, 58101"
        }"#;
        let info: ClientVersion = serde_json::from_str(json).expect("parse");
        assert_eq!(info.version_guid, "version-abc123");
        assert!(!info.behind_default);
    }
}
