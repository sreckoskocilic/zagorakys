import { useState, useEffect, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";

interface ConvertResult {
  mobi_path: string;
  mobi_size: string;
  input_size: string;
  title: string;
}

interface ConvertProgress {
  current: number;
  total: number;
  message: string;
}

interface MobiInfo {
  page_count: number;
  file_size: string;
}

interface MobiPage {
  image: string;
  page: number;
  page_count: number;
}

function App() {
  const [comicPath, setComicPath] = useState("");
  const [outputDir, setOutputDir] = useState("");
  const [quality] = useState(20);
  const [contrast, setContrast] = useState(false);
  const [converting, setConverting] = useState(false);
  const [progress, setProgress] = useState<ConvertProgress | null>(null);
  const [error, setError] = useState("");
  const [convertResult, setConvertResult] = useState<ConvertResult | null>(null);
  const [showSettings, setShowSettings] = useState(false);
  const [splitPages, setSplitPages] = useState(true);

  const [mobiPath, setMobiPath] = useState("");
  const [mobiInfo, setMobiInfo] = useState<MobiInfo | null>(null);
  const [currentPage, setCurrentPage] = useState(0);
  const [pageImage, setPageImage] = useState("");
  const [loadingPage, setLoadingPage] = useState(false);

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
      setConvertResult(null);
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
          split_pages: splitPages,
        },
      });
      setConvertResult(result);
      loadMobi(result.mobi_path);
    } catch (e) {
      setError(String(e));
    }
    setConverting(false);
    setProgress(null);
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

  const progressPercent =
    progress && progress.total > 0
      ? Math.round((progress.current / progress.total) * 100)
      : 0;

  return (
    <div className="app">
      <div className="sidebar">
        <div className="sidebar-actions">
          <button className="sidebar-btn" onClick={selectComic}>
            Select Comic
          </button>
          {comicPath && (
            <span className="selected-file">{fileName(comicPath)}</span>
          )}

          <button
            className="sidebar-btn primary"
            onClick={convert}
            disabled={!comicPath || converting}
          >
            {converting ? "Converting..." : "Convert to MOBI"}
          </button>

          {converting && progress && (
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

          {convertResult && (
            <div className="msg success">
              {convertResult.title}.mobi ({convertResult.mobi_size})
            </div>
          )}

          <div className="divider" />

          <button className="sidebar-btn" onClick={openMobi}>
            Open MOBI
          </button>
          {mobiPath && (
            <span className="selected-file">{fileName(mobiPath)}</span>
          )}
          {mobiInfo && (
            <span className="selected-file">
              {mobiInfo.page_count} pages &middot; {mobiInfo.file_size}
            </span>
          )}
        </div>

        {error && <div className="msg error">{error}</div>}

        <div className="sidebar-bottom">
          <button
            className={`settings-toggle ${showSettings ? "active" : ""}`}
            onClick={() => setShowSettings(!showSettings)}
          >
            ⚙ Settings
          </button>

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
                  checked={splitPages}
                  onChange={(e) => setSplitPages(e.target.checked)}
                />
                Split double pages
              </label>
            </div>
          )}
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
            </div>
            <div className="preview-image-container">
              <img
                src={pageImage}
                alt={`Page ${currentPage + 1}`}
                className={loadingPage ? "loading" : ""}
              />
            </div>
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
