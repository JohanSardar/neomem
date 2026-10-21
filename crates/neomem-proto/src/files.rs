use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::Path;

pub fn publish_file(path: &Path, contents: impl AsRef<[u8]>) -> io::Result<()> {
    let mut unfinished_name = OsString::from(".");
    unfinished_name.push(path.file_name().unwrap_or_default());
    unfinished_name.push(format!(".{}.unfinished", std::process::id()));
    let unfinished = path.with_file_name(unfinished_name);
    fs::write(&unfinished, contents)?;
    fs::rename(&unfinished, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_published_file_appears_whole_and_leaves_nothing_behind() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("address");
        publish_file(&path, "127.0.0.1:4000").unwrap();
        publish_file(&path, "127.0.0.1:4001").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "127.0.0.1:4001");
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }
}
