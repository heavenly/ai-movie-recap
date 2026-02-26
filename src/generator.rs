use crate::api::{elevenlabs, openai};
use crate::config::Config;
use crate::ffmpeg;
use crate::{logi, logok, logw};
use anyhow::{Context, Result};
use rand::{Rng, SeedableRng};
use regex::Regex;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::fs;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use walkdir::WalkDir;
use zip::ZipArchive;

const MIN_NUM_CLIPS: i32 = 20;
const MAX_NUM_CLIPS: i32 = 30;
const MIN_TOTAL_DURATION: i32 = (2.5 * 60.0) as i32;
const MAX_TOTAL_DURATION: i32 = (4.5 * 60.0) as i32;

fn now_seed() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

async fn file_exists(path: &Path) -> bool {
    fs::metadata(path).await.map(|m| m.is_file()).unwrap_or(false)
}

async fn dir_exists(path: &Path) -> bool {
    fs::metadata(path).await.map(|m| m.is_dir()).unwrap_or(false)
}

async fn ensure_dir(path: &Path) -> Result<()> {
    if !dir_exists(path).await {
        fs::create_dir_all(path).await?;
    }
    Ok(())
}

async fn read_entire_file(path: &Path) -> Result<String> {
    Ok(fs::read_to_string(path).await?)
}

async fn write_entire_file(path: &Path, data: &[u8]) -> Result<()> {
    fs::write(path, data).await?;
    Ok(())
}

fn timestamp_to_seconds(ts: &str) -> Option<i32> {
    let parts: Vec<&str> = ts.split([':', ',']).collect();
    if parts.len() != 4 {
        return None;
    }
    let hh = parts[0].parse::<i32>().ok()?;
    let mm = parts[1].parse::<i32>().ok()?;
    let ss = parts[2].parse::<i32>().ok()?;
    Some(hh * 3600 + mm * 60 + ss)
}

async fn convert_srt_timestamps_to_seconds(input_srt: &Path, output_srt: &Path) -> Result<bool> {
    let input = fs::File::open(input_srt).await?;
    let mut output = fs::File::create(output_srt).await?;
    let mut reader = BufReader::new(input).lines();

    while let Some(line) = reader.next_line().await? {
        let mut line = line.replace("<i>", "").replace("</i>", "");
        let parts: Vec<&str> = line.split(" --> ").collect();
        if parts.len() == 2 && parts[0].contains(':') && parts[1].contains(':') {
            let s1 = timestamp_to_seconds(parts[0]);
            let s2 = timestamp_to_seconds(parts[1]);
            if let (Some(s1), Some(s2)) = (s1, s2) {
                line = format!("{} --> {}", s1, s2);
            }
        }

        output.write_all(line.as_bytes()).await?;
        output.write_all(b"\n").await?;
    }

    Ok(true)
}

fn parse_movie_title_slug(movie_title: &str) -> String {
    let mut out = String::new();
    for ch in movie_title.chars() {
        match ch {
            '\'' | '(' | ')' => continue,
            ' ' => out.push('-'),
            _ => out.push(ch.to_ascii_lowercase()),
        }
    }

    if out.ends_with("ii") {
        out.push_str("-2");
    }
    if out.ends_with("iii") {
        out.push_str("-3");
    }
    if out.ends_with("iv") {
        out.push_str("-4");
    }
    out
}

fn to_lower_copy(s: &str) -> String {
    s.chars().flat_map(|c| c.to_lowercase()).collect()
}

