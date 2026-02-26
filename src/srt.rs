use anyhow::{Context, Result};
use tokio::fs;
use tokio::io::AsyncWriteExt;

fn timestamp_to_seconds(ts: &str) -> Option<i32> {
    let mut parts = ts.split([':', ',']);
    let hh: i32 = parts.next()?.parse().ok()?;
    let mm: i32 = parts.next()?.parse().ok()?;
    let ss: i32 = parts.next()?.parse().ok()?;
    let _ms: i32 = parts.next()?.parse().ok()?;
    Some(hh * 3600 + mm * 60 + ss)
}

pub async fn convert_srt_timestamps_to_seconds(input_srt: &str, output_srt: &str) -> Result<bool> {
    let mut input = fs::read_to_string(input_srt)
        .await
        .with_context(|| format!("read srt: {input_srt}"))?;

    while let Some(idx) = input.find("<i>") {
        input.replace_range(idx..idx + 3, "");
    }
    while let Some(idx) = input.find("</i>") {
        input.replace_range(idx..idx + 4, "");
    }

    let mut output = String::new();
    for line in input.split_inclusive('\n') {
        let trimmed = line.trim_end_matches(['\r', '\n']);
        let mut parts = trimmed.split_whitespace();
        if let (Some(a), Some(arrow), Some(b)) = (parts.next(), parts.next(), parts.next()) {
            if arrow == "-->" && a.contains(':') && b.contains(':') {
                if let (Some(s1), Some(s2)) = (timestamp_to_seconds(a), timestamp_to_seconds(b)) {
                    output.push_str(&format!("{} --> {}\n", s1, s2));
                    continue;
                }
            }
        }
        output.push_str(trimmed);
        if line.ends_with("\r\n") {
            output.push_str("\r\n");
        } else if line.ends_with('\n') {
            output.push('\n');
        }
    }

    let mut out = fs::File::create(output_srt)
        .await
        .with_context(|| format!("create srt output: {output_srt}"))?;
    out.write_all(output.as_bytes()).await?;
    out.flush().await.ok();
    Ok(true)
}
