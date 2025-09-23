// src/main.rs
use anyhow::{bail, Context, Result};
use base64::{engine::general_purpose, Engine as _};
use chrono::Utc;
use dialoguer::{Confirm, Input, Select};
use ed25519_dalek::{Keypair, Signer, Signature};
use genpdf::{elements, Alignment};
use indicatif::{ProgressBar, ProgressStyle};
use qrcodegen::{QrCode, QrCodeEcc};
use rand::rngs::OsRng;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::io::BufReader;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;
use uuid::Uuid;
use std::env;
use image::ImageFormat;
use std::fs::File;
use std::convert::TryInto;

#[derive(Debug)]
struct BlockDevice {
    name: String,
    size: String,
    model: Option<String>,
    serial: Option<String>,
    devtype: String,
}

fn run_lsblk(dry_run: bool) -> Result<Vec<BlockDevice>> {
    let out = Command::new("lsblk")
        .arg("-J")
        .arg("-o")
        .arg("NAME,MODEL,SERIAL,SIZE,TYPE,RM,TRAN")
        .output()
        .context("lsblk failed — ensure util-linux is installed")?;
    if !out.status.success() {
        bail!("lsblk returned non-zero");
    }
    let v: Value = serde_json::from_slice(&out.stdout)?;
    let mut devices = Vec::new();
    if let Some(arr) = v.get("blockdevices").and_then(|x| x.as_array()) {
        for dev in arr {
            let t = dev.get("type").and_then(|x| x.as_str()).unwrap_or("").to_string();
            // Only include real disks when not in dry-run.
            // Allow loop devices only when dry_run == true (for safe testing).
            if t != "disk" && !(dry_run && t == "loop") {
                continue;
            }
            let name = dev.get("name").and_then(|x| x.as_str()).unwrap_or("").to_string();
            let size = dev.get("size").and_then(|x| x.as_str()).unwrap_or("").to_string();
            let model = dev.get("model")
                .and_then(|x| x.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
            let serial = dev.get("serial")
                .and_then(|x| x.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
            devices.push(BlockDevice { name, size, model, serial, devtype: t });
        }
    }
    Ok(devices)
}

fn pretty_device_label(dev: &BlockDevice) -> String {
    let mut s = format!("/dev/{} — {}", dev.name, dev.size);
    if let Some(m) = &dev.model {
        s.push_str(&format!(" • {}", m));
    }
    if let Some(sn) = &dev.serial {
        let tail = if sn.len() > 4 { &sn[sn.len()-4..] } else { sn.as_str() };
        s.push_str(&format!(" • SN: ****{}", tail));
    }
    s
}

fn canonicalize_value(v: &Value) -> Value {
    match v {
        Value::Object(map) => {
            let mut btm = BTreeMap::new();
            for (k, val) in map.iter() {
                btm.insert(k.clone(), canonicalize_value(val));
            }
            let mut obj = serde_json::Map::new();
            for (k, val) in btm.into_iter() {
                obj.insert(k, val);
            }
            Value::Object(obj)
        }
        Value::Array(arr) => Value::Array(arr.iter().map(canonicalize_value).collect()),
        other => other.clone(),
    }
}

fn canonical_json_bytes(v: &Value) -> Result<Vec<u8>> {
    let canon = canonicalize_value(v);
    let mut buf = Vec::new();
    serde_json::to_writer(&mut buf, &canon)?;
    Ok(buf)
}

fn load_or_create_keypair(path: &Path) -> Result<Keypair> {
    if path.exists() {
        let content = fs::read_to_string(path)?;
        let bytes = general_purpose::STANDARD.decode(content.trim())?;
        // bytes must be 64 bytes for ed25519-dalek Keypair
        let arr: [u8; 64] = bytes.as_slice().try_into().context("sign_key.b64 malformed or wrong length")?;
        let kp = Keypair::from_bytes(&arr)?;
        Ok(kp)
    } else {
        let mut csprng = OsRng{};
        let kp = Keypair::generate(&mut csprng);
        let encoded = general_purpose::STANDARD.encode(kp.to_bytes());
        fs::write(path, encoded)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
        }
        Ok(kp)
    }
}

/// Build a PNG QR by rasterizing the qrcodegen matrix into a grayscale image.
fn write_qr_png(data: &str, path: &Path) -> Result<()> {
    // Build QR code (low-level lib that gives module matrix)
    let qr = QrCode::encode_text(data, QrCodeEcc::Medium)
        .map_err(|e| anyhow::anyhow!(format!("qr encode failed: {:?}", e)))?;

    let size = qr.size(); // number of modules (width/height)
    let scale: u32 = 8; // pixels per module (adjust visual size)
    let border: u32 = 4; // quiet zone modules
    let img_size = (size as u32) * scale + 2 * border;

    // create a grayscale image buffer (white background)
    let mut imgbuf = image::GrayImage::from_pixel(img_size, img_size, image::Luma([255u8]));

    for y in 0..size {
        for x in 0..size {
            if qr.get_module(x, y) {
                // draw black square
                let px = border + (x as u32) * scale;
                let py = border + (y as u32) * scale;
                for dy in 0..scale {
                    for dx in 0..scale {
                        imgbuf.put_pixel(px + dx, py + dy, image::Luma([0u8]));
                    }
                }
            }
        }
    }

    imgbuf.save(path)?;
    Ok(())
}

fn run_destructive_command(mut cmd: Command, dry_run: bool) -> Result<()> {
    if dry_run {
        println!("DRY-RUN: would run command: {:?}", cmd);
        return Ok(());
    }
    let mut child = cmd
        .stderr(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stdin(Stdio::null())
        .spawn()
        .context("failed to spawn destructive command")?;
    let status = child.wait()?;
    if !status.success() {
        bail!("destructive command failed: exit {:?}", status.code());
    }
    Ok(())
}

fn sample_sha256_first_mb(device_path: &str) -> Result<String> {
    let out = Command::new("dd")
        .arg(format!("if={}", device_path))
        .arg("bs=1M")
        .arg("count=1")
        .arg("status=none")
        .output()
        .context("dd failed during sample read")?;
    let mut hasher = Sha256::new();
    hasher.update(&out.stdout);
    Ok(hex::encode(hasher.finalize()))
}

fn make_pdf_with_genpdf(signed: &serde_json::Value, qr_path: &Path, out_pdf: &Path) -> Result<()> {
    // load font: prefer local assets/DejaVuSans.ttf or fallback to system path
    let font_dir_candidates = [
        "./assets",
        "/usr/share/fonts/truetype/dejavu",
        "/usr/share/fonts/truetype",
    ];
    let mut chosen = None;
    for d in &font_dir_candidates {
        let p = Path::new(d);
        if p.exists() {
            chosen = Some(p.to_path_buf());
            break;
        }
    }
    let fonts_path = chosen.context("No font directory found; include assets/DejaVuSans.ttf in project or install DejaVu fonts")?;
    let font_family = genpdf::fonts::from_files(&fonts_path, "DejaVuSans", None)
        .context("failed to load font family from path")?;

    let mut doc = genpdf::Document::new(font_family);
    doc.set_title("Device Wipe Certificate");
    doc.set_minimal_conformance();

    let mut decorator = genpdf::SimplePageDecorator::new();
    decorator.set_margins(10);
    doc.set_page_decorator(decorator);

    // Title
    let mut title = elements::Paragraph::new("Device Wipe Certificate");
    title.set_alignment(Alignment::Center);
    doc.push(title);
    doc.push(elements::Break::new(1));

    // QR image: load PNG and convert to genpdf::Image, note from_dynamic_image returns Result<Image,_>
    let file = File::open(qr_path).context("open qr png")?;
    let reader = BufReader::new(file);
    let img = image::load(reader, ImageFormat::Png).context("load qr png")?;
    let img_element = elements::Image::from_dynamic_image(img)?; // propagate errors
    // set alignment and push
    let mut img_element = img_element;
    img_element.set_alignment(Alignment::Center);
    doc.push(img_element);
    doc.push(elements::Break::new(1));

    // Pretty JSON body
    let pretty = serde_json::to_string_pretty(signed)?;
    let mut pre = elements::Paragraph::new(pretty);
    pre.set_alignment(Alignment::Left);
    doc.push(pre);
    doc.push(elements::Break::new(1));

    let mut footer = elements::Paragraph::new(format!("Generated: {}", chrono::Utc::now().to_rfc3339()));
    footer.set_alignment(Alignment::Right);
    doc.push(footer);

    let mut outfile = File::create(out_pdf)?;
    doc.render(&mut outfile)?;
    Ok(())
}

fn main() -> Result<()> {
    // DRY_RUN via env var for quick testing
    let dry_run = env::var("EWIPE_DRYRUN").is_ok();
    println!("e_wipe — core MVP. DRY_RUN={}", dry_run);

    println!("Enumerating block devices via lsblk...");
    let devices = run_lsblk(dry_run)?;
    if devices.is_empty() {
        println!("No block devices found.");
        return Ok(());
    }

    let labels: Vec<String> = devices.iter().map(|d| pretty_device_label(d)).collect();
    let idx = Select::new()
        .with_prompt("Select device to wipe")
        .items(&labels)
        .default(0)
        .interact()?;
    let selected = &devices[idx];
    let dev_path = format!("/dev/{}", selected.name);
    println!("Selected {}", pretty_device_label(selected));

    // typed confirmation token
    let token = format!("WIPE-{}-{}", selected.name, &Uuid::new_v4().to_string()[..8]);
    println!("\n*** DESTRUCTIVE ACTION ***");
    println!("Type the confirmation token to proceed: {}", token);
    let typed: String = Input::new().with_prompt("token").interact_text()?;
    if typed.trim() != token {
        bail!("token mismatch — aborting.");
    }
    if !Confirm::new().with_prompt(format!("Really wipe {}?", dev_path)).default(false).interact()? {
        println!("Aborted.");
        return Ok(());
    }

    // Plan: single-pass random overwrite (MVP). Replace later with vendor secure-erase per device type.
    let started_at = Utc::now();
    println!("Wipe plan: single-pass random overwrite (MVP).");

    let pb = ProgressBar::new_spinner();
    pb.set_style(ProgressStyle::with_template("{spinner} {wide_msg}").unwrap());
    pb.enable_steady_tick(Duration::from_millis(120));
    pb.set_message("Running wipe command...");

    // destructive command: dd via sudo (so user must enter password)
    let mut dd_cmd = Command::new("sudo");
    dd_cmd.arg("dd")
        .arg("if=/dev/urandom")
        .arg(format!("of={}", dev_path))
        .arg("bs=4M")
        .arg("status=progress")
        .arg("conv=fsync");

    run_destructive_command(dd_cmd, dry_run)?;
    pb.finish_with_message("Wipe command finished.");

    // forensic sample
    let sample_hash = if dry_run {
        "dryrun-sample".to_string()
    } else {
        sample_sha256_first_mb(&dev_path).unwrap_or_else(|_| "sample_failed".to_string())
    };
    let finished_at = Utc::now();

    // certificate assembly
    let wipe_id = Uuid::new_v4().to_string();
    let device_val = serde_json::json!({
        "name": selected.name,
        "size": selected.size,
        "model": selected.model,
        "serial_tail": selected.serial.as_ref().map(|s| s[s.len().saturating_sub(4)..].to_string()),
        "path": dev_path,
    });
    let wipe_obj = serde_json::json!({
        "method": "OVERWRITE_RANDOM_SINGLEPASS",
        "passes": 1,
        "started_at": started_at.to_rfc3339(),
        "finished_at": finished_at.to_rfc3339(),
        "operator_id": None::<String>
    });
    let forensic = serde_json::json!({
        "first_mb_sha256": sample_hash,
        "notes": "MVP sample. SSDs require secure-erase or crypto-erase; dd is unreliable for SSDs."
    });

    let mut unsigned = serde_json::Map::new();
    unsigned.insert("version".to_string(), serde_json::Value::String("1.0".to_string()));
    unsigned.insert("wipe_id".to_string(), serde_json::Value::String(wipe_id.clone()));
    unsigned.insert("device".to_string(), device_val.clone());
    unsigned.insert("wipe".to_string(), wipe_obj.clone());
    unsigned.insert("forensic".to_string(), forensic.clone());
    let unsigned_val = serde_json::Value::Object(unsigned);

    // canonicalize & sign
    let canon_bytes = canonical_json_bytes(&unsigned_val)?;
    let keypath = Path::new("sign_key.b64");
    let keypair = load_or_create_keypair(keypath)?;
    let signature: Signature = keypair.sign(&canon_bytes);
    let sig_b64 = general_purpose::STANDARD.encode(signature.to_bytes());
    let pub_hash = &Sha256::digest(&keypair.public.to_bytes());
    let keyid = hex::encode(&pub_hash)[..16].to_string();

    let signed = serde_json::json!({
        "version": "1.0",
        "wipe_id": wipe_id,
        "device": device_val,
        "wipe": wipe_obj,
        "forensic": forensic,
        "signing_key_id": keyid,
        "signature": sig_b64
    });

    // save JSON certificate
    let json_name = format!("wipe_cert_{}.json", &signed["wipe_id"].as_str().unwrap()[..8]);
    fs::write(&json_name, serde_json::to_string_pretty(&signed)?)?;
    println!("Signed JSON certificate written: {}", json_name);

    // QR
    let mut hasher = Sha256::new();
    hasher.update(serde_json::to_vec(&signed)?);
    let json_hash = hex::encode(hasher.finalize());
    let qr_str = format!("wipe-cert:sha256:{}", json_hash);
    let qr_path = Path::new("cert_qr.png");
    write_qr_png(&qr_str, qr_path)?;
    println!("QR image written: {}", qr_path.display());

    // PDF via genpdf
    let pdf_path = Path::new("wipe_certificate.pdf");
    match make_pdf_with_genpdf(&signed, qr_path, pdf_path) {
        Ok(_) => println!("PDF certificate written: {}", pdf_path.display()),
        Err(e) => println!("PDF generation failed: {}. JSON saved.", e),
    }

    println!("Done. JSON: {}  PDF: {:?}", json_name, pdf_path);
    Ok(())
}