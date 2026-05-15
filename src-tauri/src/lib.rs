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

fn build_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36")
        .cookie_store(true)
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败: {e}"))
}

/// Follow the Douyin share link, get cookies, extract video info via API.
#[tauri::command]
async fn resolve_douyin_url(app: tauri::AppHandle, url: String) -> Result<String, String> {
    emit_progress(&app, "resolve", "正在解析抖音链接...");

    let client = build_client()?;

    // Rewrite /video/xxx URLs to jingxuan?modal_id=xxx format
    // because /video/ page is an SPA shell without embedded video data
    let request_url = if let Some(id) = extract_video_id(&url) {
        if url.contains("/video/") {
            let rewritten = format!("https://www.douyin.com/jingxuan?modal_id={id}");
            emit_progress(&app, "resolve", "正在重写链接格式...");
            rewritten
        } else {
            url.clone()
        }
    } else {
        url.clone()
    };

    // Step 1: Visit the page to get redirected and collect cookies.
    let resp = client.get(&request_url)
        .header("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8")
        .header("Accept-Language", "zh-CN,zh;q=0.9,en;q=0.8")
        .send()
        .await
        .map_err(|e| format!("请求失败: {e}"))?;

    let final_url = resp.url().to_string();
    let body = resp.text().await.map_err(|e| format!("读取响应失败: {e}"))?;

    // Step 2: Try extracting video URL from the page's embedded RENDER_DATA
    if let Some(video_url) = extract_from_render_data(&body) {
        emit_progress(&app, "resolve", "从页面数据中提取到视频地址");
        return Ok(video_url);
    }

    // Step 3: Extract video ID and call the internal API (with cookies from step 1)
    if let Some(video_id) = extract_video_id(&final_url) {
        emit_progress(&app, "resolve", &format!("找到视频 ID: {video_id}"));

        // Try the primary API
        if let Some(video_url) = call_aweme_api(&client, &video_id).await {
            emit_progress(&app, "resolve", "通过 API 获取到视频地址");
            return Ok(video_url);
        }

        // Try alternative API format
        if let Some(video_url) = call_aweme_api_v2(&client, &video_id).await {
            emit_progress(&app, "resolve", "通过备用 API 获取到视频地址");
            return Ok(video_url);
        }
    }

    // Step 4: Last resort — try parsing the page HTML for video elements
    if let Some(video_url) = extract_from_html(&body) {
        return Ok(video_url);
    }

    Err(format!(
        "未能提取视频地址。\n\n可能原因：\n\
         1. 该视频需要登录才能观看\n\
         2. 链接格式不正确\n\
         3. 抖音的反爬机制升级\n\n\
         请尝试：\n\
         - 确保链接是公开视频\n\
         - 使用 douyin.com/video/xxxxx 格式的长链接"
    ))
}

/// Extract video ID from a Douyin URL.
fn extract_video_id(url: &str) -> Option<String> {
    // Priority 1: modal_id= query parameter (most reliable)
    if let Some(pos) = url.find("modal_id=") {
        let after = &url[pos + 9..];
        let id: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
        if id.len() >= 15 {
            return Some(id);
        }
    }

    // Priority 2: /video/xxx path segment
    if let Some(pos) = url.find("/video/") {
        let after = &url[pos + 7..];
        let id: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
        if id.len() >= 15 {
            return Some(id);
        }
    }

    // Pattern: ?modal_id=7361234567890123456
    if let Some(pos) = url.find("modal_id=") {
        let after = &url[pos + 9..];
        let id: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
        if id.len() >= 15 {
            return Some(id);
        }
    }

    // Pattern: /note/7361234567890123456 (image slideshows)
    if let Some(pos) = url.find("/note/") {
        let after = &url[pos + 6..];
        let id: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
        if id.len() >= 15 {
            return Some(id);
        }
    }

    None
}

