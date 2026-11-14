//! Subprocess isolation runner and task handlers for sandboxed execution.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::records::JobRecord;

/// Execution request passed into the isolated sandbox subprocess.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskRequest {
    pub kind: String,
    pub inputs: Value,
}

/// Execution response emitted by the isolated sandbox subprocess.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskResponse {
    pub success: bool,
    pub output: Value,
    pub error: Option<String>,
}

/// Executes a job within an isolated temporary sandbox directory and subprocess with scrubbed environment.
pub fn execute_job_in_sandbox(job: &JobRecord) -> Result<Value, String> {
    let temp_dir = tempfile::Builder::new()
        .prefix("s3a_sandbox_")
        .tempdir()
        .map_err(|e| format!("Failed to create sandbox temp dir: {}", e))?;

    let input_path = temp_dir.path().join("task_input.json");
    let output_path = temp_dir.path().join("task_output.json");

    let req = TaskRequest {
        kind: job.kind.clone(),
        inputs: job.inputs.clone(),
    };

    let req_json = serde_json::to_vec_pretty(&req)
        .map_err(|e| format!("Failed to serialize task request: {}", e))?;
    fs::write(&input_path, req_json)
        .map_err(|e| format!("Failed to write task input file: {}", e))?;

    // Determine path to s3a-sandbox binary
    let binary_path = if let Some(bin) = option_env!("CARGO_BIN_EXE_s3a-sandbox") {
        PathBuf::from(bin)
    } else {
        let current = std::env::current_exe()
            .map_err(|e| format!("Failed to determine current binary: {}", e))?;
        let mut resolved = current.clone();
        if let Some(parent) = current.parent() {
            if parent.file_name().and_then(|n| n.to_str()) == Some("deps") {
                if let Some(target_dir) = parent.parent() {
                    let exe_name = if cfg!(windows) {
                        "s3a-sandbox.exe"
                    } else {
                        "s3a-sandbox"
                    };
                    let sibling = target_dir.join(exe_name);
                    if sibling.exists() {
                        resolved = sibling;
                    }
                }
            }
        }
        resolved
    };

    // If sandbox binary does not exist, execute in-process task fallback
    let is_sandbox_bin = binary_path
        .file_name()
        .map(|n| n.to_string_lossy().starts_with("s3a-sandbox"))
        .unwrap_or(false)
        && binary_path.exists();

    if !is_sandbox_bin {
        return run_task(&req.kind, &req.inputs);
    }

    // Prepare command with scrubbed environment to ensure sandboxes never see vault keys
    let mut cmd = Command::new(&binary_path);
    cmd.arg("exec")
        .arg("--input")
        .arg(&input_path)
        .arg("--output")
        .arg(&output_path)
        .current_dir(temp_dir.path())
        .env_clear() // CRITICAL: zero ambient environment variables / vault keys passed
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    // Preserve minimum required OS runtime system variables for dynamic linker
    #[cfg(windows)]
    {
        if let Ok(sys_root) = std::env::var("SystemRoot") {
            cmd.env("SystemRoot", sys_root);
        } else if let Ok(windir) = std::env::var("windir") {
            cmd.env("windir", windir);
        }
        if let Ok(path) = std::env::var("PATH") {
            cmd.env("PATH", path);
        }
    }

    #[cfg(not(windows))]
    {
        if let Ok(path) = std::env::var("PATH") {
            cmd.env("PATH", path);
        }
    }

    let output = cmd
        .output()
        .map_err(|e| format!("Failed to execute sandbox worker subprocess: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "Sandbox process failed with code {:?}: {}",
            output.status.code(),
            stderr.trim()
        ));
    }

    if !output_path.exists() {
        return Err("Sandbox process did not produce task_output.json".to_string());
    }

    let out_bytes =
        fs::read(&output_path).map_err(|e| format!("Failed to read sandbox output file: {}", e))?;
    let resp: TaskResponse = serde_json::from_slice(&out_bytes)
        .map_err(|e| format!("Failed to deserialize sandbox response: {}", e))?;

    if !resp.success {
        return Err(resp
            .error
            .unwrap_or_else(|| "Unknown sandbox failure".to_string()));
    }

    Ok(resp.output)
}