fn url_encode_component(input: &str) -> String {
    let mut out = String::new();
    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.' {
            out.push(ch);
        } else if ch == ' ' {
            out.push_str("%20");
        } else {
            let mut buf = [0u8; 4];
            let bytes = ch.encode_utf8(&mut buf).as_bytes();
            for b in bytes {
                out.push('%');
                out.push_str(&format!("{:02X}", b));
            }
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
            if j + 1 < bytes.len() && bytes[j].to_ascii_lowercase() == b'b' && bytes[j + 1].to_ascii_lowercase() == b'r' {
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
            let slice = &html[i..];
            if slice.starts_with("&nbsp;") {
                out.push(' ');
                i += 6;
                continue;
            }
            if slice.starts_with("&amp;") {
                out.push('&');
                i += 5;
                continue;
            }
            if slice.starts_with("&lt;") {
                out.push('<');
                i += 4;
                continue;
            }
            if slice.starts_with("&gt;") {
                out.push('>');
                i += 4;
                continue;
            }
        }

        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

async fn http_get_text(client: &reqwest::Client, url: &str) -> Result<(reqwest::StatusCode, String)> {
    let resp = client
        .get(url)
        .header(
            "User-Agent",
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Safari/605.1.15",
        )
        .header("Accept-Encoding", "")
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await?;

    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    Ok((status, text))
}

async fn download_subtitle_srt(client: &reqwest::Client, movie_title: &str, dest_srt_path: &Path) -> Result<bool> {
    ensure_dir(Path::new("scripts")).await?;
    ensure_dir(Path::new("scripts/srt_files")).await?;

    let slug = parse_movie_title_slug(movie_title);
    let list_url = format!("https://subf2m.co/subtitles/{}/english", slug);
    let (code, page) = http_get_text(client, &list_url).await?;
    if !code.is_success() || page.is_empty() {
        if !page.is_empty() {
            let snippet = page.chars().take(200).collect::<String>();
            logw(format!("subf2m list HTTP {} for {} (body starts: {})", code.as_u16(), list_url, snippet));
        }
        return Ok(false);
    }

    let want_prefix = format!("/subtitles/{}/english/", slug);
    let mut subpage_url = String::new();

    let href_re = Regex::new(r#"href=["']([^"']+)["']"#).unwrap();
    for cap in href_re.captures_iter(&page) {
        let href = &cap[1];
        if href.starts_with(&want_prefix) {
            if href.contains("english-german") {
                continue;
            }
            subpage_url = format!("https://subf2m.co{}", href);
            break;
        }
    }

    if subpage_url.is_empty() {
        let mut tried_profiles = 0;
        for cap in href_re.captures_iter(&page) {
            let href = &cap[1];
            if !href.starts_with("/u/") {
                continue;
            }
            let profile_url = format!("https://subf2m.co{}", href);
            let (pcode, prof) = http_get_text(client, &profile_url).await?;
            if !pcode.is_success() || prof.is_empty() {
                continue;
            }
            for cap2 in href_re.captures_iter(&prof) {
                let phref = &cap2[1];
                if phref.starts_with(&want_prefix) {
                    subpage_url = format!("https://subf2m.co{}", phref);
                    break;
                }
            }
            if !subpage_url.is_empty() {
                break;
            }
            tried_profiles += 1;
            if tried_profiles >= 12 {
                break;
            }
        }
    }

    if subpage_url.is_empty() {
        logw(format!("subf2m: couldn't locate subtitle detail page for {} (slug={})", movie_title, slug));
        return Ok(false);
    }

    let (scode, subpage) = http_get_text(client, &subpage_url).await?;
    if !scode.is_success() || subpage.is_empty() {
        logw(format!("subf2m: subtitle detail HTTP {} for {}", scode.as_u16(), subpage_url));
        return Ok(false);
    }

    let mut download_url = String::new();
    for cap in href_re.captures_iter(&subpage) {
        let href = &cap[1];
        if href.ends_with("download") {
            download_url = format!("https://subf2m.co{}", href);
            break;
        }
    }

    if download_url.is_empty() {
        logw(format!("subf2m: couldn't find download link on {}", subpage_url));
        return Ok(false);
    }

    let tmpzip = PathBuf::from(format!("scripts/srt_files/{}_tmp.zip", movie_title));
    let zip_bytes = client
        .get(&download_url)
        .header(
            "User-Agent",
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Safari/605.1.15",
        )
        .header("Cookie", "")
        .timeout(std::time::Duration::from_secs(120))
        .send()
        .await?
        .bytes()
        .await?;

    fs::write(&tmpzip, &zip_bytes).await?;

    let file = std::fs::File::open(&tmpzip)?;
    let mut archive = ZipArchive::new(file)?;
    let mut extracted: Option<Vec<u8>> = None;
    for i in 0..archive.len() {
        let mut f = archive.by_index(i)?;
        let name = f.name().to_ascii_lowercase();
        if name.ends_with(".srt") {
            let mut buf = Vec::new();
            std::io::copy(&mut f, &mut buf)?;
            extracted = Some(buf);
            break;
        }
    }

    let _ = fs::remove_file(&tmpzip).await;
    if let Some(data) = extracted {
        fs::write(dest_srt_path, data).await?;
        return Ok(file_exists(dest_srt_path).await);
    }

    Ok(false)
}

fn strip_parens(input: &str) -> String {
    input.chars().filter(|c| *c != '(' && *c != ')').collect()
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

async fn imsdb_fetch_script_to_file(
    client: &reqwest::Client,
    url: &str,
    dest_txt_path: &Path,
) -> Result<Option<String>> {
    let (code, page) = http_get_text(client, url).await?;
    if code.as_u16() != 200 || page.is_empty() {
        return Ok(Some(format!("HTTP {}", code.as_u16())));
    }

    let mut start = None;
    let mut end = None;
    let lower = page.to_ascii_lowercase();
    if let Some(pre_pos) = lower.find("<pre") {
        if let Some(gt) = page[pre_pos..].find('>') {
            let s = pre_pos + gt + 1;
            if let Some(pend) = lower[s..].find("</pre>") {
                start = Some(s);
                end = Some(s + pend);
            }
        }
    }

    if start.is_none() || end.is_none() {
        let scr_pos = lower.find("class=\"scrtext\"").or_else(|| lower.find("class='scrtext'"));
        if let Some(pos) = scr_pos {
            if let Some(gt) = page[pos..].find('>') {
                let s = pos + gt + 1;
                let tdend = lower[s..].find("</td>");
                let divend = lower[s..].find("</div>");
                let best = match (tdend, divend) {
                    (Some(a), Some(b)) => Some(std::cmp::min(a, b)),
                    (Some(a), None) => Some(a),
                    (None, Some(b)) => Some(b),
                    _ => None,
                };
                if let Some(b) = best {
                    start = Some(s);
                    end = Some(s + b);
                }
            }
        }
    }

    let (start, end) = match (start, end) {
        (Some(s), Some(e)) if e > s => (s, e),
        _ => return Ok(Some("script block not found".to_string())),
    };

    let txt = html_to_text_basic(&page[start..end]);
    if txt.len() < 1000 {
        return Ok(Some(format!("extracted text too small ({})", txt.len())));
    }

    write_entire_file(dest_txt_path, txt.as_bytes()).await?;
    Ok(None)
}

async fn download_imsdb_script_ex(
    client: &reqwest::Client,
    movie_title: &str,
    dest_txt_path: &Path,
) -> Result<Option<String>> {
    ensure_dir(Path::new("scripts")).await?;
    ensure_dir(Path::new("scripts/srt_files")).await?;

    let a = movie_title.replace(' ', "-");
    let b = strip_parens(&a);
    let c = imsdb_format_title_loose(movie_title);

    let a_lo = to_lower_copy(&a);
    let b_lo = to_lower_copy(&b);
    let c_lo = to_lower_copy(&c);
    let enc_title = url_encode_component(movie_title);

    let attempts = [
        format!("https://imsdb.com/scripts/{}.html", a),
        format!("https://imsdb.com/scripts/{}.html", b),
        format!("https://imsdb.com/scripts/{}.html", c),
        format!("https://imsdb.com/scripts/{}.html", a_lo),
        format!("https://imsdb.com/scripts/{}.html", b_lo),
        format!("https://imsdb.com/scripts/{}.html", c_lo),
        format!("https://imsdb.com/Movie%20Scripts/{}%20Script.html", enc_title),
    ];

    for attempt in attempts {
        if attempt.is_empty() {
            continue;
        }
        if let Some(why) = imsdb_fetch_script_to_file(client, &attempt, dest_txt_path).await? {
            logw(format!("IMSDb attempt failed ({}) : {}", why, attempt));
        } else {
            return Ok(Some(attempt));
        }
    }

    Ok(None)
}

async fn list_files_with_ext(dir: &Path, ext1: &str, ext2: &str) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    if !dir_exists(dir).await {
        return Ok(out);
    }

    let mut entries = fs::read_dir(dir).await?;
    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        if path.is_file() {
            if let Some(ext) = path.extension().and_then(OsStr::to_str) {
                let ext_lower = ext.to_ascii_lowercase();
                if ext_lower == ext1.trim_start_matches('.') || ext_lower == ext2.trim_start_matches('.') {
                    out.push(path);
                }
            }
        }
    }

    Ok(out)
}

async fn clear_directory_contents(dir_path: &Path) -> Result<bool> {
    if !dir_exists(dir_path).await {
        return Ok(true);
    }

    for entry in WalkDir::new(dir_path).min_depth(1).contents_first(true) {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            fs::remove_dir(path).await.ok();
        } else {
            fs::remove_file(path).await.ok();
        }
    }

    Ok(true)
}

