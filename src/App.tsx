import { useState, useEffect, useCallback, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { check } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";

interface ConvertResult {
  output_path: string;
  output_size: string;
  input_size: string;
  input_bytes: number;
  output_bytes: number;
  title: string;
  elapsed: string;
  skipped: boolean;
  skip_reason: string;
}

interface ConvertProgress {
  current: number;
  total: number;
  message: string;
}

interface MobiInfo {
  page_count: number;
  file_size: string;
  title: string;
  author: string;
}

interface MobiPage {
  image: string;
  page: number;
  page_count: number;
}

const QUALITY_PRESETS = [
  { value: 10, label: "Low (10)" },
  { value: 20, label: "Standard (20)" },
  { value: 40, label: "High (40)" },
  { value: 80, label: "Maximum (80)" },
];

function snapToPreset(n: number): number {
  return QUALITY_PRESETS.reduce((prev, curr) =>
    Math.abs(curr.value - n) < Math.abs(prev.value - n) ? curr : prev
  ).value;
}

function App() {
  const [comicPath, setComicPath] = useState("");
  const [outputDir, setOutputDir] = useState(() => localStorage.getItem("zagorakys-outputdir") || "");
  const [quality, setQuality] = useState(() => {
    const v = localStorage.getItem("zagorakys-quality");
    const n = v ? Number(v) : 20;
    return snapToPreset((!n || n < 1) ? 20 : n);
  });
  const [contrast, setContrast] = useState(() => localStorage.getItem("zagorakys-contrast") === "true");
  const [converting, setConverting] = useState(false);
  const [progress, setProgress] = useState<ConvertProgress | null>(null);
  const [error, setError] = useState("");
  const [convertResult, setConvertResult] = useState<ConvertResult | null>(null);
  const [showSettings, setShowSettings] = useState(false);
  const [noSplit, setNoSplit] = useState(() => localStorage.getItem("zagorakys-nosplit") === "true");
  const [device, setDevice] = useState(() => localStorage.getItem("zagorakys-device") || "kindle4");
  const [batchFiles, setBatchFiles] = useState<string[]>([]);
  const [batchIndex, setBatchIndex] = useState(0);
  const [batchResults, setBatchResults] = useState<ConvertResult[]>([]);
  const [batchErrors, setBatchErrors] = useState<string[]>([]);
  const [batchElapsed, setBatchElapsed] = useState("");
  const [showBatchSummary, setShowBatchSummary] = useState(false);
  const [dragging, setDragging] = useState(false);
  const [skipExisting, setSkipExisting] = useState(() => localStorage.getItem("zagorakys-skip") === "true");
  const [preserveColor, setPreserveColor] = useState(() => localStorage.getItem("zagorakys-preserve-color") === "true");
  const [minResolution, setMinResolution] = useState(() => {
    const v = localStorage.getItem("zagorakys-min-resolution");
    return v ? Number(v) : 0;
  });
  const [hideCover, setHideCover] = useState(() => localStorage.getItem("zagorakys-hidecover") === "true");
  const cancelRef = useRef(false);
  const pageRequestId = useRef(0);

  const [mobiPath, setMobiPath] = useState("");
  const [mobiInfo, setMobiInfo] = useState<MobiInfo | null>(null);
  const [currentPage, setCurrentPage] = useState(0);
  const [pageImage, setPageImage] = useState("");
  const [loadingPage, setLoadingPage] = useState(false);
  const [zoom, setZoom] = useState(1);
  const [version, setVersion] = useState("");
  const [updateAvailable, setUpdateAvailable] = useState<{ version: string; body: string } | null>(null);
  const [updating, setUpdating] = useState(false);
  const [theme, setTheme] = useState(() => {
    const saved = localStorage.getItem("zagorakys-theme");
    return ["ember", "jade", "iris", "rose"].includes(saved ?? "") ? saved! : "ember";
  });

  useEffect(() => {
    if (theme === "ember") {
      document.documentElement.removeAttribute("data-theme");
    } else {
      document.documentElement.setAttribute("data-theme", theme);
    }
    localStorage.setItem("zagorakys-theme", theme);
  }, [theme]);

  useEffect(() => { localStorage.setItem("zagorakys-device", device); }, [device]);
  useEffect(() => { localStorage.setItem("zagorakys-quality", String(quality)); }, [quality]);
  useEffect(() => { localStorage.setItem("zagorakys-contrast", String(contrast)); }, [contrast]);
  useEffect(() => { localStorage.setItem("zagorakys-nosplit", String(noSplit)); }, [noSplit]);
  useEffect(() => { localStorage.setItem("zagorakys-skip", String(skipExisting)); }, [skipExisting]);
  useEffect(() => { localStorage.setItem("zagorakys-preserve-color", String(preserveColor)); }, [preserveColor]);
  useEffect(() => { localStorage.setItem("zagorakys-min-resolution", String(minResolution)); }, [minResolution]);
  useEffect(() => { localStorage.setItem("zagorakys-hidecover", String(hideCover)); }, [hideCover]);
  useEffect(() => { localStorage.setItem("zagorakys-outputdir", outputDir); }, [outputDir]);

  useEffect(() => {
    invoke<string>("get_version").then(setVersion);
    check().then((update) => {
      if (update?.available) {
        setUpdateAvailable({ version: update.version, body: update.body ?? "" });
      }
    }).catch(() => {});
  }, []);

  useEffect(() => {
    const unlisten = listen<ConvertProgress>("convert-progress", (event) => {
      setProgress(event.payload);
    });
    return () => { unlisten.then((f) => f()); };
  }, []);

  const loadPage = useCallback(
    async (path: string, page: number) => {
      const reqId = ++pageRequestId.current;
      setLoadingPage(true);
      try {
        const result = await invoke<MobiPage>("get_mobi_page", { path, page });
        if (reqId !== pageRequestId.current) return;
        setPageImage(result.image);
        setCurrentPage(result.page);
      } catch (e) {
        if (reqId !== pageRequestId.current) return;
        setError(String(e));
      }
      setLoadingPage(false);
    },
    [],
  );

  const loadMobi = useCallback(async (path: string) => {
    setError("");
    setMobiPath(path);
    setPageImage("");
    setZoom(1);
    try {
      const info = await invoke<MobiInfo>("get_mobi_info", { path });
      setMobiInfo(info);
      setCurrentPage(0);
      loadPage(path, 0);
    } catch (e) {
      setError(String(e));
      setMobiInfo(null);
    }
  }, [loadPage]);

  useEffect(() => {
    const exts = ["cbr", "cbz", "rar", "zip", "pdf", "mobi"];
    const getExt = (p: string) => {
      const dot = p.lastIndexOf(".");
      const sep = Math.max(p.lastIndexOf("/"), p.lastIndexOf("\\"));
      return dot > sep ? p.slice(dot + 1).toLowerCase() : "";
    };
    const handleDrop = async (paths: string[]) => {
      if (!paths || paths.length === 0) return;
      if (paths.length === 1) {
        const isDir = await invoke<boolean>("check_is_dir", { path: paths[0] });
        if (isDir) {
          try {
            const comics = await invoke<string[]>("list_comics", { dir: paths[0] });
            if (comics.length === 0) {
              setError("No comic files found in folder");
              return;
            }
            setComicPath("");
            setConvertResult(null);
            setBatchFiles(comics);
            setBatchResults([]);
            setBatchErrors([]);
            setBatchIndex(0);
            setShowBatchSummary(false);
            setError("");
            if (!outputDir) setOutputDir(paths[0]);
          } catch (e) { setError(String(e)); }
          return;
        }
      }
      const file = paths[0];
      const ext = getExt(file);
      if (ext === "mobi") {
        loadMobi(file);
      } else if (exts.includes(ext)) {
        if (paths.length > 1) {
          const comics = paths.filter((p) => {
            const e = getExt(p);
            return exts.includes(e) && e !== "mobi";
          });
          if (comics.length === 0) return;
          setComicPath("");
          setConvertResult(null);
          setBatchFiles(comics.sort());
          setBatchResults([]);
          setBatchErrors([]);
          setBatchIndex(0);
          setShowBatchSummary(false);
          setError("");
        } else {
          setComicPath(file);
          setBatchFiles([]);
          setConvertResult(null);
          setBatchResults([]);
          setBatchErrors([]);
          setShowBatchSummary(false);
          setError("");
          const dir = file.replace(/[/\\][^/\\]+$/, "") || file;
          if (!outputDir) setOutputDir(dir);
        }
      }
    };
    const unlisten = getCurrentWebview().onDragDropEvent((event) => {
      if (event.payload.type === "enter") {
        setDragging(true);
      } else if (event.payload.type === "leave") {
        setDragging(false);
      } else if (event.payload.type === "drop") {
        setDragging(false);
        handleDrop(event.payload.paths);
      }
    });
    return () => { unlisten.then((f) => f()); };
  }, [outputDir, loadMobi]);

  const selectComic = async () => {
    const selected = await open({
      multiple: false,
      filters: [
        { name: "Comic Archives", extensions: ["cbr", "cbz", "rar", "zip", "pdf"] },
      ],
    });
    if (selected) {
      setComicPath(selected as string);
      setBatchFiles([]);
      setConvertResult(null);
      setBatchResults([]);
      setBatchErrors([]);
      setShowBatchSummary(false);
      setError("");
      const dir = (selected as string).replace(/[/\\][^/\\]+$/, "") || (selected as string);
      if (!outputDir) setOutputDir(dir);
    }
  };

  const selectOutputDir = async () => {
    const selected = await open({ directory: true });
    if (selected) setOutputDir(selected as string);
  };

  const convert = async () => {
    if (!comicPath) return;
    const dir = outputDir || comicPath.replace(/[/\\][^/\\]+$/, "") || comicPath;
    setConverting(true);
    setError("");
    setConvertResult(null);
    setProgress(null);
    try {
      const result = await invoke<ConvertResult>("convert_comic", {
        options: {
          input_path: comicPath,
          output_dir: dir,
          quality: typeof quality === "number" && quality >= 1 ? quality : 20,
          contrast,
          no_split: device === "optimize" ? true : noSplit,
          device,
          skip_existing: skipExisting,
          preserve_color: device === "optimize" ? preserveColor : false,
          min_resolution: minResolution,
        },
      });
      setConvertResult(result);
      if (result.output_path.endsWith(".mobi") || result.output_path.endsWith(".cbz")) {
        loadMobi(result.output_path);
      }
    } catch (e) {
      const msg = String(e);
      if (!msg.includes("Cancelled")) setError(msg);
    }
    setConverting(false);
    setProgress(null);
  };

  const selectFolder = async () => {
    const selected = await open({ directory: true });
    if (!selected) return;
    const comics = await invoke<string[]>("list_comics", { dir: selected as string });
    if (comics.length === 0) {
      setError("No comic files found in folder");
      return;
    }
    setComicPath("");
    setConvertResult(null);
    setBatchFiles(comics);
    setBatchResults([]);
    setBatchErrors([]);
    setBatchIndex(0);
    setShowBatchSummary(false);
    setError("");
    if (!outputDir) setOutputDir(selected as string);
  };

  const batchConvert = async () => {
    if (batchFiles.length === 0) return;
    const dir = outputDir || batchFiles[0].replace(/[/\\][^/\\]+$/, "") || batchFiles[0];
    setConverting(true);
    cancelRef.current = false;
    setError("");
    setBatchResults([]);
    setBatchErrors([]);
    setBatchElapsed("");
    setShowBatchSummary(false);
    const start = Date.now();
    const results: ConvertResult[] = [];
    const errors: string[] = [];
    for (let i = 0; i < batchFiles.length; i++) {
      if (cancelRef.current) break;
      setBatchIndex(i);
      setProgress({ current: i, total: batchFiles.length, message: `${fileName(batchFiles[i])} (${i + 1}/${batchFiles.length})` });
      try {
        const result = await invoke<ConvertResult>("convert_comic", {
          options: {
            input_path: batchFiles[i],
            output_dir: dir,
            quality: typeof quality === "number" && quality >= 1 ? quality : 20,
            contrast,
            no_split: device === "optimize" ? true : noSplit,
            device,
            skip_existing: skipExisting,
            preserve_color: device === "optimize" ? preserveColor : false,
            min_resolution: minResolution,
          },
        });
        results.push(result);
        setBatchResults([...results]);
      } catch (e) {
        if (!String(e).includes("Cancelled")) {
          errors.push(fileName(batchFiles[i]));
          setBatchErrors([...errors]);
        }
      }
    }
    const secs = (Date.now() - start) / 1000;
    const totalSecs = Math.max(1, Math.floor(secs));
    const mm = Math.floor(totalSecs / 60);
    const ss = totalSecs % 60;
    setBatchElapsed(`${mm}:${String(ss).padStart(2, "0")}`);
    setConverting(false);
    setProgress(null);
    if (results.length > 0 || errors.length > 0) {
      setShowBatchSummary(true);
    }
    if (results.length > 0) {
      const last = results[results.length - 1];
      if (!last.skipped && (last.output_path.endsWith(".mobi") || last.output_path.endsWith(".cbz"))) {
        loadMobi(last.output_path);
      }
    }
  };

  const cancelConvert = async () => {
    cancelRef.current = true;
    await invoke("cancel_convert");
  };

  const openMobi = async () => {
    const selected = await open({
      multiple: false,
      filters: [{ name: "Books", extensions: ["mobi", "cbz"] }],
    });
    if (selected) loadMobi(selected as string);
  };

  const removeBatchFile = (index: number) => {
    const newFiles = batchFiles.filter((_, i) => i !== index);
    setBatchFiles(newFiles);
    if (newFiles.length === 0) {
      setBatchResults([]);
      setBatchErrors([]);
    }
  };

  const zoomIn = () => setZoom(z => Math.min(3, +(z + 0.25).toFixed(2)));
  const zoomOut = () => setZoom(z => Math.max(0.25, +(z - 0.25).toFixed(2)));

  const prevPage = () => {
    if (currentPage > 0) loadPage(mobiPath, currentPage - 1);
  };

  const nextPage = () => {
    if (mobiInfo && currentPage < mobiInfo.page_count - 1)
      loadPage(mobiPath, currentPage + 1);
  };

  const firstPage = () => {
    if (currentPage !== 0) loadPage(mobiPath, 0);
  };

  const lastPage = () => {
    if (mobiInfo && currentPage !== mobiInfo.page_count - 1)
      loadPage(mobiPath, mobiInfo.page_count - 1);
  };

  const goToPage = (page: number) => {
    if (!mobiInfo) return;
    const clamped = Math.max(0, Math.min(page, mobiInfo.page_count - 1));
    if (clamped !== currentPage) loadPage(mobiPath, clamped);
  };

  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (e.key === "ArrowLeft") prevPage();
      if (e.key === "ArrowRight") nextPage();
      if (e.key === "Home") { e.preventDefault(); firstPage(); }
      if (e.key === "End") { e.preventDefault(); lastPage(); }
      if ((e.key === "=" || e.key === "+") && (e.metaKey || e.ctrlKey)) { e.preventDefault(); zoomIn(); }
      if (e.key === "-" && (e.metaKey || e.ctrlKey)) { e.preventDefault(); zoomOut(); }
      if (e.key === "0" && (e.metaKey || e.ctrlKey)) { e.preventDefault(); setZoom(1); }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [currentPage, mobiPath, mobiInfo, zoom]);

  const isBatch = batchFiles.length > 0;
  const hasInput = comicPath || isBatch;

  const handleConvert = () => {
    if (isBatch) batchConvert();
    else convert();
  };

  const convertLabel = () => {
    if (isBatch) return `${device === "optimize" ? "Optimize" : "Convert"} ${batchFiles.length} Files`;
    if (device === "optimize") return "Optimize";
    return device.startsWith("kobo") ? "Convert to CBZ" : "Convert to MOBI";
  };

  const progressPercent =
    progress && progress.total > 0
      ? Math.round((progress.current / progress.total) * 100)
      : 0;

  return (
    <div className="app">
      {error && (
        <div className="error-banner">
          <span>{error}</span>
          <button className="error-close" onClick={() => setError("")}>&times;</button>
        </div>
      )}

      <div className="toolbar">
        <span className="toolbar-label">Device</span>
        <select
          className="toolbar-select"
          value={device}
          onChange={(e) => setDevice(e.target.value)}
          disabled={converting}
        >
          <option value="kindle4">Kindle 4 (600×800)</option>
          <option value="kindle-paperwhite">Kindle Paperwhite (1072×1448)</option>
          <option value="kindle-oasis">Kindle Oasis (1264×1680)</option>
          <option value="kobo-clara-hd">Kobo Clara HD (1072×1448)</option>
          <option value="optimize">Optimize (reduce size)</option>
        </select>

        <div className="toolbar-div" />

        <span className="toolbar-label">Quality</span>
        <select
          className="toolbar-select"
          value={quality}
          onChange={(e) => setQuality(Number(e.target.value))}
          disabled={converting}
        >
          {QUALITY_PRESETS.map((p) => (
            <option key={p.value} value={p.value}>{p.label}</option>
          ))}
        </select>

        <div className="toolbar-spacer" />

        {converting ? (
          <button className="btn-cancel" onClick={cancelConvert}>Cancel</button>
        ) : (
          <button className="btn-convert" onClick={handleConvert} disabled={!hasInput}>
            {convertLabel()}
          </button>
        )}

        <div className="toolbar-div" />

        <button className="toolbar-btn" onClick={openMobi} disabled={converting}>
          Open Book
        </button>
        <button
          className={`toolbar-btn gear${showSettings ? " active" : ""}`}
          onClick={() => setShowSettings(!showSettings)}
        >
          &#9881;
        </button>
      </div>

      {converting && progress && !isBatch && (
        <div className="progress-strip">
          <div className="progress-bar">
            <div className="progress-fill" style={{ width: `${progressPercent}%` }} />
          </div>
          <span className="progress-text">{progress.message}</span>
        </div>
      )}

      {converting && isBatch && (
        <div className="progress-strip">
          <div className="progress-bar">
            <div
              className="progress-fill"
              style={{ width: `${Math.round(((batchIndex + (batchResults.length > batchIndex ? 1 : 0)) / batchFiles.length) * 100)}%` }}
            />
          </div>
          <span className="progress-text">
            {fileName(batchFiles[batchIndex])} ({batchIndex + 1}/{batchFiles.length})
          </span>
        </div>
      )}

      <div className="main">
        <div className="file-panel">
          <div className="fpanel-head">
            <span className="fpanel-title">Queue</span>
            {(isBatch || comicPath) && (
              <span className="fpanel-count">
                {isBatch ? `${batchFiles.length} files` : "1 file"}
              </span>
            )}
          </div>

          <div className="fpanel-actions">
            <button className="fpanel-btn" onClick={selectComic} disabled={converting}>
              + File
            </button>
            <button className="fpanel-btn" onClick={selectFolder} disabled={converting}>
              + Folder
            </button>
          </div>

          {showBatchSummary && !converting ? (
            <div className="batch-summary">
              <div className="batch-summary-header">
                <span className="batch-summary-title">Done in {batchElapsed}</span>
                <button className="batch-summary-close" onClick={() => setShowBatchSummary(false)}>
                  &times;
                </button>
              </div>
              <div className="batch-summary-stats">
                {(() => {
                  const converted = batchResults.filter((r) => !r.skipped);
                  const skipped = batchResults.filter((r) => r.skipped);
                  return (
                    <>
                      <div className="batch-stat">
                        <span className="batch-stat-value">{converted.length}</span>
                        <span className="batch-stat-label">converted</span>
                      </div>
                      {skipped.length > 0 && (
                        <div className="batch-stat batch-stat-skip">
                          <span className="batch-stat-value">{skipped.length}</span>
                          <span className="batch-stat-label">skipped</span>
                        </div>
                      )}
                      {batchErrors.length > 0 && (
                        <div className="batch-stat batch-stat-error">
                          <span className="batch-stat-value">{batchErrors.length}</span>
                          <span className="batch-stat-label">failed</span>
                        </div>
                      )}
                      {converted.length > 0 && (
                        <div className="batch-stat">
                          <span className="batch-stat-value">
                            {formatBytes(converted.reduce((s, r) => s + r.input_bytes, 0))} →{" "}
                            {formatBytes(converted.reduce((s, r) => s + r.output_bytes, 0))}
                          </span>
                          <span className="batch-stat-label">compression</span>
                        </div>
                      )}
                    </>
                  );
                })()}
              </div>
              <div className="batch-file-list">
                {batchResults.map((r, i) => (
                  <div
                    key={i}
                    className={`batch-file-item ${r.skipped ? (r.skip_reason.startsWith("low res") ? "batch-file-lowres" : "batch-file-skip") : "batch-file-ok"}`}
                  >
                    <span className="batch-file-icon">{r.skipped ? "–" : "✓"}</span>
                    <span className="batch-file-name" title={fileName(r.output_path)}>
                      {r.title || fileName(r.output_path)}
                    </span>
                    <span className="batch-file-size">
                      {r.skipped ? r.skip_reason || "exists" : r.output_size}
                    </span>
                  </div>
                ))}
                {batchErrors.map((name, i) => (
                  <div key={`err-${i}`} className="batch-file-item batch-file-fail">
                    <span className="batch-file-icon">&#10007;</span>
                    <span className="batch-file-name" title={name}>{name}</span>
                  </div>
                ))}
              </div>
            </div>
          ) : (
            <div className="file-list">
              {comicPath && !isBatch && (
                <div className="file-item active">
                  <span className="file-item-pre cur">&#9654;</span>
                  <span className="file-item-name" title={comicPath}>
                    {fileName(comicPath)}
                  </span>
                </div>
              )}
              {isBatch &&
                batchFiles.map((f, i) => (
                  <div
                    key={i}
                    className={`file-item${converting && i === batchIndex ? " active" : ""}`}
                  >
                    <span className={`file-item-pre${converting && i === batchIndex ? " cur" : ""}`}>
                      {converting && i === batchIndex ? "▶" : "  "}
                    </span>
                    <span className="file-item-name" title={f}>
                      {fileName(f)}
                    </span>
                    {!converting && (
                      <button className="file-item-x" onClick={() => removeBatchFile(i)}>
                        &#10005;
                      </button>
                    )}
                  </div>
                ))}
            </div>
          )}

          <div className="drop-zone">
            <div className="dz-icon">&#8681;</div>
            <div className="dz-text">Drop files or folders</div>
            <div className="dz-hint">CBR CBZ RAR ZIP PDF</div>
          </div>
        </div>

        <div className="preview-pane">
          {pageImage ? (
            <>
              <div className="preview-nav">
                <button onClick={firstPage} disabled={currentPage === 0} title="First page">
                  &#9198;
                </button>
                <button onClick={prevPage} disabled={currentPage === 0}>&#9664;</button>
                <input
                  type="number"
                  className="page-input"
                  min={1}
                  max={mobiInfo?.page_count ?? 1}
                  value={currentPage + 1}
                  onChange={(e) => {
                    const val = parseInt(e.target.value, 10);
                    if (!isNaN(val)) goToPage(val - 1);
                  }}
                />
                <span>/ {mobiInfo?.page_count ?? "?"}</span>
                <button
                  onClick={nextPage}
                  disabled={!mobiInfo || currentPage >= mobiInfo.page_count - 1}
                >
                  &#9654;
                </button>
                <button
                  onClick={lastPage}
                  disabled={!mobiInfo || currentPage >= mobiInfo.page_count - 1}
                  title="Last page"
                >
                  &#9197;
                </button>
                <span className="nav-separator">|</span>
                <button onClick={zoomOut} disabled={zoom <= 0.25}>&minus;</button>
                <input
                  type="number"
                  className="zoom-input"
                  min={25}
                  max={300}
                  step={25}
                  value={Math.round(zoom * 100)}
                  onChange={(e) => {
                    const val = parseInt(e.target.value, 10);
                    if (!isNaN(val)) setZoom(Math.max(0.25, Math.min(3, val / 100)));
                  }}
                />
                <span className="zoom-pct">%</span>
                <button onClick={zoomIn} disabled={zoom >= 3}>+</button>
              </div>
              <div className={`preview-image-container${zoom > 1 ? " zoomed" : ""}`}>
                <div
                  className="kindle-frame"
                  style={
                    zoom !== 1
                      ? {
                          transform: `scale(${zoom})`,
                          transformOrigin: zoom > 1 ? "0 0" : "center center",
                        }
                      : undefined
                  }
                >
                  {hideCover ? (
                    <img
                      src={pageImage}
                      alt={`Page ${currentPage + 1}`}
                      className={`preview-bare${loadingPage ? " loading" : ""}`}
                    />
                  ) : (
                    <div className="kindle-bezel">
                      <span className="kindle-label">Kindle</span>
                      <div className="kindle-screen">
                        <img
                          src={pageImage}
                          alt={`Page ${currentPage + 1}`}
                          className={loadingPage ? "loading" : ""}
                        />
                      </div>
                    </div>
                  )}
                </div>
              </div>
            </>
          ) : (
            <div className="preview-empty">
              <p>Drop files here or select from sidebar</p>
              <span className="preview-empty-hint">CBR, CBZ, RAR, ZIP, PDF</span>
            </div>
          )}

          {showSettings && (
            <div className="settings-slide">
              <div className="settings-header">
                <span className="settings-title">Settings</span>
                <button className="settings-close" onClick={() => setShowSettings(false)}>
                  &times;
                </button>
              </div>
              <div className="settings-body">
                <div className="setting-group">
                  <span className="setting-group-label">Conversion</span>
                  <label className="checkbox-label">
                    <input
                      type="checkbox"
                      checked={contrast}
                      onChange={(e) => setContrast(e.target.checked)}
                    />
                    Enhance contrast
                  </label>
                  {!device.startsWith("kobo") && device !== "optimize" && (
                    <label className="checkbox-label">
                      <input
                        type="checkbox"
                        checked={noSplit}
                        onChange={(e) => setNoSplit(e.target.checked)}
                      />
                      Don't split double pages
                    </label>
                  )}
                  {device === "optimize" && (
                    <label className="checkbox-label">
                      <input
                        type="checkbox"
                        checked={preserveColor}
                        onChange={(e) => setPreserveColor(e.target.checked)}
                      />
                      Preserve color
                    </label>
                  )}
                  <label className="checkbox-label">
                    <input
                      type="checkbox"
                      checked={skipExisting}
                      onChange={(e) => setSkipExisting(e.target.checked)}
                    />
                    Skip already converted
                  </label>
                  <label className="checkbox-label">
                    <input
                      type="checkbox"
                      checked={hideCover}
                      onChange={(e) => setHideCover(e.target.checked)}
                    />
                    Hide Kindle frame
                  </label>
                </div>

                <div className="setting-group">
                  <span className="setting-group-label">Skip low-res</span>
                  <select
                    className="setting-select"
                    value={minResolution}
                    onChange={(e) => setMinResolution(Number(e.target.value))}
                  >
                    <option value={0}>Off</option>
                    <option value={600}>600px (web scans)</option>
                    <option value={800}>800px (low quality)</option>
                    <option value={1000}>1000px (below HD)</option>
                    <option value={1200}>1200px (below Kindle PW)</option>
                  </select>
                </div>

                <div className="setting-group">
                  <span className="setting-group-label">Output</span>
                  <button className="setting-btn" onClick={selectOutputDir}>
                    Choose Folder
                  </button>
                  <span className="setting-path">{outputDir || "Same as input"}</span>
                </div>

                <div className="setting-group">
                  <span className="setting-group-label">Theme</span>
                  <select
                    className="setting-select"
                    value={theme}
                    onChange={(e) => setTheme(e.target.value)}
                  >
                    <option value="ember">Ember</option>
                    <option value="jade">Jade</option>
                    <option value="iris">Iris</option>
                    <option value="rose">Rose</option>
                  </select>
                </div>

                {version && (
                  <div className="version-section">
                    <span className="version-label">Zagorakys v{version}</span>
                    {updateAvailable && (
                      <button
                        className="update-btn"
                        disabled={updating}
                        onClick={async () => {
                          setUpdating(true);
                          try {
                            const update = await check();
                            if (update?.available) {
                              await update.downloadAndInstall();
                              await relaunch();
                            }
                          } catch (e) {
                            setError(String(e));
                            setUpdating(false);
                          }
                        }}
                      >
                        {updating ? "Updating..." : `Update to v${updateAvailable.version}`}
                      </button>
                    )}
                  </div>
                )}
              </div>
            </div>
          )}
        </div>
      </div>

      <div className="status-bar">
        {mobiInfo ? (
          <>
            <span className="status-item">
              <strong>{shortPath(convertResult?.output_path || mobiPath)}</strong>
            </span>
            <span className="status-sep">|</span>
            {convertResult && !convertResult.skipped && !isBatch && convertResult.input_bytes > 0 ? (
              <>
                <span className="status-item">
                  {convertResult.input_size} → {convertResult.output_size} (
                  {convertResult.output_bytes <= convertResult.input_bytes ? "−" : "+"}
                  {Math.abs(
                    Math.round((1 - convertResult.output_bytes / convertResult.input_bytes) * 100)
                  )}
                  %)
                </span>
                <span className="status-sep">|</span>
                <span className="status-item">{mobiInfo.page_count} pages</span>
                <span className="status-sep">|</span>
                <span className="status-item">{convertResult.elapsed}</span>
              </>
            ) : (
              <>
                <span className="status-item">{mobiInfo.file_size}</span>
                <span className="status-sep">|</span>
                <span className="status-item">{mobiInfo.page_count} pages</span>
              </>
            )}
          </>
        ) : (comicPath || isBatch) ? (
          <>
            <span className="status-item">
              <strong>
                {isBatch
                  ? shortPath(batchFiles[0].replace(/[/\\][^/\\]+$/, ""))
                  : shortPath(comicPath)}
              </strong>
            </span>
            {isBatch && (
              <>
                <span className="status-sep">|</span>
                <span className="status-item">{batchFiles.length} files</span>
              </>
            )}
          </>
        ) : null}
      </div>

      {dragging && (
        <div className="drag-overlay">
          <div className="drag-overlay-content">Drop to convert</div>
        </div>
      )}
    </div>
  );
}

function fileName(path: string): string {
  return path.split(/[/\\]/).pop() || path;
}

function shortPath(path: string): string {
  const home = path.match(/^(\/Users\/[^/]+|\/home\/[^/]+|[A-Za-z]:\\Users\\[^\\]+)/)?.[1];
  return home ? "~" + path.slice(home.length) : path;
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${Math.round(bytes / 1024 / 1024)} MB`;
  return `${(bytes / 1024 / 1024 / 1024).toFixed(1)} GB`;
}

export default App;