/// Call the primary Douyin API endpoint for video detail.
async fn call_aweme_api(client: &reqwest::Client, video_id: &str) -> Option<String> {
    let api_url = format!(
        "https://www.douyin.com/aweme/v1/web/aweme/detail/?aweme_id={video_id}&aid=6383&device_platform=web"
    );

    let resp = client
        .get(&api_url)
        .header("Referer", "https://www.douyin.com/")
        .header("Accept", "application/json")
        .send()
        .await
        .ok()?;

    let json: serde_json::Value = resp.json().await.ok()?;

    let video = json.get("aweme_detail")?.get("video")?;

    // Try multiple field names for the video URL
    for key in &["play_addr", "play_addr_h264", "download_addr", "play_addr_bytevc1"] {
        if let Some(url_list) = video.get(key)?.get("url_list")?.as_array() {
            for url_val in url_list {
                if let Some(url) = url_val.as_str() {
                    return Some(url.to_string());
                }
            }
        }
    }

    // Try bit_rate (quality variants)
    if let Some(bit_rates) = video.get("bit_rate")?.as_array() {
        for br in bit_rates {
            if let Some(play_addr) = br.get("play_addr") {
                if let Some(url_list) = play_addr.get("url_list")?.as_array() {
                    for url_val in url_list {
                        if let Some(url) = url_val.as_str() {
                            return Some(url.to_string());
                        }
                    }
                }
            }
        }
    }

    None
}

/// Alternative API endpoint (sometimes needed for certain video types).
async fn call_aweme_api_v2(client: &reqwest::Client, video_id: &str) -> Option<String> {
    let api_url = format!(
        "https://www.iesdouyin.com/web/api/v2/aweme/iteminfo/?item_ids={video_id}"
    );

    let resp = client
        .get(&api_url)
        .header("Referer", "https://www.douyin.com/")
        .header("Accept", "application/json")
        .send()
        .await
        .ok()?;

    let json: serde_json::Value = resp.json().await.ok()?;
    let items = json.get("item_list")?.as_array()?;
    let item = items.first()?;
    let video = item.get("video")?;

    for key in &["play_addr", "play_addr_h264", "download_addr"] {
        if let Some(url_list) = video.get(key)?.get("url_list")?.as_array() {
            for url_val in url_list {
                if let Some(url) = url_val.as_str() {
                    return Some(url.to_string());
                }
            }
        }
    }

    None
}

/// Extract video URL from RENDER_DATA / _ROUTER_DATA embedded in the page.
fn extract_from_render_data(html: &str) -> Option<String> {
    let patterns = ["RENDER_DATA", "_ROUTER_DATA"];

    for pattern in &patterns {
        // Find the script tag with matching id
        if let Some(start) = html.find(&format!("id=\"{pattern}\"")) {
            let after_tag = &html[start..];
            if let Some(tag_end) = after_tag.find('>') {
                let after_open = &after_tag[tag_end + 1..];
                if let Some(script_end) = after_open.find("</script>") {
                    let raw = &after_open[..script_end];
                    // The data might be URL-encoded; try decoding
                    let decoded = urlencoding(raw);
                    if let Some(url) = find_video_url_in_json(&decoded) {
                        return Some(url);
                    }
                    // Also try the raw text
                    if let Some(url) = find_video_url_in_json(raw) {
                        return Some(url);
                    }
                }
            }
        }
    }

    // Also search for "playAddr" or "play_addr" directly in the HTML
    // Some SSR pages embed video URLs directly in __NEXT_DATA__ or similar
    for marker in &["play_addr", "playAddr", "video_id"] {
        if let Some(pos) = html.find(marker) {
            let window = &html[pos..std::cmp::min(pos + 5000, html.len())];
            if let Some(url) = find_video_url_in_json(window) {
                return Some(url);
            }
        }
    }

    None
}

/// Try to URL-decode a string (RENDER_DATA is often URL-encoded JSON).
fn urlencoding(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(hex) = u8::from_str_radix(
                &String::from_utf8_lossy(&bytes[i + 1..i + 3]),
                16,
            ) {
                result.push(hex as char);
                i += 3;
                continue;
            }
        }
        result.push(bytes[i] as char);
        i += 1;
    }
    result
}

/// Search JSON text for video CDN URLs.