async fn process_movie(cfg: &Config, client: &reqwest::Client, movie_path: &Path, movie_title: &str, num_clips: i32) -> Result<bool> {
    ensure_dir(Path::new("clips")).await?;
    ensure_dir(Path::new("clips/audio")).await?;
    ensure_dir(Path::new("output")).await?;
    ensure_dir(Path::new("tiktok_output")).await?;
    ensure_dir(Path::new("scripts")).await?;
    ensure_dir(Path::new("scripts/srt_files")).await?;
    ensure_dir(Path::new("movies_retired")).await?;

    let srt_in = PathBuf::from(format!("scripts/srt_files/{}.srt", movie_title));
    let srt_mod = PathBuf::from(format!("scripts/srt_files/{}_modified.srt", movie_title));
    let script_txt = PathBuf::from(format!("scripts/srt_files/{}_summary.txt", movie_title));

    if !file_exists(&srt_in).await {
        logi(format!("No SRT found for {}; attempting download...", movie_title));
        if !download_subtitle_srt(client, movie_title, &srt_in).await? {
            logw(format!("Subtitle download failed for {}. Place your SRT at: {}", movie_title, srt_in.display()));
            return Ok(false);
        }
        logok(format!("Downloaded SRT: {}", srt_in.display()));
    } else {
        logok(format!("Found SRT: {}", srt_in.display()));
    }

    if !file_exists(&srt_mod).await {
        logi(format!("Converting SRT timestamps -> seconds: {} -> {}", srt_in.display(), srt_mod.display()));
        if !convert_srt_timestamps_to_seconds(&srt_in, &srt_mod).await? {
            logw(format!("Failed to convert SRT for {}", movie_title));
            return Ok(false);
        }
        logok(format!("Converted subtitles (seconds): {}", srt_mod.display()));
    } else {
        logok(format!("Using cached converted subtitles: {}", srt_mod.display()));
    }

    if let Ok(meta) = fs::metadata(&script_txt).await {
        if meta.len() < 200 {
            logw(format!("IMSDb script file looks too small ({} bytes). Deleting to retry: {}", meta.len(), script_txt.display()));
            let _ = fs::remove_file(&script_txt).await;
        }
    }

    if file_exists(&script_txt).await {
        let size = fs::metadata(&script_txt).await.map(|m| m.len()).unwrap_or(0);
        logok(format!("Found cached IMSDb script: {} ({} bytes)", script_txt.display(), size));
    } else {
        logi(format!("Attempting IMSDb script scrape for {} (optional context)...", movie_title));
        if let Some(url) = download_imsdb_script_ex(client, movie_title, &script_txt).await? {
            let label = if url.is_empty() { "unknown" } else { &url };
            logok(format!("IMSDb script saved: {} (source: {})", script_txt.display(), label));
        } else {
            logw(format!("IMSDb scrape failed for {} (this is OK; continuing with subtitles-only).", movie_title));
        }
    }

    let subs_seconds = read_entire_file(&srt_mod).await.context("Failed to read converted subtitles")?;
    logok(format!("Loaded subtitles for planning: {} ({} bytes)", srt_mod.display(), subs_seconds.len()));

    let mut imsdb_script: Option<String> = None;
    if file_exists(&script_txt).await {
        if let Ok(text) = read_entire_file(&script_txt).await {
            if !text.is_empty() {
                logok(format!("Loaded IMSDb script for extra context: {} ({} bytes)", script_txt.display(), text.len()));
                imsdb_script = Some(text);
            } else {
                logw(format!("IMSDb script file existed but was empty/unreadable: {}", script_txt.display()));
            }
        }
    } else {
        logi("No IMSDb script available; using subtitles only.".to_string());
    }

    logi(format!("Requesting OpenAI clip plan ({} clips target)...", num_clips));
    let (mut plan, retry_no_script) = openai::openai_make_plan(
        client,
        cfg,
        movie_title,
        &subs_seconds,
        imsdb_script.as_deref().unwrap_or(""),
        num_clips,
    )
    .await?;

    if plan.items.is_empty() && retry_no_script && imsdb_script.is_some() {
        logw(format!("OpenAI request failed with IMSDb context; retrying without IMSDb script for {}", movie_title));
        let (retry_plan, _) = openai::openai_make_plan(
            client,
            cfg,
            movie_title,
            &subs_seconds,
            "",
            num_clips,
        )
        .await?;
        plan = retry_plan;
    }

    if plan.items.is_empty() {
        logw(format!("No plan returned for {}", movie_title));
        return Ok(false);
    }

    let concat_list_path = PathBuf::from(format!("clips/{}_concat_list.txt", movie_title));
    let mut listf = fs::File::create(&concat_list_path).await?;

    let mut made = 0usize;
    for (idx, clip) in plan.items.iter().enumerate() {
        let start_s = clip.start;
        let end_s = clip.end;
        let clip_index = idx + 1;
        if start_s <= 0 {
            logw(format!("Skipping clip {} (start<=0)", clip_index));
            continue;
        }
        if end_s <= start_s {
            logw(format!("Skipping clip {} (end<=start)", clip_index));
            continue;
        }

        let nar_mp3 = PathBuf::from(format!("clips/audio/{}_audio_{}.mp3", movie_title, clip_index));
        logi(format!("TTS clip {}/{} -> {}", clip_index, plan.items.len(), nar_mp3.display()));
        if !elevenlabs::elevenlabs_tts_to_mp3(client, cfg, &clip.narration, &nar_mp3).await? {
            logw(format!("TTS failed clip {} for {}", clip_index, movie_title));
            continue;
        }

        let nar_dur = match ffmpeg::ffprobe_duration_seconds(&nar_mp3).await {
            Ok(v) => v,
            Err(_) => {
                logw(format!("Bad narration duration for clip {}", clip_index));
                continue;
            }
        };

        let out_clip_name = format!("{}_clip_{}.mp4", movie_title, clip_index);
        let out_clip = PathBuf::from(format!("clips/{}", out_clip_name));
        logi(format!("Building clip {}: {} -> {} sec (narr={:.2}s) => {}", clip_index, start_s, end_s, nar_dur, out_clip.display()));
        if !ffmpeg::ffmpeg_make_adjusted_clip(movie_path, start_s, end_s, &nar_mp3, nar_dur, &out_clip).await? {
            logw(format!("Failed to build adjusted clip {}", clip_index));
            continue;
        }

        listf
            .write_all(format!("file '{}'\n", out_clip_name).as_bytes())
            .await?;
        made += 1;
        logok(format!("Built clip {} OK: {}", clip_index, out_clip.display()));
    }
    listf.flush().await?;

    if made == 0 {
        logw(format!("No clips produced for {}", movie_title));
        return Ok(false);
    }
    logok(format!("Clips produced: {} (concat list: {})", made, concat_list_path.display()));

    let tmp_concat = PathBuf::from(format!("clips/{}_concat_tmp.mp4", movie_title));
    logi(format!("Concatenating clips -> {}", tmp_concat.display()));
    if !ffmpeg::ffmpeg_concat_videos(&concat_list_path, &tmp_concat).await? {
        logw(format!("Concat failed for {}", movie_title));
        return Ok(false);
    }
    logok(format!("Concat OK: {}", tmp_concat.display()));

    let final_dur = match ffmpeg::ffprobe_duration_seconds(&tmp_concat).await {
        Ok(v) => v,
        Err(_) => {
            logw(format!("Bad final duration for {}", movie_title));
            return Ok(false);
        }
    };
    logok(format!("Final duration: {:.2} seconds", final_dur));

    let songs = list_files_with_ext(Path::new("backgroundmusic"), ".mp3", ".m4a").await?;
    if songs.is_empty() {
        logw("No backgroundmusic files found; output will be narration-only.".to_string());
        let out_final_only = PathBuf::from(format!("output/{}.mp4", movie_title));
        let _ = fs::rename(&tmp_concat, &out_final_only).await;
        logok(format!("Wrote output (no BGM): {}", out_final_only.display()));
    } else {
        let mut rng = rand::rngs::StdRng::seed_from_u64(now_seed());
        let bgm_list = PathBuf::from(format!("clips/{}_bgm_list.txt", movie_title));
        let mut bgml = fs::File::create(&bgm_list).await?;

        logi(format!("Building BGM track list ({} songs available)...", songs.len()));

        let mut covered = 0.0;
        let mut part = 0;
        while covered + 0.01 < final_dur {
            let idx = rng.gen_range(0..songs.len());
            let song = &songs[idx];
            let sd = match ffmpeg::ffprobe_duration_seconds(song).await {
                Ok(v) => v,
                Err(_) => continue,
            };
            if sd <= 60.0 {
                continue;
            }
            let start = 40.0;
            let avail = sd - start;
            if avail <= 1.0 {
                continue;
            }
            let need = final_dur - covered;
            let take = if avail < need { avail } else { need };

            let part_name = format!("{}_bgm_part_{}.m4a", movie_title, part + 1);
            let part_path = PathBuf::from(format!("clips/{}", part_name));

            if !ffmpeg::ffmpeg_trim_audio(song, start, take, &part_path).await? {
                continue;
            }
            bgml
                .write_all(format!("file '{}'\n", part_name).as_bytes())
                .await?;
            covered += take;
            part += 1;
            if part > 200 {
                break;
            }
        }
        bgml.flush().await?;

        logok(format!("BGM parts created: {} (covered {:.2}s / {:.2}s)", part, covered, final_dur));

        let bgm_out = PathBuf::from(format!("clips/{}_bgm.m4a", movie_title));
        logi(format!("Concatenating BGM -> {}", bgm_out.display()));
        if !ffmpeg::ffmpeg_concat_audio(&bgm_list, &bgm_out).await? {
            logw("BGM concat failed; output narration-only.".to_string());
            let out_final_only = PathBuf::from(format!("output/{}.mp4", movie_title));
            let _ = fs::rename(&tmp_concat, &out_final_only).await;
            logok(format!("Wrote output (no BGM): {}", out_final_only.display()));
        } else {
            logok(format!("BGM concat OK: {}", bgm_out.display()));
            let out_final_only = PathBuf::from(format!("output/{}.mp4", movie_title));
            logi(format!("Mixing narration + BGM -> {}", out_final_only.display()));
            if !ffmpeg::ffmpeg_mix_bgm(&tmp_concat, &bgm_out, &out_final_only).await? {
                logw("Mix failed; output narration-only.".to_string());
                let _ = fs::rename(&tmp_concat, &out_final_only).await;
            } else {
                let _ = fs::remove_file(&tmp_concat).await;
            }
            logok(format!("Wrote output: {}", out_final_only.display()));
        }
    }

    let out_final = PathBuf::from(format!("output/{}.mp4", movie_title));
    let out_vert = PathBuf::from(format!("tiktok_output/{}_vertical.mp4", movie_title));
    logi(format!("Rendering vertical -> {}", out_vert.display()));
    if !ffmpeg::ffmpeg_make_vertical(&out_final, &out_vert).await? {
        logw(format!("Vertical render failed for {}", movie_title));
    } else {
        logok(format!("Vertical render OK: {}", out_vert.display()));
    }

    let retired = PathBuf::from(format!("movies_retired/{}.mp4", movie_title));
    let _ = fs::rename(movie_path, &retired).await;
    logok(format!("Retired source movie -> {}", retired.display()));

    Ok(true)
}

