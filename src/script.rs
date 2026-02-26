use anyhow::{Context, Result};
use once_cell::sync::OnceCell;
use regex::Regex;
use std::path::PathBuf;
use std::time::Duration;
use tokio::fs;
use tokio::io::AsyncWriteExt;
use tracing::warn;

const IMSDB_BASE: &str = "https://imsdb.com";
const USER_AGENT: &str =
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Safari/605.1.15";

pub struct ScriptDownloader {
    pub client: reqwest::Client,
}

impl ScriptDownloader {
    pub fn new() -> Result<Self> {
        let client = reqwest::Client::builder()
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

    pub async fn download_imsdb_script_ex(
        &self,
        movie_title: &str,
        dest_txt_path: PathBuf,
    ) -> Result<(bool, Option<String>)> {
        fs::create_dir_all("scripts").await.ok();
        fs::create_dir_all("scripts/srt_files").await.ok();
        if let Some(parent) = dest_txt_path.parent() {
            fs::create_dir_all(parent).await.ok();
        }

        let mut a = String::new();
        for ch in movie_title.chars() {
            if ch == ' ' {
                a.push('-');
            } else {
                a.push(ch);
            }
        }
        let b = strip_parens(&a);
        let c = imsdb_format_title_loose(movie_title);

        let a_lo = to_lower_copy(&a);
        let b_lo = to_lower_copy(&b);
        let c_lo = to_lower_copy(&c);

        let enc_title = url_encode_component(movie_title);

        let url0 = format!("{IMSDB_BASE}/scripts/{a}.html");
        let url1 = format!("{IMSDB_BASE}/scripts/{b}.html");
        let url2 = format!("{IMSDB_BASE}/scripts/{c}.html");
        let url3 = format!("{IMSDB_BASE}/scripts/{a_lo}.html");
        let url4 = format!("{IMSDB_BASE}/scripts/{b_lo}.html");
        let url5 = format!("{IMSDB_BASE}/scripts/{c_lo}.html");
        let url6 = format!("{IMSDB_BASE}/Movie%20Scripts/{enc_title}%20Script.html");

        let attempts = [url0, url1, url2, url3, url4, url5, url6];
        for attempt in attempts.iter() {
            if attempt.is_empty() {
                continue;
            }

            let (ok, why) = self
                .imsdb_fetch_script_to_file(attempt, &dest_txt_path)
                .await?;
            if ok {
                return Ok((true, Some(attempt.clone())));
            }

            warn!("IMSDb attempt failed ({why}): {attempt}");
        }

        Ok((false, None))
    }

    async fn imsdb_fetch_script_to_file(
        &self,
        url: &str,
        dest_txt_path: &PathBuf,
    ) -> Result<(bool, String)> {
        let resp = self.client.get(url).send().await?;
        if resp.status() != reqwest::StatusCode::OK {
            return Ok((false, format!("HTTP {}", resp.status())));
        }
        let page = resp.text().await?;
        if page.is_empty() {
            return Ok((false, "HTTP empty body".to_string()));
        }

        let (start, end) = match locate_script_region(&page) {
            Some(range) => range,
            None => {
                return Ok((false, "script block not found".to_string()));
            }
        };

        let raw = &page[start..end];
        let text = html_to_text_basic(raw);
        if text.len() < 1000 {
            return Ok((false, format!("extracted text too small ({})", text.len())));
        }

        let mut out = fs::File::create(dest_txt_path)
            .await
            .with_context(|| format!("write script: {}", dest_txt_path.display()))?;
        out.write_all(text.as_bytes()).await?;
        out.flush().await.ok();

        Ok((true, String::new()))
    }
}

fn strip_parens(input: &str) -> String {
    let mut out = String::new();
    for ch in input.chars() {
        if ch == '(' || ch == ')' {
            continue;
        }
        out.push(ch);
    }
    out
}

fn imsdb_format_title_loose(movie_title: &str) -> String {
    let mut out = String::new();
    for ch in movie_title.chars() {
        if ch == '(' || ch == ')' || ch == '\'' {
            continue;
        }
        let mut c = ch;
        if c == ' ' {
            c = '-';
        }
        if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
            out.push(c);
        }
    }
    out
}

