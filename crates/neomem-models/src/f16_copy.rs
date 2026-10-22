use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use half::f16;

use crate::gguf::{number, skip};

const COPIES_FOLDER: &str = "f16";
const GGUF_VERSION: u32 = 3;
const ALIGNMENT_KEY: &[u8] = b"general.alignment";
const FILE_TYPE_KEY: &[u8] = b"general.file_type";
const MOSTLY_F16: u32 = 1;
const DEFAULT_ALIGNMENT: u64 = 32;
const UINT32_VALUE: u64 = 4;
const LONGEST_NAME: u64 = 4096;
const MOST_DIMENSIONS: u64 = 4;
const F32: u32 = 0;
const F16: u32 = 1;
const Q8_0: u32 = 8;
const BF16: u32 = 30;
const Q8_0_VALUES: u64 = 32;
const Q8_0_BYTES: u64 = 34;
const BLOCKS_PER_CHUNK: u64 = 4096;

pub(crate) fn fastest_weights(path: &Path) -> PathBuf {
    if !cfg!(feature = "vulkan") || !path.is_file() {
        return path.to_path_buf();
    }
    widened(path).unwrap_or_else(|error| {
        tracing::warn!(model = %path.display(), %error, "no F16 copy of the model; loading it as it is");
        path.to_path_buf()
    })
}

fn widened(path: &Path) -> io::Result<PathBuf> {
    let (Some(folder), Some(name)) = (path.parent(), path.file_name()) else {
        return Ok(path.to_path_buf());
    };
    let copy = folder.join(COPIES_FOLDER).join(name);
    if is_newer(&copy, path) {
        return Ok(copy);
    }
    let layout = Layout::read(&mut BufReader::new(File::open(path)?))?;
    if !layout.widens() {
        return Ok(path.to_path_buf());
    }
    fs::create_dir_all(folder.join(COPIES_FOLDER))?;
    let partial = copy.with_extension(format!("partial-{}", std::process::id()));
    let written = layout.write_widened(path, &partial).and_then(|()| fs::rename(&partial, &copy));
    if written.is_err() {
        let _ = fs::remove_file(&partial);
    }
    written.map(|()| copy)
}

fn is_newer(copy: &Path, original: &Path) -> bool {
    let modified = |path: &Path| fs::metadata(path).and_then(|metadata| metadata.modified()).ok();
    matches!((modified(copy), modified(original)), (Some(copied), Some(changed)) if copied >= changed)
}

#[derive(Debug)]
struct Layout {
    metadata_count: u64,
    metadata: Vec<u8>,
    file_type_at: Option<usize>,
    alignment: u64,
    tensors: Vec<Tensor>,
    data_start: u64,
}

impl Layout {
    fn read(reader: &mut (impl Read + Seek)) -> io::Result<Self> {
        let mut magic = [0; 4];
        reader.read_exact(&mut magic)?;
        if &magic != b"GGUF" || number(reader, 4)? != u64::from(GGUF_VERSION) {
            return Err(io::Error::other("only GGUF version 3 files get an F16 copy"));
        }
        let tensor_count = number(reader, 8)?;
        let metadata_count = number(reader, 8)?;
        let metadata_start = reader.stream_position()?;
        let mut alignment = DEFAULT_ALIGNMENT;
        let mut file_type_at = None;
        for _ in 0..metadata_count {
            let key = name(reader)?;
            let kind = number(reader, 4)?;
            if key == FILE_TYPE_KEY && kind == UINT32_VALUE {
                file_type_at = Some(
                    usize::try_from(reader.stream_position()? - metadata_start).map_err(io::Error::other)?,
                );
            }
            if key == ALIGNMENT_KEY && kind == UINT32_VALUE {
                alignment = number(reader, 4)?;
            } else {
                skip(reader, kind, 0)?;
            }
        }
        if alignment == 0 {
            return Err(io::Error::other("GGUF alignment is zero"));
        }
        let metadata_end = reader.stream_position()?;
        reader.seek(SeekFrom::Start(metadata_start))?;
        let mut metadata = vec![0; usize::try_from(metadata_end - metadata_start).map_err(io::Error::other)?];
        reader.read_exact(&mut metadata)?;
        let tensors = (0..tensor_count).map(|_| Tensor::read(reader)).collect::<io::Result<Vec<_>>>()?;
        let data_start = aligned(reader.stream_position()?, alignment);
        Ok(Self { metadata_count, metadata, file_type_at, alignment, tensors, data_start })
    }

