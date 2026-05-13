import { useState, useEffect, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";

interface ConvertResult {
  output_path: string;
  output_size: string;
  input_size: string;
  title: string;
  elapsed: string;
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
  const [quality, setQuality] = useState(20);
  const [contrast, setContrast] = useState(false);
  const [converting, setConverting] = useState(false);
  const [progress, setProgress] = useState<ConvertProgress | null>(null);
  const [error, setError] = useState("");
  const [convertResult, setConvertResult] = useState<ConvertResult | null>(null);
  const [showSettings, setShowSettings] = useState(false);
  const [noSplit, setNoSplit] = useState(false);
  const [device, setDevice] = useState(() => localStorage.getItem("zagorakys-device") || "kindle4");
  const [batchFiles, setBatchFiles] = useState<string[]>([]);
  const [batchIndex, setBatchIndex] = useState(0);
  const [batchResults, setBatchResults] = useState<ConvertResult[]>([]);
  const [batchElapsed, setBatchElapsed] = useState("");

  const [mobiPath, setMobiPath] = useState("");
  const [mobiInfo, setMobiInfo] = useState<MobiInfo | null>(null);
  const [currentPage, setCurrentPage] = useState(0);
  const [pageImage, setPageImage] = useState("");
  const [loadingPage, setLoadingPage] = useState(false);
  const [zoom, setZoom] = useState(1);
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

  useEffect(() => {
    const unlisten = listen<ConvertProgress>("convert-progress", (event) => {
      setProgress(event.payload);
    });
    return () => {
      unlisten.then((f) => f());
    };
  }, []);

  const selectComic = async () => {
    const selected = await open({
      multiple: false,
      filters: [
        { name: "Comic Archives", extensions: ["cbr", "cbz", "rar", "zip"] },
      ],
    });
    if (selected) {
      setComicPath(selected as string);
      setBatchFiles([]);
      setConvertResult(null);
      setBatchResults([]);
      setError("");
      const dir = (selected as string).replace(/[/\\][^/\\]+$/, "");
      if (!outputDir) setOutputDir(dir);
    }
  };

  const selectOutputDir = async () => {
    const selected = await open({ directory: true });
    if (selected) setOutputDir(selected as string);
  };

  const convert = async () => {
    if (!comicPath) return;
    const dir = outputDir || comicPath.replace(/[/\\][^/\\]+$/, "");
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
      if (result.output_path.endsWith(".mobi")) {
        loadMobi(result.output_path);
      }
    } catch (e) {
      setError(String(e));
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
    setBatchIndex(0);
    setError("");
    if (!outputDir) setOutputDir(selected as string);
  };

  const batchConvert = async () => {
    if (batchFiles.length === 0) return;
    const dir = outputDir || batchFiles[0].replace(/[/\\][^/\\]+$/, "");
    setConverting(true);
    setError("");
    setBatchResults([]);
    setBatchElapsed("");
    const start = Date.now();
    const results: ConvertResult[] = [];
    for (let i = 0; i < batchFiles.length; i++) {
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
          },
        });
        results.push(result);
        setBatchResults([...results]);
      } catch (e) {
        setError(`${fileName(batchFiles[i])}: ${e}`);
      }
    }
    const secs = (Date.now() - start) / 1000;
    setBatchElapsed(secs >= 60 ? `${Math.floor(secs / 60)}m ${(secs % 60).toFixed(1)}s` : `${secs.toFixed(1)}s`);
    setConverting(false);
    setProgress(null);
    if (results.length > 0) {
      const last = results[results.length - 1];
      if (last.output_path.endsWith(".mobi")) {
        loadMobi(last.output_path);
      }
    }
  };

  const openMobi = async () => {
    const selected = await open({
      multiple: false,
      filters: [{ name: "MOBI files", extensions: ["mobi"] }],
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
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  });

  const isBatch = batchFiles.length > 0;
  const hasInput = comicPath || isBatch;

  const handleConvert = () => {
    if (isBatch) batchConvert();
    else convert();
  };

  const convertLabel = () => {
    if (!converting) {
      if (isBatch) return `Convert ${batchFiles.length} Files`;
      return "Convert to MOBI";
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
            Select Comic
          </button>

          <button className="sidebar-btn" onClick={selectFolder} disabled={converting}>
            Select Folder
          </button>

          <button
            className="sidebar-btn primary"
            onClick={handleConvert}
            disabled={!hasInput || converting}
          >
            {convertLabel()}
          </button>

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

          <div className="divider" />

          <button className="sidebar-btn" onClick={openMobi}>
            Open MOBI
          </button>
        </div>

        {error && <div className="msg error">{error}</div>}

        <div className="sidebar-bottom">
          {showSettings && (
            <div className="settings-panel">
              <div className="setting-row">
                <button className="setting-btn" onClick={selectOutputDir}>
                  Output Folder
                </button>
                <span className="filepath">
                  {outputDir || "Same as input"}
                </span>
              </div>

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

              <label className="checkbox-label">
                <input
                  type="checkbox"
                  checked={noSplit}
                  onChange={(e) => setNoSplit(e.target.checked)}
                />
                Don't split double pages
              </label>

              <div className="setting-row">
                <select
                  className="theme-select"
                  value={device}
                  onChange={(e) => setDevice(e.target.value)}
                >
                  <option value="kindle4">Kindle 4 (600×800)</option>
                  <option value="kindle-paperwhite">Kindle Paperwhite (1072×1448)</option>
                  <option value="kindle-oasis">Kindle Oasis (1264×1680)</option>
                  <option value="kobo-clara-hd">Kobo Clara HD (1072×1448)</option>
                </select>
              </div>

              <div className="setting-row">
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
            <div
              className="preview-image-container"
              style={zoom > 1 ? { overflow: 'auto', alignItems: 'flex-start', justifyContent: 'flex-start' } : undefined}
            >
              <div className="kindle-frame">
                <div className="kindle-bezel">
                  <span className="kindle-label">Kindle</span>
                  <div className="kindle-screen">
                    <img
                      src={pageImage}
                      alt={`Page ${currentPage + 1}`}
                      className={loadingPage ? "loading" : ""}
                      style={zoom !== 1 ? {
                        maxWidth: 'none',
                        maxHeight: 'none',
                        width: `${zoom * 100}%`,
                      } : undefined}
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
            <p>Convert a comic or open a .mobi file</p>
          </div>
        )}
      </div>
    </div>
  );
}

function fileName(path: string): string {
  return path.split(/[/\\]/).pop() || path;
}

export default App;