/// Check if a URL looks like a video URL (not a static resource like PNG/CSS/JS).
fn is_video_url(url: &str) -> bool {
    if !url.starts_with("http") {
        return false;
    }
    // Reject static resources
    for ext in &[".png", ".jpg", ".jpeg", ".gif", ".webp", ".svg", ".css", ".js", ".ico"] {
        if url.contains(ext) {
            return false;
        }
    }
    // Video indicators
    if url.contains("video") || url.contains("mime_type=video") {
        return true;
    }
    // Known video CDNs
    for cdn in &["douyinvod.com", "ixigua.com", "bytecdn.cn", "bytedns.net", "pstatp.com"] {
        if url.contains(cdn) {
            return true;
        }
    }
    false
}
fn find_video_url_in_json(json: &str) -> Option<String> {
    let url_keys = [
        "\"url_list\":[\"",
        "\"play_addr\":{\"url_list\":[\"",
        "\"download_addr\":{\"url_list\":[\"",
        "\"play_addr_h264\":{\"url_list\":[\"",
        "playAddr\":\"",
    ];

    for key in &url_keys {
        if let Some(pos) = json.find(key) {
            let after_key = &json[pos + key.len()..];
            if let Some(end) = after_key.find('"') {
                let url = unescape_json(&after_key[..end]);
                if is_video_url(&url) {
                    return Some(url);
                }
            }
        }
    }

    // Fallback: search for known CDN domains
    for cdn in &["douyinvod.com", "ixigua.com", "bytecdn.cn", "bytedns.net", "pstatp.com"] {
        if let Some(pos) = json.find(cdn) {
            let start = json[..pos].rfind("http").unwrap_or(pos);
            let end = json[pos..]
                .find(|c: char| c == '"' || c == ',' || c == '}' || c == ']')
                .map(|i| pos + i)
                .unwrap_or(json.len());
            let url = unescape_json(&json[start..end]);
            if is_video_url(&url) {
                return Some(url);
            }
        }
    }

    None
}

fn unescape_json(s: &str) -> String {
    s.replace("\\u0026", "&")
        .replace("\\u002F", "/")
        .replace("\\/", "/")
        .replace("\\\"", "\"")
}