    fn widens(&self) -> bool {
        self.tensors.iter().any(|tensor| tensor.kind == Q8_0)
            && self.tensors.iter().all(|tensor| tensor.bytes().is_some())
    }

    fn widened_metadata(&self) -> Vec<u8> {
        let mut metadata = self.metadata.clone();
        if let Some(file_type) = self.file_type_at.and_then(|at| metadata.get_mut(at..at + 4)) {
            file_type.copy_from_slice(&MOSTLY_F16.to_le_bytes());
        }
        metadata
    }

    fn widened_offsets(&self) -> io::Result<Vec<u64>> {
        let mut next = 0;
        self.tensors
            .iter()
            .map(|tensor| {
                let offset = next;
                next = aligned(offset + tensor.widened_bytes().ok_or_else(unknown_kind)?, self.alignment);
                Ok(offset)
            })
            .collect()
    }

    fn write_widened(&self, source: &Path, target: &Path) -> io::Result<()> {
        let mut input = BufReader::new(File::open(source)?);
        let mut output = BufWriter::new(File::create(target)?);
        output.write_all(b"GGUF")?;
        output.write_all(&GGUF_VERSION.to_le_bytes())?;
        output.write_all(&u64::try_from(self.tensors.len()).map_err(io::Error::other)?.to_le_bytes())?;
        output.write_all(&self.metadata_count.to_le_bytes())?;
        output.write_all(&self.widened_metadata())?;
        let offsets = self.widened_offsets()?;
        for (tensor, offset) in self.tensors.iter().zip(&offsets) {
            tensor.write_info(&mut output, *offset)?;
        }
        let data_start = aligned(output.stream_position()?, self.alignment);
        for (tensor, offset) in self.tensors.iter().zip(&offsets) {
            pad_to(&mut output, data_start + offset)?;
            input.seek(SeekFrom::Start(self.data_start + tensor.offset))?;
            tensor.copy_widened(&mut input, &mut output)?;
        }
        output.into_inner().map_err(io::IntoInnerError::into_error)?.sync_all()
    }
}

#[derive(Debug)]
struct Tensor {
    name: Vec<u8>,
    dims: Vec<u64>,
    kind: u32,
    offset: u64,
}

impl Tensor {
    fn read(reader: &mut impl Read) -> io::Result<Self> {
        let name = name(reader)?;
        let dimensions = number(reader, 4)?;
        if dimensions > MOST_DIMENSIONS {
            return Err(io::Error::other("GGUF tensor has too many dimensions"));
        }
        let dims = (0..dimensions).map(|_| number(reader, 8)).collect::<io::Result<Vec<_>>>()?;
        let kind = u32::try_from(number(reader, 4)?).map_err(io::Error::other)?;
        let offset = number(reader, 8)?;
        Ok(Self { name, dims, kind, offset })
    }

    fn values(&self) -> u64 {
        self.dims.iter().product()
    }

    fn bytes(&self) -> Option<u64> {
        let values = self.values();
        match self.kind {
            F32 => Some(values * 4),
            F16 | BF16 => Some(values * 2),
            Q8_0 if values.is_multiple_of(Q8_0_VALUES) => Some(values / Q8_0_VALUES * Q8_0_BYTES),
            _ => None,
        }
    }

    fn widened_kind(&self) -> u32 {
        if self.kind == Q8_0 { F16 } else { self.kind }
    }

    fn widened_bytes(&self) -> Option<u64> {
        if self.kind == Q8_0 { Some(self.values() * 2) } else { self.bytes() }
    }

    fn write_info(&self, output: &mut impl Write, offset: u64) -> io::Result<()> {
        output.write_all(&u64::try_from(self.name.len()).map_err(io::Error::other)?.to_le_bytes())?;
        output.write_all(&self.name)?;
        output.write_all(&u32::try_from(self.dims.len()).map_err(io::Error::other)?.to_le_bytes())?;
        for dim in &self.dims {
            output.write_all(&dim.to_le_bytes())?;
        }
        output.write_all(&self.widened_kind().to_le_bytes())?;
        output.write_all(&offset.to_le_bytes())
    }

