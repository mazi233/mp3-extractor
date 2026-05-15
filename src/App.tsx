import { useState, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { openPath } from "@tauri-apps/plugin-opener";

interface Progress {
  stage: string;
  message: string;
}

type Status =
  | { kind: "idle" }
  | { kind: "processing"; stage: string; message: string }
  | { kind: "done"; outputPath: string }
  | { kind: "error"; message: string };

function App() {
  const [url, setUrl] = useState("");
  const [status, setStatus] = useState<Status>({ kind: "idle" });
  const [outputDir, setOutputDir] = useState("");

  const handleProcess = useCallback(async () => {
    if (!url.trim()) return;

    setStatus({ kind: "processing", stage: "start", message: "开始处理..." });

    // Listen for progress events
    const unlisten = await listen<Progress>("extract-progress", (event) => {
      setStatus({
        kind: "processing",
        stage: event.payload.stage,
        message: event.payload.message,
      });
    });

    try {
      // Step 1: Resolve Douyin URL to video URL
      const videoUrl: string = await invoke("resolve_douyin_url", { url: url.trim() });

      // Step 2: Download the video
      const videoPath: string = await invoke("download_video", { url: videoUrl });

      // Step 3: Determine output path
      const timestamp = Date.now();
      const outPath = outputDir
        ? `${outputDir}/douyin_${timestamp}.mp3`
        : `${videoPath.replace("video.mp4", "")}douyin_${timestamp}.mp3`;

      // Step 4: Extract audio
      const result: string = await invoke("extract_audio", {
        videoPath,
        outputPath: outPath,
      });

      setStatus({ kind: "done", outputPath: result });
    } catch (err) {
      setStatus({ kind: "error", message: String(err) });
    } finally {
      unlisten();
    }
  }, [url, outputDir]);

  const handleChooseDir = async () => {
    try {
      const { open: openDialog } = await import("@tauri-apps/plugin-dialog");
      const selected = await openDialog({ directory: true, multiple: false, title: "选择输出目录" });
      if (selected && typeof selected === "string") {
        setOutputDir(selected);
      }
    } catch {
      // dialog not available in browser dev mode
    }
  };

  const handleOpenFile = async () => {
    if (status.kind === "done") {
      await openPath(status.outputPath);
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

      <div className="options">
        <button onClick={handleChooseDir} disabled={isProcessing}>
          📁 选择输出目录
        </button>
        {outputDir && <span className="dir-hint">{outputDir}</span>}
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
            <button className="link-btn" onClick={handleOpenFile}>
              打开文件
            </button>
            <span className="path">{status.outputPath}</span>
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
