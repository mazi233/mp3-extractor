import { useState, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

interface Progress {
  stage: string;
  message: string;
}

// ── Tab 1: Download Video ──

function TabDownloadVideo() {
  const [url, setUrl] = useState("");
  const [status, setStatus] = useState<
    { kind: "idle" } | { kind: "busy"; msg: string } | { kind: "ready"; videoPath: string } | { kind: "error"; msg: string }
  >({ kind: "idle" });

  const handleFetch = async () => {
    if (!url.trim()) return;
    setStatus({ kind: "busy", msg: "解析链接..." });
    const unlisten = await listen<Progress>("extract-progress", (e) =>
      setStatus({ kind: "busy", msg: e.payload.message }),
    );
    try {
      const vu: string = await invoke("resolve_douyin_url", { url: url.trim() });
      const vp: string = await invoke("download_video", { url: vu });
      setStatus({ kind: "ready", videoPath: vp });
    } catch (err) {
      setStatus({ kind: "error", msg: String(err) });
    } finally { unlisten(); }
  };

  const handleSave = async () => {
    if (status.kind !== "ready") return;
    const { save } = await import("@tauri-apps/plugin-dialog");
    const d = await save({ title: "保存视频", defaultPath: "douyin_video.mp4", filters: [{ name: "MP4", extensions: ["mp4"] }] });
    if (!d) return;
    await invoke("copy_file", { src: status.videoPath, dst: d });
    setStatus({ kind: "idle" });
  };

  return (
    <div className="tab">
      <div className="input-section">
        <label>粘贴抖音分享文本或链接</label>
        <input
          placeholder="直接粘贴抖音分享文本或链接"
          value={url}
          onChange={(e) => setUrl(extractDouyinUrl(e.target.value))}
          onKeyDown={(e) => e.key === "Enter" && handleFetch()}
        />
      </div>
      {status.kind === "idle" && <button className="primary-btn" onClick={handleFetch} disabled={!url.trim()}>获取视频</button>}
      {status.kind === "busy" && <div className="status-processing"><div className="spinner" /><span>{status.msg}</span></div>}
      {status.kind === "ready" && (
        <div className="status-actions">
          <span>✅ 视频已获取</span>
          <button className="primary-btn" onClick={handleSave}>💾 保存视频</button>
        </div>
      )}
      {status.kind === "error" && <div className="status-error"><span>❌ {status.msg}</span></div>}
    </div>
  );
}

// ── Tab 2: Download & Convert to MP3 ──

function TabDownloadMp3() {
  const [url, setUrl] = useState("");
  const [status, setStatus] = useState<
    { kind: "idle" }
    | { kind: "busy"; msg: string }
    | { kind: "ready"; videoPath: string }
    | { kind: "encoding"; msg: string; videoPath: string }
    | { kind: "mp3_ready"; videoPath: string; mp3Path: string }
    | { kind: "error"; msg: string }
  >({ kind: "idle" });

  const handleFetch = async () => {
    if (!url.trim()) return;
    setStatus({ kind: "busy", msg: "解析链接..." });
    const unlisten = await listen<Progress>("extract-progress", (e) =>
      setStatus((p) => p.kind === "busy" ? { kind: "busy", msg: e.payload.message } : p),
    );
    try {
      const vu: string = await invoke("resolve_douyin_url", { url: url.trim() });
      const vp: string = await invoke("download_video", { url: vu });
      setStatus({ kind: "ready", videoPath: vp });
    } catch (err) {
      setStatus({ kind: "error", msg: String(err) });
    } finally { unlisten(); }
  };

  const handleConvert = async () => {
    if (status.kind !== "ready") return;
    const vp = status.videoPath;
    setStatus({ kind: "encoding", msg: "正在转换...", videoPath: vp });
    const unlisten = await listen<Progress>("extract-progress", (e) =>
      setStatus((p) => p.kind === "encoding" ? { kind: "encoding", msg: e.payload.message, videoPath: vp } : p),
    );
    try {
      const mp = `${vp.replace("video.mp4", "")}audio.mp3`;
      await invoke("extract_audio", { videoPath: vp, outputPath: mp });
      setStatus({ kind: "mp3_ready", videoPath: vp, mp3Path: mp });
    } catch (err) {
      setStatus({ kind: "error", msg: String(err) });
    } finally { unlisten(); }
  };

  const handleSaveMp3 = async () => {
    if (status.kind !== "mp3_ready") return;
    const { save } = await import("@tauri-apps/plugin-dialog");
    const d = await save({ title: "保存 MP3", defaultPath: "douyin_audio.mp3", filters: [{ name: "MP3", extensions: ["mp3"] }] });
    if (!d) return;
    await invoke("copy_file", { src: status.mp3Path, dst: d });
  };

  return (
    <div className="tab">
      <div className="input-section">
        <label>粘贴抖音分享文本或链接</label>
        <input
          placeholder="直接粘贴抖音分享文本或链接"
          value={url}
          onChange={(e) => setUrl(extractDouyinUrl(e.target.value))}
          onKeyDown={(e) => e.key === "Enter" && handleFetch()}
        />
      </div>
      {status.kind === "idle" && <button className="primary-btn" onClick={handleFetch} disabled={!url.trim()}>获取视频</button>}
      {status.kind === "busy" && <div className="status-processing"><div className="spinner" /><span>{status.msg}</span></div>}
      {status.kind === "ready" && (
        <div className="status-actions">
          <span>✅ 视频已获取</span>
          <button className="primary-btn" onClick={handleConvert}>🎵 转换并保存 MP3</button>
        </div>
      )}
      {status.kind === "encoding" && <div className="status-processing"><div className="spinner" /><span>{status.msg}</span></div>}
      {status.kind === "mp3_ready" && (
        <div className="status-actions">
          <span>✅ MP3 转换完成</span>
          <button className="primary-btn" onClick={handleSaveMp3}>💾 保存 MP3</button>
        </div>
      )}
      {status.kind === "error" && <div className="status-error"><span>❌ {status.msg}</span></div>}
    </div>
  );
}

// ── Tab 3: Local Video → MP3 ──

function TabLocalConvert() {
  const [videoPath, setVideoPath] = useState<string | null>(null);
  const [status, setStatus] = useState<
    { kind: "idle" }
    | { kind: "encoding"; msg: string }
    | { kind: "ready"; mp3Path: string }
    | { kind: "error"; msg: string }
  >({ kind: "idle" });

  const handlePickFile = async () => {
    const { open } = await import("@tauri-apps/plugin-dialog");
    const f = await open({
      title: "选择视频文件",
      filters: [{ name: "视频", extensions: ["mp4", "mov", "mkv", "avi", "webm", "flv", "m4v"] }],
      multiple: false,
    });
    if (f && typeof f === "string") {
      setVideoPath(f);
      setStatus({ kind: "idle" });
    }
  };

  const handleConvert = async () => {
    if (!videoPath) return;
    setStatus({ kind: "encoding", msg: "正在转换..." });
    const unlisten = await listen<Progress>("extract-progress", (e) =>
      setStatus({ kind: "encoding", msg: e.payload.message }),
    );
    try {
      const out = `${videoPath}.mp3`;
      await invoke("extract_audio", { videoPath, outputPath: out });
      setStatus({ kind: "ready", mp3Path: out });
    } catch (err) {
      setStatus({ kind: "error", msg: String(err) });
    } finally { unlisten(); }
  };

  const handleSave = async () => {
    if (status.kind !== "ready") return;
    const { save } = await import("@tauri-apps/plugin-dialog");
    const d = await save({ title: "保存 MP3", defaultPath: "audio.mp3", filters: [{ name: "MP3", extensions: ["mp3"] }] });
    if (!d) return;
    await invoke("copy_file", { src: status.mp3Path, dst: d });
  };

  return (
    <div className="tab">
      <div className="input-section">
        <label>选择本地视频文件</label>
        <div className="file-pick-row">
          <button className="primary-btn secondary" onClick={handlePickFile}>📁 选择视频</button>
          <span className="file-path">{videoPath || "未选择文件"}</span>
        </div>
      </div>
      {status.kind === "idle" && videoPath && (
        <button className="primary-btn" onClick={handleConvert}>🎵 转为 MP3</button>
      )}
      {status.kind === "encoding" && <div className="status-processing"><div className="spinner" /><span>{status.msg}</span></div>}
      {status.kind === "ready" && (
        <div className="status-actions">
          <span>✅ 转换完成</span>
          <button className="primary-btn" onClick={handleSave}>💾 保存 MP3</button>
        </div>
      )}
      {status.kind === "error" && <div className="status-error"><span>❌ {status.msg}</span></div>}
    </div>
  );
}

// ── URL extraction helper ──

function extractDouyinUrl(text: string): string {
  if (/^https?:\/\/(v\.douyin\.com|www\.douyin\.com)\//.test(text.trim())) return text.trim();
  for (const p of [
    /https?:\/\/v\.douyin\.com\/[A-Za-z0-9]+\/?/,
    /https?:\/\/www\.douyin\.com\/video\/\d+/,
    /https?:\/\/www\.douyin\.com\/jingxuan\?modal_id=\d+/,
    /https?:\/\/www\.douyin\.com\/user\/[^?\s]+\?modal_id=\d+/,
  ]) {
    const m = text.match(p);
    if (m) return m[0];
  }
  return text.trim();
}

// ── App shell ──

type Tab = "download" | "download_mp3" | "local_convert";

function App() {
  const [tab, setTab] = useState<Tab>("download_mp3");

  return (
    <div className="app">
      <h1>🎵 抖音视频工具</h1>
      <div className="tab-bar">
        <button className={`tab-btn ${tab === "download" ? "active" : ""}`} onClick={() => setTab("download")}>下载视频</button>
        <button className={`tab-btn ${tab === "download_mp3" ? "active" : ""}`} onClick={() => setTab("download_mp3")}>下载并转 MP3</button>
        <button className={`tab-btn ${tab === "local_convert" ? "active" : ""}`} onClick={() => setTab("local_convert")}>本地视频转 MP3</button>
      </div>
      {tab === "download" && <TabDownloadVideo />}
      {tab === "download_mp3" && <TabDownloadMp3 />}
      {tab === "local_convert" && <TabLocalConvert />}
    </div>
  );
}

export default App;