    fn copy_widened(&self, input: &mut impl Read, output: &mut impl Write) -> io::Result<()> {
        if self.kind != Q8_0 {
            let bytes = self.bytes().ok_or_else(unknown_kind)?;
            io::copy(&mut input.take(bytes), output)?;
            return Ok(());
        }
        let mut blocks_left = self.values() / Q8_0_VALUES;
        let mut blocks = Vec::new();
        let mut values = Vec::new();
        while blocks_left > 0 {
            let count = blocks_left.min(BLOCKS_PER_CHUNK);
            blocks.resize(usize::try_from(count * Q8_0_BYTES).map_err(io::Error::other)?, 0);
            input.read_exact(&mut blocks)?;
            values.clear();
            for block in blocks.chunks_exact(usize::try_from(Q8_0_BYTES).map_err(io::Error::other)?) {
                widen_block(block, &mut values);
            }
            output.write_all(&values)?;
            blocks_left -= count;
        }
        Ok(())
    }
}

fn widen_block(block: &[u8], values: &mut Vec<u8>) {
    let (scale, quants) = block.split_at(2);
    let scale = f16::from_le_bytes([scale[0], scale[1]]).to_f32();
    for quant in quants {
        let value = scale * f32::from(i8::from_le_bytes([*quant]));
        values.extend_from_slice(&f16::from_f32(value).to_le_bytes());
    }
}

fn name(reader: &mut impl Read) -> io::Result<Vec<u8>> {
    let length = number(reader, 8)?;
    if length > LONGEST_NAME {
        return Err(io::Error::other("oversized GGUF name"));
    }
    let mut name = vec![0; usize::try_from(length).map_err(io::Error::other)?];
    reader.read_exact(&mut name)?;
    Ok(name)
}

fn aligned(position: u64, alignment: u64) -> u64 {
    position.div_ceil(alignment) * alignment
}

fn pad_to(output: &mut (impl Write + Seek), position: u64) -> io::Result<()> {
    let current = output.stream_position()?;
    let padding = position.checked_sub(current).ok_or_else(|| io::Error::other("GGUF tensors overlap"))?;
    io::copy(&mut io::repeat(0).take(padding), output)?;
    Ok(())
}

