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
    /// sha256 of the unpacked `extism-js` binary.
    extism_js_binary: &'static str,
    binaryen: Artifact,
    /// Files of the unpacked binaryen that run, relative to its root, with their sha256.
    binaryen_files: &'static [(&'static str, &'static str)],
}

const MACOS_ARM64: Platform = Platform {
    extism_js: Artifact {
        name: "extism-js-aarch64-macos-v1.7.0.gz",
        sha256: "12c01c2bb2240a6a05a4f8babe680c793c397d84e6a1497a7ca1312e78a475c3",
    },
    extism_js_binary: "e113a396c950a68b81f5acb2f3c42d7bc389996197bd58a16da6f9e1ca5731c3",
    binaryen: Artifact {
        name: "binaryen-version_133-arm64-macos.tar.gz",
        sha256: "ad66da82ac13f163e424b1643f16c6dfcccc98b5966296b43e52d3cab04f84a8",
    },
    binaryen_files: &[
        (
            "bin/wasm-merge",
            "c41e8ffbed5ff46448109fa286e1e97ac55623648f1f3428652887b47a8b699f",
        ),
        (
            "bin/wasm-metadce",
            "61c8497824813e6d7631c52c5c9e57a71f8528863397fa7781f7a0c8c65b08f4",
        ),
        (
            "bin/wasm-opt",
            "81041e09f332df94db1c2009a64d8f3b85f0431a2c920ccd014d3ceebb343402",
        ),
        (
            "lib/libbinaryen.dylib",
            "61055e190d84d5db6d1dec63456e0c24dad324ecfd4d2e23740dca217ed89e5a",
        ),
    ],
};

const MACOS_X64: Platform = Platform {
    extism_js: Artifact {
        name: "extism-js-x86_64-macos-v1.7.0.gz",
        sha256: "cce4a756eceb34b5aaac5ea864607d6c8b0610535e2c1b0c05a96bdbda69c7bb",
    },
    extism_js_binary: "938e9727735811e2214fc991214b11dfa2591d98fa0c286df16319c34764c559",
    binaryen: Artifact {
        name: "binaryen-version_133-x86_64-macos.tar.gz",
        sha256: "13a9b90be775c6389ce3d1f879cb8627bea56708ba8c122983941d53a8199b95",
    },
    binaryen_files: &[
        (
            "bin/wasm-merge",
            "ced52080987874f99b96abe15a3a7b5cb3c72f661a8413daea09a8a379d512b9",
        ),
        (
            "bin/wasm-metadce",
            "d4fb48c633e0eceacba9d28c96ce778f7f6cd5f52710e5fc2faec9bb5ee5f411",
        ),
        (
            "bin/wasm-opt",
            "e26344b1d0d0986ac4a1090f58e478470eb2a52ba0625ae9a0f880511bb31d51",
        ),
        (
            "lib/libbinaryen.dylib",
            "26388343133e968f58c18807552b83c43944d11cf512e3920798890adc554f38",
        ),
    ],
};

const LINUX_X64: Platform = Platform {
    extism_js: Artifact {
        name: "extism-js-x86_64-linux-v1.7.0.gz",
        sha256: "63b72da2f5e88655522dc21477de549f238a2f40546a69ce4e0fce7e78654035",
    },
    extism_js_binary: "bf15c04c89976431fd05f285e2d20c0f4de379308699be394097fac8321bc377",
    binaryen: Artifact {
        name: "binaryen-version_133-x86_64-linux.tar.gz",
        sha256: "2dc9c7813f5375db93d96ead4b78222fcc3e2677bbb832297af4797782a37489",
    },
    binaryen_files: &[
        (
            "bin/wasm-merge",
            "4b55992e09b833bcc6719dbc0540783dfdb05dba319c03eb1059ade20211c957",
        ),
        (
            "bin/wasm-metadce",
            "1a47c3bb9e82fc0b49ab6a394eb76ac6470611764878a026d2ad5838d16b67a8",
        ),
        (
            "bin/wasm-opt",
            "8f25e9fd5db0fc5f210003aaa432922feb2e52d309e430def2f929e34da9466b",
        ),
    ],
};