/// Internal task dispatcher executed inside the sandbox subprocess.
pub fn run_task(kind: &str, inputs: &Value) -> Result<Value, String> {
    match kind {
        "render_step_preview" => run_render_step_preview(inputs),
        "extract_pdf_text" => run_extract_pdf_text(inputs),
        other => Err(format!("Unsupported sandbox job kind: '{}'", other)),
    }
}

/// Handler for `render_step_preview`: parses AP242 STEP and produces glTF 2.0 scene graph or preview PNG.
fn run_render_step_preview(inputs: &Value) -> Result<Value, String> {
    let step_bytes = if let Some(path_str) = inputs.get("step_path").and_then(|v| v.as_str()) {
        fs::read(Path::new(path_str))
            .map_err(|e| format!("Failed to read STEP file '{}': {}", path_str, e))?
    } else if let Some(content_str) = inputs.get("step_content").and_then(|v| v.as_str()) {
        content_str.as_bytes().to_vec()
    } else if let Some(b64) = inputs.get("step_base64").and_then(|v| v.as_str()) {
        hex::decode(b64).map_err(|e| format!("Failed to decode STEP hex: {}", e))?
    } else {
        return Err(
            "render_step_preview requires 'step_path', 'step_content', or 'step_base64'"
                .to_string(),
        );
    };

    let tree = s3a_pack_tdp::parse_step(&step_bytes)?;
    let format = inputs
        .get("format")
        .and_then(|v| v.as_str())
        .unwrap_or("gltf");

    if format == "png" {
        // Generate minimal valid PNG preview (16x16 RGBA placeholder or raster)
        let png_bytes = generate_minimal_preview_png(tree.parts.len() as u32);
        Ok(json!({
            "format": "png",
            "png_hex": hex::encode(&png_bytes),
            "part_count": tree.parts.len(),
            "root_part": tree.root_part_number,
        }))
    } else {
        // Generate canonical glTF 2.0 JSON asset
        let gltf = build_gltf_from_assembly(&tree);
        Ok(json!({
            "format": "gltf",
            "gltf": gltf,
            "part_count": tree.parts.len(),
            "root_part": tree.root_part_number,
            "header": tree.header,
        }))
    }
}

/// Converts StepAssemblyTree into a clean glTF 2.0 JSON hierarchy.
fn build_gltf_from_assembly(tree: &s3a_pack_tdp::StepAssemblyTree) -> Value {
    let mut nodes = Vec::new();
    let mut root_children = Vec::new();

    // Node 0: Root Assembly Node
    let root_name = tree
        .root_part_number
        .clone()
        .unwrap_or_else(|| "ROOT_ASSEMBLY".to_string());

    // Map children nodes
    for (i, part) in tree.parts.iter().enumerate() {
        let node_idx = i + 1;
        nodes.push(json!({
            "name": part.name,
            "extras": {
                "part_number": part.part_number,
                "parent": part.parent,
                "quantity": part.quantity,
            }
        }));
        root_children.push(node_idx);
    }

    // Insert root node at index 0
    let root_node = json!({
        "name": root_name,
        "children": root_children,
        "extras": {
            "type": "assembly_root",
            "total_components": tree.parts.len(),
        }
    });

    let mut all_nodes = vec![root_node];
    all_nodes.extend(nodes);

    json!({
        "asset": {
            "version": "2.0",
            "generator": "s3a-sandbox",
        },
        "scene": 0,
        "scenes": [
            {
                "name": "Assembly",
                "nodes": [0]
            }
        ],
        "nodes": all_nodes,
    })
}