fn unknown_kind() -> io::Error {
    io::Error::other("GGUF tensor type has no F16 copy")
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TinyTensor {
        name: &'static str,
        dims: Vec<u64>,
        kind: u32,
        data: Vec<u8>,
    }

    fn tiny_gguf(path: &Path, alignment: u32, tensors: &[TinyTensor]) {
        let mut file = Vec::new();
        file.extend_from_slice(b"GGUF");
        file.extend_from_slice(&GGUF_VERSION.to_le_bytes());
        file.extend_from_slice(&u64::try_from(tensors.len()).unwrap().to_le_bytes());
        file.extend_from_slice(&3_u64.to_le_bytes());
        for (key, kind, value) in [
            ("general.name", 8_u32, [&4_u64.to_le_bytes()[..], b"tiny"].concat()),
            ("general.file_type", 4, 7_u32.to_le_bytes().to_vec()),
            ("general.alignment", 4, alignment.to_le_bytes().to_vec()),
        ] {
            file.extend_from_slice(&u64::try_from(key.len()).unwrap().to_le_bytes());
            file.extend_from_slice(key.as_bytes());
            file.extend_from_slice(&kind.to_le_bytes());
            file.extend_from_slice(&value);
        }
        let mut offset = 0_u64;
        let mut offsets = Vec::new();
        for tensor in tensors {
            offsets.push(offset);
            file.extend_from_slice(&u64::try_from(tensor.name.len()).unwrap().to_le_bytes());
            file.extend_from_slice(tensor.name.as_bytes());
            file.extend_from_slice(&u32::try_from(tensor.dims.len()).unwrap().to_le_bytes());
            for dim in &tensor.dims {
                file.extend_from_slice(&dim.to_le_bytes());
            }
            file.extend_from_slice(&tensor.kind.to_le_bytes());
            file.extend_from_slice(&offset.to_le_bytes());
            offset = aligned(offset + u64::try_from(tensor.data.len()).unwrap(), u64::from(alignment));
        }
        let data_start = aligned(u64::try_from(file.len()).unwrap(), u64::from(alignment));
        for (tensor, offset) in tensors.iter().zip(offsets) {
            file.resize(usize::try_from(data_start + offset).unwrap(), 0);
            file.extend_from_slice(&tensor.data);
        }
        fs::write(path, file).unwrap();
    }

    fn q8_0_block(scale: f32, quants: impl Iterator<Item = i8>) -> Vec<u8> {
        let mut block = f16::from_f32(scale).to_le_bytes().to_vec();
        block.extend(quants.map(|quant| quant.to_le_bytes()[0]));
        block
    }

    fn tensor_values(path: &Path, index: usize) -> (u32, Vec<u8>) {
        let mut reader = BufReader::new(File::open(path).unwrap());
        let layout = Layout::read(&mut reader).unwrap();
        let tensor = &layout.tensors[index];
        assert_eq!(tensor.offset % layout.alignment, 0);
        reader.seek(SeekFrom::Start(layout.data_start + tensor.offset)).unwrap();
        let mut data = vec![0; usize::try_from(tensor.bytes().unwrap()).unwrap()];
        reader.read_exact(&mut data).unwrap();
        (tensor.kind, data)
    }

    #[test]
    fn q8_0_tensors_become_f16_and_everything_else_is_kept() {
        let folder = tempfile::tempdir().unwrap();
        let model = folder.path().join("tiny-Q8_0.gguf");
        let floats: Vec<u8> = [1.0_f32, 2.0, 3.0, 4.0].iter().flat_map(|value| value.to_le_bytes()).collect();
        let blocks = [q8_0_block(0.5, -16..16), q8_0_block(2.0, 0..32)].concat();
        tiny_gguf(
            &model,
            64,
            &[
                TinyTensor { name: "norm", dims: vec![4], kind: F32, data: floats.clone() },
                TinyTensor { name: "weights", dims: vec![32, 2], kind: Q8_0, data: blocks },
            ],
        );

        let copy = widened(&model).unwrap();

        assert_eq!(copy, folder.path().join("f16").join("tiny-Q8_0.gguf"));
        assert_eq!(tensor_values(&copy, 0), (F32, floats));
        let expected: Vec<u8> = (-16..16)
            .map(|quant| 0.5 * f32::from(i8::try_from(quant).unwrap()))
            .chain((0..32).map(|quant| 2.0 * f32::from(i8::try_from(quant).unwrap())))
            .flat_map(|value| f16::from_f32(value).to_le_bytes())
            .collect();
        assert_eq!(tensor_values(&copy, 1), (F16, expected));
        let original = Layout::read(&mut BufReader::new(File::open(&model).unwrap())).unwrap();
        let widened = Layout::read(&mut BufReader::new(File::open(&copy).unwrap())).unwrap();
        assert_eq!(widened.metadata, original.widened_metadata());
        assert_eq!(
            widened.file_type_at.map(|at| &widened.metadata[at..at + 4]),
            Some(&MOSTLY_F16.to_le_bytes()[..])
        );
        assert_eq!(widened.alignment, 64);
    }

    #[test]
    fn a_fresh_copy_is_reused_and_a_model_without_q8_0_is_loaded_as_it_is() {
        let folder = tempfile::tempdir().unwrap();
        let model = folder.path().join("model.gguf");
        let blocks = q8_0_block(1.0, 0..32);
        tiny_gguf(&model, 32, &[TinyTensor { name: "weights", dims: vec![32], kind: Q8_0, data: blocks }]);
        let copy = widened(&model).unwrap();
        fs::write(&copy, b"kept").unwrap();
        assert_eq!(widened(&model).unwrap(), copy);
        assert_eq!(fs::read(&copy).unwrap(), b"kept");

        let plain = folder.path().join("plain.gguf");
        let floats = 1.0_f32.to_le_bytes().to_vec();
        tiny_gguf(&plain, 32, &[TinyTensor { name: "norm", dims: vec![1], kind: F32, data: floats }]);
        assert_eq!(widened(&plain).unwrap(), plain);
        assert!(!folder.path().join("f16").join("plain.gguf").exists());
    }
}
