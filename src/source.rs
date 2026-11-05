//! What vislox watches: resolve the path given on the command line into the
//! config file(s) behind it.
//!
//! - a `lox.toml` project (file or directory): the deployed `base` config and
//!   the compiled `out` config, whose difference is what a push would apply;
//! - a `.Loxone` file or a backup `.zip`;
//! - a `lox config init` repository: `<serial>/config.Loxone`.

use anyhow::{Context, Result, bail};
use loxone_config::Project;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct Source {
    /// The config whose history is shown (the deployed config of a project).
    pub current: PathBuf,
    /// The compiled config a push would deploy (projects only).
    pub proposed: Option<PathBuf>,
    /// Short description for the title bar.
    pub label: String,
}

impl Source {
    pub fn resolve(path: &Path, serial: Option<&str>) -> Result<Source> {
        let path = path
            .canonicalize()
            .with_context(|| format!("`{}` does not exist", path.display()))?;

        let project_file = if path.is_dir() {
            Project::find(&path)
        } else if path.file_name().is_some_and(|n| n == "lox.toml") {
            Some(path.clone())
        } else {
            None
        };
        if let Some(file) = project_file {
            let p = Project::load(&file).map_err(|e| anyhow::anyhow!("{e}"))?;
            let name = p
                .root
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "project".into());
            return Ok(Source {
                current: p.base.clone(),
                proposed: Some(p.out.clone()),
                label: format!("project {name}"),
            });
        }

        if path.is_file() {
            return Ok(Source {
                label: path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                current: path,
                proposed: None,
            });
        }

        // A config repository: one subdirectory per Miniserver.
        let mut found: Vec<PathBuf> = std::fs::read_dir(&path)?
            .filter_map(|e| e.ok())
            .map(|e| e.path().join("config.Loxone"))
            .filter(|p| p.is_file())
            .collect();
        found.sort();
        if let Some(serial) = serial {
            found.retain(|p| {
                p.parent()
                    .and_then(|d| d.file_name())
                    .is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(serial))
            });
        }
        match found.len() {
            0 => bail!(
                "`{}` holds no lox.toml, and no <serial>/config.Loxone{}",
                path.display(),
                serial.map(|s| format!(" for serial {s}")).unwrap_or_default()
            ),
            1 => {
                let file = found.remove(0);
                let ms = file
                    .parent()
                    .and_then(|d| d.file_name())
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                Ok(Source {
                    current: file,
                    proposed: None,
                    label: format!("Miniserver {ms}"),
                })
            }
            _ => bail!(
                "`{}` holds configs of several Miniservers ({}); pick one with --serial",
                path.display(),
                found
                    .iter()
                    .filter_map(|p| p.parent()?.file_name())
                    .map(|n| n.to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    }
}

/// The program XML of a `.Loxone` file or a backup ZIP.
pub fn program_xml(bytes: &[u8]) -> Result<Vec<u8>> {
    Ok(loxone_config::archive::program_xml(bytes)
        .map_err(|e| anyhow::anyhow!("{e}"))?
        .into_owned())
}