const LINUX_ARM64: Platform = Platform {
    extism_js: Artifact {
        name: "extism-js-aarch64-linux-v1.7.0.gz",
        sha256: "025f4050b199d68413c159bde1187271ae270021a9f7171e7beb509922821f2a",
    },
    extism_js_binary: "a1993ddd49fd39ce53c9308bb80b8c6da4ade045402375dfc18a059228c52cb9",
    binaryen: Artifact {
        name: "binaryen-version_133-aarch64-linux.tar.gz",
        sha256: "89c07ea56faf38d0fbecf36ca8ec0721756716185f265b568e133d427f299bf8",
    },
    binaryen_files: &[
        (
            "bin/wasm-merge",
            "4b000fc39b1cf0f5a49bc09a043839d0e034cd93ae7bd6e2809543254e959ecf",
        ),
        (
            "bin/wasm-metadce",
            "45c2dc610a5590724b43a5e7cbbfbd36d79037f2792caf84a0446650211159d2",
        ),
        (
            "bin/wasm-opt",
            "e5a823487bb83ed522625cc5d9c383e94e0479bfdb1ba387a387505df27daa6a",
        ),
    ],
};

const WINDOWS_X64: Platform = Platform {
    extism_js: Artifact {
        name: "extism-js-x86_64-windows-v1.7.0.gz",
        sha256: "409ac023f88f79d763fbf9dd5bb5d9b2fd595ac452e0822d49bf7d05bf6794be",
    },
    extism_js_binary: "f72d5bc7837a4541a91b783093c87f85ed10d8cd0acdb2d36f8d3b4edaac3a78",
    binaryen: Artifact {
        name: "binaryen-version_133-x86_64-windows.tar.gz",
        sha256: "17a2cbeac6b5693c5fbafab3838d3c65fd9c1eb38b05f5baec6c657e8c84995b",
    },
    binaryen_files: &[
        (
            "bin/wasm-merge.exe",
            "5a57d1af6de3bafd85756605eb91eb9907c59de312fad9b8a8b558d4333af632",
        ),
        (
            "bin/wasm-metadce.exe",
            "db1c519448a524ddb7ead698676abdf9c907b9d279b6049806b89fc35ef40796",
        ),
        (
            "bin/wasm-opt.exe",
            "4217d75f81cf33c4032d82b5d531120069af1a4b43eb7737977bdb2c99a8f4ba",
        ),
    ],
};

fn platform() -> Result<&'static Platform> {
    Ok(match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => &MACOS_ARM64,
        ("macos", "x86_64") => &MACOS_X64,
        ("linux", "x86_64") => &LINUX_X64,
        ("linux", "aarch64") => &LINUX_ARM64,
        ("windows", "x86_64") => &WINDOWS_X64,
        (os, arch) => bail!(
            "TypeScript screeners need extism-js and binaryen, which publish no build for {os}/{arch}; \
             write the screener in Rust or build on macOS, Linux (x86_64/arm64) or Windows x86_64"
        ),
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

fn sha256_hex(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

fn verify(bytes: &[u8], artifact: &Artifact) -> Result<()> {
    let actual = sha256_hex(bytes);
    if actual != artifact.sha256 {
        bail!(
            "{}: sha256 {actual} does not match the pinned {}; the download is corrupt or was replaced",
            artifact.name,
            artifact.sha256
        );
    }
    Ok(())
}

/// `Ok(false)` for a missing file or a different hash: the cache is incomplete or was tampered
/// with, and gets downloaded again.
fn file_matches(path: &Path, sha256: &str) -> Result<bool> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(sha256_hex(&bytes) == sha256),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(anyhow!(e).context(format!("cannot read {}", path.display()))),
    }
}

