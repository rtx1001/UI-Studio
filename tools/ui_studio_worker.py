from __future__ import annotations

import argparse
import contextlib
import gc
import json
import os
import re
import sys
import time
import zlib
from pathlib import Path
from typing import Any

from PIL import Image, ImageDraw, ImageFilter, ImageOps, ImageStat


PORTABLE_ROOT_VALUE = os.environ.get("UI_STUDIO_PORTABLE_ROOT")
PORTABLE_ROOT = Path(PORTABLE_ROOT_VALUE).resolve() if PORTABLE_ROOT_VALUE else None
DREAMY_ROOT = PORTABLE_ROOT if PORTABLE_ROOT else Path(__file__).resolve().parents[2]
RESAMPLE = Image.Resampling.LANCZOS
FLUX_MODEL_ROOT = DREAMY_ROOT / "models" / "FLUX.2-klein-4B"
DEFAULT_FLUX_LORA = (PORTABLE_ROOT / "models" / "lora" / "DreamRoom-Objects-v1.safetensors") if PORTABLE_ROOT else DREAMY_ROOT / "training" / "runs" / "dreamroom-object-detail-full-v3-flux2-klein-4b-r40-768" / "DreamRoom-Objects-v1.safetensors"
FLUX_RUNTIME = (PORTABLE_ROOT / "runtime" / "diffsynth") if PORTABLE_ROOT else DREAMY_ROOT / "external" / "DiffSynth-Studio"
FLUX_PYTHON = (FLUX_RUNTIME / "python" / "python.exe") if PORTABLE_ROOT else (FLUX_RUNTIME / ".venv" / "Scripts" / "python.exe")


