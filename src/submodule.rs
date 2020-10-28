use crate::Git;
use once_cell_regex::regex;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};
use std::{
    error::Error as StdError,
    fmt::Display,
    path::{Path, PathBuf},
};

#[derive(Debug)]
pub enum Source {
    NameMissing,
    IndexCheckFailed(std::io::Error),
    InitCheckFailed(std::io::Error),
    PathInvalidUtf8,
    AddFailed(bossy::Error),
    InitFailed(bossy::Error),
    CheckoutFailed {
        commit: String,
        source: bossy::Error,
    },
}

#[derive(Debug)]
pub struct Error {
    submodule: Submodule,
    source: Source,
}

impl Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.source {
            Source::NameMissing => write!(
                f,
                "Failed to infer name for submodule at remote {:?}; please specify a name explicitly.",
                self.submodule.remote
            ),
            Source::IndexCheckFailed(err) => write!(
                f,
                "Failed to check \".gitmodules\" for submodule {:?}: {}",
                self.submodule.name().unwrap(), err,
            ),
            Source::InitCheckFailed(err) => write!(
                f,
                "Failed to check \".git/config\" for submodule {:?}: {}",
                self.submodule.name().unwrap(), err,
            ),
            Source::PathInvalidUtf8 => write!(
                f,
                "Submodule path {:?} wasn't valid utf-8.",
                self.submodule.path,
            ),
            Source::AddFailed(err) => write!(
                f,
                "Failed to add submodule {:?} with remote {:?} and path {:?}: {}",
                self.submodule.name().unwrap(), self.submodule.remote, self.submodule.path, err
            ),
            Source::InitFailed(err) => write!(
                f,
                "Failed to init submodule {:?} with remote {:?} and path {:?}: {}",
                self.submodule.name().unwrap(), self.submodule.remote, self.submodule.path, err
            ),
            Source::CheckoutFailed { commit, source } => write!(
                f,
                "Failed to checkout commit {:?} from submodule {:?} with remote {:?} and path {:?}: {}",
                commit, self.submodule.name().unwrap(), self.submodule.remote, self.submodule.path, source
            ),
        }
    }
}

impl StdError for Error {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match &self.source {
            Source::NameMissing | Source::PathInvalidUtf8 => None,
            Source::IndexCheckFailed(err) | Source::InitCheckFailed(err) => Some(err),
            Source::AddFailed(err) | Source::InitFailed(err) => Some(err),
            Source::CheckoutFailed { source, .. } => Some(source),
        }
    }
}

#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(Deserialize, Serialize))]
pub struct Submodule {
    name: Option<String>,
    remote: String,
    path: PathBuf,
}

impl Submodule {
    pub fn with_remote_and_path(remote: impl Into<String>, path: impl Into<PathBuf>) -> Self {
        Self {
            name: None,
            remote: remote.into(),
            path: path.into(),
        }
    }

    pub fn name(&self) -> Option<&str> {
        self.name.as_deref().or_else(|| {
            let name = regex!(r"(?P<name>\w+)\.git")
                .captures(&self.remote)
                // Indexing would return `str` instead of `&str`, which doesn't
                // play nice with our lifetime needs here...
                .map(|caps| caps.name("name").unwrap().as_str());
            log::info!("detected submodule name: {:?}", name);
            name
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn in_index(&self, git: Git<'_>, name: &str) -> std::io::Result<bool> {
        git.modules().map(|modules| {
            modules
                .filter(|modules| modules.contains(&format!("[submodule {:?}]", name)))
                .is_some()
        })
    }

    fn initialized(&self, git: Git<'_>, name: &str) -> std::io::Result<bool> {
        git.config().map(|config| {
            config
                .filter(|config| config.contains(&format!("[submodule {:?}]", name)))
                .is_some()
        })
    }

    pub fn init(&self, git: Git<'_>, commit: Option<&str>) -> Result<(), Error> {
        let name = self.name().ok_or_else(|| Error {
            submodule: self.clone(),
            source: Source::NameMissing,
        })?;
        let in_index = self.in_index(git, &name).map_err(|source| Error {
            submodule: self.clone(),
            source: Source::IndexCheckFailed(source),
        })?;
        let initialized = if !in_index {
            let path_str = self.path.to_str().ok_or_else(|| Error {
                submodule: self.clone(),
                source: Source::PathInvalidUtf8,
            })?;
            log::info!("adding submodule: {:#?}", self);
            git.command()
                .with_args(&["submodule", "add", "--name", &name, &self.remote, path_str])
                .run_and_wait()
                .map_err(|source| Error {
                    submodule: self.clone(),
                    source: Source::AddFailed(source),
                })?;
            false
        } else {
            log::info!("submodule already in index: {:#?}", self);
            self.initialized(git, &name).map_err(|source| Error {
                submodule: self.clone(),
                source: Source::InitCheckFailed(source),
            })?
        };
        if !initialized {
            log::info!("initializing submodule: {:#?}", self);
            git.command()
                .with_parsed_args("submodule update --init --recursive")
                .run_and_wait()
                .map_err(|source| Error {
                    submodule: self.clone(),
                    source: Source::InitFailed(source),
                })?;
        } else {
            log::info!("submodule already initalized: {:#?}", self);
        }
        if let Some(commit) = commit {
            let path = git.root().join(self.path());
            log::info!(
                "checking out commit {:?} in submodule at {:?}",
                commit,
                path
            );
            Git::new(&path)
                .command()
                .with_args(&["checkout", commit])
                .run_and_wait()
                .map_err(|source| Error {
                    submodule: self.clone(),
                    source: Source::CheckoutFailed {
                        commit: commit.to_owned(),
                        source,
                    },
                })?;
        }
        Ok(())
    }
}
