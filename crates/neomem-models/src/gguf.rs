use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

use neomem_core::ModelError;

pub(crate) fn embedding_dims(path: &Path) -> Result<usize, ModelError> {
    read_dims(path).map_err(|error| ModelError::Unavailable(format!("reading {}: {error}", path.display())))
}

fn read_dims(path: &Path) -> io::Result<usize> {
    let mut file = File::open(path)?;
    let mut magic = [0; 4];
    file.read_exact(&mut magic)?;
    if &magic != b"GGUF" || !matches!(number(&mut file, 4)?, 2 | 3) {
        return Err(io::Error::other("unsupported GGUF header"));
    }
    let _tensors = number(&mut file, 8)?;
    let entries = number(&mut file, 8)?;
    let mut dims = None;
    for _ in 0..entries {
        let length = number(&mut file, 8)?;
        if length > 4096 {
            return Err(io::Error::other("oversized GGUF metadata key"));
        }
        let mut key = vec![0; usize::try_from(length).map_err(io::Error::other)?];
        file.read_exact(&mut key)?;
        let kind = number(&mut file, 4)?;
        let key = String::from_utf8_lossy(&key);
        if kind == 4 && (key.ends_with(".embedding_length") || key.ends_with(".embedding_length_out")) {
            let value = usize::try_from(number(&mut file, 4)?).map_err(io::Error::other)?;
            if dims.is_none() || key.ends_with(".embedding_length_out") {
                dims = Some(value);
            }
        } else {
            skip(&mut file, kind, 0)?;
        }
    }
    dims.filter(|dims| *dims > 0).ok_or_else(|| io::Error::other("GGUF has no embedding dimension"))
}

pub(crate) fn number(reader: &mut impl Read, bytes: usize) -> io::Result<u64> {
    let mut data = [0; 8];
    reader.read_exact(&mut data[..bytes])?;
    Ok(u64::from_le_bytes(data))
}

pub(crate) fn skip(file: &mut (impl Read + Seek), kind: u64, depth: usize) -> io::Result<()> {
    let bytes = match kind {
        0 | 1 | 7 => 1,
        2 | 3 => 2,
        4..=6 => 4,
        10..=12 => 8,
        8 => number(file, 8)?,
        9 if depth < 8 => {
            let element = number(file, 4)?;
            let count = number(file, 8)?;
            for _ in 0..count {
                skip(file, element, depth + 1)?;
            }
            return Ok(());
        }
        _ => return Err(io::Error::other("unsupported GGUF metadata type")),
    };
    file.seek(SeekFrom::Current(i64::try_from(bytes).map_err(io::Error::other)?))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    #[test]
    fn dimensions_are_read_without_loading_tensor_data() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(b"GGUF").unwrap();
        file.write_all(&3_u32.to_le_bytes()).unwrap();
        file.write_all(&0_u64.to_le_bytes()).unwrap();
        file.write_all(&2_u64.to_le_bytes()).unwrap();
        for (key, value) in [("gemma.embedding_length", 768_u32), ("gemma.embedding_length_out", 512)] {
            file.write_all(&u64::try_from(key.len()).unwrap().to_le_bytes()).unwrap();
            file.write_all(key.as_bytes()).unwrap();
            file.write_all(&4_u32.to_le_bytes()).unwrap();
            file.write_all(&value.to_le_bytes()).unwrap();
        }
        assert_eq!(embedding_dims(file.path()).unwrap(), 512);
    }

    #[test]
    fn a_truncated_or_non_gguf_file_is_rejected() {
        let file = tempfile::NamedTempFile::new().unwrap();
        assert!(embedding_dims(file.path()).is_err());
    }
}
