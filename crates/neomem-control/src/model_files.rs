use std::fmt::Write as _;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::Path;

use anyhow::bail;
use sha2::{Digest, Sha256};

use crate::repo;

#[derive(Debug, Clone, Copy)]
pub struct ModelFile {
    pub name: &'static str,
    pub sources: &'static [Download],
}

#[derive(Debug, Clone, Copy)]
pub struct Download {
    pub url: &'static str,
    pub sha256: &'static str,
}

pub const EMBEDDINGGEMMA: ModelFile = ModelFile {
    name: "embeddinggemma-300m-Q8_0.gguf",
    sources: &[
        Download {
            url: "https://huggingface.co/ggml-org/embeddinggemma-300M-GGUF/resolve/main/embeddinggemma-300M-Q8_0.gguf",
            sha256: "b5ce9d77a3fc4b3b39ccb5643c36777911cc4eb46a66962eadfa3f5f60490d63",
        },
        Download {
            url: "https://huggingface.co/unsloth/embeddinggemma-300m-GGUF/resolve/main/embeddinggemma-300M-Q8_0.gguf",
            sha256: "a0f7b4e13c397a6e1b32c2de75b1f65a14c92ec524d5f674d94a4290a1c4969b",
        },
    ],
};

pub const QWEN3_EMBEDDING: ModelFile = ModelFile {
    name: "qwen3-embedding-0.6b-Q8_0.gguf",
    sources: &[
        Download {
            url: "https://huggingface.co/Qwen/Qwen3-Embedding-0.6B-GGUF/resolve/main/Qwen3-Embedding-0.6B-Q8_0.gguf",
            sha256: "06507c7b42688469c4e7298b0a1e16deff06caf291cf0a5b278c308249c3e439",
        },
        Download {
            url: "https://huggingface.co/Mungert/Qwen3-Embedding-0.6B-GGUF/resolve/main/Qwen3-Embedding-0.6B-q8_0.gguf",
            sha256: "a0e820fb3f8f448d3582862f9161bfaf58a63f89b46353f061e017597655821c",
        },
    ],
};

pub const QWEN3_RERANKER: ModelFile = ModelFile {
    name: "qwen3-reranker-0.6b-Q8_0.gguf",
    sources: &[
        Download {
            url: "https://huggingface.co/ggml-org/Qwen3-Reranker-0.6B-Q8_0-GGUF/resolve/main/qwen3-reranker-0.6b-q8_0.gguf",
            sha256: "22c9979ce4fbcdc5acdc310c6641c32797eff1aa980b8f7a2db8a8ea23429a48",
        },
        Download {
            url: "https://huggingface.co/Mungert/Qwen3-Reranker-0.6B-GGUF/resolve/main/Qwen3-Reranker-0.6B-q8_0.gguf",
            sha256: "8b5337e5baadf83fdd6f7a865dde4b3627fc53a1c8e56cc2f83260dfdd089c49",
        },
    ],
};

pub const ALL: &[ModelFile] = &[EMBEDDINGGEMMA, QWEN3_EMBEDDING, QWEN3_RERANKER];

pub fn fetch_all() -> anyhow::Result<bool> {
    let models_dir = repo::shared_models_dir(&repo::root()?);
    ensure(&models_dir, ALL)?;
    println!("neomemctl: every model file is in {}", models_dir.display());
    Ok(true)
}

pub fn ensure(models_dir: &Path, files: &[ModelFile]) -> anyhow::Result<()> {
    fs::create_dir_all(models_dir)?;
    for file in files {
        let target = models_dir.join(file.name);
        if !target.is_file() {
            fetch(file, &target)?;
        }
    }
    Ok(())
}

fn fetch(file: &ModelFile, target: &Path) -> anyhow::Result<()> {
    let mut problems = Vec::new();
    for source in file.sources {
        eprintln!("neomemctl: downloading {} from {}", file.name, source.url);
        match download(source, target) {
            Ok(()) => return Ok(()),
            Err(error) => problems.push(format!("{}: {error:#}", source.url)),
        }
    }
    bail!("could not download {}: {}", file.name, problems.join("; "))
}

fn download(source: &Download, target: &Path) -> anyhow::Result<()> {
    let partial = target.with_extension("gguf.part");
    let mut response = ureq::get(source.url).call()?;
    let mut reader = response.body_mut().as_reader();
    let mut output = File::create(&partial)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 1 << 20];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        output.write_all(&buffer[..read])?;
    }
    output.sync_all()?;
    let digest = hasher.finalize().iter().fold(String::new(), |mut hex, byte| {
        let _ = write!(hex, "{byte:02x}");
        hex
    });
    if digest != source.sha256 {
        fs::remove_file(&partial)?;
        bail!("the download's sha256 {digest} is not the expected {}", source.sha256);
    }
    fs::rename(&partial, target)?;
    Ok(())
}
