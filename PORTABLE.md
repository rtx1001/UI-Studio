# UI Studio portable package

The initial portable package is a thin distribution containing the Windows application, its worker adapter, and the resource manifest. AI model weights and runtimes are intentionally not committed to Git or embedded in the ZIP.

## Package layout

```text
UI-Studio/
  UI-Studio.exe
  README.md
  PORTABLE.md
  tools/
    ui_studio_worker.py
  resources/
    resource-manifest.json
  runtime/                 downloaded later
  models/                  downloaded later
```

The resource modal detects missing components, resumes interrupted files with HTTP byte ranges, retries temporary network failures, shows byte-level progress, verifies declared sizes and SHA-256 hashes, and safely installs the split FLUX runtime plus model components. See `resources/FRESH-WINDOWS-READINESS.md` in the source project for the release-asset contract and clean-machine test procedure.

Before the resource page opens, UI Studio performs a local compatibility scan. Downloads remain disabled unless `nvidia-smi` confirms a CUDA-capable NVIDIA GPU and working display driver. A CUDA Toolkit installation is not required because the portable runtime contains the required CUDA-enabled PyTorch libraries.
