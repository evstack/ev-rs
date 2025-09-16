//! Individual startup checks.

use crate::errors::StartupError;
use std::path::Path;

/// Check that the storage path exists and is writable.
pub fn check_storage_path(path: &str) -> Result<(), StartupError> {
    let path = Path::new(path);

    if !path.exists() {
        return Err(StartupError::PathNotFound(path.display().to_string()));
    }

    if !path.is_dir() {
        return Err(StartupError::NotADirectory(path.display().to_string()));
    }

    // Check write access by attempting to create a temp file
    let test_file = path.join(".evolve_write_test");
    match std::fs::write(&test_file, b"test") {
        Ok(()) => {
            // Clean up
            let _ = std::fs::remove_file(&test_file);
            Ok(())
        }
        Err(_) => Err(StartupError::NotWritable(path.display().to_string())),
    }
}

/// Check that sufficient disk space is available.
///
/// This is a best-effort check; some platforms may not support
/// querying disk space.
#[cfg(unix)]
pub fn check_disk_space(path: &str, required_mb: u64) -> Result<Option<String>, StartupError> {
    use std::os::unix::fs::MetadataExt;

    let path = Path::new(path);

    // Try to get filesystem stats using statvfs
    // This is a simplified check - in production you'd use libc::statvfs
    // For now, we'll do a basic check that the path exists and is accessible
    match std::fs::metadata(path) {
        Ok(metadata) => {
            // On Unix, we can get block size but not free space without libc
            // Return a warning that we can't check disk space
            let _ = metadata.blksize();
            Ok(Some(format!(
                "Disk space check not fully implemented; required {}MB at {}",
                required_mb,
                path.display()
            )))
        }
        Err(e) => Err(StartupError::Failed(format!(
            "Failed to check disk space at {}: {}",
            path.display(),
            e
        ))),
    }
}

#[cfg(not(unix))]
pub fn check_disk_space(path: &str, required_mb: u64) -> Result<Option<String>, StartupError> {
    // On non-Unix platforms, return a warning
    Ok(Some(format!(
        "Disk space check not supported on this platform; required {}MB at {}",
        required_mb, path
    )))
}

/// Verify storage integrity by attempting to open and read from storage.
///
/// This function takes a closure that performs the actual storage check,
/// allowing it to be independent of the specific storage implementation.
pub fn check_state_integrity<F, E>(check_fn: F) -> Result<(), StartupError>
where
    F: FnOnce() -> Result<(), E>,
    E: std::fmt::Display,
{
    check_fn().map_err(|e| StartupError::IntegrityCheckFailed(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_check_storage_path_exists() {
        let temp_dir = TempDir::new().unwrap();
        let result = check_storage_path(temp_dir.path().to_str().unwrap());
        assert!(result.is_ok());
    }

    #[test]
    fn test_check_storage_path_not_exists() {
        let result = check_storage_path("/nonexistent/path/that/does/not/exist");
        assert!(matches!(result, Err(StartupError::PathNotFound(_))));
    }

    #[test]
    fn test_check_storage_path_is_file() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("file.txt");
        std::fs::write(&file_path, b"test").unwrap();

        let result = check_storage_path(file_path.to_str().unwrap());
        assert!(matches!(result, Err(StartupError::NotADirectory(_))));
    }

    #[test]
    fn test_check_state_integrity_success() {
        let result = check_state_integrity(|| Ok::<(), &str>(()));
        assert!(result.is_ok());
    }

    #[test]
    fn test_check_state_integrity_failure() {
        let result = check_state_integrity(|| Err("database corrupted"));
        assert!(matches!(result, Err(StartupError::IntegrityCheckFailed(_))));
    }
}