fn to_lower_copy(input: &str) -> String {
    input.chars().map(|c| c.to_ascii_lowercase()).collect()
}

fn url_encode_component(input: &str) -> String {
    let mut out = String::new();
    for b in input.as_bytes() {
        let c = *b as char;
        if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
            out.push(c);
        } else if c == ' ' {
            out.push_str("%20");
        } else {
            out.push('%');
            out.push_str(&format!("{:02X}", b));
        }
    }
    out
}

fn html_to_text_basic(html: &str) -> String {
    let mut out = String::new();
    let bytes = html.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'<' {
            let mut j = i + 1;
            while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            if j + 1 < bytes.len()
                && bytes[j].to_ascii_lowercase() == b'b'
                && bytes[j + 1].to_ascii_lowercase() == b'r'
            {
                out.push('\n');
            }
            while i < bytes.len() && bytes[i] != b'>' {
                i += 1;
            }
            if i < bytes.len() {
                i += 1;
            }
            continue;
        }

        if bytes[i] == b'&' {
            let remainder = &html[i..];
            if remainder.starts_with("&nbsp;") {
                out.push(' ');
                i += 6;
                continue;
            }
            if remainder.starts_with("&amp;") {
                out.push('&');
                i += 5;
                continue;
            }
            if remainder.starts_with("&lt;") {
                out.push('<');
                i += 4;
                continue;
            }
            if remainder.starts_with("&gt;") {
                out.push('>');
                i += 4;
                continue;
            }
        }

        out.push(bytes[i] as char);
        i += 1;
    }

    if out.is_empty() {
        return String::new();
    }

    out
}

fn locate_script_region(page: &str) -> Option<(usize, usize)> {
    if let Some(start) = strcasestr_local(page, "<pre") {
        if let Some(gt) = page[start..].find('>') {
            let pre_start = start + gt + 1;
            if let Some(end_rel) = strcasestr_local(&page[pre_start..], "</pre>") {
                let pre_end = pre_start + end_rel;
                return Some((pre_start, pre_end));
            }
        }
    }

    let scr_idx = match find_class_scrtext(page) {
        Some(idx) => idx,
        None => return None,
    };
    let content_start = match page[scr_idx..].find('>') {
        Some(gt) => gt + scr_idx + 1,
        None => return None,
    };
    let td_end = strcasestr_local(&page[content_start..], "</td>");
    let div_end = strcasestr_local(&page[content_start..], "</div>");

    let best = match (td_end, div_end) {
        (Some(a), Some(b)) => Some(std::cmp::min(a, b)),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        _ => None,
    }?;

    let end = content_start + best;
    Some((content_start, end))
}

fn find_class_scrtext(page: &str) -> Option<usize> {
    let re = scrtext_regex().ok()?;
    re.find(page).map(|m| m.start())
}

fn scrtext_regex() -> Result<&'static Regex> {
    static SCR_RE: OnceCell<Regex> = OnceCell::new();
    SCR_RE.get_or_try_init(|| {
        Regex::new(r#"(?i)class\s*=\s*(['\"])scrtext\1"#)
            .context("failed to compile scrtext regex")
    })
}

fn strcasestr_local(haystack: &str, needle: &str) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    let h_bytes = haystack.as_bytes();
    let n_bytes = needle.as_bytes();

    for i in 0..h_bytes.len() {
        let mut h_idx = i;
        let mut n_idx = 0;
        while h_idx < h_bytes.len()
            && n_idx < n_bytes.len()
            && h_bytes[h_idx].to_ascii_lowercase() == n_bytes[n_idx].to_ascii_lowercase()
        {
            h_idx += 1;
            n_idx += 1;
        }
        if n_idx == n_bytes.len() {
            return Some(i);
        }
    }
    None
}
