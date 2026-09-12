//! Live fast-flag allowlist fetched from Roblox's client-settings API.
//!
//! `https://clientsettingscdn.<domain>/v2/settings/application/<app>`
//! returns every flag the client currently honours. Red Strap uses it to
//! warn about flags Roblox will ignore (and to power "remove unknown").
//! The CDN host is tried first with a fallback to the origin host.

use std::collections::{BTreeSet, HashMap};

use serde::Deserialize;

use crate::consts::{ALLOWLIST_APP_PLAYER, ALLOWLIST_APP_STUDIO, DEFAULT_CHANNEL};
use crate::error::Result;

#[derive(Debug, Deserialize)]
struct ClientFlagSettings {
    #[serde(rename = "applicationSettings")]
    application_settings: Option<HashMap<String, String>>,
}

/// Fetch the allowlist for one application/channel pair.
pub async fn fetch_application_settings(
    client: &reqwest::Client,
    domain: &str,
    app: &str,
    channel: &str,
) -> Result<HashMap<String, String>> {
    let mut path = format!("/v2/settings/application/{app}");
    if !channel.eq_ignore_ascii_case(DEFAULT_CHANNEL) {
        path.push_str(&format!("/bucket/{}", channel.to_lowercase()));
    }

    let urls = [
        format!("https://clientsettingscdn.{domain}{path}"),
        format!("https://clientsettings.{domain}{path}"),
    ];

    let mut last_error = None;
    for url in urls {
        match try_fetch(client, &url).await {
            Ok(map) => return Ok(map),
            Err(e) => {
                tracing::warn!("allowlist fetch failed for {url}: {e}");
                last_error = Some(e);
            }
        }
    }

    Err(last_error.unwrap_or_else(|| {
        crate::error::Error::Other(String::from("allowlist request failed"))
    }))
}

async fn try_fetch(client: &reqwest::Client, url: &str) -> Result<HashMap<String, String>> {
    let settings: ClientFlagSettings = crate::http::get_json(client, url).await?;
    Ok(settings.application_settings.unwrap_or_default())
}

/// Fetch player + studio allowlists and merge them into one set.
pub async fn fetch_combined(
    client: &reqwest::Client,
    domain: &str,
    channel: &str,
) -> Result<BTreeSet<String>> {
    let mut out = BTreeSet::new();
    // Player list is required; Studio is best-effort (older channels may
    // not publish one, and validation must still work).
    for (key, value) in fetch_application_settings(client, domain, ALLOWLIST_APP_PLAYER, channel)
        .await?
    {
        let _ = value;
        out.insert(key);
    }
    match fetch_application_settings(client, domain, ALLOWLIST_APP_STUDIO, channel).await {
        Ok(map) => {
            out.extend(map.into_keys());
        }
        Err(e) => {
            tracing::info!("studio allowlist unavailable, continuing with player list: {e}");
        }
    }
    Ok(out)
}