fn extract_from_html(html: &str) -> Option<String> {
    let document = Html::parse_document(html);

    let video_sel = Selector::parse("video source").ok()?;
    if let Some(el) = document.select(&video_sel).next() {
        if let Some(src) = el.value().attr("src") {
            if !src.starts_with("blob:") {
                return Some(src.to_string());
            }
        }
    }

    let meta_sel = Selector::parse(r#"meta[property="og:video"]"#).ok()?;
    if let Some(el) = document.select(&meta_sel).next() {
        if let Some(content) = el.value().attr("content") {
            if !content.starts_with("blob:") {
                return Some(content.to_string());
            }
        }
    }

    None
}

/// Download a video from a URL to a temporary file.
#[tauri::command]
async fn download_video(app: tauri::AppHandle, url: String) -> Result<String, String> {
    emit_progress(&app, "download", "正在下载视频...");

    let client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36")
        .referer(false)
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败: {e}"))?;

    let response = client
        .get(&url)
        .header("Referer", "https://www.douyin.com/")
        .send()
        .await
        .map_err(|e| format!("下载请求失败: {e}"))?;

    let total = response.content_length().unwrap_or(0);
    let bytes = response
        .bytes()
        .await
        .map_err(|e| format!("下载数据失败: {e}"))?;

    if bytes.len() < 1024 {
        return Err("下载的视频文件过小，可能是无效的视频地址".to_string());
    }

    if total > 0 {
        emit_progress(&app, "download", &format!("已下载 {:.1} MB", bytes.len() as f64 / 1048576.0));
    }

    let tmp_dir = tempfile::tempdir().map_err(|e| format!("创建临时目录失败: {e}"))?;
    let file_path = tmp_dir.keep().join("video.mp4");

    std::fs::write(&file_path, &bytes).map_err(|e| format!("写入文件失败: {e}"))?;

    Ok(file_path.to_string_lossy().to_string())
}


/// Map an arbitrary sample rate to the nearest value supported by LAME.
fn clamp_sample_rate(rate: u32) -> u32 {
    const VALID: &[u32] = &[8000, 11025, 12000, 16000, 22050, 24000, 32000, 44100, 48000];
    VALID
        .iter()
        .min_by_key(|&&v| (v as i64 - rate as i64).unsigned_abs())
        .copied()
        .unwrap_or(44100)
}


fn convert_audio(input: &str, output: &str) -> Result<(), String> {
    let file = std::fs::File::open(input)
        .map_err(|e| format!("打开视频文件失败: {e}"))?;

    let file_size = file.metadata()
        .map_err(|e| format!("读取文件信息失败: {e}"))?
        .len();

    // redlux: MP4 demuxing + HE-AAC decoding via fdk-aac (statically linked)
    let decoder = redlux::Decoder::new_mpeg4(file, file_size)
        .map_err(|e| format!("解析视频失败: {e}"))?;

    let sample_rate = match decoder.sample_rate() {
        0 => 44100,
        r => clamp_sample_rate(r),
    };
    let channel_count = (decoder.channels() as usize).max(1).min(2);

    // Build MP3 encoder (statically linked LAME)
    let mut mp3_encoder = mp3lame_encoder::Builder::new()
        .ok_or("创建 MP3 编码器失败")?
        .with_sample_rate(sample_rate)
        .map_err(|e| format!("设置采样率失败 (sample_rate={sample_rate}): {e}"))?
        .with_num_channels(channel_count as u8)
        .map_err(|e| format!("设置声道数失败: {e}"))?
        .with_quality(mp3lame_encoder::Quality::Best)
        .map_err(|e| format!("设置编码质量失败: {e}"))?
        .build()
        .map_err(|e| format!("初始化 MP3 编码器失败: {e}"))?;

    let mut mp3_output: Vec<u8> = Vec::with_capacity(1024 * 1024);

    // redlux iterates individual interleaved i16 samples
    // Encode in 1152-sample-per-channel MP3 frames
    const FRAME_SAMPLES: usize = 1152;
    let chunk_size = FRAME_SAMPLES * channel_count;
    let mut buf: Vec<i16> = Vec::with_capacity(chunk_size);

    for sample in decoder {
        buf.push(sample);
        if buf.len() >= chunk_size {
            let input = mp3lame_encoder::InterleavedPcm(&buf[..chunk_size]);
            let needed = mp3lame_encoder::max_required_buffer_size(chunk_size);
            mp3_output.reserve(needed);
            let spare = mp3_output.spare_capacity_mut();
            let written = mp3_encoder.encode(input, spare)
                .map_err(|e| format!("MP3 编码失败: {e}"))?;
            unsafe { mp3_output.set_len(mp3_output.len() + written); }
            let rem = buf.split_off(chunk_size);
            buf = rem;
        }
    }

    // Encode final partial frame
    if !buf.is_empty() {
        let input = mp3lame_encoder::InterleavedPcm(&buf);
        let needed = mp3lame_encoder::max_required_buffer_size(buf.len());
        mp3_output.reserve(needed);
        let spare = mp3_output.spare_capacity_mut();
        let written = mp3_encoder.encode(input, spare)
            .map_err(|e| format!("MP3 编码失败: {e}"))?;
        unsafe { mp3_output.set_len(mp3_output.len() + written); }
    }

    // Flush
    mp3_output.reserve(7200);
    let spare = mp3_output.spare_capacity_mut();
    let written = mp3_encoder.flush::<mp3lame_encoder::FlushNoGap>(spare)
        .map_err(|e| format!("MP3 收尾失败: {e}"))?;
    unsafe { mp3_output.set_len(mp3_output.len() + written); }

    std::fs::write(output, &mp3_output)
        .map_err(|e| format!("写入 MP3 文件失败: {e}"))?;

    Ok(())
}
/// Extract audio track from video and encode to MP3.
/// Uses redlux (fdk-aac, statically linked) + mp3lame-encoder (statically linked LAME).
#[tauri::command]
async fn extract_audio(
    app: tauri::AppHandle,
    video_path: String,
    output_path: String,
) -> Result<String, String> {
    emit_progress(&app, "convert", "正在提取音频...");

    // Run the CPU-intensive conversion in a blocking thread
    let out = output_path.clone();
    tokio::task::spawn_blocking(move || convert_audio(&video_path, &out))
        .await
        .map_err(|e| format!("音频转换线程异常: {e}"))??;

    emit_progress(&app, "done", "转换完成！");

    Ok(output_path)
}

#[tauri::command]
fn copy_file(src: String, dst: String) -> Result<(), String> {
    std::fs::copy(&src, &dst).map_err(|e| format!("复制文件失败: {e}"))?;
    Ok(())
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
            copy_file,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");

}

