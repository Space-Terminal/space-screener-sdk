use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use flate2::read::GzDecoder;
use reqwest::blocking::Client;
use sha2::{Digest, Sha256};

/// Pinned by version AND hash: the binary of v1.7.0 reports itself as 1.6.1.
pub const EXTISM_JS_VERSION: &str = "1.7.0";
pub const BINARYEN_VERSION: &str = "version_133";
/// Folder for downloaded tools (default: the OS cache folder + `space-screener/toolchain`).
pub const CACHE_ENV: &str = "ST_CACHE_DIR";
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(900);

struct Artifact {
    name: &'static str,
    sha256: &'static str,
}

struct Platform {
    extism_js: Artifact,
    binaryen: Artifact,
}

fn platform() -> Result<Platform> {
    let (extism_js, binaryen) = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => (
            (
                "extism-js-aarch64-macos-v1.7.0.gz",
                "12c01c2bb2240a6a05a4f8babe680c793c397d84e6a1497a7ca1312e78a475c3",
            ),
            (
                "binaryen-version_133-arm64-macos.tar.gz",
                "ad66da82ac13f163e424b1643f16c6dfcccc98b5966296b43e52d3cab04f84a8",
            ),
        ),
        ("macos", "x86_64") => (
            (
                "extism-js-x86_64-macos-v1.7.0.gz",
                "cce4a756eceb34b5aaac5ea864607d6c8b0610535e2c1b0c05a96bdbda69c7bb",
            ),
            (
                "binaryen-version_133-x86_64-macos.tar.gz",
                "13a9b90be775c6389ce3d1f879cb8627bea56708ba8c122983941d53a8199b95",
            ),
        ),
        ("linux", "x86_64") => (
            (
                "extism-js-x86_64-linux-v1.7.0.gz",
                "63b72da2f5e88655522dc21477de549f238a2f40546a69ce4e0fce7e78654035",
            ),
            (
                "binaryen-version_133-x86_64-linux.tar.gz",
                "2dc9c7813f5375db93d96ead4b78222fcc3e2677bbb832297af4797782a37489",
            ),
        ),
        ("linux", "aarch64") => (
            (
                "extism-js-aarch64-linux-v1.7.0.gz",
                "025f4050b199d68413c159bde1187271ae270021a9f7171e7beb509922821f2a",
            ),
            (
                "binaryen-version_133-aarch64-linux.tar.gz",
                "89c07ea56faf38d0fbecf36ca8ec0721756716185f265b568e133d427f299bf8",
            ),
        ),
        ("windows", "x86_64") => (
            (
                "extism-js-x86_64-windows-v1.7.0.gz",
                "409ac023f88f79d763fbf9dd5bb5d9b2fd595ac452e0822d49bf7d05bf6794be",
            ),
            (
                "binaryen-version_133-x86_64-windows.tar.gz",
                "17a2cbeac6b5693c5fbafab3838d3c65fd9c1eb38b05f5baec6c657e8c84995b",
            ),
        ),
        (os, arch) => bail!(
            "TypeScript screeners need extism-js and binaryen, which publish no build for {os}/{arch}; \
             write the screener in Rust or build on macOS, Linux (x86_64/arm64) or Windows x86_64"
        ),
    };
    Ok(Platform {
        extism_js: Artifact {
            name: extism_js.0,
            sha256: extism_js.1,
        },
        binaryen: Artifact {
            name: binaryen.0,
            sha256: binaryen.1,
        },
    })
}

pub struct Tools {
    pub extism_js: PathBuf,
    /// binaryen's `bin/`: wasm-merge, wasm-metadce, wasm-opt. extism-js needs it on PATH too.
    pub binaryen_bin: PathBuf,
}

impl Tools {
    pub fn binaryen(&self, tool: &str) -> PathBuf {
        self.binaryen_bin
            .join(format!("{tool}{}", std::env::consts::EXE_SUFFIX))
    }
}

fn cache_dir() -> Result<PathBuf> {
    if let Some(dir) = std::env::var_os(CACHE_ENV) {
        return Ok(PathBuf::from(dir));
    }
    dirs::cache_dir()
        .map(|d| d.join("space-screener").join("toolchain"))
        .with_context(|| format!("cannot locate the cache folder; set {CACHE_ENV}"))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn verify(bytes: &[u8], artifact: &Artifact) -> Result<()> {
    let actual = hex(&Sha256::digest(bytes));
    if actual != artifact.sha256 {
        bail!(
            "{}: sha256 {actual} does not match the pinned {}; the download is corrupt or was replaced",
            artifact.name,
            artifact.sha256
        );
    }
    Ok(())
}

fn download(url: &str, artifact: &Artifact) -> Result<Vec<u8>> {
    eprintln!("downloading {} (one time)…", artifact.name);
    let client = Client::builder()
        .timeout(DOWNLOAD_TIMEOUT)
        .user_agent(concat!("st/", env!("CARGO_PKG_VERSION")))
        .build()
        .context("cannot build the HTTP client")?;
    let resp = client
        .get(url)
        .send()
        .with_context(|| format!("cannot download {url}"))?;
    if !resp.status().is_success() {
        bail!("cannot download {url}: HTTP {}", resp.status());
    }
    let bytes = resp
        .bytes()
        .with_context(|| format!("cannot download {url}"))?
        .to_vec();
    verify(&bytes, artifact)?;
    Ok(bytes)
}

/// A temporary sibling of `target`, renamed into place once complete: an interrupted download
/// never leaves a half-written tool behind.
fn staging(target: &Path) -> PathBuf {
    let name = target
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("tool");
    target.with_file_name(format!(".{name}.partial-{}", std::process::id()))
}

#[cfg(unix)]
fn make_executable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
        .with_context(|| format!("cannot make {} executable", path.display()))
}

