# UI Studio

Standalone Windows desktop workspace for inventorying and batch-reskinning Unity UI image assets while preserving exact dimensions, relative paths, filenames, extensions, and PNG transparency.

## Run

```powershell
npm install
npm run tauri dev
```

FLUX.2 Klein batch reskinning, weighted palette controls, Input/Output asset browsers, a built-in DreamRoom Objects LoRA toggle, and reproducible Style Lock profiles are available. See `ARCHITECTURE.md` for safety rules, model lifecycle, and current limitations.

## Portable distribution

`tools/build_portable.ps1` creates a thin Windows ZIP and SHA-256 checksum under `release/`. Models, training data, samples, and caches are deliberately excluded. On first use, the in-app Download button scans GPU, NVIDIA driver/CUDA support, VRAM, RAM, CPU, and free disk space before enabling any large resource download. The app then installs a verified portable FLUX runtime and pinned model files beside the executable. See `PORTABLE.md` and `resources/resource-manifest.json` for the package layout.
