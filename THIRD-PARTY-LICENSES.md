# Third-party licenses

Zagorakys bundles and builds on open-source software. The notices below cover
the bundled command-line binaries and the main libraries. Full per-dependency
license texts for Rust crates can be regenerated with `cargo about generate`.

---

## Bundled binaries

### MuPDF / mutool — AGPL-3.0-or-later

Zagorakys ships `mutool` (from MuPDF) on Windows and invokes it to render and
decrypt PDFs. MuPDF is © Artifex Software, Inc. and is licensed under the GNU
Affero General Public License, version 3 or later.

The full AGPL-3.0 text is included with this application (see `LICENSES/AGPL-3.0.txt`)
and is available at https://www.gnu.org/licenses/agpl-3.0.txt.

**Written offer of source.** The corresponding source for the MuPDF version
bundled here is available from https://mupdf.com/ and
https://git.ghostscript.com/?p=mupdf.git. The source for Zagorakys itself is at
https://github.com/sreckoskocilic/zagorakys.

A commercial MuPDF license (which removes the AGPL obligations) can be obtained
from Artifex Software, Inc.

### UnRAR — freeware with restrictions

Zagorakys ships `UnRAR` on Windows and invokes it to extract CBR/RAR archives.
UnRAR is © Alexander Roshal. Its license permits use and redistribution with the
following key restriction:

> The UnRAR sources may be used in any software to handle RAR archives without
> limitations free of charge, but cannot be used to develop the RAR (WinRAR)
> compatible archiver and to re-create the RAR compression algorithm, which is
> proprietary. Distribution of modified UnRAR sources in separate form or as a
> part of other software is permitted, provided that the full text of this
> paragraph, starting from "The UnRAR sources", is included.

Full license: https://www.rarlab.com/license.htm

---

## Libraries

### Rust (backend)

| Crate | License |
|-------|---------|
| tauri, tauri-plugin-* | MIT OR Apache-2.0 |
| lopdf | MIT |
| image | MIT OR Apache-2.0 |
| zip | MIT |
| rayon | MIT OR Apache-2.0 |
| walkdir | MIT OR Unlicense |
| tempfile | MIT OR Apache-2.0 |
| serde | MIT OR Apache-2.0 |
| base64 | MIT OR Apache-2.0 |
| kindling (kindling-mobi) | MIT — © 2026 Francisco Riordan |

### JavaScript (frontend)

| Package | License |
|---------|---------|
| react, react-dom | MIT |
| @tauri-apps/api, @tauri-apps/plugin-* | MIT OR Apache-2.0 |
| vite | MIT |
| typescript | Apache-2.0 |

MIT and Apache-2.0 require that their copyright notices and license text be
preserved; the full texts are distributed with each package and reproduced in
the generated `cargo about` output.
