# Fresh Windows portable readiness

UI Studio itself is portable, but a clean-machine AI deployment has three layers:

1. **Application shell** — `UI-Studio.exe`, worker, and resource manifest.
2. **Downloadable AI resources** — portable FLUX runtime, FLUX.2 Klein model files, tokenizer, VAE, and optional DreamRoom LoRA.
3. **Machine prerequisites** — Microsoft Edge WebView2 Runtime and a compatible NVIDIA display driver. The app can detect and explain these prerequisites, but must not redistribute the NVIDIA driver.

## Release assets to prepare

Run `tools/prepare_resource_release.ps1` on the validated development machine. It creates:

- Split `UI-Studio-FLUX-Runtime-windows-x64.zip.partNN` assets, each below GitHub's 2 GiB per-file limit. Together they contain `runtime/diffsynth/python`, a self-contained Python 3.10 runtime, and the pinned DiffSynth package.
- `DreamRoom-Objects-v1.safetensors`, ready to install directly at `models/lora/DreamRoom-Objects-v1.safetensors`.
- `release-assets.json`, containing exact byte sizes and SHA-256 hashes.

The archives already contain their final portable-relative paths, so the downloader extracts them into the folder containing `UI-Studio.exe`.

The FLUX.2 Klein model remains sourced from its pinned Hugging Face revision in `resource-manifest.json`. Each downloaded file must be written to a temporary `.part` path, checked against its declared size and SHA-256 where available, and atomically renamed only after validation.

## Hosting

The public GitHub repository serves the split runtime and LoRA as release assets. GitHub limits each release asset to less than 2 GiB, so the runtime ZIP is split into numbered parts. The manifest pins every part by exact size and SHA-256. FLUX.2 Klein model files are fetched directly from a pinned Hugging Face revision.

## Implemented downloader behavior

- Scan GPU, NVIDIA driver/CUDA support, VRAM, system RAM, CPU, Windows version, and free disk space before showing downloads.
- Hard-block downloads when no CUDA-capable NVIDIA GPU and working NVIDIA driver are detected.
- Download individual resources or all missing resources in a group.
- Show exact individual and group download sizes.
- Write downloads to temporary `.part` paths.
- Resume interrupted `.part` downloads with validated HTTP byte ranges and automatic retry backoff.
- Show live downloaded/total bytes and whether a transfer resumed.
- Verify declared byte sizes and SHA-256 hashes before installation.
- Reassemble and verify the split runtime archive.
- Inspect archive paths, extract to staging, verify required runtime files, and replace the runtime only after validation.
- Refresh resource status automatically after installation.
- Preserve installed resources when the application executable is upgraded.
- Keep model transfers off the UI thread.

## Current limitations

- Active transfers do not yet have a manual pause or cancellation button. Closing the app preserves completed partial bytes for the next attempt.
- WebView2 installation remains a Windows prerequisite; current Windows 11 systems normally include it.

## Clean-machine release test

Test from a newly created Windows user or Windows Sandbox/VM with no Python, Git, CUDA toolkit, Visual Studio, or project folders installed. The only unavoidable machine-level dependencies should be WebView2 and the NVIDIA display driver. Confirm that the app starts with blank settings, downloads resources beside itself, survives interruption, runs a small FLUX job, preserves output paths and alpha, and starts again without re-downloading completed resources.