#[cfg(not(unix))]
fn make_executable(_: &Path) -> Result<()> {
    Ok(())
}

fn ensure_extism_js(cache: &Path, artifact: &Artifact) -> Result<PathBuf> {
    let dir = cache.join(format!("extism-js-{EXTISM_JS_VERSION}"));
    let exe = dir.join(format!("extism-js{}", std::env::consts::EXE_SUFFIX));
    if exe.is_file() {
        return Ok(exe);
    }
    let url = format!(
        "https://github.com/extism/js-pdk/releases/download/v{EXTISM_JS_VERSION}/{}",
        artifact.name
    );
    let gz = download(&url, artifact)?;
    let mut binary = Vec::new();
    GzDecoder::new(gz.as_slice())
        .read_to_end(&mut binary)
        .with_context(|| format!("cannot unpack {}", artifact.name))?;
    std::fs::create_dir_all(&dir).with_context(|| format!("cannot create {}", dir.display()))?;
    let partial = staging(&exe);
    std::fs::write(&partial, &binary)
        .with_context(|| format!("cannot write {}", partial.display()))?;
    make_executable(&partial)?;
    std::fs::rename(&partial, &exe).with_context(|| format!("cannot install {}", exe.display()))?;
    Ok(exe)
}

fn ensure_binaryen(cache: &Path, artifact: &Artifact) -> Result<PathBuf> {
    let root = cache.join(format!("binaryen-{BINARYEN_VERSION}"));
    let bin = root.join("bin");
    let tools = ["wasm-merge", "wasm-metadce", "wasm-opt"];
    let complete = |bin: &Path| {
        tools.iter().all(|t| {
            bin.join(format!("{t}{}", std::env::consts::EXE_SUFFIX))
                .is_file()
        })
    };
    if complete(&bin) {
        return Ok(bin);
    }
    let url = format!(
        "https://github.com/WebAssembly/binaryen/releases/download/{BINARYEN_VERSION}/{}",
        artifact.name
    );
    let archive = download(&url, artifact)?;
    let partial = staging(&root);
    if partial.exists() {
        std::fs::remove_dir_all(&partial)
            .with_context(|| format!("cannot clean {}", partial.display()))?;
    }
    std::fs::create_dir_all(&partial)
        .with_context(|| format!("cannot create {}", partial.display()))?;
    tar::Archive::new(GzDecoder::new(archive.as_slice()))
        .unpack(&partial)
        .with_context(|| format!("cannot unpack {}", artifact.name))?;
    // The archive holds one folder, `binaryen-version_133/`.
    let unpacked = partial.join(format!("binaryen-{BINARYEN_VERSION}"));
    if !complete(&unpacked.join("bin")) {
        bail!("{} does not contain {}", artifact.name, tools.join(", "));
    }
    if root.exists() {
        std::fs::remove_dir_all(&root)
            .with_context(|| format!("cannot replace {}", root.display()))?;
    }
    std::fs::rename(&unpacked, &root)
        .with_context(|| format!("cannot install {}", root.display()))?;
    std::fs::remove_dir_all(&partial).ok();
    Ok(bin)
}

/// extism-js and binaryen for this OS, downloaded once into the cache and checked by sha256.
pub fn ensure() -> Result<Tools> {
    let platform = platform()?;
    let cache = cache_dir()?;
    std::fs::create_dir_all(&cache)
        .with_context(|| format!("cannot create {}", cache.display()))?;
    let extism_js = ensure_extism_js(&cache, &platform.extism_js)
        .map_err(|e| anyhow!("{e:#}\n(set {CACHE_ENV} to use another cache folder)"))?;
    let binaryen_bin = ensure_binaryen(&cache, &platform.binaryen)?;
    Ok(Tools {
        extism_js,
        binaryen_bin,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checks_the_pinned_hash() {
        let artifact = Artifact {
            name: "x",
            sha256: "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824",
        };
        assert!(verify(b"hello", &artifact).is_ok());
        assert!(verify(b"hello!", &artifact).is_err());
    }

    #[test]
    fn this_platform_is_pinned() {
        if cfg!(any(
            all(
                target_os = "macos",
                any(target_arch = "aarch64", target_arch = "x86_64")
            ),
            all(
                target_os = "linux",
                any(target_arch = "aarch64", target_arch = "x86_64")
            ),
            all(target_os = "windows", target_arch = "x86_64")
        )) {
            let p = platform().unwrap();
            assert_eq!(p.extism_js.sha256.len(), 64);
            assert!(p.binaryen.name.contains(BINARYEN_VERSION));
        }
    }
}
