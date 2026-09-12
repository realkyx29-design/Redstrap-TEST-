//! Shared HTTP client construction and small download helpers.
//!
//! A single `reqwest::Client` is built at startup and shared everywhere so
//! connection pooling, TLS session reuse, and timeouts stay consistent.

use std::path::Path;

use serde::de::DeserializeOwned;

use crate::consts::{HTTP_CONNECT_TIMEOUT_SECS, HTTP_TIMEOUT_SECS};
use crate::error::{Error, Result};

/// Build the shared client: 60 s total timeout, 15 s connect timeout, and a
/// `RedStrap/<version>` user agent (plus build metadata when embedded).
pub fn build_client() -> Result<reqwest::Client> {
    let user_agent = format!("{}/{}", crate::consts::APP_NAME, env!("CARGO_PKG_VERSION"));
    reqwest::Client::builder()
        .user_agent(user_agent)
        .timeout(std::time::Duration::from_secs(HTTP_TIMEOUT_SECS))
        .connect_timeout(std::time::Duration::from_secs(HTTP_CONNECT_TIMEOUT_SECS))
        .build()
        .map_err(Error::Http)
}

/// GET a URL and deserialize the JSON body. HTTP error statuses fail.
pub async fn get_json<T: DeserializeOwned>(client: &reqwest::Client, url: &str) -> Result<T> {
    let response = client.get(url).send().await?.error_for_status()?;
    response.json::<T>().await.map_err(Error::Http)
}

/// GET a URL and return the raw bytes. HTTP error statuses fail.
pub async fn get_bytes(client: &reqwest::Client, url: &str) -> Result<Vec<u8>> {
    let response = client.get(url).send().await?.error_for_status()?;
    response.bytes().await.map(|b| b.to_vec()).map_err(Error::Http)
}

/// Stream a download to `dest` (via a `.part` sibling, renamed on success).
///
/// `on_chunk` is invoked with the cumulative byte count after every network
/// chunk so callers can update progress displays. There is no resume support
/// by design: Roblox packages are content-addressed by MD5, so completed
/// files are simply skipped and partial files are restarted, which keeps the
/// cache impossible to corrupt.
pub async fn download_to_file(
    client: &reqwest::Client,
    url: &str,
    dest: &Path,
    mut on_chunk: impl FnMut(u64),
) -> Result<u64> {
    if let Some(parent) = dest.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|e| Error::with_path(&parent.to_path_buf(), e))?;
        }
    }

    let part_name = format!(
        "{}.part",
        dest.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("download")
    );
    let part = dest.with_file_name(part_name);

    let mut response = client.get(url).send().await?.error_for_status()?;
    let mut file =
        std::fs::File::create(&part).map_err(|e| Error::with_path(&part.clone(), e))?;

    let mut total: u64 = 0;
    loop {
        match response.chunk().await? {
            Some(chunk) => {
                use std::io::Write;
                file.write_all(&chunk)
                    .map_err(|e| Error::with_path(&part.clone(), e))?;
                total += chunk.len() as u64;
                on_chunk(total);
            }
            None => break,
        }
    }

    file.sync_all()
        .map_err(|e| Error::with_path(&part.clone(), e))?;
    drop(file);
    std::fs::rename(&part, dest).map_err(|e| Error::with_path(&dest.to_path_buf(), e))?;
    Ok(total)
}
