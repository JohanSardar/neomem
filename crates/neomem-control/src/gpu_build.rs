use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, bail};

use crate::model_files::{self, Download};

const SPIRV_HEADERS_VERSION: &str = "vulkan-sdk-1.4.341.0";
const SPIRV_HEADERS: Download = Download {
    url: "https://github.com/KhronosGroup/SPIRV-Headers/archive/refs/tags/vulkan-sdk-1.4.341.0.tar.gz",
    sha256: "cab0a654c4917e16367483296b44cdb1d614e3120c721beafcd37e3a8580486c",
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GpuBuild {
    pub feature: &'static str,
    pub environment: Vec<(String, OsString)>,
}

pub fn choose() -> anyhow::Result<Option<GpuBuild>> {
    let backend = match std::env::var("NEOMEM_GPU").ok().as_deref() {
        Some("vulkan") => Some(Backend::Vulkan),
        Some("cuda") => Some(Backend::Cuda),
        Some(_) => None,
        None => detected(),
    };
    Ok(match backend {
        None => None,
        Some(Backend::Cuda) => Some(GpuBuild { feature: "neomemd/cuda", environment: Vec::new() }),
        Some(Backend::Vulkan) => Some(GpuBuild { feature: "neomemd/vulkan", environment: spirv_headers()? }),
    })
}

pub fn name(build: Option<&GpuBuild>) -> &'static str {
    build.map_or("cpu", |build| build.feature.trim_start_matches("neomemd/"))
}

#[derive(Debug, Clone, Copy)]
enum Backend {
    Vulkan,
    Cuda,
}

fn detected() -> Option<Backend> {
    if !cfg!(target_os = "linux") {
        return None;
    }
    if answers("nvidia-smi", &[]) && answers("nvcc", &["--version"]) {
        return Some(Backend::Cuda);
    }
    let vulkan_headers = Path::new("/usr/include/vulkan/vulkan.h").is_file();
    if vulkan_headers && answers("vulkaninfo", &["--summary"]) && answers("glslc", &["--version"]) {
        return Some(Backend::Vulkan);
    }
    None
}

fn answers(program: &str, arguments: &[&str]) -> bool {
    Command::new(program)
        .args(arguments)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn spirv_headers() -> anyhow::Result<Vec<(String, OsString)>> {
    let installed = ["/usr/include", "/usr/local/include"]
        .iter()
        .any(|dir| Path::new(dir).join("spirv/unified1/spirv.hpp").is_file());
    if installed {
        return Ok(Vec::new());
    }
    let prefix = cache_dir()?.join(format!("spirv-headers-{SPIRV_HEADERS_VERSION}"));
    if !prefix.join("share/cmake/SPIRV-Headers/SPIRV-HeadersConfig.cmake").is_file() {
        install_spirv_headers(&prefix)?;
    }
    let include_path = header_include_path(&prefix.join("include"), std::env::var_os("CPLUS_INCLUDE_PATH"));
    Ok(vec![
        ("CMAKE_PREFIX_PATH".to_owned(), prefix.into_os_string()),
        ("CPLUS_INCLUDE_PATH".to_owned(), include_path),
    ])
}

fn install_spirv_headers(prefix: &Path) -> anyhow::Result<()> {
    eprintln!("neomemctl: the Vulkan build needs SPIR-V headers; installing them in {}", prefix.display());
    let work = tempfile::Builder::new().prefix("neomemctl-spirv-").tempdir()?;
    let archive = work.path().join("spirv-headers.tar.gz");
    model_files::download(&SPIRV_HEADERS, &archive)?;
    run(Command::new("tar").arg("xzf").arg(&archive).arg("-C").arg(work.path()))?;
    let source = work.path().join(format!("SPIRV-Headers-{SPIRV_HEADERS_VERSION}"));
    let build = work.path().join("build");
    run(Command::new("cmake")
        .arg("-S")
        .arg(&source)
        .arg("-B")
        .arg(&build)
        .arg(format!("-DCMAKE_INSTALL_PREFIX={}", prefix.display()))
        .arg("-DSPIRV_HEADERS_ENABLE_TESTS=OFF"))?;
    run(Command::new("cmake").arg("--install").arg(&build))
}

fn run(command: &mut Command) -> anyhow::Result<()> {
    let output = command.output().with_context(|| format!("running {command:?}"))?;
    if !output.status.success() {
        bail!("{command:?} failed: {}", String::from_utf8_lossy(&output.stderr));
    }
    Ok(())
}

fn cache_dir() -> anyhow::Result<PathBuf> {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| Path::new(&home).join(".cache")))
        .context("neither XDG_CACHE_HOME nor HOME is set")?;
    let dir = base.join("neomem");
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn header_include_path(include: &Path, inherited: Option<OsString>) -> OsString {
    if let Some(existing) = &inherited
        && std::env::split_paths(existing).any(|path| path == include)
    {
        return existing.clone();
    }
    let mut path = include.as_os_str().to_owned();
    if let Some(existing) = inherited {
        path.push(":");
        path.push(existing);
    }
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_existing_header_path_is_preserved_without_invalidating_native_builds() {
        let include = Path::new("/cache/spirv/include");
        let existing = OsString::from("/other/include:/cache/spirv/include");
        assert_eq!(header_include_path(include, Some(existing.clone())), existing);
        assert_eq!(header_include_path(include, None), include.as_os_str());
        assert_eq!(
            header_include_path(include, Some(OsString::from("/other/include"))),
            OsString::from("/cache/spirv/include:/other/include")
        );
    }
}
