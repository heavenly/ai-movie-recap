use raylib::prelude::*;
use std::sync::{
    atomic::{AtomicBool, AtomicI32, Ordering},
    Arc, Mutex,
};

use ai_movie_shorts::generator::run_generation;
use ai_movie_shorts::init;
use ai_movie_shorts::platform;
use ai_movie_shorts::set_log_hook;

const LOG_MAX_LINES: usize = 300;
const LOG_LINE_MAX: usize = 600;

const COLOR_BG: Color = Color::new(25, 25, 25, 255);
const COLOR_BTN: Color = Color::new(40, 90, 170, 255);
const COLOR_BTN_HOVER: Color = Color::new(70, 120, 200, 255);
const COLOR_BTN_DISABLED: Color = Color::new(60, 60, 60, 255);
const COLOR_LOG_BG: Color = Color::new(18, 18, 18, 255);
const COLOR_LOG_TEXT: Color = Color::new(210, 210, 210, 255);

struct AppState {
    running: Arc<AtomicBool>,
    last_rc: Arc<AtomicI32>,
    log_buffer: Arc<Mutex<Vec<String>>>,
}

fn push_log_line(buffer: &Arc<Mutex<Vec<String>>>, line: &str) {
    let mut guard = buffer.lock().unwrap_or_else(|e| e.into_inner());
    if guard.len() >= LOG_MAX_LINES {
        let excess = guard.len() + 1 - LOG_MAX_LINES;
        guard.drain(0..excess);
    }
    let mut text = line.to_string();
    if text.len() > LOG_LINE_MAX {
        text.truncate(LOG_LINE_MAX);
    }
    guard.push(text);
}

fn draw_button(
    d: &mut RaylibDrawHandle,
    rect: Rectangle,
    label: &str,
    enabled: bool,
    font_size: f32,
) -> bool {
    let mouse = d.get_mouse_position();
    let hot = rect.check_collision_point_rec(mouse);

    let bg = if !enabled {
        COLOR_BTN_DISABLED
    } else if hot {
        COLOR_BTN_HOVER
    } else {
        COLOR_BTN
    };

    d.draw_rectangle_rounded(rect, 0.25, 10, bg);
    d.draw_rectangle_rounded_lines(rect, 0.25, 10, Color::new(20, 20, 20, 255));

    let ts = d.measure_text(label, font_size as i32);
    let pos_x = rect.x + (rect.width - ts as f32) * 0.5;
    let pos_y = rect.y + (rect.height - font_size) * 0.5;
    
    d.draw_text(label, pos_x as i32, pos_y as i32, font_size as i32, Color::RAYWHITE);

    enabled && hot && d.is_mouse_button_released(MouseButton::MOUSE_BUTTON_LEFT)
}

fn draw_log_panel(d: &mut RaylibDrawHandle, rect: Rectangle, lines: &[String]) {
    d.draw_rectangle_rec(rect, COLOR_LOG_BG);
    d.draw_rectangle_lines_ex(rect, 2.0, Color::new(40, 40, 40, 255));

    let font_size = 14;
    let pad = 8.0;
    let line_h = 16.0;
    let max_lines = ((rect.height - 2.0 * pad) / line_h).floor().max(1.0) as usize;

    let start = lines.len().saturating_sub(max_lines);

    let mut y = rect.y + pad;
    for line in lines.iter().skip(start) {
        let pos_x = rect.x + pad;
        d.draw_text(line, pos_x as i32, y as i32, font_size, COLOR_LOG_TEXT);
        y += line_h;
    }
}

fn start_generation_thread(state: &AppState) {
    if state.running.load(Ordering::SeqCst) {
        return;
    }

    state.running.store(true, Ordering::SeqCst);
    state.last_rc.store(0, Ordering::SeqCst);

    let running = Arc::clone(&state.running);
    let last_rc = Arc::clone(&state.last_rc);
    let log_buffer = Arc::clone(&state.log_buffer);

    std::thread::spawn(move || {
        let hook_buffer = Arc::clone(&log_buffer);
        let hook = Arc::new(Mutex::new(move |line: &str| {
            push_log_line(&hook_buffer, line);
        }));

        set_log_hook(Some(hook));
        let rt = match tokio::runtime::Runtime::new() {
            Ok(rt) => rt,
            Err(err) => {
                push_log_line(&log_buffer, &format!("[ERROR] {}", err));
                push_log_line(&log_buffer, "Failed to initialize async runtime");
                last_rc.store(1, Ordering::SeqCst);
                running.store(false, Ordering::SeqCst);
                set_log_hook(None);
                return;
            }
        };

        let result = rt.block_on(run_generation());
        match result {
            Ok(code) => last_rc.store(code, Ordering::SeqCst),
            Err(err) => {
                push_log_line(&log_buffer, &format!("[ERROR] {}", err));
                last_rc.store(1, Ordering::SeqCst);
            }
        }

        set_log_hook(None);
        running.store(false, Ordering::SeqCst);
    });
}

