use scraper::{Html, Selector};
use serde::Serialize;
use tauri::Emitter;

#[derive(Clone, Serialize)]
struct Progress {
    stage: &'static str,
    message: String,
}

fn emit_progress(app: &tauri::AppHandle, stage: &'static str, message: &str) {
    let _ = app.emit("extract-progress", Progress {
        stage,
        message: message.to_string(),
    });
}

/// Follow a Douyin share link (v.douyin.com/xxx) and extract the direct video URL.
/// Douyin share pages embed video info in a `<script id="RENDER_DATA">` tag or similar.
#[tauri::command]
async fn resolve_douyin_url(app: tauri::AppHandle, url: String) -> Result<String, String> {
    emit_progress(&app, "resolve", "正在解析抖音链接...");

    let client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (iPhone; CPU iPhone OS 16_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/16.0 Mobile/15E148 Safari/604.1")
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败: {e}"))?;

    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("请求失败: {e}"))?;

    let final_url = response.url().to_string();
    let body = response
        .text()
        .await
        .map_err(|e| format!("读取响应失败: {e}"))?;

    // Try method 1: look for video URL in RENDER_DATA script tag
    if let Some(video_url) = extract_from_render_data(&body) {
        return Ok(video_url);
    }

    // Try method 2: parse the page for video source elements
    if let Some(video_url) = extract_from_html(&body) {
        return Ok(video_url);
    }

    // Try method 3: use the douyin API endpoint (item_ids extracted from URL)
    if let Some(video_url) = extract_from_api(&client, &final_url).await {
        return Ok(video_url);
    }

    Err("未能从页面中提取视频地址，请确认链接有效".to_string())
}

/// Extract video URL from `window._ROUTER_DATA` or `RENDER_DATA` JSON blob in the page.
fn extract_from_render_data(html: &str) -> Option<String> {
    // Douyin embeds data in various script tags — search for known patterns
    let patterns = [
        "RENDER_DATA",
        "_ROUTER_DATA",
    ];

    for pattern in &patterns {
        if let Some(start) = html.find(&format!("id=\"{pattern}\"")) {
            // Find the script tag content
            let after_tag = &html[start..];
            if let Some(tag_end) = after_tag.find('>') {
                let after_open = &after_tag[tag_end + 1..];
                if let Some(script_end) = after_open.find("</script>") {
                    let json_str = &after_open[..script_end];
                    if let Some(url) = find_video_url_in_json(json_str) {
                        return Some(url);
                    }
                }
            }
        }
    }
    None
}

/// Search JSON text for video playback URLs.
fn find_video_url_in_json(json: &str) -> Option<String> {
    // Common Douyin video URL patterns in their JSON data
    // Keys like "play_addr", "download_addr", "bit_rate", etc.
    let keys = [
        "\"play_addr\":{\"url_list\":[\"",
        "\"play_addr_h264\":{\"url_list\":[\"",
        "\"download_addr\":{\"url_list\":[\"",
        "playAddr\":\"",
    ];

    for key in &keys {
        if let Some(pos) = json.find(key) {
            let after_key = &json[pos + key.len()..];
            if let Some(end) = after_key.find('"') {
                let url = &after_key[..end];
                // Handle escaped slashes
                let url = url.replace("\\u0026", "&").replace("\\/", "/");
                if url.starts_with("http") {
                    return Some(url);
                }
            }
        }
    }

    // Also search for any http URL that looks like a video CDN
    // Douyin video CDNs: *.douyinvod.com, *.ixigua.com, etc.
    for cdn in &["douyinvod.com", "ixigua.com", "bytecdn.cn", "bytedance.com"] {
        if let Some(pos) = json.find(cdn) {
            // Extract the surrounding URL
            let start = json[..pos].rfind("http").unwrap_or(pos);
            let end = json[pos..]
                .find(|c: char| c == '"' || c == ',' || c == '}' || c == ']')
                .map(|i| pos + i)
                .unwrap_or(json.len());
            let url = &json[start..end];
            let url = url.replace("\\u0026", "&").replace("\\/", "/");
            return Some(url.to_string());
        }
    }

    None
}