fn files_match(root: &Path, files: &[(&str, &str)]) -> Result<bool> {
    for (file, sha256) in files {
        if !file_matches(&root.join(file), sha256)? {
            return Ok(false);
        }
    }
    Ok(true)
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

fn ensure_extism_js(cache: &Path, platform: &Platform) -> Result<PathBuf> {
    let dir = cache.join(format!("extism-js-{EXTISM_JS_VERSION}"));
    let exe = dir.join(format!("extism-js{}", std::env::consts::EXE_SUFFIX));
    if file_matches(&exe, platform.extism_js_binary)? {
        return Ok(exe);
    }
    if exe.exists() {
        eprintln!(
            "warning: {} does not match its pinned hash; downloading it again",
            exe.display()
        );
    }
    let artifact = &platform.extism_js;
    let url = format!(
        "https://github.com/extism/js-pdk/releases/download/v{EXTISM_JS_VERSION}/{}",
        artifact.name
    );
    let gz = download(&url, artifact)?;
    let mut binary = Vec::new();
    GzDecoder::new(gz.as_slice())
        .read_to_end(&mut binary)
        .with_context(|| format!("cannot unpack {}", artifact.name))?;
    if sha256_hex(&binary) != platform.extism_js_binary {
        bail!(
            "{}: the unpacked binary does not match its pinned hash",
            artifact.name
        );
    }
    std::fs::create_dir_all(&dir).with_context(|| format!("cannot create {}", dir.display()))?;
    let partial = staging(&exe);
    std::fs::write(&partial, &binary)
        .with_context(|| format!("cannot write {}", partial.display()))?;
    make_executable(&partial)?;
    std::fs::rename(&partial, &exe).with_context(|| format!("cannot install {}", exe.display()))?;
    Ok(exe)
}

fn ensure_binaryen(cache: &Path, platform: &Platform) -> Result<PathBuf> {
    let root = cache.join(format!("binaryen-{BINARYEN_VERSION}"));
    if files_match(&root, platform.binaryen_files)? {
        return Ok(root.join("bin"));
    }
    if root.exists() {
        eprintln!(
            "warning: {} is incomplete or does not match its pinned hashes; downloading it again",
            root.display()
        );
    }
    let artifact = &platform.binaryen;
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
    if !files_match(&unpacked, platform.binaryen_files)? {
        bail!(
            "{}: the unpacked tools do not match their pinned hashes",
            artifact.name
        );
    }
    if root.exists() {
        std::fs::remove_dir_all(&root)
            .with_context(|| format!("cannot replace {}", root.display()))?;
    }
    std::fs::rename(&unpacked, &root)
        .with_context(|| format!("cannot install {}", root.display()))?;
    std::fs::remove_dir_all(&partial).ok();
    Ok(root.join("bin"))
}

/// extism-js and binaryen for this OS, downloaded once into the cache; the archives and the
/// unpacked tools are checked against pinned sha256 hashes, the tools on every build.
pub fn ensure() -> Result<Tools> {
    let platform = platform()?;
    let cache = cache_dir()?;
    std::fs::create_dir_all(&cache)
        .with_context(|| format!("cannot create {}", cache.display()))?;
    let extism_js = ensure_extism_js(&cache, platform)
        .map_err(|e| anyhow!("{e:#}\n(set {CACHE_ENV} to use another cache folder)"))?;
    let binaryen_bin = ensure_binaryen(&cache, platform)?;
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
            assert_eq!(p.extism_js_binary.len(), 64);
            assert!(p.binaryen.name.contains(BINARYEN_VERSION));
            for tool in ["wasm-merge", "wasm-metadce", "wasm-opt"] {
                let file = format!("bin/{tool}{}", std::env::consts::EXE_SUFFIX);
                assert!(p.binaryen_files.iter().any(|(f, _)| *f == file), "{file}");
            }
        }
    }

    #[test]
    fn a_changed_or_missing_file_does_not_match() {
        let dir = std::env::temp_dir().join(format!("st-toolchain-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("bin")).unwrap();
        std::fs::write(dir.join("bin/tool"), b"hello").unwrap();
        let hello = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";
        assert!(files_match(&dir, &[("bin/tool", hello)]).unwrap());
        std::fs::write(dir.join("bin/tool"), b"hellO").unwrap();
        assert!(!files_match(&dir, &[("bin/tool", hello)]).unwrap());
        assert!(!files_match(&dir, &[("lib/missing.dylib", hello)]).unwrap());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
