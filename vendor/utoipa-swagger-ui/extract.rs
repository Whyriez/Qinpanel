//! Qpanel security backport for the build-time Swagger UI asset extractor.

use std::{
    fs,
    io::{self, Read, Seek},
    path::Path,
};
use zip::{ZipArchive, result::ZipError};

pub fn extract_dist<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
    destination: &Path,
) -> Result<String, ZipError> {
    let mut top_level_folder = String::new();
    for index in 0..archive.len() {
        let mut file = archive.by_index(index)?;
        let filepath = file
            .enclosed_name()
            .ok_or_else(|| ZipError::InvalidArchive("invalid asset path".into()))?;
        if index == 0 {
            top_level_folder = filepath
                .iter()
                .next()
                .and_then(|part| part.to_str())
                .ok_or_else(|| ZipError::InvalidArchive("invalid asset directory".into()))?
                .to_owned();
        }
        if filepath.iter().nth(1) != Some(std::ffi::OsStr::new("dist")) {
            continue;
        }
        // Assets must never create links or change permissions through links.
        if file.is_symlink() {
            return Err(ZipError::UnsupportedArchive("symlink assets are forbidden"));
        }
        let out_path = destination.join(filepath);
        if file.is_dir() {
            fs::create_dir_all(&out_path)?;
        } else {
            if let Some(parent) = out_path.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut output = fs::File::create(&out_path)?;
            io::copy(&mut file, &mut output)?;
        }
        #[cfg(unix)]
        if let Some(mode) = file.unix_mode() {
            use std::os::unix::fs::PermissionsExt;
            // Never restore SUID, SGID or sticky bits from archive metadata.
            fs::set_permissions(&out_path, fs::Permissions::from_mode(mode & 0o777))?;
        }
    }
    Ok(top_level_folder)
}
