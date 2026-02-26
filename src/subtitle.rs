use anyhow::{Context, Result};
use once_cell::sync::OnceCell;
use regex::Regex;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::fs;
use tokio::io::AsyncWriteExt;
use tracing::warn;

const SUBF2M_BASE: &str = "https://subf2m.co";
const USER_AGENT: &str =
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Safari/605.1.15";

pub struct SubtitleDownloader {
    pub client: reqwest::Client,
}

impl SubtitleDownloader {
    pub fn new() -> Result<Self> {
        let client = reqwest::Client::builder()
            .cookie_store(true)
            .user_agent(USER_AGENT)
            .timeout(Duration::from_secs(120))
            .connect_timeout(Duration::from_secs(30))
            .build()
            .context("failed to build reqwest client")?;
        Ok(Self { client })
    }

    pub fn with_client(client: reqwest::Client) -> Self {
        Self { client }
    }

    pub async fn download_subtitle_srt(
        &self,
        movie_title: &str,
        dest_srt_path: PathBuf,
    ) -> Result<bool> {
        fs::create_dir_all("scripts").await.ok();
        fs::create_dir_all("scripts/srt_files").await.ok();
        if let Some(parent) = dest_srt_path.parent() {
            fs::create_dir_all(parent).await.ok();
        }

        let slug = parse_movie_title_slug(movie_title);
        let list_url = format!("{SUBF2M_BASE}/subtitles/{}/english", slug);

        let list_page = match self.fetch_text(&list_url).await? {
            Some(body) => body,
            None => {
                warn!("subf2m list HTTP failure for {list_url}");
                return Ok(false);
            }
        };

        let want_subpage_prefix = format!("/subtitles/{}/english/", slug);
        let mut subpage_url = None;

        for href in extract_hrefs(&list_page)? {
            if href.starts_with(&want_subpage_prefix) {
                if href.contains("english-german") {
                    continue;
                }
                subpage_url = Some(format!("{SUBF2M_BASE}{href}"));
                break;
            }
        }

        if subpage_url.is_none() {
            let mut tried_profiles = 0;
            for href in extract_hrefs(&list_page)? {
                if !href.starts_with("/u/") {
                    continue;
                }

                let profile_url = format!("{SUBF2M_BASE}{href}");
                let profile_page = match self.fetch_text(&profile_url).await? {
                    Some(body) => body,
                    None => {
                        continue;
                    }
                };

                for phref in extract_hrefs(&profile_page)? {
                    if phref.starts_with(&want_subpage_prefix) {
                        subpage_url = Some(format!("{SUBF2M_BASE}{phref}"));
                        break;
                    }
                }

                if subpage_url.is_some() {
                    break;
                }

                tried_profiles += 1;
                if tried_profiles >= 12 {
                    break;
                }
            }
        }

        let Some(subpage_url) = subpage_url else {
            warn!("subf2m: couldn't locate subtitle detail page for {movie_title} (slug={slug})");
            return Ok(false);
        };

        let subpage = match self.fetch_text(&subpage_url).await? {
            Some(body) => body,
            None => {
                warn!("subf2m: subtitle detail HTTP failure for {subpage_url}");
                return Ok(false);
            }
        };

        let mut download_url = None;
        for href in extract_hrefs(&subpage)? {
            if str_ends_with(&href, "download") {
                download_url = Some(format!("{SUBF2M_BASE}{href}"));
                break;
            }
        }

        let Some(download_url) = download_url else {
            warn!("subf2m: couldn't find download link on {subpage_url}");
            return Ok(false);
        };

        let tmpzip_path = PathBuf::from("scripts/srt_files").join(format!("{movie_title}_tmp.zip"));
        let download_resp = self.client.get(&download_url).send().await?;
        if !download_resp.status().is_success() {
            warn!("subf2m: zip download HTTP {} for {}", download_resp.status(), download_url);
            return Ok(false);
        }

        let zip_bytes = download_resp.bytes().await?;
        let mut tmp_file = fs::File::create(&tmpzip_path)
            .await
            .with_context(|| format!("create temp zip: {}", tmpzip_path.display()))?;
        tmp_file.write_all(&zip_bytes).await?;
        tmp_file.flush().await.ok();

        let extracted = extract_srt_from_zip(&tmpzip_path, &dest_srt_path).await?;
        let _ = fs::remove_file(&tmpzip_path).await;

        if !extracted {
            return Ok(false);
        }

        Ok(fs::metadata(&dest_srt_path).await.is_ok())
    }

    async fn fetch_text(&self, url: &str) -> Result<Option<String>> {
        let resp = self.client.get(url).send().await?;
        let status = resp.status();
        if !status.is_success() {
            return Ok(None);
        }
        let text = resp.text().await?;
        if text.is_empty() {
            return Ok(None);
        }
        Ok(Some(text))
    }
}

fn parse_movie_title_slug(movie_title: &str) -> String {
    let mut out = String::new();
    for ch in movie_title.chars() {
        if ch == '\'' || ch == '(' || ch == ')' {
            continue;
        }
        if ch == ' ' {
            out.push('-');
        } else {
            out.push(ch.to_ascii_lowercase());
        }
    }

    let base = out.clone();
    let n = base.len();
    if n >= 2 && base.ends_with("ii") {
        out.push_str("-2");
    }
    if n >= 3 && base.ends_with("iii") {
        out.push_str("-3");
    }
    if n >= 2 && base.ends_with("iv") {
        out.push_str("-4");
    }

    out
}

fn str_ends_with(s: &str, suffix: &str) -> bool {
    s.ends_with(suffix)
}

fn extract_hrefs(html: &str) -> Result<Vec<String>> {
    let re = href_regex()?;
    let mut out = Vec::new();
    for cap in re.captures_iter(html) {
        if let Some(m) = cap.get(2) {
            out.push(m.as_str().to_string());
        }
    }
    Ok(out)
}

fn href_regex() -> Result<&'static Regex> {
    static HREF_RE: OnceCell<Regex> = OnceCell::new();
    HREF_RE.get_or_try_init(|| {
        Regex::new(r#"(?i)href\s*=\s*(['\"])(.*?)\1"#)
            .context("failed to compile href regex")
    })
}

async fn extract_srt_from_zip(tmpzip_path: &Path, dest_srt_path: &Path) -> Result<bool> {
    let tmpzip_path = tmpzip_path.to_owned();
    let dest_srt_path = dest_srt_path.to_owned();

    tokio::task::spawn_blocking(move || -> Result<bool> {
        let file = std::fs::File::open(&tmpzip_path)
            .with_context(|| format!("open zip: {}", tmpzip_path.display()))?;
        let mut archive = zip::ZipArchive::new(file).context("read zip archive")?;

        for i in 0..archive.len() {
            let mut entry = archive.by_index(i).context("read zip entry")?;
            if entry.is_dir() {
                continue;
            }

            let name = entry.name().to_string();
            if !name.to_ascii_lowercase().ends_with(".srt") {
                continue;
            }

            let mut out = std::fs::File::create(&dest_srt_path)
                .with_context(|| format!("create srt: {}", dest_srt_path.display()))?;
            std::io::copy(&mut entry, &mut out).context("extract srt")?;
            return Ok(true);
        }

        Ok(false)
    })
    .await?
}
