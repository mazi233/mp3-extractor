import { useState, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

interface Progress {
  stage: string;
  message: string;
}

type Status =
  | { kind: "idle" }
  | { kind: "processing"; stage: string; message: string }
  | { kind: "done"; tempPath: string }
  | { kind: "saved"; path: string }
  | { kind: "error"; message: string };

function App() {
  const [url, setUrl] = useState("");
  const [status, setStatus] = useState<Status>({ kind: "idle" });

  const handleProcess = useCallback(async () => {
    if (!url.trim()) return;

    setStatus({ kind: "processing", stage: "start", message: "开始处理..." });

    const unlisten = await listen<Progress>("extract-progress", (event) => {
      setStatus({
        kind: "processing",
        stage: event.payload.stage,
        message: event.payload.message,
      });
    });

    try {
      const videoUrl: string = await invoke("resolve_douyin_url", { url: url.trim() });
      const videoPath: string = await invoke("download_video", { url: videoUrl });

      // Always output to temp dir
      const outPath = `${videoPath.replace("video.mp4", "")}audio.mp3`;

      const result: string = await invoke("extract_audio", {
        videoPath,
        outputPath: outPath,
      });

      setStatus({ kind: "done", tempPath: result });
    } catch (err) {
      setStatus({ kind: "error", message: String(err) });
    } finally {
      unlisten();
    }
  }, [url]);

  const handleSave = async () => {
    if (status.kind !== "done") return;

    try {
      const { save } = await import("@tauri-apps/plugin-dialog");
      const dest = await save({
        title: "保存 MP3 文件",
        defaultPath: "douyin_audio.mp3",
        filters: [{ name: "MP3 音频", extensions: ["mp3"] }],
      });

      if (!dest) return; // user cancelled

      await invoke("copy_file", { src: status.tempPath, dst: dest });
      setStatus({ kind: "saved", path: dest });

      // Show success - user can manually open
    } catch (err) {
      setStatus({ kind: "error", message: `保存失败: ${err}` });
    }
  };

  const isProcessing = status.kind === "processing";

  return (
    <div className="app">
      <h1>🎵 抖音视频转音频</h1>

      <div className="input-section">
        <label htmlFor="url">粘贴抖音分享链接</label>
        <input
          id="url"
          type="text"
          placeholder="https://v.douyin.com/xxxxx/ 或 https://www.douyin.com/video/xxxxx"
          value={url}
          onChange={(e) => setUrl(e.target.value)}
          disabled={isProcessing}
          onKeyDown={(e) => e.key === "Enter" && handleProcess()}
        />
      </div>

      <button
        className="primary-btn"
        onClick={handleProcess}
        disabled={isProcessing || !url.trim()}
      >
        {isProcessing ? "处理中..." : "开始提取音频"}
      </button>

      <div className="status">
        {status.kind === "processing" && (
          <div className="status-processing">
            <div className="spinner" />
            <span>{status.message}</span>
          </div>
        )}

        {status.kind === "done" && (
          <div className="status-done">
            <span>✅ 转换完成！</span>
            <button className="primary-btn" onClick={handleSave} style={{ marginTop: 12 }}>
              保存 MP3 文件
            </button>
          </div>
        )}

        {status.kind === "saved" && (
          <div className="status-done">
            <span>✅ 已保存！</span>
            <span className="path">{status.path}</span>
          </div>
        )}

        {status.kind === "error" && (
          <div className="status-error">
            <span>❌ {status.message}</span>
          </div>
        )}
      </div>
    </div>
  );
}

export default App;
