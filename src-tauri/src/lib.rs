use image::{GenericImageView, ImageReader};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs,
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};
use walkdir::WalkDir;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResourceManifest {
    resources: Vec<ManifestResource>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ManifestResource {
    id: String,
    #[serde(default)]
    base_url: Option<String>,
    #[serde(default)]
    destination: Option<String>,
    #[serde(default)]
    downloads: Vec<ManifestDownload>,
    #[serde(default)]
    files: Vec<ManifestFile>,
    #[serde(default)]
    archive: Option<ManifestArchive>,
}

#[derive(Deserialize)]
struct ManifestDownload {
    url: String,
    destination: String,
    bytes: Option<u64>,
    sha256: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ManifestArchive {
    parts: Vec<String>,
    bytes: u64,
    sha256: String,
    destination: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ManifestFile {
    path: String,
    component: String,
    bytes: Option<u64>,
    sha256: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DownloadResult {
    resource_id: String,
    files: usize,
    bytes: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GpuInfo {
    name: String,
    memory_mib: u64,
    driver_version: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SystemScan {
    compatible: bool,
    nvidia_detected: bool,
    cuda_available: bool,
    cuda_version: Option<String>,
    gpus: Vec<GpuInfo>,
    cpu_name: String,
    logical_cores: usize,
    memory_bytes: Option<u64>,
    free_disk_bytes: Option<u64>,
    os_name: String,
    warning: Option<String>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct AssetInfo {
    path: String,
    relative_path: String,
    directory: String,
    filename: String,
    extension: String,
    width: u32,
    height: u32,
    has_alpha: bool,
    bytes: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct InventoryIssue {
    path: String,
    kind: String,
    message: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct InventoryReport {
    root: String,
    assets: Vec<AssetInfo>,
    issues: Vec<InventoryIssue>,
    collisions: Vec<Vec<String>>,
    total_files_seen: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProcessResult {
    relative_path: String,
    output_path: String,
    status: String,
    valid: bool,
    message: String,
    source_width: u32,
    source_height: u32,
    output_width: Option<u32>,
    output_height: Option<u32>,
    alpha_preserved: bool,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct FluxBatchRequest {
    source_root: String,
    output_root: String,
    relative_paths: Vec<String>,
    style_prompt: String,
    palette: String,
    lora_enabled: bool,
    lora_strength: f32,
    denoising_strength: f32,
    structure_preservation: f32,
    inference_steps: u32,
    overwrite: bool,
    style_lock_enabled: bool,
    style_seed: u32,
    style_fingerprint: String,
    working_resolution: u32,
    minimum_resolution_enabled: bool,
    content_aware_scaling: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorkerItem {
    relative_path: String,
    status: String,
    message: String,
}

#[derive(Deserialize)]
struct WorkerBatch {
    results: Vec<WorkerItem>,
}

fn supported_extension(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|v| v.to_str())
            .map(|v| v.to_ascii_lowercase())
            .as_deref(),
        Some("png" | "jpg" | "jpeg")
    )
}

fn looks_like_image(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|v| v.to_str())
            .map(|v| v.to_ascii_lowercase())
            .as_deref(),
        Some(
            "png"
                | "jpg"
                | "jpeg"
                | "gif"
                | "bmp"
                | "webp"
                | "tif"
                | "tiff"
                | "svg"
                | "psd"
                | "tga"
        )
    )
}

fn inspect(path: &Path, root: &Path) -> Result<AssetInfo, String> {
    let reader = ImageReader::open(path)
        .map_err(|e| e.to_string())?
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    let image = reader.decode().map_err(|e| e.to_string())?;
    let relative = path
        .strip_prefix(root)
        .map_err(|_| "Asset is outside source root".to_string())?;
    let rel = relative.to_string_lossy().replace('\\', "/");
    let directory = relative
        .parent()
        .map(|v| v.to_string_lossy().replace('\\', "/"))
        .unwrap_or_default();
    Ok(AssetInfo {
        path: path.to_string_lossy().to_string(),
        relative_path: rel,
        directory: if directory == "." {
            String::new()
        } else {
            directory
        },
        filename: path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string(),
        extension: path
            .extension()
            .unwrap_or_default()
            .to_string_lossy()
            .to_ascii_lowercase(),
        width: image.width(),
        height: image.height(),
        has_alpha: image.color().has_alpha(),
        bytes: fs::metadata(path).map(|m| m.len()).unwrap_or(0),
    })
}

fn alpha_matches(
    source: &Path,
    output: &Path,
    source_has_alpha: bool,
    output_has_alpha: bool,
) -> bool {
    if source_has_alpha != output_has_alpha {
        return false;
    }
    if !source_has_alpha {
        return true;
    }
    let Ok(source_image) = image::open(source) else {
        return false;
    };
    let Ok(output_image) = image::open(output) else {
        return false;
    };
    if source_image.dimensions() != output_image.dimensions() {
        return false;
    }
    source_image
        .to_rgba8()
        .pixels()
        .zip(output_image.to_rgba8().pixels())
        .all(|(a, b)| a[3] == b[3])
}

#[tauri::command]
fn inventory_folder(root: String) -> Result<InventoryReport, String> {
    let root = fs::canonicalize(&root).map_err(|e| format!("Cannot open source folder: {e}"))?;
    if !root.is_dir() {
        return Err("Source path must be a folder".into());
    }
    let mut assets = Vec::new();
    let mut issues = Vec::new();
    let mut total_files_seen = 0;
    for entry in WalkDir::new(&root).follow_links(false).into_iter() {
        let entry = match entry {
            Ok(v) => v,
            Err(e) => {
                issues.push(InventoryIssue {
                    path: e
                        .path()
                        .map(|p| p.to_string_lossy().to_string())
                        .unwrap_or_default(),
                    kind: "read-error".into(),
                    message: e.to_string(),
                });
                continue;
            }
        };
        if !entry.file_type().is_file() {
            continue;
        }
        total_files_seen += 1;
        let path = entry.path();
        if supported_extension(path) {
            match inspect(path, &root) {
                Ok(asset) => assets.push(asset),
                Err(message) => issues.push(InventoryIssue {
                    path: path.to_string_lossy().to_string(),
                    kind: "corrupt".into(),
                    message,
                }),
            }
        } else if looks_like_image(path) {
            issues.push(InventoryIssue {
                path: path.to_string_lossy().to_string(),
                kind: "unsupported".into(),
                message: "Supported formats are PNG, JPG, and JPEG".into(),
            });
        }
    }
    assets.sort_by(|a, b| {
        a.relative_path
            .to_ascii_lowercase()
            .cmp(&b.relative_path.to_ascii_lowercase())
    });
    let mut collision_map: HashMap<String, Vec<String>> = HashMap::new();
    for asset in &assets {
        collision_map
            .entry(asset.relative_path.to_ascii_lowercase())
            .or_default()
            .push(asset.relative_path.clone());
    }
    let collisions = collision_map
        .into_values()
        .filter(|v| v.len() > 1)
        .collect();
    Ok(InventoryReport {
        root: root.to_string_lossy().to_string(),
        assets,
        issues,
        collisions,
        total_files_seen,
    })
}

fn safe_relative(value: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(value);
    if path.is_absolute()
        || path
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err(format!("Unsafe relative path: {value}"));
    }
    if !supported_extension(&path) {
        return Err(format!("Unsupported output extension: {value}"));
    }
    Ok(path)
}

#[tauri::command]
fn process_placeholder(
    source_root: String,
    output_root: String,
    relative_paths: Vec<String>,
    overwrite: bool,
) -> Result<Vec<ProcessResult>, String> {
    let source_root =
        fs::canonicalize(source_root).map_err(|e| format!("Cannot open source folder: {e}"))?;
    fs::create_dir_all(&output_root).map_err(|e| format!("Cannot create output folder: {e}"))?;
    let output_root =
        fs::canonicalize(output_root).map_err(|e| format!("Cannot open output folder: {e}"))?;
    if source_root == output_root || output_root.starts_with(&source_root) {
        return Err("Output folder must be separate from and outside the source folder".into());
    }
    let mut results = Vec::new();
    for relative_string in relative_paths {
        let relative = match safe_relative(&relative_string) {
            Ok(v) => v,
            Err(message) => {
                results.push(ProcessResult {
                    relative_path: relative_string,
                    output_path: String::new(),
                    status: "failed".into(),
                    valid: false,
                    message,
                    source_width: 0,
                    source_height: 0,
                    output_width: None,
                    output_height: None,
                    alpha_preserved: false,
                });
                continue;
            }
        };
        let source = source_root.join(&relative);
        let output = output_root.join(&relative);
        let source_info = match inspect(&source, &source_root) {
            Ok(v) => v,
            Err(message) => {
                results.push(ProcessResult {
                    relative_path: relative_string,
                    output_path: output.to_string_lossy().to_string(),
                    status: "failed".into(),
                    valid: false,
                    message,
                    source_width: 0,
                    source_height: 0,
                    output_width: None,
                    output_height: None,
                    alpha_preserved: false,
                });
                continue;
            }
        };
        if output.exists() && !overwrite {
            results.push(ProcessResult {
                relative_path: relative_string,
                output_path: output.to_string_lossy().to_string(),
                status: "skipped".into(),
                valid: false,
                message: "Output already exists; overwrite is disabled".into(),
                source_width: source_info.width,
                source_height: source_info.height,
                output_width: None,
                output_height: None,
                alpha_preserved: false,
            });
            continue;
        }
        let copied = output
            .parent()
            .map(fs::create_dir_all)
            .transpose()
            .and_then(|_| fs::copy(&source, &output).map(|_| ()));
        if let Err(error) = copied {
            results.push(ProcessResult {
                relative_path: relative_string,
                output_path: output.to_string_lossy().to_string(),
                status: "failed".into(),
                valid: false,
                message: error.to_string(),
                source_width: source_info.width,
                source_height: source_info.height,
                output_width: None,
                output_height: None,
                alpha_preserved: false,
            });
            continue;
        }
        let output_info = inspect(&output, &output_root);
        match output_info {
            Ok(info) => {
                let dimensions_ok =
                    source_info.width == info.width && source_info.height == info.height;
                let alpha_ok = source_info.extension != "png"
                    || alpha_matches(&source, &output, source_info.has_alpha, info.has_alpha);
                let name_ok = source_info.relative_path == info.relative_path;
                let valid = dimensions_ok && alpha_ok && name_ok;
                results.push(ProcessResult {
                    relative_path: relative_string,
                    output_path: output.to_string_lossy().to_string(),
                    status: if valid { "complete" } else { "failed" }.into(),
                    valid,
                    message: if valid {
                        "Exact size, path, name, extension, and pixel alpha validated".into()
                    } else {
                        "Output failed structural validation".into()
                    },
                    source_width: source_info.width,
                    source_height: source_info.height,
                    output_width: Some(info.width),
                    output_height: Some(info.height),
                    alpha_preserved: alpha_ok,
                });
            }
            Err(message) => results.push(ProcessResult {
                relative_path: relative_string,
                output_path: output.to_string_lossy().to_string(),
                status: "failed".into(),
                valid: false,
                message,
                source_width: source_info.width,
                source_height: source_info.height,
                output_width: None,
                output_height: None,
                alpha_preserved: false,
            }),
        }
    }
    Ok(results)
}

#[tauri::command]
fn read_binary(path: String) -> Result<Vec<u8>, String> {
    fs::read(path).map_err(|e| e.to_string())
}

#[tauri::command]
fn write_report(path: String, content: String) -> Result<(), String> {
    fs::write(path, content).map_err(|e| e.to_string())
}

fn ui_studio_root() -> Result<PathBuf, String> {
    if let Ok(executable) = std::env::current_exe() {
        if let Some(directory) = executable.parent() {
            if directory.join("tools").join("ui_studio_worker.py").is_file() {
                return Ok(directory.to_path_buf());
            }
        }
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(PathBuf::from)
        .ok_or_else(|| "Cannot locate UI Studio root".to_string())
}

fn safe_resource_relative(value: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(value);
    if value.is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(format!("Unsafe resource destination: {value}"));
    }
    Ok(path)
}

fn resource_install_root(studio_root: &Path) -> PathBuf {
    let executable_directory = std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(PathBuf::from));
    let portable = studio_root
        .join("resources")
        .join("resource-manifest.json")
        .is_file()
        && executable_directory
            .as_ref()
            .is_some_and(|directory| directory == studio_root);
    if portable {
        studio_root.to_path_buf()
    } else {
        studio_root.parent().unwrap_or(studio_root).to_path_buf()
    }
}

fn download_one(
    client: &reqwest::blocking::Client,
    url: &str,
    destination: &Path,
    expected_bytes: Option<u64>,
    expected_sha256: Option<&str>,
) -> Result<u64, String> {
    if !url.starts_with("https://") {
        return Err(format!("Resource URL must use HTTPS: {url}"));
    }
    if expected_bytes.is_none() && expected_sha256.is_none() {
        return Err(format!("Resource has no size or SHA-256 validation: {url}"));
    }
    let parent = destination
        .parent()
        .ok_or_else(|| "Resource destination has no parent".to_string())?;
    fs::create_dir_all(parent).map_err(|error| format!("Cannot create resource folder: {error}"))?;
    let part = PathBuf::from(format!("{}.part", destination.to_string_lossy()));
    let mut response = client
        .get(url)
        .send()
        .and_then(|response| response.error_for_status())
        .map_err(|error| format!("Download failed for {url}: {error}"))?;
    let mut output = fs::File::create(&part)
        .map_err(|error| format!("Cannot create temporary download {}: {error}", part.display()))?;
    let mut digest = Sha256::new();
    let mut downloaded = 0u64;
    let mut buffer = [0u8; 1024 * 256];
    loop {
        let count = response
            .read(&mut buffer)
            .map_err(|error| format!("Download interrupted for {url}: {error}"))?;
        if count == 0 {
            break;
        }
        output
            .write_all(&buffer[..count])
            .map_err(|error| format!("Cannot write {}: {error}", part.display()))?;
        digest.update(&buffer[..count]);
        downloaded += count as u64;
    }
    output
        .sync_all()
        .map_err(|error| format!("Cannot finalize {}: {error}", part.display()))?;
    if let Some(expected) = expected_bytes {
        if downloaded != expected {
            return Err(format!("Size check failed for {url}: expected {expected}, received {downloaded}"));
        }
    }
    if let Some(expected) = expected_sha256 {
        let actual = format!("{:x}", digest.finalize());
        if !actual.eq_ignore_ascii_case(expected) {
            return Err(format!("SHA-256 check failed for {url}"));
        }
    }
    if destination.exists() {
        fs::remove_file(destination)
            .map_err(|error| format!("Cannot replace {}: {error}", destination.display()))?;
    }
    fs::rename(&part, destination)
        .map_err(|error| format!("Cannot install {}: {error}", destination.display()))?;
    Ok(downloaded)
}

fn verified_file(path: &Path, expected_bytes: u64, expected_sha256: &str) -> Result<(), String> {
    let mut input = fs::File::open(path)
        .map_err(|error| format!("Cannot read {}: {error}", path.display()))?;
    let actual_bytes = input
        .metadata()
        .map_err(|error| format!("Cannot inspect {}: {error}", path.display()))?
        .len();
    if actual_bytes != expected_bytes {
        return Err(format!(
            "Size check failed for {}: expected {expected_bytes}, received {actual_bytes}",
            path.display()
        ));
    }
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 1024 * 256];
    loop {
        let count = input
            .read(&mut buffer)
            .map_err(|error| format!("Cannot verify {}: {error}", path.display()))?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    let actual = format!("{:x}", digest.finalize());
    if !actual.eq_ignore_ascii_case(expected_sha256) {
        return Err(format!("SHA-256 check failed for {}", path.display()));
    }
    Ok(())
}

fn install_archive(
    install_root: &Path,
    resource_id: &str,
    archive: &ManifestArchive,
) -> Result<(), String> {
    let destination = safe_resource_relative(&archive.destination)?;
    if destination != PathBuf::from("runtime/diffsynth") {
        return Err(format!("Unsupported archive destination for {resource_id}"));
    }
    let download_root = install_root.join("resources").join("downloads");
    let joined = download_root.join(format!("{resource_id}.zip"));
    let mut output = fs::File::create(&joined)
        .map_err(|error| format!("Cannot assemble {}: {error}", joined.display()))?;
    for part in &archive.parts {
        let relative = safe_resource_relative(part)?;
        let part_path = install_root.join(relative);
        let mut input = fs::File::open(&part_path)
            .map_err(|error| format!("Cannot read archive part {}: {error}", part_path.display()))?;
        std::io::copy(&mut input, &mut output)
            .map_err(|error| format!("Cannot assemble runtime archive: {error}"))?;
    }
    output
        .sync_all()
        .map_err(|error| format!("Cannot finalize {}: {error}", joined.display()))?;
    drop(output);
    verified_file(&joined, archive.bytes, &archive.sha256)?;

    let listing = Command::new("tar.exe")
        .arg("-tf")
        .arg(&joined)
        .output()
        .map_err(|error| format!("Cannot inspect runtime archive: {error}"))?;
    if !listing.status.success() {
        return Err("The runtime archive could not be inspected".to_string());
    }
    for entry in String::from_utf8_lossy(&listing.stdout).lines() {
        let normalized = entry.trim().trim_start_matches("./");
        if normalized.is_empty() {
            continue;
        }
        let relative = safe_resource_relative(normalized.trim_end_matches('/'))?;
        if !relative.starts_with("runtime/diffsynth") {
            return Err(format!("Runtime archive contains an unexpected path: {entry}"));
        }
    }

    let stage = install_root.join("resources").join("runtime-install-stage");
    if stage.exists() {
        fs::remove_dir_all(&stage)
            .map_err(|error| format!("Cannot clear runtime staging directory: {error}"))?;
    }
    fs::create_dir_all(&stage)
        .map_err(|error| format!("Cannot create runtime staging directory: {error}"))?;
    let extracted = Command::new("tar.exe")
        .arg("-xf")
        .arg(&joined)
        .arg("-C")
        .arg(&stage)
        .status()
        .map_err(|error| format!("Cannot extract runtime archive: {error}"))?;
    if !extracted.success() {
        return Err("The runtime archive could not be extracted".to_string());
    }
    let staged_runtime = stage.join("runtime").join("diffsynth");
    if !staged_runtime.join("python").join("python.exe").is_file()
        || !staged_runtime.join("diffsynth").is_dir()
    {
        return Err("The extracted FLUX runtime is incomplete".to_string());
    }

    let final_runtime = install_root.join(destination);
    let backup = install_root.join("resources").join("runtime-install-backup");
    if backup.exists() {
        fs::remove_dir_all(&backup)
            .map_err(|error| format!("Cannot clear old runtime backup: {error}"))?;
    }
    if final_runtime.exists() {
        fs::rename(&final_runtime, &backup)
            .map_err(|error| format!("Cannot stage the existing runtime for replacement: {error}"))?;
    }
    if let Err(error) = fs::rename(&staged_runtime, &final_runtime) {
        if backup.exists() {
            let _ = fs::rename(&backup, &final_runtime);
        }
        return Err(format!("Cannot install the FLUX runtime: {error}"));
    }
    if backup.exists() {
        fs::remove_dir_all(&backup)
            .map_err(|error| format!("Cannot remove the old runtime backup: {error}"))?;
    }
    let _ = fs::remove_dir_all(&stage);
    let _ = fs::remove_file(&joined);
    for part in &archive.parts {
        if let Ok(relative) = safe_resource_relative(part) {
            let _ = fs::remove_file(install_root.join(relative));
        }
    }
    Ok(())
}

fn download_resource_blocking(resource_id: &str) -> Result<DownloadResult, String> {
    if !cuda_driver_available() {
        return Err("Downloads are disabled because no CUDA-capable NVIDIA GPU with a working NVIDIA driver was detected".to_string());
    }
    let studio_root = ui_studio_root()?;
    let manifest_path = studio_root.join("resources").join("resource-manifest.json");
    let manifest: ResourceManifest = serde_json::from_slice(
        &fs::read(&manifest_path)
            .map_err(|error| format!("Cannot read {}: {error}", manifest_path.display()))?,
    )
    .map_err(|error| format!("Invalid resource manifest: {error}"))?;
    let install_root = resource_install_root(&studio_root);
    let client = reqwest::blocking::Client::builder()
        .user_agent("UI-Studio/0.1.0")
        .build()
        .map_err(|error| format!("Cannot initialize downloader: {error}"))?;
    let mut downloaded_files = 0usize;
    let mut downloaded_bytes = 0u64;

    if let Some(resource) = manifest.resources.iter().find(|resource| resource.id == resource_id) {
        for download in &resource.downloads {
            let relative = safe_resource_relative(&download.destination)?;
            downloaded_bytes += download_one(
                &client,
                &download.url,
                &install_root.join(relative),
                download.bytes,
                download.sha256.as_deref(),
            )?;
            downloaded_files += 1;
        }
        if let Some(archive) = &resource.archive {
            install_archive(&install_root, resource_id, archive)?;
        }
    }

    for resource in &manifest.resources {
        let matching: Vec<&ManifestFile> = resource
            .files
            .iter()
            .filter(|file| file.component == resource_id)
            .collect();
        if matching.is_empty() {
            continue;
        }
        let base_url = resource
            .base_url
            .as_deref()
            .ok_or_else(|| format!("{resource_id} has no download base URL"))?;
        let base_destination = resource
            .destination
            .as_deref()
            .ok_or_else(|| format!("{resource_id} has no destination"))?;
        for file in matching {
            let relative = safe_resource_relative(&format!("{base_destination}/{}", file.path))?;
            let url = format!("{base_url}{}?download=true", file.path.replace('\\', "/"));
            downloaded_bytes += download_one(
                &client,
                &url,
                &install_root.join(relative),
                file.bytes,
                file.sha256.as_deref(),
            )?;
            downloaded_files += 1;
        }
    }

    if downloaded_files == 0 {
        return Err(format!("{resource_id} is awaiting a published release asset"));
    }
    Ok(DownloadResult {
        resource_id: resource_id.to_string(),
        files: downloaded_files,
        bytes: downloaded_bytes,
    })
}

#[tauri::command]
async fn download_resource(resource_id: String) -> Result<DownloadResult, String> {
    tauri::async_runtime::spawn_blocking(move || download_resource_blocking(&resource_id))
        .await
        .map_err(|error| format!("Resource download task failed: {error}"))?
}

fn hidden_command(program: &Path) -> Command {
    let mut command = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    command
}

fn nvidia_smi_output(arguments: &[&str]) -> Option<std::process::Output> {
    let mut candidates = vec![PathBuf::from("nvidia-smi.exe")];
    for variable in ["ProgramW6432", "ProgramFiles"] {
        if let Ok(root) = std::env::var(variable) {
            candidates.push(
                PathBuf::from(root)
                    .join("NVIDIA Corporation")
                    .join("NVSMI")
                    .join("nvidia-smi.exe"),
            );
        }
    }
    candidates.into_iter().find_map(|candidate| {
        hidden_command(&candidate)
            .args(arguments)
            .output()
            .ok()
            .filter(|output| output.status.success())
    })
}

fn cuda_driver_available() -> bool {
    nvidia_smi_output(&[
        "--query-gpu=name",
        "--format=csv,noheader,nounits",
    ])
    .is_some_and(|output| !String::from_utf8_lossy(&output.stdout).trim().is_empty())
}

fn detect_cuda_version() -> Option<String> {
    let output = nvidia_smi_output(&[])?;
    let text = String::from_utf8_lossy(&output.stdout);
    let marker = ["CUDA UMD Version:", "CUDA Version:"]
        .into_iter()
        .find(|marker| text.contains(marker))?;
    let start = text.find(marker)? + marker.len();
    let version = text[start..]
        .trim_start()
        .split_whitespace()
        .next()?
        .trim_matches('|')
        .to_string();
    (!version.is_empty() && version != "N/A").then_some(version)
}

fn scan_system_blocking() -> Result<SystemScan, String> {
    let root = resource_install_root(&ui_studio_root()?);
    let query = nvidia_smi_output(&[
        "--query-gpu=name,memory.total,driver_version",
        "--format=csv,noheader,nounits",
    ]);
    let gpus = query
        .as_ref()
        .map(|output| {
            String::from_utf8_lossy(&output.stdout)
                .lines()
                .filter_map(|line| {
                    let fields: Vec<&str> = line.split(',').map(str::trim).collect();
                    if fields.len() < 3 || fields[0].is_empty() {
                        return None;
                    }
                    Some(GpuInfo {
                        name: fields[0].to_string(),
                        memory_mib: fields[1].parse().unwrap_or(0),
                        driver_version: fields[2].to_string(),
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let nvidia_detected = !gpus.is_empty();
    let cuda_version = nvidia_detected.then(detect_cuda_version).flatten();
    // nvidia-smi is supplied by the NVIDIA display driver. A successful GPU
    // query proves that the driver exposes CUDA to the bundled PyTorch runtime;
    // users do not need a separately installed CUDA Toolkit.
    let cuda_available = nvidia_detected;

    let powershell = r#"$ErrorActionPreference='Stop'; $cpu=(Get-ItemProperty -LiteralPath 'HKLM:\HARDWARE\DESCRIPTION\System\CentralProcessor\0').ProcessorNameString; Add-Type -AssemblyName Microsoft.VisualBasic; $memory=[Microsoft.VisualBasic.Devices.ComputerInfo]::new().TotalPhysicalMemory; $osKey=Get-ItemProperty -LiteralPath 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion'; $item=Get-Item -LiteralPath $env:UI_STUDIO_SCAN_ROOT; $disk=[IO.DriveInfo]::new($item.PSDrive.Root); [pscustomobject]@{cpu=[string]$cpu; memoryBytes=[uint64]$memory; freeDiskBytes=[uint64]$disk.AvailableFreeSpace; os=([string]$osKey.ProductName+' '+[string]$osKey.DisplayVersion).Trim()} | ConvertTo-Json -Compress"#;
    let mut command = hidden_command(Path::new("powershell.exe"));
    command
        .args(["-NoLogo", "-NoProfile", "-NonInteractive", "-Command", powershell])
        .env("UI_STUDIO_SCAN_ROOT", &root);
    let system = command
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| {
            let text = String::from_utf8_lossy(&output.stdout);
            serde_json::from_str::<serde_json::Value>(text.trim().trim_start_matches('\u{feff}')).ok()
        });
    let cpu_name = system
        .as_ref()
        .and_then(|value| value.get("cpu"))
        .and_then(|value| value.as_str())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("Unknown processor")
        .trim()
        .to_string();
    let memory_bytes = system
        .as_ref()
        .and_then(|value| value.get("memoryBytes"))
        .and_then(|value| value.as_u64());
    let free_disk_bytes = system
        .as_ref()
        .and_then(|value| value.get("freeDiskBytes"))
        .and_then(|value| value.as_u64());
    let os_name = system
        .as_ref()
        .and_then(|value| value.get("os"))
        .and_then(|value| value.as_str())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("Windows")
        .trim()
        .to_string();
    let logical_cores = std::thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(1);

    let mut cautions = Vec::new();
    if !cuda_available {
        cautions.push("No CUDA-capable NVIDIA GPU with a working NVIDIA driver was detected. Resource downloads are disabled because FLUX cannot run on this PC.".to_string());
    } else {
        if gpus.iter().map(|gpu| gpu.memory_mib).max().unwrap_or(0) < 6144 {
            cautions.push("Less than 6 GB of GPU memory was detected. FLUX may run out of memory at larger working resolutions.".to_string());
        }
        if memory_bytes.is_some_and(|bytes| bytes < 16 * 1024 * 1024 * 1024) {
            cautions.push("Less than 16 GB of system memory was detected. Processing may be unstable or very slow.".to_string());
        }
        if free_disk_bytes.is_some_and(|bytes| bytes < 30 * 1024 * 1024 * 1024) {
            cautions.push("Less than 30 GB is free on the app drive. The complete runtime and models may not fit.".to_string());
        }
    }

    Ok(SystemScan {
        compatible: cuda_available,
        nvidia_detected,
        cuda_available,
        cuda_version,
        gpus,
        cpu_name,
        logical_cores,
        memory_bytes,
        free_disk_bytes,
        os_name,
        warning: (!cautions.is_empty()).then(|| cautions.join(" ")),
    })
}

#[tauri::command]
async fn scan_system() -> Result<SystemScan, String> {
    tauri::async_runtime::spawn_blocking(scan_system_blocking)
        .await
        .map_err(|error| format!("System scan task failed: {error}"))?
}

#[tauri::command]
fn settings_scope() -> Result<String, String> {
    let root = ui_studio_root()?;
    let executable_directory = std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(PathBuf::from));
    let portable = root.join("resources").join("resource-manifest.json").is_file()
        && executable_directory.as_ref().is_some_and(|directory| directory == &root);
    if !portable {
        return Ok("development".to_string());
    }
    let normalized = root.to_string_lossy().to_ascii_lowercase();
    let mut hash = 14695981039346656037u64;
    for byte in normalized.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(1099511628211);
    }
    Ok(format!("portable-{}-{hash:016x}", env!("CARGO_PKG_VERSION")))
}

fn run_worker(arguments: &[String], ai_runtime: bool) -> Result<String, String> {
    let studio_root = ui_studio_root()?;
    let portable = studio_root.join("resources").join("resource-manifest.json").is_file()
        && std::env::current_exe()
            .ok()
            .and_then(|path| path.parent().map(PathBuf::from))
            .is_some_and(|directory| directory == studio_root);
    let python = if portable && ai_runtime {
        studio_root
            .join("runtime")
            .join("diffsynth")
            .join("python")
            .join("python.exe")
    } else if portable {
        studio_root
            .join("runtime")
            .join("python")
            .join("python.exe")
    } else {
        let dreamy_root = studio_root
            .parent()
            .ok_or_else(|| "Cannot locate Dreamy runtime root".to_string())?;
        if ai_runtime {
            dreamy_root
                .join("external")
                .join("DiffSynth-Studio")
                .join(".venv")
                .join("Scripts")
                .join("python.exe")
        } else {
            dreamy_root.join(".venv").join("Scripts").join("python.exe")
        }
    };
    let worker = studio_root.join("tools").join("ui_studio_worker.py");
    if !python.is_file() {
        return Err(format!("Python runtime is missing: {}", python.display()));
    }
    if !worker.is_file() {
        return Err(format!("UI Studio worker is missing: {}", worker.display()));
    }
    let mut command = Command::new(python);
    command.arg(worker).args(arguments);
    if portable {
        command.env("UI_STUDIO_PORTABLE_ROOT", &studio_root);
        if ai_runtime {
            command.env("PYTHONPATH", studio_root.join("runtime").join("diffsynth"));
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let output = command
        .output()
        .map_err(|e| format!("Could not start UI Studio worker: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if stderr.is_empty() {
            "UI Studio worker failed without an error message".into()
        } else {
            stderr
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

#[tauri::command]
async fn probe_flux() -> Result<serde_json::Value, String> {
    tauri::async_runtime::spawn_blocking(|| {
        match run_worker(&["probe".into()], true) {
            Ok(text) => serde_json::from_str(&text)
                .map_err(|e| format!("Invalid model probe response: {e}")),
            Err(worker_error) => {
                let root = ui_studio_root()?;
                if root.join("resources").join("resource-manifest.json").is_file() {
                    Ok(portable_resource_probe(&root))
                } else {
                    Err(worker_error)
                }
            }
        }
    })
    .await
    .map_err(|e| format!("Model probe task failed: {e}"))?
}

fn portable_resource_probe(root: &Path) -> serde_json::Value {
    let flux_runtime = root.join("runtime").join("diffsynth");
    let flux_python = flux_runtime.join("python").join("python.exe");
    let flux_root = root.join("models").join("FLUX.2-klein-4B");
    let flux_transformer = flux_root.join("transformer").join("diffusion_pytorch_model.safetensors");
    let flux_encoder = flux_root.join("text_encoder");
    let flux_vae = flux_root.join("vae").join("diffusion_pytorch_model.safetensors");
    let flux_tokenizer = flux_root.join("tokenizer");
    let lora = root.join("models").join("lora").join("DreamRoom-Objects-v1.safetensors");
    let flux_encoder_ready = flux_encoder.join("model-00001-of-00002.safetensors").is_file()
        && flux_encoder.join("model-00002-of-00002.safetensors").is_file();
    let resources = vec![
        serde_json::json!({"id":"flux-runtime","group":"FLUX.2 Klein 4B","name":"FLUX application runtime","description":"Portable Python environment and DiffSynth pipeline used by the FLUX backend.","path":flux_runtime.to_string_lossy(),"available":flux_python.is_file() && flux_runtime.join("diffsynth").is_dir(),"downloadBytes":3004596619u64}),
        serde_json::json!({"id":"flux-transformer","group":"FLUX.2 Klein 4B","name":"FLUX.2 Klein transformer","description":"Main 4B-parameter image generation model used for FLUX repainting.","path":flux_transformer.to_string_lossy(),"available":flux_transformer.is_file(),"downloadBytes":7751109744u64}),
        serde_json::json!({"id":"flux-text-encoder","group":"FLUX.2 Klein 4B","name":"FLUX text encoder","description":"Model shards that translate prompts and palette direction for FLUX.","path":flux_encoder.to_string_lossy(),"available":flux_encoder_ready,"downloadBytes":8045016597u64}),
        serde_json::json!({"id":"flux-vae","group":"FLUX.2 Klein 4B","name":"FLUX image VAE","description":"Encodes and reconstructs images for the FLUX repaint pipeline.","path":flux_vae.to_string_lossy(),"available":flux_vae.is_file(),"downloadBytes":168121699u64}),
        serde_json::json!({"id":"flux-tokenizer","group":"FLUX.2 Klein 4B","name":"FLUX tokenizer","description":"Tokenizer files required to interpret the style prompt.","path":flux_tokenizer.to_string_lossy(),"available":flux_tokenizer.is_dir(),"downloadBytes":15882232u64}),
        serde_json::json!({"id":"dreamroom-lora","group":"FLUX LoRA","name":"DreamRoom Objects LoRA","description":"Built-in object-detail style add-on. FLUX can run without it when the LoRA switch is off.","path":lora.to_string_lossy(),"available":lora.is_file(),"optional":true,"downloadBytes":120449264u64}),
    ];
    let flux_available = flux_python.is_file() && flux_runtime.join("diffsynth").is_dir() && flux_transformer.is_file() && flux_encoder_ready && flux_vae.is_file() && flux_tokenizer.is_dir();
    serde_json::json!({"available":flux_available,"engine":"FLUX.2 Klein 4B","missing":[],"resources":resources})
}

fn validated_worker_result(
    source_root: &Path,
    output_root: &Path,
    item: WorkerItem,
) -> ProcessResult {
    let relative = match safe_relative(&item.relative_path) {
        Ok(value) => value,
        Err(message) => {
            return ProcessResult {
                relative_path: item.relative_path,
                output_path: String::new(),
                status: "failed".into(),
                valid: false,
                message,
                source_width: 0,
                source_height: 0,
                output_width: None,
                output_height: None,
                alpha_preserved: false,
            }
        }
    };
    let source = source_root.join(&relative);
    let output = output_root.join(&relative);
    let source_info = match inspect(&source, source_root) {
        Ok(value) => value,
        Err(message) => {
            return ProcessResult {
                relative_path: item.relative_path,
                output_path: output.to_string_lossy().to_string(),
                status: "failed".into(),
                valid: false,
                message,
                source_width: 0,
                source_height: 0,
                output_width: None,
                output_height: None,
                alpha_preserved: false,
            }
        }
    };
    if item.status != "complete" {
        return ProcessResult {
            relative_path: item.relative_path,
            output_path: output.to_string_lossy().to_string(),
            status: item.status,
            valid: false,
            message: item.message,
            source_width: source_info.width,
            source_height: source_info.height,
            output_width: None,
            output_height: None,
            alpha_preserved: false,
        };
    }
    match inspect(&output, output_root) {
        Ok(output_info) => {
            let dimensions_ok =
                source_info.width == output_info.width && source_info.height == output_info.height;
            let alpha_ok = source_info.extension != "png"
                || alpha_matches(
                    &source,
                    &output,
                    source_info.has_alpha,
                    output_info.has_alpha,
                );
            let name_ok = source_info.relative_path == output_info.relative_path
                && source_info.extension == output_info.extension;
            let valid = dimensions_ok && alpha_ok && name_ok;
            ProcessResult {
                relative_path: item.relative_path,
                output_path: output.to_string_lossy().to_string(),
                status: if valid { "complete" } else { "failed" }.into(),
                valid,
                message: if valid {
                    format!(
                        "{}; exact size, path, name, extension, and pixel alpha validated",
                        item.message
                    )
                } else {
                    "FLUX output failed structural validation".into()
                },
                source_width: source_info.width,
                source_height: source_info.height,
                output_width: Some(output_info.width),
                output_height: Some(output_info.height),
                alpha_preserved: alpha_ok,
            }
        }
        Err(message) => ProcessResult {
            relative_path: item.relative_path,
            output_path: output.to_string_lossy().to_string(),
            status: "failed".into(),
            valid: false,
            message,
            source_width: source_info.width,
            source_height: source_info.height,
            output_width: None,
            output_height: None,
            alpha_preserved: false,
        },
    }
}

fn run_backend_batch(
    request: FluxBatchRequest,
    worker_command: &str,
    ai_runtime: bool,
    label: &str,
) -> Result<Vec<ProcessResult>, String> {
    let source_root = fs::canonicalize(&request.source_root)
        .map_err(|e| format!("Cannot open source folder: {e}"))?;
    fs::create_dir_all(&request.output_root)
        .map_err(|e| format!("Cannot create output folder: {e}"))?;
    let output_root = fs::canonicalize(&request.output_root)
        .map_err(|e| format!("Cannot open output folder: {e}"))?;
    if source_root == output_root || output_root.starts_with(&source_root) {
        return Err("Output folder must be separate from and outside the source folder".into());
    }
    if request.relative_paths.is_empty() {
        return Ok(Vec::new());
    }
    for relative in &request.relative_paths {
        safe_relative(relative)?;
    }
    if worker_command == "flux-batch" && request.lora_enabled {
        if !(0.0..=1.5).contains(&request.lora_strength) {
            return Err("FLUX LoRA strength must be between 0 and 1.5".into());
        }
    }
    if !(0.0..=1.0).contains(&request.denoising_strength) {
        return Err("Denoising strength must be between 0 and 1".into());
    }
    if !(0.0..=1.0).contains(&request.structure_preservation) {
        return Err("Structure preservation must be between 0 and 1".into());
    }
    if !matches!(request.inference_steps, 4 | 8 | 12) {
        return Err("Inference steps must be 4, 8, or 12".into());
    }
    if !matches!(request.working_resolution, 256 | 512 | 768 | 1024 | 2048) {
        return Err("FLUX working resolution must be 256, 512, 768, 1024, or 2048".into());
    }
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_nanos();
    let manifest = std::env::temp_dir().join(format!(
        "ui-studio-{}-{}-{nonce}.json",
        label.to_ascii_lowercase(),
        std::process::id()
    ));
    fs::write(
        &manifest,
        serde_json::to_vec(&request).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let worker_result = run_worker(
        &[
            worker_command.into(),
            "--manifest".into(),
            manifest.to_string_lossy().to_string(),
        ],
        ai_runtime,
    );
    let _ = fs::remove_file(&manifest);
    let text = worker_result?;
    let batch: WorkerBatch =
        serde_json::from_str(&text).map_err(|e| format!("Invalid {label} worker response: {e}"))?;
    Ok(batch
        .results
        .into_iter()
        .map(|item| validated_worker_result(&source_root, &output_root, item))
        .collect())
}

#[tauri::command]
async fn run_flux_batch(request: FluxBatchRequest) -> Result<Vec<ProcessResult>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        run_backend_batch(request, "flux-batch", true, "FLUX")
    })
    .await
    .map_err(|e| format!("FLUX batch task failed: {e}"))?
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            inventory_folder,
            process_placeholder,
            read_binary,
            write_report,
            settings_scope,
            probe_flux,
            scan_system,
            download_resource,
            run_flux_batch
        ])
        .run(tauri::generate_context!())
        .expect("error while running UI Studio");
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageBuffer, Rgba};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn sandbox() -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("ui-studio-test-{nonce}"))
    }

    #[test]
    fn inventory_and_mirrored_copy_preserve_contract() {
        let base = sandbox();
        let source = base.join("source");
        let output = base.join("output");
        fs::create_dir_all(source.join("HUD/icons")).unwrap();
        let pixels = ImageBuffer::from_pixel(13, 9, Rgba([120u8, 80, 40, 128]));
        pixels.save(source.join("HUD/icons/Button.PNG")).unwrap();
        fs::write(source.join("broken.jpg"), b"not an image").unwrap();
        fs::write(source.join("legacy.gif"), b"GIF89a").unwrap();

        let inventory = inventory_folder(source.to_string_lossy().to_string()).unwrap();
        assert_eq!(inventory.assets.len(), 1);
        assert_eq!(inventory.issues.len(), 2);
        assert_eq!(inventory.assets[0].width, 13);
        assert_eq!(inventory.assets[0].height, 9);
        assert!(inventory.assets[0].has_alpha);

        let results = process_placeholder(
            source.to_string_lossy().to_string(),
            output.to_string_lossy().to_string(),
            vec!["HUD/icons/Button.PNG".into()],
            false,
        )
        .unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].valid);
        assert_eq!(results[0].output_width, Some(13));
        assert_eq!(results[0].output_height, Some(9));
        assert!(results[0].alpha_preserved);
        assert!(output.join("HUD/icons/Button.PNG").exists());

        let repeated = process_placeholder(
            source.to_string_lossy().to_string(),
            output.to_string_lossy().to_string(),
            vec!["HUD/icons/Button.PNG".into()],
            false,
        )
        .unwrap();
        assert_eq!(repeated[0].status, "skipped");
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn rejects_output_inside_source_and_parent_traversal() {
        let base = sandbox();
        let source = base.join("source");
        fs::create_dir_all(&source).unwrap();
        let error = process_placeholder(
            source.to_string_lossy().to_string(),
            source.join("export").to_string_lossy().to_string(),
            vec![],
            false,
        )
        .unwrap_err();
        assert!(error.contains("outside the source"));
        assert!(safe_relative("../escape.png").is_err());
        fs::remove_dir_all(base).unwrap();
    }
}
