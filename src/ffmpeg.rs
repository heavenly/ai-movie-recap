use crate::{logi, logw};
use anyhow::{Context, Result};
use std::path::Path;
use tokio::process::Command;

const MAX_VIDEO_SPEEDUP: f64 = 1.75;

async fn run_cmd(args: &[String]) -> Result<()> {
    if args.is_empty() {
        return Ok(());
    }

    let mut cmd = Command::new(&args[0]);
    if args.len() > 1 {
        cmd.args(&args[1..]);
    }

    let status = cmd.status().await.context("Command execution failed")?;
    if !status.success() {
        return Err(anyhow::anyhow!("Command failed: {:?}", args));
    }

    Ok(())
}

pub async fn ffprobe_video_dimensions(path: &Path) -> Result<(i32, i32)> {
    let output = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=width,height",
            "-of",
            "csv=s=x:p=0",
        ])
        .arg(path)
        .output()
        .await
        .context("ffprobe execution failed")?;

    if !output.status.success() {
        return Err(anyhow::anyhow!("ffprobe failed"));
    }

    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let mut parts = text.split('x');
    let w = parts
        .next()
        .and_then(|v| v.parse::<i32>().ok())
        .unwrap_or(0);
    let h = parts
        .next()
        .and_then(|v| v.parse::<i32>().ok())
        .unwrap_or(0);

    if w <= 0 || h <= 0 {
        return Err(anyhow::anyhow!("Invalid dimensions"));
    }

    Ok((w, h))
}

pub async fn ffprobe_duration_seconds(path: &Path) -> Result<f64> {
    let output = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
        ])
        .arg(path)
        .output()
        .await
        .context("ffprobe duration failed")?;

    if !output.status.success() {
        return Err(anyhow::anyhow!("ffprobe failed"));
    }

    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let duration = text.parse::<f64>().unwrap_or(-1.0);
    if duration <= 0.1 {
        return Err(anyhow::anyhow!("Invalid duration"));
    }
    Ok(duration)
}

pub async fn ffmpeg_make_adjusted_clip(
    input_mp4: &Path,
    start_s: i32,
    end_s: i32,
    narration_mp3: &Path,
    narration_dur: f64,
    out_mp4: &Path,
) -> Result<bool> {
    let orig_seg_dur = (end_s - start_s) as f64;
    if orig_seg_dur <= 0.1 || narration_dur <= 0.1 {
        return Ok(false);
    }

    let mut use_start = start_s;
    let mut use_end = end_s;
    let mut speed = orig_seg_dur / narration_dur;

    if speed > MAX_VIDEO_SPEEDUP {
        let mut desired_src_dur = narration_dur * MAX_VIDEO_SPEEDUP;
        if desired_src_dur > orig_seg_dur {
            desired_src_dur = orig_seg_dur;
        }
        if desired_src_dur < 1.0 {
            desired_src_dur = 1.0;
        }

        let center = (start_s as f64 + end_s as f64) / 2.0;
        let half = desired_src_dur / 2.0;
        let mut ns = center - half;
        let mut ne = center + half;

        if ns < start_s as f64 {
            ns = start_s as f64;
            ne = ns + desired_src_dur;
        }
        if ne > end_s as f64 {
            ne = end_s as f64;
            ns = ne - desired_src_dur;
        }
        if ns < start_s as f64 {
            ns = start_s as f64;
        }
        if ne > end_s as f64 {
            ne = end_s as f64;
        }

        use_start = ns.round() as i32;
        use_end = ne.round() as i32;
        if use_end <= use_start {
            use_end = use_start + 1;
        }

        let new_seg_dur = (use_end - use_start) as f64;
        speed = new_seg_dur / narration_dur;
        if speed > MAX_VIDEO_SPEEDUP {
            speed = MAX_VIDEO_SPEEDUP;
        }

        logi(format!(
            "Speed-cap applied: planned {}-{} ({:.2}s) vs narr {:.2}s => {:.2}x. Using {}-{} ({:.2}s) => {:.2}x.",
            start_s,
            end_s,
            orig_seg_dur,
            narration_dur,
            orig_seg_dur / narration_dur,
            use_start,
            use_end,
            (use_end - use_start) as f64,
            speed
        ));
    }

    if speed < 0.05 {
        speed = 0.05;
    }
    if speed > 20.0 {
        speed = 20.0;
    }

    let args = vec![
        "ffmpeg".to_string(),
        "-y".to_string(),
        "-hide_banner".to_string(),
        "-loglevel".to_string(),
        "error".to_string(),
        "-ss".to_string(),
        use_start.to_string(),
        "-to".to_string(),
        use_end.to_string(),
        "-i".to_string(),
        input_mp4.display().to_string(),
        "-i".to_string(),
        narration_mp3.display().to_string(),
        "-filter_complex".to_string(),
        format!("[0:v]setpts=PTS/{:.10}[v]", speed),
        "-map".to_string(),
        "[v]".to_string(),
        "-map".to_string(),
        "1:a".to_string(),
        "-c:v".to_string(),
        "libx264".to_string(),
        "-pix_fmt".to_string(),
        "yuv420p".to_string(),
        "-preset".to_string(),
        "veryfast".to_string(),
        "-crf".to_string(),
        "22".to_string(),
        "-c:a".to_string(),
        "aac".to_string(),
        "-b:a".to_string(),
        "192k".to_string(),
        "-shortest".to_string(),
        out_mp4.display().to_string(),
    ];

    run_cmd(&args).await?;
    Ok(out_mp4.exists())
}