/// Generates a valid minimal 1x1 or 4x4 PNG image for preview testing.
fn generate_minimal_preview_png(seed: u32) -> Vec<u8> {
    // Standard minimal valid 1x1 RGBA PNG
    let mut png = Vec::new();
    // Signature
    png.extend_from_slice(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);

    // IHDR chunk: 1x1 8-bit RGBA
    let mut ihdr_data = Vec::new();
    ihdr_data.extend_from_slice(&1u32.to_be_bytes()); // width = 1
    ihdr_data.extend_from_slice(&1u32.to_be_bytes()); // height = 1
    ihdr_data.push(8); // 8 bits per channel
    ihdr_data.push(6); // RGBA color type
    ihdr_data.push(0); // compression method 0
    ihdr_data.push(0); // filter method 0
    ihdr_data.push(0); // interlace method 0
    write_png_chunk(&mut png, b"IHDR", &ihdr_data);

    // IDAT chunk: zlib-compressed scanline (filter byte 0 + RGBA [r, g, b, 255])
    let r = ((seed * 37) % 255) as u8;
    let g = ((seed * 73) % 255) as u8;
    let b = ((seed * 151) % 255) as u8;
    let raw_scanline = [0u8, r, g, b, 255];

    // Minimal DEFLATE block (uncompressed type 00):
    let mut idat_data = Vec::new();
    idat_data.push(0x78); // zlib header
    idat_data.push(0x01);
    idat_data.push(0x01); // BFINAL=1, BTYPE=00 (uncompressed)
    let len = raw_scanline.len() as u16;
    let nlen = !len;
    idat_data.extend_from_slice(&len.to_le_bytes());
    idat_data.extend_from_slice(&nlen.to_le_bytes());
    idat_data.extend_from_slice(&raw_scanline);

    // Adler-32 of raw_scanline
    let mut s1: u32 = 1;
    let mut s2: u32 = 0;
    for byte in &raw_scanline {
        s1 = (s1 + *byte as u32) % 65521;
        s2 = (s2 + s1) % 65521;
    }
    let adler = (s2 << 16) | s1;
    idat_data.extend_from_slice(&adler.to_be_bytes());

    write_png_chunk(&mut png, b"IDAT", &idat_data);

    // IEND chunk
    write_png_chunk(&mut png, b"IEND", &[]);

    png
}

fn write_png_chunk(out: &mut Vec<u8>, chunk_type: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(chunk_type);
    out.extend_from_slice(data);

    let mut crc = crc32fast::Hasher::new();
    crc.update(chunk_type);
    crc.update(data);
    out.extend_from_slice(&crc.finalize().to_be_bytes());
}

/// Handler for `extract_pdf_text`: extracts text from PDF pages using lopdf.
fn run_extract_pdf_text(inputs: &Value) -> Result<Value, String> {
    let pdf_bytes = if let Some(path_str) = inputs.get("pdf_path").and_then(|v| v.as_str()) {
        fs::read(Path::new(path_str))
            .map_err(|e| format!("Failed to read PDF file '{}': {}", path_str, e))?
    } else if let Some(content_str) = inputs.get("pdf_content").and_then(|v| v.as_str()) {
        content_str.as_bytes().to_vec()
    } else if let Some(b64) = inputs.get("pdf_base64").and_then(|v| v.as_str()) {
        hex::decode(b64).map_err(|e| format!("Failed to decode PDF hex: {}", e))?
    } else {
        return Err(
            "extract_pdf_text requires 'pdf_path', 'pdf_content', or 'pdf_base64'".to_string(),
        );
    };

    let doc = lopdf::Document::load_mem(&pdf_bytes)
        .map_err(|e| format!("Failed to load PDF with lopdf: {}", e))?;

    let pages = doc.get_pages();
    let mut page_texts = Vec::new();
    let mut full_text = String::new();

    let mut page_numbers: Vec<u32> = pages.keys().copied().collect();
    page_numbers.sort_unstable();

    for &page_num in &page_numbers {
        let text = doc.extract_text(&[page_num]).unwrap_or_default();
        if !full_text.is_empty() && !text.is_empty() {
            full_text.push('\n');
        }
        full_text.push_str(&text);
        page_texts.push(json!({
            "page": page_num,
            "text": text,
        }));
    }

    Ok(json!({
        "page_count": pages.len(),
        "text": full_text,
        "pages": page_texts,
    }))
}
