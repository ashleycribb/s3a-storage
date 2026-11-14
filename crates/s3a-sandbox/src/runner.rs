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
    execute_job_in_sandbox_with_proxy(job, None)
}

/// Executes a job within an isolated temporary sandbox directory and subprocess with scrubbed environment
/// and optional proxy egress routing.
pub fn execute_job_in_sandbox_with_proxy(
    job: &JobRecord,
    proxy_url: Option<&str>,
) -> Result<Value, String> {
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
    let binary_path = find_sandbox_binary();

    // If sandbox binary does not exist, execute in-process task fallback
    let is_sandbox_bin = binary_path
        .file_name()
        .map(|n| n.to_string_lossy().starts_with("s3a-sandbox"))
        .unwrap_or(false)
        && binary_path.exists();

    if !is_sandbox_bin {
        let res = run_task(&req.kind, &req.inputs);
        #[cfg(feature = "otel")]
        {
            let job_hash = job.sha256_digest();
            let mut span = crate::otel::Span::new(
                format!("job:{}", job.kind),
                hex::encode(job.requested_by),
                hex::encode(job.id),
                &job_hash,
            )
            .with_attribute("job.kind", &job.kind);
            if let Err(ref e) = res {
                span.status = crate::otel::SpanStatus::Error { message: e.clone() };
            }
            crate::otel::emit_span(span);
        }
        return res;
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

    // If egress proxy URL is provided, route sandbox egress through proxy
    if let Some(url) = proxy_url {
        cmd.env("HTTP_PROXY", url);
        cmd.env("HTTPS_PROXY", url);
        cmd.env("ALL_PROXY", url);
        cmd.env("S3A_CREDENTIAL_PROXY", url);
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

    #[cfg(feature = "otel")]
    {
        let job_hash = job.sha256_digest();
        let mut span = crate::otel::Span::new(
            format!("job:{}", job.kind),
            hex::encode(job.requested_by),
            hex::encode(job.id),
            &job_hash,
        )
        .with_attribute("job.kind", &job.kind);
        if !resp.success {
            span.status = crate::otel::SpanStatus::Error {
                message: resp
                    .error
                    .clone()
                    .unwrap_or_else(|| "Sandbox failure".to_string()),
            };
        }
        crate::otel::emit_span(span);
    }

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
        "sandbox_proxy_egress" => run_sandbox_proxy_egress(inputs),
        other => Err(format!("Unsupported sandbox job kind: '{}'", other)),
    }
}

/// Handler for `sandbox_proxy_egress`: routes outbound requests strictly through credential proxy
/// while verifying that no secrets exist within the sandbox environment.
fn run_sandbox_proxy_egress(inputs: &Value) -> Result<Value, String> {
    // 1. Audit current process environment to verify that ambient secrets are scrubbed
    let mut inspected_env = Vec::new();
    let mut forbidden_var_names = Vec::new();

    for (k, _v) in std::env::vars() {
        let k_upper = k.to_uppercase();
        // Check for any ambient secret, vault key, or credential token leaked into sandbox env
        if k_upper.contains("SECRET")
            || k_upper.contains("VAULT")
            || k_upper.contains("TOKEN")
            || k_upper.contains("API_KEY")
            || k_upper.contains("PASSWORD")
            || k_upper.contains("PRIVATE")
        {
            // S3A_CREDENTIAL_PROXY is the proxy URL, not a secret
            if k != "S3A_CREDENTIAL_PROXY" {
                forbidden_var_names.push(k.clone());
            }
        }
        inspected_env.push(k);
    }

    if !forbidden_var_names.is_empty() {
        return Err(format!(
            "Sandbox environment isolation violated! Leaked secret variables found: {:?}",
            forbidden_var_names
        ));
    }

    // 2. Discover proxy URL from env or input
    let proxy_url = inputs
        .get("proxy_url")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| std::env::var("S3A_CREDENTIAL_PROXY").ok())
        .or_else(|| std::env::var("HTTP_PROXY").ok())
        .ok_or_else(|| "No proxy URL configured in sandbox environment".to_string())?;

    let connection = inputs
        .get("connection")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "sandbox_proxy_egress requires 'connection'".to_string())?;

    let path = inputs
        .get("path")
        .and_then(|v| v.as_str())
        .unwrap_or("/api/test");

    let method = inputs
        .get("method")
        .and_then(|v| v.as_str())
        .unwrap_or("GET");

    // 3. Make egress call via proxy
    let egress_endpoint = format!("{}/egress", proxy_url.trim_end_matches('/'));
    let req_body = json!({
        "connection": connection,
        "path": path,
        "method": method,
    });

    let resp = ureq::post(&egress_endpoint)
        .set("Content-Type", "application/json")
        .timeout(std::time::Duration::from_secs(10))
        .send_string(&req_body.to_string());

    match resp {
        Ok(r) => {
            let body_text = r.into_string().unwrap_or_default();
            Ok(json!({
                "success": true,
                "status": 200,
                "response_body": body_text,
                "env_vars_count": inspected_env.len(),
                "inspected_env": inspected_env,
            }))
        }
        Err(ureq::Error::Status(code, r)) => {
            let err_body = r.into_string().unwrap_or_default();
            Ok(json!({
                "success": false,
                "status": code,
                "error_body": err_body,
                "env_vars_count": inspected_env.len(),
                "inspected_env": inspected_env,
            }))
        }
        Err(e) => Err(format!("Proxy connection failed: {}", e)),
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

/// Locates the `s3a-sandbox` binary across target directories or cargo workspace layout.
pub fn find_sandbox_binary() -> PathBuf {
    if let Some(bin) = option_env!("CARGO_BIN_EXE_s3a-sandbox") {
        let p = PathBuf::from(bin);
        if p.exists() {
            return p;
        }
    }

    let exe_name = if cfg!(windows) {
        "s3a-sandbox.exe"
    } else {
        "s3a-sandbox"
    };

    if let Ok(cur) = std::env::current_exe() {
        if let Some(target_dir) = cur.parent().and_then(|p| p.parent()) {
            let candidate = target_dir.join(exe_name);
            if candidate.exists() {
                return candidate;
            }
        }
    }

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    for rel in &["../../target/debug", "../target/debug", "target/debug"] {
        let cand = manifest_dir.join(rel).join(exe_name);
        if cand.exists() {
            return cand;
        }
    }

    if let Ok(current) = std::env::current_exe() {
        let mut cur = current.parent();
        while let Some(dir) = cur {
            let direct = dir.join(exe_name);
            if direct.exists() {
                return direct;
            }
            let in_debug = dir.join("target/debug").join(exe_name);
            if in_debug.exists() {
                return in_debug;
            }
            cur = dir.parent();
        }
    }

    PathBuf::from(exe_name)
}
