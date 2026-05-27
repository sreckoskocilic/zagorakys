# Zagorakys

Converts comic archives to MOBI for Kindle or CBZ for Kobo. Can also just shrink file size without converting. Has a built-in previewer.

Supports CBR, CBZ, RAR, ZIP, and PDF files.

## Install (Windows)

1. Go to [Releases](https://github.com/sreckoskocilic/zagorakys/releases)
2. Download the `.exe` installer
3. Run it. Windows will show a SmartScreen warning since the app isn't signed — click "More info" then "Run anyway"

## How to use

1. Open the app
2. Pick a comic with **+ File**, or a folder with **+ Folder** for batch
3. Choose your device and quality from the toolbar (Kindle 4, Paperwhite, Oasis, Kobo Clara HD, Optimize, or PDF Optimize)
4. Hit convert
5. Output goes next to the original, or wherever you set the output folder

Drag and drop works too — files or folders, straight onto the window.

After conversion the file opens in the previewer. Flip through, make sure it looks right, then transfer to your device.

You can also open any `.mobi` or `.cbz` with **Open Book** to just preview without converting.

## Toolbar

- **Device** — your e-reader model, Optimize (reduce size), or PDF Optimize (reduce PDF size)
- **Quality** — JPEG quality preset: Low (10), Standard (20), High (40), Maximum (80)

## Settings

- **Output Folder** — where converted files go (default: same as input)
- **Enhance contrast** — boost contrast for e-ink
- **Don't split double pages** — keep two-page spreads as one page
- **Preserve color** — keep colors when optimizing (default: grayscale)
- **Skip already converted** — skip files that already have output in target folder
- **Skip low-res** — skip images below a minimum resolution
- **Hide Kindle frame** — hide the bezel in the previewer
- **Max image size** — resize limit when optimizing PDFs (No resize, 1024px, 1500px, 2048px)
- **Theme** — Ember, Jade, Iris, or Rose
