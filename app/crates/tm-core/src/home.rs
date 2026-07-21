use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::Result;

/// Fixed production data root. Debug/test processes may use an absolute
/// `TM_HOME` below this root; release processes always use this exact path.
pub const DEFAULT_TM_HOME: &str = r"C:\Users\tkfk0\Desktop\codex\TM";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TmHome {
    root: PathBuf,
}

impl TmHome {
    #[must_use]
    pub fn from_env() -> Self {
        #[cfg(any(debug_assertions, test))]
        {
            std::env::var_os("TM_HOME")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
                .filter(|path| is_allowed_debug_override(path))
                .map_or_else(Self::default, Self::new)
        }

        #[cfg(not(any(debug_assertions, test)))]
        {
            Self::default()
        }
    }

    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    #[must_use]
    pub fn app_dir(&self) -> PathBuf {
        self.root.join("app")
    }

    #[must_use]
    pub fn data_dir(&self) -> PathBuf {
        self.root.join("data")
    }

    #[must_use]
    pub fn database_path(&self) -> PathBuf {
        self.data_dir().join("tm.sqlite3")
    }

    #[must_use]
    pub fn attachments_dir(&self) -> PathBuf {
        self.data_dir().join("attachments")
    }

    #[must_use]
    pub fn logs_dir(&self) -> PathBuf {
        self.data_dir().join("logs")
    }

    #[must_use]
    pub fn backups_dir(&self) -> PathBuf {
        self.root.join("backups")
    }

    #[must_use]
    pub fn database_backups_dir(&self) -> PathBuf {
        self.backups_dir().join("database")
    }

    #[must_use]
    pub fn source_backups_dir(&self) -> PathBuf {
        self.backups_dir().join("source")
    }

    #[must_use]
    pub fn exports_dir(&self) -> PathBuf {
        self.root.join("exports")
    }

    #[must_use]
    pub fn dist_dir(&self) -> PathBuf {
        self.root.join("dist")
    }

    pub fn ensure_layout(&self) -> Result<()> {
        for directory in [
            self.app_dir(),
            self.data_dir(),
            self.attachments_dir(),
            self.logs_dir(),
            self.database_backups_dir(),
            self.source_backups_dir(),
            self.exports_dir(),
            self.dist_dir(),
        ] {
            std::fs::create_dir_all(directory)?;
        }
        Ok(())
    }
}

#[cfg(any(debug_assertions, test))]
fn is_allowed_debug_override(path: &Path) -> bool {
    path.is_absolute()
        && path.starts_with(Path::new(DEFAULT_TM_HOME))
        && !path.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir | std::path::Component::CurDir
            )
        })
}

impl Default for TmHome {
    fn default() -> Self {
        Self::new(DEFAULT_TM_HOME)
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{DEFAULT_TM_HOME, is_allowed_debug_override};

    #[cfg(windows)]
    #[test]
    fn debug_override_must_be_absolute_normalized_and_inside_tm_home() {
        let allowed = Path::new(DEFAULT_TM_HOME).join("dist").join("test-runs");
        assert!(is_allowed_debug_override(&allowed));
        assert!(!is_allowed_debug_override(Path::new("relative-test-home")));
        assert!(!is_allowed_debug_override(Path::new(r"C:\temp\tm-test")));
        assert!(!is_allowed_debug_override(
            &Path::new(DEFAULT_TM_HOME).join("..").join("outside")
        ));
    }

    #[cfg(not(windows))]
    #[test]
    fn windows_debug_override_is_rejected_on_other_platforms() {
        assert!(!is_allowed_debug_override(Path::new(DEFAULT_TM_HOME)));
        assert!(!is_allowed_debug_override(Path::new("relative-test-home")));
    }
}