def generation_layout(width: int, height: int, target: int = 1024, minimum_enabled: bool = True) -> tuple[tuple[int, int], tuple[int, int, int, int]]:
    if target not in {256, 512, 768, 1024, 2048}:
        raise ValueError("Working resolution must be 256, 512, 768, 1024, or 2048")
    # The selected tier is a minimum, never a downscale target. Large assets
    # retain their native working dimensions; only smaller assets are enlarged.
    scale = max(1.0, target / max(1, width, height)) if minimum_enabled else 1.0
    content_width = max(1, round(width * scale))
    content_height = max(1, round(height * scale))
    canvas_width = max(64, ((content_width + 63) // 64) * 64)
    canvas_height = max(64, ((content_height + 63) // 64) * 64)
    left = (canvas_width - content_width) // 2
    top = (canvas_height - content_height) // 2
    return (canvas_width, canvas_height), (left, top, left + content_width, top + content_height)


def source_content_box(source: Image.Image, enabled: bool) -> tuple[int, int, int, int]:
    full = (0, 0, source.width, source.height)
    source_has_alpha = "A" in source.getbands() or "transparency" in source.info
    if not enabled or not source_has_alpha:
        return full
    alpha = source.convert("RGBA").getchannel("A")
    visible = alpha.point(lambda value: 255 if value > 2 else 0)
    bounds = visible.getbbox()
    if not bounds:
        return full
    left, top, right, bottom = bounds
    content_width, content_height = right - left, bottom - top
    margin = max(2, round(max(content_width, content_height) * 0.06))
    expanded = (
        max(0, left - margin), max(0, top - margin),
        min(source.width, right + margin), min(source.height, bottom + margin),
    )
    expanded_area = (expanded[2] - expanded[0]) * (expanded[3] - expanded[1])
    return full if expanded_area >= source.width * source.height * 0.98 else expanded


def matte_color(image: Image.Image) -> tuple[int, int, int]:
    rgba = image.convert("RGBA")
    alpha = rgba.getchannel("A")
    if not alpha.getbbox():
        return 32, 32, 32
    rgb = Image.new("RGB", rgba.size, (0, 0, 0))
    rgb.paste(rgba.convert("RGB"), mask=alpha)
    mean = ImageStat.Stat(rgb, mask=alpha).mean
    luminance = sum(mean) / 3
    return (235, 235, 235) if luminance < 115 else (24, 24, 24)


def reference_image(source: Image.Image, size: tuple[int, int], content_box: tuple[int, int, int, int]) -> Image.Image:
    rgba = source.convert("RGBA")
    color = matte_color(rgba)
    source_matte = Image.new("RGBA", rgba.size, (*color, 255))
    source_matte.alpha_composite(rgba)
    left, top, right, bottom = content_box
    fitted = source_matte.convert("RGB").resize((right - left, bottom - top), RESAMPLE)
    canvas = Image.new("RGB", size, color)
    canvas.paste(fitted, (left, top))
    return canvas


def palette_reference(palette: str, width: int = 512, height: int = 128) -> Image.Image | None:
    entries = [(match.group(1), max(0.0, float(match.group(2)))) for match in re.finditer(r"(#[0-9a-fA-F]{6})\s+(\d+(?:\.\d+)?)%", palette)]
    if not entries:
        return None
    total = sum(weight for _, weight in entries) or float(len(entries))
    swatch = Image.new("RGB", (width, height))
    draw = ImageDraw.Draw(swatch)
    left = 0
    for index, (hex_color, weight) in enumerate(entries):
        right = width if index == len(entries) - 1 else round(left + width * weight / total)
        draw.rectangle((left, 0, max(left, right), height), fill=hex_color)
        left = right
    return swatch


def structure_reference(reference: Image.Image, preservation: float, remove_source_color: bool) -> Image.Image | None:
    preservation = max(0.0, min(1.0, preservation))
    if preservation <= 0.01:
        return None
    conditioned = reference.convert("RGB")
    if remove_source_color:
        conditioned = ImageOps.grayscale(conditioned).convert("RGB")
    blur_radius = (1.0 - preservation) * max(1.0, min(conditioned.size) * 0.018)
    if blur_radius > 0.25:
        conditioned = conditioned.filter(ImageFilter.GaussianBlur(blur_radius))
    return conditioned


def prompt_for(job: dict[str, Any]) -> str:
    style = " ".join(str(job.get("stylePrompt") or "").split())
    palette = " ".join(str(job.get("palette") or "").split())
    structure = max(0.0, min(1.0, float(job.get("structurePreservation", 0.6))))
    structure_direction = "Preserve the precise silhouette, proportions, placement, borders, and empty space." if structure >= 0.8 else "Keep the recognizable composition and placement, while allowing a visible redesign of internal shapes and surface details." if structure >= 0.4 else "Keep only the broad subject and arrangement; allow substantial redesign inside the original canvas."
    parts = [
        "Repaint this game UI asset as a clearly transformed new artwork, not a near-copy or simple color filter.",
        structure_direction,
        "Visibly change materials, brushwork, edge treatment, lighting, shading, texture, and color relationships. Do not crop, rotate, add text, or place effects outside the original canvas.",
    ]
    if style:
        parts.append(f"Art style: {style}.")
    if palette:
        parts.append(f"Use the supplied palette swatch reference as the actual target colors, with this desired dominance: {palette}. Replace the source color scheme rather than tinting it.")
    if job.get("styleLockEnabled"):
        parts.append("STYLE LOCK: use identical brushwork, edge treatment, material language, lighting direction, contrast, and palette behavior across every image in this batch.")
    return " ".join(parts)


def safe_path(root: Path, relative: str) -> Path:
    path = Path(relative)
    if path.is_absolute() or ".." in path.parts:
        raise ValueError(f"Unsafe relative path: {relative}")
    resolved = (root / path).resolve()
    if root.resolve() not in resolved.parents:
        raise ValueError(f"Path escapes root: {relative}")
    return resolved


def save_result(
    raw_bytes: bytes, source: Image.Image, destination: Path,
    generation_content_box: tuple[int, int, int, int],
    source_content_box_value: tuple[int, int, int, int],
) -> None:
    from io import BytesIO
    left, top, right, bottom = source_content_box_value
    generated_crop = Image.open(BytesIO(raw_bytes)).convert("RGB").crop(generation_content_box).resize((right - left, bottom - top), RESAMPLE)
    generated = source.convert("RGB")
    generated.paste(generated_crop, (left, top))
    destination.parent.mkdir(parents=True, exist_ok=True)
    extension = destination.suffix.lower()
    if extension == ".png":
        source_has_alpha = "A" in source.getbands() or "transparency" in source.info
        if source_has_alpha:
            original = source.convert("RGBA")
            output = generated.convert("RGBA")
            output.putalpha(original.getchannel("A"))
            output.save(destination, format="PNG", compress_level=6)
        else:
            generated.save(destination, format="PNG", compress_level=6)
    else:
        generated.save(destination, format="JPEG", quality=95, subsampling=0, optimize=True)


def load_flux_pipeline(job: dict[str, Any]):
    import torch
    from diffsynth.pipelines.flux2_image import Flux2ImagePipeline, ModelConfig

    required = [
        FLUX_MODEL_ROOT / "transformer" / "diffusion_pytorch_model.safetensors",
        FLUX_MODEL_ROOT / "vae" / "diffusion_pytorch_model.safetensors",
        FLUX_MODEL_ROOT / "tokenizer",
    ]
    text_encoder = sorted((FLUX_MODEL_ROOT / "text_encoder").glob("model*.safetensors"))
    missing = [str(path) for path in required if not path.exists()]
    if not text_encoder:
        missing.append(str(FLUX_MODEL_ROOT / "text_encoder" / "model*.safetensors"))
    if missing:
        raise FileNotFoundError("Missing FLUX.2 Klein runtime files: " + ", ".join(missing))
    vram_config = {
        "offload_dtype": "disk", "offload_device": "disk",
        "onload_dtype": torch.float8_e4m3fn, "onload_device": "cpu",
        "preparing_dtype": torch.float8_e4m3fn, "preparing_device": "cuda",
        "computation_dtype": torch.bfloat16, "computation_device": "cuda",
    }
    pipe = Flux2ImagePipeline.from_pretrained(
        torch_dtype=torch.bfloat16,
        device="cuda",
        model_configs=[
            ModelConfig(path=[str(path) for path in text_encoder], **vram_config),
            ModelConfig(path=str(required[0]), **vram_config),
            ModelConfig(path=str(required[1])),
        ],
        tokenizer_config=ModelConfig(path=str(required[2])),
        vram_limit=torch.cuda.mem_get_info("cuda")[1] / (1024**3) - 0.5,
    )
    if job.get("loraEnabled"):
        if not DEFAULT_FLUX_LORA.is_file():
            raise FileNotFoundError(f"Built-in DreamRoom Objects LoRA is missing: {DEFAULT_FLUX_LORA}")
        pipe.load_lora(pipe.dit, str(DEFAULT_FLUX_LORA), alpha=max(0.0, min(1.5, float(job.get("loraStrength", 1.0)))))
    return pipe, torch


def run_flux_batch(manifest_path: Path) -> dict[str, Any]:
    job = json.loads(manifest_path.read_text(encoding="utf-8"))
    denoising_strength = float(job.get("denoisingStrength", 0.72))
    if not 0.0 <= denoising_strength <= 1.0:
        raise ValueError("Denoising strength must be between 0 and 1")
    structure_preservation = float(job.get("structurePreservation", 0.6))
    if not 0.0 <= structure_preservation <= 1.0:
        raise ValueError("Structure preservation must be between 0 and 1")
    inference_steps = int(job.get("inferenceSteps", 8))
    if inference_steps not in {4, 8, 12}:
        raise ValueError("Inference steps must be 4, 8, or 12")
    source_root = Path(job["sourceRoot"]).resolve()
    output_root = Path(job["outputRoot"]).resolve()
    output_root.mkdir(parents=True, exist_ok=True)
    working_resolution = int(job.get("workingResolution", 1024))
    minimum_resolution_enabled = bool(job.get("minimumResolutionEnabled", True))
    if working_resolution not in {256, 512, 768, 1024, 2048}:
        raise ValueError("Working resolution must be 256, 512, 768, 1024, or 2048")
    content_aware_scaling = bool(job.get("contentAwareScaling", True))
    started = time.perf_counter()
    results: list[dict[str, Any]] = []
    with contextlib.redirect_stdout(sys.stderr):
        pipe, torch = load_flux_pipeline(job)
    try:
        for index, relative in enumerate(job["relativePaths"]):
            item_started = time.perf_counter()
            try:
                source_path = safe_path(source_root, relative)
                output_path = safe_path(output_root, relative)
                if output_path.exists() and not job.get("overwrite", False):
                    results.append({"relativePath": relative, "status": "skipped", "message": "Output exists; overwrite disabled"})
                    continue
                with Image.open(source_path) as opened:
                    source = opened.copy()
                source_box = source_content_box(source, content_aware_scaling)
                working_source = source.crop(source_box)
                size, content_box = generation_layout(*working_source.size, working_resolution, minimum_resolution_enabled)
                reference = reference_image(working_source, size, content_box)
                palette_image = palette_reference(str(job.get("palette") or ""))
                source_condition = structure_reference(reference, structure_preservation, palette_image is not None)
                edit_images = [image for image in (source_condition, palette_image) if image is not None]
                init_image = reference if denoising_strength < 0.999 else None
                mask = None
                if init_image is not None:
                    mask = Image.new("L", size, 0)
                    ImageDraw.Draw(mask).rectangle(content_box, fill=255)
                seed = int(job.get("styleSeed", 24681357)) & 0xFFFFFFFF if job.get("styleLockEnabled") else zlib.crc32(f"ui-studio-flux:{relative}:{job.get('stylePrompt','')}:{job.get('palette','')}".encode("utf-8"))
                with contextlib.redirect_stdout(sys.stderr), torch.inference_mode():
                    generated = pipe(
                        prompt_for(job), edit_image=edit_images or None, edit_image_auto_resize=False, input_image=init_image,
                        inpaint_mask=mask, inpaint_blur_size=3, inpaint_blur_sigma=1.0,
                        denoising_strength=denoising_strength, seed=seed, rand_device="cuda",
                        num_inference_steps=inference_steps, width=size[0], height=size[1],
                    ).convert("RGB")
                from io import BytesIO
                buffer = BytesIO()
                generated.save(buffer, format="PNG")
                save_result(buffer.getvalue(), source, output_path, content_box, source_box)
                fit_note = ""
                if source_box != (0, 0, source.width, source.height):
                    fit_note = f" | visible fit {source_box[2] - source_box[0]} x {source_box[3] - source_box[1]}"
                lora_note = f" | LoRA {float(job.get('loraStrength', 1.0)):.2f}" if job.get("loraEnabled") else " | base model"
                resolution_note = f"{working_resolution}px minimum" if minimum_resolution_enabled else "native resolution"
                results.append({"relativePath": relative, "status": "complete", "message": f"FLUX {resolution_note} on {size[0]} x {size[1]} canvas{fit_note}{lora_note} | {job.get('styleFingerprint', 'untracked style')}", "seconds": round(time.perf_counter() - item_started, 2), "index": index})
            except Exception as error:
                results.append({"relativePath": relative, "status": "failed", "message": str(error), "index": index})
    finally:
        del pipe
        gc.collect()
        if torch.cuda.is_available():
            torch.cuda.empty_cache()
    return {"results": results, "engine": "FLUX.2 Klein 4B", "minimumResolutionEnabled": minimum_resolution_enabled, "workingResolution": working_resolution if minimum_resolution_enabled else None, "denoisingStrength": denoising_strength, "structurePreservation": structure_preservation, "inferenceSteps": inference_steps, "contentAwareScaling": content_aware_scaling, "loraEnabled": bool(job.get("loraEnabled")), "loraName": "DreamRoom-Objects-v1" if job.get("loraEnabled") else None, "loraPath": str(DEFAULT_FLUX_LORA) if job.get("loraEnabled") else None, "loraStrength": job.get("loraStrength") if job.get("loraEnabled") else None, "styleFingerprint": job.get("styleFingerprint"), "styleLocked": bool(job.get("styleLockEnabled")), "styleSeed": job.get("styleSeed") if job.get("styleLockEnabled") else None, "modelLifecycle": "one FLUX load for batch; GPU released on completion", "elapsedSeconds": round(time.perf_counter() - started, 2)}


def probe() -> dict[str, Any]:
    flux_transformer = FLUX_MODEL_ROOT / "transformer" / "diffusion_pytorch_model.safetensors"
    flux_text_encoder = FLUX_MODEL_ROOT / "text_encoder"
    flux_vae = FLUX_MODEL_ROOT / "vae" / "diffusion_pytorch_model.safetensors"
    flux_tokenizer = FLUX_MODEL_ROOT / "tokenizer"
    flux_package = FLUX_RUNTIME / "diffsynth"
    resources = [
        {"id": "flux-runtime", "group": "FLUX.2 Klein 4B", "name": "FLUX application runtime", "description": "Portable Python environment and DiffSynth pipeline used by the FLUX backend.", "path": str(FLUX_RUNTIME), "available": FLUX_PYTHON.is_file() and flux_package.is_dir(), "downloadBytes": 3004596619},
        {"id": "flux-transformer", "group": "FLUX.2 Klein 4B", "name": "FLUX.2 Klein transformer", "description": "Main 4B-parameter image generation model used for FLUX repainting.", "path": str(flux_transformer), "available": flux_transformer.is_file(), "downloadBytes": 7751109744},
        {"id": "flux-text-encoder", "group": "FLUX.2 Klein 4B", "name": "FLUX text encoder", "description": "Model shards that translate prompts and palette direction for FLUX.", "path": str(flux_text_encoder), "available": flux_text_encoder.is_dir() and any(flux_text_encoder.glob("model*.safetensors")), "downloadBytes": 8045016597},
        {"id": "flux-vae", "group": "FLUX.2 Klein 4B", "name": "FLUX image VAE", "description": "Encodes and reconstructs images for the FLUX repaint pipeline.", "path": str(flux_vae), "available": flux_vae.is_file(), "downloadBytes": 168121699},
        {"id": "flux-tokenizer", "group": "FLUX.2 Klein 4B", "name": "FLUX tokenizer", "description": "Tokenizer files required to interpret the style prompt.", "path": str(flux_tokenizer), "available": flux_tokenizer.is_dir(), "downloadBytes": 15882232},
        {"id": "dreamroom-lora", "group": "FLUX LoRA", "name": "DreamRoom Objects LoRA", "description": "Built-in object-detail style add-on. FLUX can run without it when the LoRA switch is off.", "path": str(DEFAULT_FLUX_LORA), "available": DEFAULT_FLUX_LORA.is_file(), "optional": True, "downloadBytes": 120449264},
    ]
    flux_ids = {"flux-runtime", "flux-transformer", "flux-text-encoder", "flux-vae", "flux-tokenizer"}
    flux_missing = [resource["path"] for resource in resources if resource["id"] in flux_ids and not resource["available"]]
    return {"available": not flux_missing, "engine": "FLUX.2 Klein 4B", "missing": flux_missing, "resources": resources}


def main() -> None:
    parser = argparse.ArgumentParser()
    commands = parser.add_subparsers(dest="command", required=True)
    commands.add_parser("probe")
    flux_batch = commands.add_parser("flux-batch")
    flux_batch.add_argument("--manifest", type=Path, required=True)
    args = parser.parse_args()
    result = probe() if args.command == "probe" else run_flux_batch(args.manifest)
    print(json.dumps(result))


if __name__ == "__main__":
    main()