fn output_already_exists(movie_title: &str) -> bool {
    let out = PathBuf::from(format!("output/{}.mp4", movie_title));
    out.exists()
}

fn strip_ext(filename: &str) -> String {
    Path::new(filename)
        .file_stem()
        .and_then(OsStr::to_str)
        .unwrap_or(filename)
        .to_string()
}

pub async fn run_generation() -> Result<i32> {
    let cfg = Config::load("config.json").await?;
    let client = reqwest::Client::builder()
        .cookie_store(true)
        .build()
        .context("Failed to build HTTP client")?;

    ensure_dir(Path::new("movies")).await?;
    ensure_dir(Path::new("output")).await?;
    ensure_dir(Path::new("backgroundmusic")).await?;
    ensure_dir(Path::new("clips")).await?;
    ensure_dir(Path::new("scripts")).await?;
    ensure_dir(Path::new("scripts/srt_files")).await?;
    ensure_dir(Path::new("tiktok_output")).await?;
    ensure_dir(Path::new("movies_retired")).await?;

    logi("Clearing clips/ folder...".to_string());
    if !clear_directory_contents(Path::new("clips")).await? {
        logw("Failed to fully clear clips/ (continuing anyway).".to_string());
    } else {
        logok("Cleared clips/ folder.".to_string());
    }

    ensure_dir(Path::new("clips")).await?;
    ensure_dir(Path::new("clips/audio")).await?;

    let mut rng = rand::rngs::StdRng::seed_from_u64(now_seed());
    let num_clips = rng.gen_range(MIN_NUM_CLIPS..=MAX_NUM_CLIPS);

    let mut processed = 0;
    let mut entries = fs::read_dir("movies").await?;
    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        if path.extension().and_then(OsStr::to_str).map(|s| s.eq_ignore_ascii_case("mp4")) != Some(true) {
            continue;
        }
        let title = strip_ext(entry.file_name().to_string_lossy().as_ref());
        if output_already_exists(&title) {
            logi(format!("Skipping {} (already in output/)", title));
            continue;
        }

        logi(format!("\n=== Processing: {} ===", title));
        if process_movie(&cfg, &client, &path, &title, num_clips).await? {
            processed += 1;
            logok(format!("DONE: {}", title));
        } else {
            logw(format!("FAILED: {}", title));
        }
    }

    logi(format!("\nAll done. Processed: {}", processed));
    Ok(processed)
}

#[allow(dead_code)]
fn validate_duration_range(duration: f64) -> bool {
    let duration = duration.round() as i32;
    duration >= MIN_TOTAL_DURATION && duration <= MAX_TOTAL_DURATION
}