/// Extract video URL from HTML meta/video tags.
fn extract_from_html(html: &str) -> Option<String> {
    let document = Html::parse_document(html);

    // Look for <video> source
    let video_sel = Selector::parse("video source").ok()?;
    if let Some(el) = document.select(&video_sel).next() {
        if let Some(src) = el.value().attr("src") {
            return Some(src.to_string());
        }
    }

    // Look for og:video meta
    let meta_sel = Selector::parse(r#"meta[property="og:video"]"#).ok()?;
    if let Some(el) = document.select(&meta_sel).next() {
        if let Some(content) = el.value().attr("content") {
            return Some(content.to_string());
        }
    }

    // Look for video tag directly
    let video_sel = Selector::parse("video").ok()?;
    if let Some(el) = document.select(&video_sel).next() {
        if let Some(src) = el.value().attr("src") {
            return Some(src.to_string());
        }
    }

    None
}

/// Try to use Douyin's internal API to get video info.
async fn extract_from_api(client: &reqwest::Client, page_url: &str) -> Option<String> {
    // Extract video ID from URL patterns:
    // https://www.douyin.com/video/7361234567890123456
    // https://www.douyin.com/user/xxx?modal_id=7361234567890123456
    let video_id = if let Some(pos) = page_url.find("/video/") {
        let after = &page_url[pos + 7..];
        after.chars().take_while(|c| c.is_ascii_digit()).collect::<String>()
    } else if let Some(pos) = page_url.find("modal_id=") {
        let after = &page_url[pos + 9..];
        after.chars().take_while(|c| c.is_ascii_digit()).collect::<String>()
    } else {
        return None;
    };

    if video_id.is_empty() {
        return None;
    }

    let api_url = format!("https://www.douyin.com/aweme/v1/web/aweme/detail/?aweme_id={video_id}");
    let resp = client
        .get(&api_url)
        .header("Referer", "https://www.douyin.com/")
        .send()
        .await
        .ok()?;

    let json: serde_json::Value = resp.json().await.ok()?;
    let video = json
        .get("aweme_detail")?
        .get("video")?;

    // Try play_addr first (highest quality), fall back to others
    for key in &["play_addr", "play_addr_h264", "download_addr"] {
        if let Some(url) = video.get(key)?.get("url_list")?.get(0)?.as_str() {
            return Some(url.to_string());
        }
    }

    None
}

/// Download a video from a URL to a temporary file.
#[tauri::command]
async fn download_video(app: tauri::AppHandle, url: String) -> Result<String, String> {
    emit_progress(&app, "download", "正在下载视频...");

    let client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (iPhone; CPU iPhone OS 16_0 like Mac OS X) AppleWebKit/605.1.15")
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败: {e}"))?;

    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("下载请求失败: {e}"))?;

    let total = response.content_length().unwrap_or(0);
    let bytes = response
        .bytes()
        .await
        .map_err(|e| format!("下载数据失败: {e}"))?;

    if total > 0 {
        emit_progress(&app, "download", &format!("已下载 {} MB", bytes.len() / 1024 / 1024));
    }

    let tmp_dir = tempfile::tempdir().map_err(|e| format!("创建临时目录失败: {e}"))?;
    let file_path = tmp_dir.keep().join("video.mp4");

    std::fs::write(&file_path, &bytes).map_err(|e| format!("写入文件失败: {e}"))?;

    Ok(file_path.to_string_lossy().to_string())
}

/// Extract audio from video using ffmpeg (auto-downloaded).
#[tauri::command]
async fn extract_audio(
    app: tauri::AppHandle,
    video_path: String,
    output_path: String,
) -> Result<String, String> {
    emit_progress(&app, "convert", "正在提取音频...");

    // ffmpeg-sidecar auto-downloads ffmpeg on first use
    let exit_status = ffmpeg_sidecar::command::FfmpegCommand::new()
        .args([
            "-y",
            "-i", &video_path,
            "-vn",
            "-acodec", "libmp3lame",
            "-q:a", "2",
            &output_path,
        ])
        .spawn()
        .map_err(|e| format!("启动 ffmpeg 失败: {e}"))?
        .wait()
        .map_err(|e| format!("等待 ffmpeg 完成失败: {e}"))?;

    if !exit_status.success() {
        return Err("ffmpeg 转换失败".to_string());
    }

    emit_progress(&app, "done", "转换完成！");

    Ok(output_path)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            resolve_douyin_url,
            download_video,
            extract_audio,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