pub async fn ffmpeg_concat_videos(list_txt: &Path, out_mp4: &Path) -> Result<bool> {
    let args = vec![
        "ffmpeg".to_string(),
        "-y".to_string(),
        "-hide_banner".to_string(),
        "-loglevel".to_string(),
        "error".to_string(),
        "-f".to_string(),
        "concat".to_string(),
        "-safe".to_string(),
        "0".to_string(),
        "-i".to_string(),
        list_txt.display().to_string(),
        "-c:v".to_string(),
        "libx264".to_string(),
        "-pix_fmt".to_string(),
        "yuv420p".to_string(),
        "-preset".to_string(),
        "veryfast".to_string(),
        "-crf".to_string(),
        "22".to_string(),
        "-c:a".to_string(),
        "aac".to_string(),
        "-b:a".to_string(),
        "192k".to_string(),
        "-movflags".to_string(),
        "+faststart".to_string(),
        out_mp4.display().to_string(),
    ];
    run_cmd(&args).await?;
    Ok(out_mp4.exists())
}

pub async fn ffmpeg_trim_audio(
    in_audio: &Path,
    start_s: f64,
    dur_s: f64,
    out_m4a: &Path,
) -> Result<bool> {
    let args = vec![
        "ffmpeg".to_string(),
        "-y".to_string(),
        "-hide_banner".to_string(),
        "-loglevel".to_string(),
        "error".to_string(),
        "-ss".to_string(),
        format!("{:.3}", start_s),
        "-i".to_string(),
        in_audio.display().to_string(),
        "-t".to_string(),
        format!("{:.3}", dur_s),
        "-c:a".to_string(),
        "aac".to_string(),
        "-b:a".to_string(),
        "192k".to_string(),
        out_m4a.display().to_string(),
    ];
    run_cmd(&args).await?;
    Ok(out_m4a.exists())
}

pub async fn ffmpeg_concat_audio(list_txt: &Path, out_m4a: &Path) -> Result<bool> {
    let args = vec![
        "ffmpeg".to_string(),
        "-y".to_string(),
        "-hide_banner".to_string(),
        "-loglevel".to_string(),
        "error".to_string(),
        "-f".to_string(),
        "concat".to_string(),
        "-safe".to_string(),
        "0".to_string(),
        "-i".to_string(),
        list_txt.display().to_string(),
        "-c".to_string(),
        "copy".to_string(),
        out_m4a.display().to_string(),
    ];
    run_cmd(&args).await?;
    Ok(out_m4a.exists())
}

pub async fn ffmpeg_mix_bgm(video_in: &Path, bgm_in: &Path, video_out: &Path) -> Result<bool> {
    let args = vec![
        "ffmpeg".to_string(),
        "-y".to_string(),
        "-hide_banner".to_string(),
        "-loglevel".to_string(),
        "error".to_string(),
        "-i".to_string(),
        video_in.display().to_string(),
        "-i".to_string(),
        bgm_in.display().to_string(),
        "-filter_complex".to_string(),
        "[0:a]volume=2.5[a0];[1:a]volume=0.1[a1];[a0][a1]amix=inputs=2:duration=first:dropout_transition=2[a]".to_string(),
        "-map".to_string(),
        "0:v".to_string(),
        "-map".to_string(),
        "[a]".to_string(),
        "-c:v".to_string(),
        "copy".to_string(),
        "-c:a".to_string(),
        "aac".to_string(),
        "-b:a".to_string(),
        "192k".to_string(),
        "-movflags".to_string(),
        "+faststart".to_string(),
        video_out.display().to_string(),
    ];
    run_cmd(&args).await?;
    Ok(video_out.exists())
}

pub async fn ffmpeg_make_vertical(in_mp4: &Path, out_mp4: &Path) -> Result<bool> {
    let (_w, h) = match ffprobe_video_dimensions(in_mp4).await {
        Ok(v) => v,
        Err(_) => return Ok(false),
    };
    let dur = match ffprobe_duration_seconds(in_mp4).await {
        Ok(v) => v,
        Err(_) => return Ok(false),
    };

    let mut out_w = ((h as f64) * 9.0 / 16.0 + 0.5) as i32;
    let mut out_h = h;
    out_w &= !1;
    out_h &= !1;

    let filter = format!(
        "[0:v]crop=iw*0.6:ih:iw*0.2:0,scale={}:{},force_original_aspect_ratio=decrease,pad={}:{}:(ow-iw)/2:(oh-ih)/2:black[v]",
        out_w, out_h, out_w, out_h
    );

    let args = vec![
        "ffmpeg".to_string(),
        "-y".to_string(),
        "-hide_banner".to_string(),
        "-loglevel".to_string(),
        "error".to_string(),
        "-i".to_string(),
        in_mp4.display().to_string(),
        "-t".to_string(),
        format!("{:.3}", dur),
        "-filter_complex".to_string(),
        filter,
        "-map".to_string(),
        "[v]".to_string(),
        "-map".to_string(),
        "0:a?".to_string(),
        "-c:v".to_string(),
        "libx264".to_string(),
        "-pix_fmt".to_string(),
        "yuv420p".to_string(),
        "-preset".to_string(),
        "veryfast".to_string(),
        "-crf".to_string(),
        "22".to_string(),
        "-c:a".to_string(),
        "aac".to_string(),
        "-b:a".to_string(),
        "192k".to_string(),
        "-movflags".to_string(),
        "+faststart".to_string(),
        out_mp4.display().to_string(),
    ];

    if let Err(err) = run_cmd(&args).await {
        logw(format!("Vertical render failed: {}", err));
        return Ok(false);
    }

    Ok(out_mp4.exists())
}
