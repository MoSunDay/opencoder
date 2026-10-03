//! Follow the execution's Windows profile without changing host folder settings.
use std::ffi::OsString;
use std::path::PathBuf;

fn absolute(value: Option<OsString>) -> Option<PathBuf> {
    value.map(PathBuf::from).filter(|path| path.is_absolute())
}

fn directory(variable: &str, fallback: fn() -> Option<PathBuf>) -> Option<PathBuf> {
    if cfg!(windows) {
        absolute(std::env::var_os(variable)).or_else(fallback)
    } else {
        fallback()
    }
}

pub fn home_dir() -> Option<PathBuf> {
    directory("USERPROFILE", dirs::home_dir)
}

pub fn data_local_dir() -> Option<PathBuf> {
    directory("LOCALAPPDATA", dirs::data_local_dir)
}

pub fn config_dir() -> Option<PathBuf> {
    directory("APPDATA", dirs::config_dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_directory_overrides_require_absolute_paths() {
        let root = tempfile::tempdir().unwrap();
        assert_eq!(
            absolute(Some(root.path().as_os_str().into())),
            Some(root.path().into())
        );
        for value in [None, Some(OsString::new()), Some("relative/profile".into())] {
            assert_eq!(absolute(value), None);
        }
    }
}
