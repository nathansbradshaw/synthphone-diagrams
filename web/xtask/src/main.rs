use std::{
    collections::hash_map::DefaultHasher,
    env, fs,
    hash::{Hash, Hasher},
    io::{Read, Write},
    net::TcpListener,
    path::{Path, PathBuf},
    process::{Command, ExitCode},
};
fn run(command: &str, args: &[&str], root: &Path) -> Result<(), String> {
    let status = Command::new(command)
        .args(args)
        .current_dir(root)
        .status()
        .map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{command} failed: {status}"))
    }
}
fn main() -> ExitCode {
    match execute() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}
fn copy_assets(from: &Path, to: &Path, hash: &mut DefaultHasher) -> Result<(), String> {
    fs::create_dir_all(to).map_err(|e| e.to_string())?;
    let mut entries = fs::read_dir(from)
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        if entry.path().extension().is_some_and(|e| e == "webp") {
            let bytes = fs::read(entry.path()).map_err(|e| e.to_string())?;
            bytes.hash(hash);
            fs::write(to.join(entry.file_name()), bytes).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}
fn execute() -> Result<(), String> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf();
    match env::args().nth(1).as_deref().unwrap_or("build") {
        "build" => {
            let target = env::var_os("CARGO_TARGET_DIR")
                .map(PathBuf::from)
                .unwrap_or(root.join("target"));
            let wasm = target.join("wasm32-unknown-unknown/release/dsp_viewer_web.wasm");
            let out = root.join("dist/pkg");
            fs::create_dir_all(&out).map_err(|e| e.to_string())?;
            run(
                "cargo",
                &[
                    "build",
                    "--locked",
                    "--release",
                    "--target",
                    "wasm32-unknown-unknown",
                    "--no-default-features",
                ],
                &root,
            )?;
            fs::copy(&wasm, root.join("dist/engine.wasm")).map_err(|e| e.to_string())?;
            run(
                "cargo",
                &[
                    "build",
                    "--locked",
                    "--release",
                    "--target",
                    "wasm32-unknown-unknown",
                ],
                &root,
            )?;
            run(
                "wasm-bindgen",
                &[
                    wasm.to_str().unwrap(),
                    "--target",
                    "web",
                    "--out-dir",
                    out.to_str().unwrap(),
                    "--no-typescript",
                ],
                &root,
            )?;
            for name in [
                "index.html",
                "instrument.css",
                "photographic.css",
                "audio-worklet.js",
            ] {
                fs::copy(root.join(name), root.join("dist").join(name))
                    .map_err(|e| e.to_string())?;
            }
            fs::write(root.join("dist/.nojekyll"), "").map_err(|e| e.to_string())?;
            let mut hash = DefaultHasher::new();
            copy_assets(&root.join("assets"), &root.join("dist/assets"), &mut hash)?;
            for name in [
                "index.html",
                "instrument.css",
                "photographic.css",
                "audio-worklet.js",
                "engine.wasm",
                "pkg/dsp_viewer_web.js",
                "pkg/dsp_viewer_web_bg.wasm",
            ] {
                fs::read(root.join("dist").join(name))
                    .map_err(|e| e.to_string())?
                    .hash(&mut hash);
            }
            let build = format!("{:016x}", hash.finish());
            let html =
                fs::read_to_string(root.join("dist/index.html")).map_err(|e| e.to_string())?;
            fs::write(
                root.join("dist/index.html"),
                html.replace("__BUILD__", &build),
            )
            .map_err(|e| e.to_string())?;
            let revision = Command::new("git")
                .args(["rev-parse", "HEAD"])
                .current_dir(&root)
                .output()
                .map_err(|e| e.to_string())?;
            let revision = String::from_utf8_lossy(&revision.stdout).trim().to_string();
            let dirty = !Command::new("git")
                .args(["diff", "--quiet", "HEAD", "--", "."])
                .current_dir(&root)
                .status()
                .map_err(|e| e.to_string())?
                .success();
            fs::write(root.join("dist/version.json"), format!("{{\"source_commit\":\"{revision}\",\"build\":\"{build}\",\"dirty_source\":{dirty}}}\n")).map_err(|e|e.to_string())?;
            println!("Built {}", root.join("dist").display());
            Ok(())
        }
        "serve" => {
            let port = env::args()
                .nth(2)
                .unwrap_or("3040".into())
                .parse::<u16>()
                .map_err(|e| e.to_string())?;
            let listener = TcpListener::bind(("127.0.0.1", port)).map_err(|e| e.to_string())?;
            println!("Local: http://127.0.0.1:{port}");
            for connection in listener.incoming() {
                let Ok(mut stream) = connection else { continue };
                let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(2)));
                let mut request = [0; 8192];
                let Ok(n) = stream.read(&mut request) else {
                    continue;
                };
                let req = String::from_utf8_lossy(&request[..n]);
                let path = req
                    .split_whitespace()
                    .nth(1)
                    .unwrap_or("/")
                    .split('?')
                    .next()
                    .unwrap_or("/");
                let path = if path == "/" {
                    "index.html"
                } else {
                    path.trim_start_matches('/')
                };
                if path.contains("..") || path.contains('%') {
                    continue;
                }
                let file = root.join("dist").join(path);
                let body = fs::read(&file);
                let mime = match file.extension().and_then(|e| e.to_str()).unwrap_or("") {
                    "html" => "text/html; charset=utf-8",
                    "css" => "text/css",
                    "js" => "text/javascript",
                    "wasm" => "application/wasm",
                    "webp" => "image/webp",
                    _ => "application/octet-stream",
                };
                let (status, body) = match body {
                    Ok(b) => ("200 OK", b),
                    Err(_) => ("404 Not Found", b"Not found".to_vec()),
                };
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status}\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(&body);
            }
            Ok(())
        }
        _ => Err("Use build or serve [port]".into()),
    }
}
