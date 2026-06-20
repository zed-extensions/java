use std::path::Path;

/// Convert a filesystem path to a `file://` URI, matching how language servers'
/// `publishDiagnostics` and the editor key documents.
///
/// On Unix the path already starts with `/`, so `file://` + path gives the
/// correct `file:///…` form with no extra work.
///
/// On Windows the backslashes are replaced with `/` and an extra `/` is
/// prepended before the drive letter, so we get `file:///C:/…` rather than
/// `file://C:\…`.
#[cfg(unix)]
pub fn path_to_file_uri(path: &Path) -> String {
    format!("file://{}", path.display())
}

#[cfg(windows)]
pub fn path_to_file_uri(path: &Path) -> String {
    let s = path.display().to_string().replace('\\', "/");
    format!("file:///{s}")
}
