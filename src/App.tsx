import { useState, useEffect, useCallback, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { getCurrentWebview } from "@tauri-apps/api/webview";

interface ConvertResult {
  output_path: string;
  output_size: string;
  input_size: string;
  input_bytes: number;
  output_bytes: number;
  title: string;
  elapsed: string;
  skipped: boolean;
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

function App() {
  const [comicPath, setComicPath] = useState("");
  const [outputDir, setOutputDir] = useState("");
  const [quality, setQuality] = useState(() => {
    const v = localStorage.getItem("zagorakys-quality");
    return v ? Number(v) : 20;
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
  const cancelRef = useRef(false);

  const [mobiPath, setMobiPath] = useState("");
  const [mobiInfo, setMobiInfo] = useState<MobiInfo | null>(null);
  const [currentPage, setCurrentPage] = useState(0);
  const [pageImage, setPageImage] = useState("");
  const [loadingPage, setLoadingPage] = useState(false);
  const [zoom, setZoom] = useState(1);
  const [version, setVersion] = useState("");
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

  useEffect(() => {
    localStorage.setItem("zagorakys-device", device);
  }, [device]);

  useEffect(() => { localStorage.setItem("zagorakys-quality", String(quality)); }, [quality]);
  useEffect(() => { localStorage.setItem("zagorakys-contrast", String(contrast)); }, [contrast]);
  useEffect(() => { localStorage.setItem("zagorakys-nosplit", String(noSplit)); }, [noSplit]);
  useEffect(() => { localStorage.setItem("zagorakys-skip", String(skipExisting)); }, [skipExisting]);

  useEffect(() => {
    invoke<string>("get_version").then(setVersion);
  }, []);

  useEffect(() => {
    const unlisten = listen<ConvertProgress>("convert-progress", (event) => {
      setProgress(event.payload);
    });
    return () => {
      unlisten.then((f) => f());
    };
  }, []);

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
  }, [outputDir]);

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
          quality,
          contrast,
          no_split: noSplit,
          device,
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
            quality,
            contrast,
            no_split: noSplit,
            device,
            skip_existing: skipExisting,
          },
        });
        results.push(result);
        setBatchResults([...results]);
      } catch (e) {
        errors.push(fileName(batchFiles[i]));
        setBatchErrors([...errors]);
      }
    }
    const secs = (Date.now() - start) / 1000;
    setBatchElapsed(secs >= 60 ? `${Math.floor(secs / 60)}m ${(secs % 60).toFixed(1)}s` : `${secs.toFixed(1)}s`);
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

  const loadMobi = async (path: string) => {
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
  };

  const loadPage = useCallback(
    async (path: string, page: number) => {
      setLoadingPage(true);
      try {
        const result = await invoke<MobiPage>("get_mobi_page", { path, page });
        setPageImage(result.image);
        setCurrentPage(result.page);
      } catch (e) {
        setError(String(e));
      }
      setLoadingPage(false);
    },
    [],
  );

  const zoomIn = () => setZoom(z => Math.min(3, +(z + 0.25).toFixed(2)));
  const zoomOut = () => setZoom(z => Math.max(0.25, +(z - 0.25).toFixed(2)));

  const prevPage = () => {
    if (currentPage > 0) loadPage(mobiPath, currentPage - 1);
  };

  const nextPage = () => {
    if (mobiInfo && currentPage < mobiInfo.page_count - 1)
      loadPage(mobiPath, currentPage + 1);
  };

  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (e.key === "ArrowLeft") prevPage();
      if (e.key === "ArrowRight") nextPage();
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
    if (!converting) {
      if (isBatch) return `${device === "optimize" ? "Optimize" : "Convert"} ${batchFiles.length} Files`;
      if (device === "optimize") return "Optimize CBZ";
      return device.startsWith("kobo") ? "Convert to CBZ" : "Convert to MOBI";
    }
    if (isBatch) return `Converting ${batchIndex + 1}/${batchFiles.length}...`;
    return "Converting...";
  };

  const progressPercent =
    progress && progress.total > 0
      ? Math.round((progress.current / progress.total) * 100)
      : 0;

  return (
    <div className="app">
      <div className="sidebar">
        <div className="sidebar-actions">
          <button className="sidebar-btn" onClick={selectComic} disabled={converting}>
            Select File
          </button>

          <button className="sidebar-btn" onClick={selectFolder} disabled={converting}>
            Select Folder
          </button>

          {comicPath && !isBatch && (
            <div className="selected-file" title={comicPath}>{fileName(comicPath)}</div>
          )}

          {isBatch && !converting && !showBatchSummary && (
            <div className="batch-file-preview">
              <span className="batch-preview-label">{batchFiles.length} files</span>
              <div className="batch-file-list">
                {batchFiles.map((f, i) => (
                  <div key={i} className="batch-file-item">
                    <span className="batch-file-name" title={f}>{fileName(f)}</span>
                  </div>
                ))}
              </div>
            </div>
          )}

          <button
            className="sidebar-btn primary"
            onClick={handleConvert}
            disabled={!hasInput || converting}
          >
            {convertLabel()}
          </button>

          {converting && (
            <button className="sidebar-btn cancel-btn" onClick={cancelConvert}>
              Cancel
            </button>
          )}

          {converting && isBatch && (
            <div className="progress-section">
              <div className="progress-bar">
                <div
                  className="progress-fill"
                  style={{ width: `${Math.round(((batchIndex + (batchResults.length > batchIndex ? 1 : 0)) / batchFiles.length) * 100)}%` }}
                />
              </div>
              <p className="progress-text">{fileName(batchFiles[batchIndex])} ({batchIndex + 1}/{batchFiles.length})</p>
            </div>
          )}

          {converting && !isBatch && progress && (
            <div className="progress-section">
              <div className="progress-bar">
                <div
                  className="progress-fill"
                  style={{ width: `${progressPercent}%` }}
                />
              </div>
              <p className="progress-text">{progress.message}</p>
            </div>
          )}

          {showBatchSummary && !converting && (() => {
            const converted = batchResults.filter(r => !r.skipped);
            const skipped = batchResults.filter(r => r.skipped);
            return (
            <div className="batch-summary">
              <div className="batch-summary-header">
                <span className="batch-summary-title">Batch Complete</span>
                <button className="batch-summary-close" onClick={() => setShowBatchSummary(false)}>&times;</button>
              </div>
              <div className="batch-summary-stats">
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
                <div className="batch-stat">
                  <span className="batch-stat-value">{batchElapsed}</span>
                  <span className="batch-stat-label">elapsed</span>
                </div>
              </div>
              {converted.length > 0 && (
                <div className="batch-summary-sizes">
                  {formatBytes(converted.reduce((s, r) => s + r.input_bytes, 0))}
                  {" → "}
                  {formatBytes(converted.reduce((s, r) => s + r.output_bytes, 0))}
                </div>
              )}
              <div className="batch-file-list">
                {batchResults.map((r, i) => (
                  <div key={i} className={`batch-file-item ${r.skipped ? "batch-file-skip" : "batch-file-ok"}`}>
                    <span className="batch-file-icon">{r.skipped ? "–" : "✓"}</span>
                    <span className="batch-file-name" title={fileName(r.output_path)}>{r.title || fileName(r.output_path)}</span>
                    <span className="batch-file-size">{r.skipped ? "exists" : r.output_size}</span>
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
            );
          })()}

          <div className="divider" />

          <button className="sidebar-btn" onClick={openMobi}>
            Open Book
          </button>
        </div>

        {error && <div className="msg error">{error}</div>}

        <div className="sidebar-bottom">
          {showSettings && (
            <div className="settings-panel">
              <div className="setting-group">
                <span className="setting-group-label">Device</span>
                <select
                  className="theme-select"
                  value={device}
                  onChange={(e) => setDevice(e.target.value)}
                >
                  <option value="kindle4">Kindle 4 (600×800)</option>
                  <option value="kindle-paperwhite">Kindle Paperwhite (1072×1448)</option>
                  <option value="kindle-oasis">Kindle Oasis (1264×1680)</option>
                  <option value="kobo-clara-hd">Kobo Clara HD (1072×1448)</option>
                  <option value="optimize">Optimize (reduce size)</option>
                </select>
              </div>

              <div className="setting-group">
                <span className="setting-group-label">Conversion</span>
                <label className="checkbox-label">
                  Quality: {quality}
                  <input
                    type="range"
                    min={1}
                    max={100}
                    value={quality}
                    onChange={(e) => setQuality(Number(e.target.value))}
                  />
                </label>
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
                <label className="checkbox-label">
                  <input
                    type="checkbox"
                    checked={skipExisting}
                    onChange={(e) => setSkipExisting(e.target.checked)}
                  />
                  Skip already converted
                </label>
              </div>

              <div className="setting-group">
                <span className="setting-group-label">Output</span>
                <button className="setting-btn" onClick={selectOutputDir}>
                  Output Folder
                </button>
                <span className="filepath">
                  {outputDir || "Same as input"}
                </span>
              </div>

              <div className="setting-group">
                <span className="setting-group-label">Theme</span>
                <select
                  className="theme-select"
                  value={theme}
                  onChange={(e) => setTheme(e.target.value)}
                >
                  <option value="ember">Ember</option>
                  <option value="jade">Jade</option>
                  <option value="iris">Iris</option>
                  <option value="rose">Rose</option>
                </select>
              </div>

              {version && <span className="version-label">Zagorakys v{version}</span>}
            </div>
          )}
          <button
            className="sidebar-btn"
            onClick={() => setShowSettings(!showSettings)}
          >
            Settings
          </button>
        </div>
      </div>

      <div className="preview-pane">
        {pageImage ? (
          <>
            <div className="preview-nav">
              <button onClick={prevPage} disabled={currentPage === 0}>
                ◀
              </button>
              <span>
                {currentPage + 1} / {mobiInfo?.page_count ?? "?"}
              </span>
              <button
                onClick={nextPage}
                disabled={
                  !mobiInfo || currentPage >= mobiInfo.page_count - 1
                }
              >
                ▶
              </button>
              <span className="nav-separator">|</span>
              <button onClick={zoomOut} disabled={zoom <= 0.25}>−</button>
              <span className="zoom-label">{Math.round(zoom * 100)}%</span>
              <button onClick={zoomIn} disabled={zoom >= 3}>+</button>
            </div>
            <div className={`preview-image-container${zoom > 1 ? " zoomed" : ""}`}>
              <div className="kindle-frame" style={zoom !== 1 ? { transform: `scale(${zoom})`, transformOrigin: zoom > 1 ? '0 0' : 'center center' } : undefined}>
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
              </div>
            </div>
            {mobiInfo && (
              <div className="preview-status">
                <span className="status-title">{mobiInfo.title || fileName(mobiPath)}</span>
                {mobiInfo.author && mobiInfo.author !== "kindling" && <span className="status-author">{mobiInfo.author}</span>}
                <span className="status-meta">{mobiInfo.page_count} pages &middot; {mobiInfo.file_size}</span>
                {(convertResult?.elapsed || batchElapsed) && <span className="status-meta">{convertResult?.elapsed || batchElapsed}</span>}
              </div>
            )}
          </>
        ) : (
          <div className="preview-empty">
            <p>Drop files here or select from sidebar</p>
            <span className="preview-empty-hint">CBR, CBZ, RAR, ZIP, PDF</span>
          </div>
        )}
        {dragging && (
          <div className="drag-overlay">
            <div className="drag-overlay-content">Drop to convert</div>
          </div>
        )}
      </div>
    </div>
  );
}

function fileName(path: string): string {
  return path.split(/[/\\]/).pop() || path;
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
  return `${(bytes / 1024 / 1024 / 1024).toFixed(1)} GB`;
}

export default App;
