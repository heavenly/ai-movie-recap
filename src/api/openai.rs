use crate::clip_plan::ClipPlanList;
use crate::config::Config;
use crate::{logi, logw};
use anyhow::{Context, Result};
use reqwest::Client;
use serde_json::json;

const MAX_SUB_CHARS: usize = 320_000;
const MAX_SCRIPT_CHARS: usize = 80_000;

fn sanitize_utf8_lossy(input: &str) -> String {
    String::from_utf8_lossy(input.as_bytes()).into_owned()
}

fn trim_copy_utf8_safe(input: &str, max_bytes: usize) -> String {
    if input.len() <= max_bytes {
        return input.to_string();
    }

    let mut cut = max_bytes.min(input.len());
    while cut > 0 && !input.is_char_boundary(cut) {
        cut -= 1;
    }
    input[..cut].to_string()
}

fn openai_extract_output_text(resp_json: &str) -> Option<String> {
    let root: serde_json::Value = serde_json::from_str(resp_json).ok()?;

    if let Some(err) = root.get("error") {
        if let Some(msg) = err.get("message").and_then(|v| v.as_str()) {
            logw(format!("OpenAI error message: {}", msg));
        }
        if let Some(typ) = err.get("type").and_then(|v| v.as_str()) {
            logw(format!("OpenAI error type: {}", typ));
        }
        if let Some(code) = err.get("code").and_then(|v| v.as_str()) {
            logw(format!("OpenAI error code: {}", code));
        }
        return None;
    }

    let output = root.get("output")?.as_array()?;
    for item in output {
        let content = item.get("content").and_then(|v| v.as_array());
        if let Some(content) = content {
            for entry in content {
                let typ = entry.get("type").and_then(|v| v.as_str());
                let text = entry.get("text").and_then(|v| v.as_str());
                if typ == Some("output_text") {
                    if let Some(text) = text {
                        return Some(text.to_string());
                    }
                }
            }
        }
    }

    None
}

fn openai_resp_should_retry_without_script(resp_json: &str) -> bool {
    if resp_json.is_empty() {
        return false;
    }

    let root: serde_json::Value = match serde_json::from_str(resp_json) {
        Ok(value) => value,
        Err(_) => return false,
    };

    let err = root.get("error");
    if err.is_none() {
        return false;
    }

    let mut yes = false;
    let err = err.unwrap();
    let msg = err.get("message").and_then(|v| v.as_str());
    let code = err.get("code").and_then(|v| v.as_str());

    if let Some(code) = code {
        let code_lower = code.to_lowercase();
        if code_lower == "context_length_exceeded"
            || code_lower == "invalid_json"
            || code_lower.contains("context")
        {
            yes = true;
        }
    }

    if let Some(msg) = msg {
        let msg_lower = msg.to_lowercase();
        let triggers = [
            "too large",
            "message is too long",
            "maximum context length",
            "context length",
            "reduce",
            "token",
            "request is too large",
            "unicode decode error",
            "invalid unicode",
            "invalid body",
        ];
        if triggers.iter().any(|t| msg_lower.contains(t)) {
            yes = true;
        }
    }

    yes
}

pub async fn openai_make_plan(
    client: &Client,
    cfg: &Config,
    movie_title: &str,
    subs_seconds_text: &str,
    optional_script_text: &str,
    num_clips: i32,
) -> Result<(ClipPlanList, bool)> {
    let title_utf8 = sanitize_utf8_lossy(movie_title);
    let subs_utf8 = sanitize_utf8_lossy(subs_seconds_text);
    let script_utf8 = sanitize_utf8_lossy(optional_script_text);

    let subs_trim = trim_copy_utf8_safe(&subs_utf8, MAX_SUB_CHARS);
    let script_trim = trim_copy_utf8_safe(&script_utf8, MAX_SCRIPT_CHARS);

    let prompt = format!(
        "You are given TWO inputs.\nMovie: {}\n\nINPUT A (Subtitles with timestamps in SECONDS):\n{}\n\nINPUT B (Optional script text WITHOUT timestamps; may be empty):\n{}\n\nTASK:\n- Choose {} non-overlapping time ranges that best cover the full plot arc.\n- ONLY use INPUT A for selecting start/end times (seconds). INPUT B is for story context.\n- Each time range should usually be 8-16 seconds long (end-start). Avoid >20 seconds.\n- Keep narrations punchy but not tiny: about 20-35 words total, in 3-5 short sentences.\n- Prefer ranges with clear visual action (reveals, confrontations, entrances, big moments).\n- Skip any range that starts at 0.\n- Return STRICT JSON with this shape ONLY:\n  {{\"clips\":[{{\"start\":120,\"end\":145,\"narration\":\"...\"}}, ...]}}\n- Clips must be increasing by start time.\n- Each narration must be at least 3 full sentences, casual commentator vibe.\n- The first narration must start with: \"Here we go, let's go over the movie {}.\".\n",
        title_utf8, subs_trim, script_trim, num_clips, title_utf8
    );

    let body = json!({
        "model": "gpt-5.2",
        "reasoning": {"effort": "high"},
        "input": [
            {"role": "system", "content": "You are a helpful assistant designed to output JSON."},
            {"role": "user", "content": prompt},
        ],
        "text": {"format": {"type": "json_object"}},
    });

    let has_script = !optional_script_text.is_empty();
    let timeout_s = if has_script { 14_400 } else { 3_600 };

    let resp = client
        .post("https://api.openai.com/v1/responses")
        .bearer_auth(&cfg.openai_key)
        .json(&body)
        .timeout(std::time::Duration::from_secs(timeout_s))
        .send()
        .await
        .context("OpenAI request failed")?;

    let status = resp.status();
    let raw = resp.text().await.unwrap_or_default();

    if !status.is_success() {
        logw(format!("OpenAI HTTP {}", status.as_u16()));
        if !raw.is_empty() {
            let snippet = raw.chars().take(800).collect::<String>();
            logw(format!("OpenAI raw body: {}", snippet));
        }

        let retry = has_script && openai_resp_should_retry_without_script(&raw);
        return Ok((ClipPlanList::default(), retry));
    }

    let out_text = openai_extract_output_text(&raw);
    if out_text.is_none() {
        logw("OpenAI response parse failed.".to_string());
        if !raw.is_empty() {
            let snippet = raw.chars().take(800).collect::<String>();
            logw(format!("OpenAI raw body: {}", snippet));
        }
        let retry = has_script && openai_resp_should_retry_without_script(&raw);
        return Ok((ClipPlanList::default(), retry));
    }

    let plan = ClipPlanList::from_json(&out_text.unwrap())?;
    logi(format!("OpenAI plan received: {} clips", plan.items.len()));
    Ok((plan, false))
}
