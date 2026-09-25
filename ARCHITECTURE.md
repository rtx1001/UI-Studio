# UI Studio implementation plan

UI Studio is an independent Tauri application. It reuses only Dream Room Studio's proven shell choices: Tauri 2, Vite/TypeScript, native folder dialogs, persistent browser settings, dark desktop styling, and Windows subprocess hiding. It has its own package, Rust crate, application identifier, port, settings key, build directory, and executable.

## Safety invariants

- Source assets are read-only. Batch output is always rooted in a different selected folder.
- Every supported source path is normalized relative to the selected source root. Absolute paths and parent traversal are rejected.
- Output mirrors the relative path, filename, and extension exactly.
- Before a file is accepted, UI Studio decodes source and output and checks canvas width/height, format extension, and PNG alpha capability.
- Existing output files are not replaced unless the user explicitly enables overwrite.
- Inventory reports corrupt/unsupported files and case-insensitive relative-path collisions before processing.

## Vertical slice

1. Rust inventory recursively scans PNG/JPG/JPEG files and reads dimensions/alpha metadata.
2. The Explorer-like TypeScript UI provides resizable folder/settings panels, Input and Output browsers, folder navigation, breadcrumbs, search, filters, selection, responsive thumbnails, and full-screen zoom/pan preview.
3. Style, weighted palette, and FLUX LoRA settings persist independently from Dream Room Studio.
4. Checking an input asset immediately adds it to the queue. FLUX.2 Klein runs through an isolated worker with one model load per batch. Completed queue entries remain active repaint targets and are overwritten when the next batch runs.
5. Completed images appear in the Output browser using the same folder and thumbnail experience as Input. Clicking a completed queue entry opens a read-only before/after comparison; there is no approval or report workflow.

## FLUX integration

UI Studio uses FLUX.2 Klein 4B through the local DiffSynth pipeline. The trained `DreamRoom-Objects-v1` LoRA is a built-in option with only an on/off switch and strength control; users never select or edit its path. It remains off by default. The worker receives the original image plus style and weighted-palette direction, loads FLUX once per batch, and releases GPU memory afterward. Results are restored to the exact original canvas; PNGs receive the original alpha channel pixel-for-pixel so anti-aliased edges are not clipped. Rust independently decodes and validates every returned file before it appears in Output.

### Style Lock

Style Lock makes a batch reproducible by snapshotting one normalized style profile for every queued item. The profile fixes FLUX sampling parameters, style prompt, weighted palette, shared seed, and optional LoRA. Its fingerprint is shown in the UI and included in every result message. The palette editor always keeps dominance weights totaling 100%; changing one color redistributes the remaining percentage proportionally across the others.

The FLUX denoising slider defaults to `0.72`. Structure preservation independently controls how strongly the source image conditions the repaint, defaulting to `0.60`; lower values permit a stronger redesign without changing the exact output canvas contract. The palette is supplied both as text and as a generated weighted swatch reference so the selected colors are visual conditioning rather than hex text alone. Users can select 4, 8, or 12 inference steps, with 8 as the balanced default. Seed controls use explicit previous/next buttons and a cryptographically random 32-bit value for Randomize.

Style Lock improves consistency but cannot mathematically guarantee identical art direction from a generative model. Output should still be inspected, especially for text-bearing assets.

### Working resolution

Minimum working resolution can be switched on or off. When enabled, the user can select a 256, 512, 768, 1024, or 2048 px minimum tier: assets whose longest relevant side is smaller are enlarged, while larger assets are never reduced. When disabled, every source or content-aware crop stays at its natural size. In both modes, the model canvas is padded upward to safe multiples of 64 without scaling down the content, and the generated image is restored to the exact original dimensions before Rust validation. With content-aware fit enabled, “relevant side” means the visible alpha-bounded content rather than an oversized transparent canvas. Very large native assets can require substantially more VRAM and processing time.

Content-aware fit is enabled by default for transparent PNGs. UI Studio finds the visible alpha bounds, expands them by a 6% safety margin, and fits that region to the selected working tier. After generation it crops the fitted model region back into the same source coordinates and reapplies the complete original alpha channel. This gives small sprites with transparent padding substantially more useful model resolution without moving them on the original canvas. Opaque PNGs and JPG/JPEG files always use the full canvas to avoid damaging real backgrounds.

The application uses a Photoshop-like neutral-grey theme with an orange accent and fully custom-styled controls rather than native system widgets. The application shell itself never scrolls. Scrolling is isolated to the folder tree, asset grid, queue, and settings inspector, each with a custom dark scrollbar and contained overscroll behavior. The folder and settings panels have draggable splitters and persist their widths. The thick bottom status surface shows a full-width animated progress indicator while scans or model batches are running, then resolves to success or failure.

The next milestone is production hardening: download cancellation, text-bearing asset detection and warnings, retry seeds/variation controls, and broader evaluation on representative Unity folders.