fn snapshot_logs(buffer: &Arc<Mutex<Vec<String>>>) -> Vec<String> {
    buffer.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

fn clear_logs(buffer: &Arc<Mutex<Vec<String>>>) {
    buffer.lock().unwrap_or_else(|e| e.into_inner()).clear();
}

fn main() {
    tracing_subscriber::fmt::init();

    // Initialize directories first
    let rt = tokio::runtime::Runtime::new().expect("Failed to create async runtime");
    rt.block_on(async {
        if let Err(e) = init::ensure_directories().await {
            eprintln!("[ERROR] Failed to create directories: {}", e);
        }
        if !init::check_ffmpeg().await {
            eprintln!("[WARNING] FFmpeg not found in PATH. Please install FFmpeg.");
        }
    });

    let (mut rl, thread) = raylib::init()
        .size(920, 560)
        .resizable()
        .title("AI Movie Shorts")
        .build();
    rl.set_target_fps(60);

    // Try to load custom font, use default if it fails
    let font_path = "resources/Inter-Regular.ttf";
    let font_loaded = std::path::Path::new(font_path).exists();
    if font_loaded {
        let _ = rl.load_font_ex(&thread, font_path, 64, None);
    } else {
        eprintln!("[INFO] Font not found at {}, using default font", font_path);
    }

    let state = AppState {
        running: Arc::new(AtomicBool::new(false)),
        last_rc: Arc::new(AtomicI32::new(0)),
        log_buffer: Arc::new(Mutex::new(Vec::with_capacity(LOG_MAX_LINES))),
    };

    while !rl.window_should_close() {
        let mut d = rl.begin_drawing(&thread);
        d.clear_background(COLOR_BG);

        d.draw_text("Folders", 30, 20, 24, Color::RAYWHITE);

        if draw_button(
            &mut d,
            Rectangle::new(30.0, 60.0, 260.0, 44.0),
            "Open Movies Folder",
            true,
            18.0,
        ) {
            platform::open_folder("movies");
        }

        if draw_button(
            &mut d,
            Rectangle::new(30.0, 115.0, 260.0, 44.0),
            "Open Retired Movies Folder",
            true,
            18.0,
        ) {
            platform::open_folder("movies_retired");
        }

        if draw_button(
            &mut d,
            Rectangle::new(30.0, 170.0, 260.0, 44.0),
            "Open Output Folder",
            true,
            18.0,
        ) {
            platform::open_folder("output");
        }

        if draw_button(
            &mut d,
            Rectangle::new(30.0, 225.0, 260.0, 44.0),
            "Open SRT Folder",
            true,
            18.0,
        ) {
            platform::open_folder("scripts/srt_files");
        }

        let can_start = !state.running.load(Ordering::SeqCst);
        let start_label = if can_start {
            "START GENERATION"
        } else {
            "RUNNING..."
        };

        if draw_button(
            &mut d,
            Rectangle::new(30.0, 280.0, 260.0, 70.0),
            start_label,
            can_start,
            22.0,
        ) {
            clear_logs(&state.log_buffer);
            start_generation_thread(&state);
        }

        let status = format!(
            "Status: {}   (last exit code: {})",
            if state.running.load(Ordering::SeqCst) {
                "RUNNING"
            } else {
                "IDLE"
            },
            state.last_rc.load(Ordering::SeqCst)
        );
        d.draw_text(&status, 30, 370, 18, Color::new(220, 220, 220, 255));

        d.draw_text("Log", 320, 20, 24, Color::RAYWHITE);
        let lines = snapshot_logs(&state.log_buffer);
        draw_log_panel(
            &mut d,
            Rectangle::new(320.0, 60.0, 570.0, 470.0),
            &lines,
        );
    }
}
