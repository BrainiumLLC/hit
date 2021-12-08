use crate::Git;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("Failed to fetch repo: {0}")]
    FetchFailed(#[source] bossy::Error),
    #[error("Failed to get checkout revision: {0}")]
    RevParseLocalFailed(#[source] bossy::Error),
    #[error("Failed to get upstream revision: {0}")]
    RevParseRemoteFailed(#[source] bossy::Error),
    #[error("Failed to get commit log: {0}")]
    LogFailed(#[source] bossy::Error),
    #[error("Failed to create parent directory {path:?}: {source}")]
    ParentDirCreationFailed {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("Failed to clone repo: {0}")]
    CloneFailed(#[source] bossy::Error),
    #[error("Failed to reset repo: {0}")]
    ResetFailed(#[source] bossy::Error),
    #[error("Failed to clean repo: {0}")]
    CleanFailed(#[source] bossy::Error),
}

#[derive(Clone, Copy, Debug)]
pub enum Status {
    Stale,
    Fresh,
}

impl Status {
    pub fn stale(self) -> bool {
        matches!(self, Self::Stale)
    }
}

#[derive(Clone, Debug)]
pub struct Repo {
    path: PathBuf,
}

impl Repo {
    pub fn from_path(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn git(&self) -> Git<'_> {
        Git::new(self.path())
    }

    pub fn status(&self) -> Result<Status, Error> {
        let status = if !self.path().is_dir() {
            Status::Stale
        } else {
            let git = self.git();
            git.command_parse("fetch origin")
                .run_and_wait()
                .map_err(Error::FetchFailed)?;
            let local = git
                .command_parse("rev-parse HEAD")
                .run_and_wait_for_output()
                .map_err(Error::RevParseLocalFailed)?;
            let remote = git
                .command_parse("rev-parse @{u}")
                .run_and_wait_for_output()
                .map_err(Error::RevParseRemoteFailed)?;
            if local.stdout() != remote.stdout() {
                Status::Stale
            } else {
                Status::Fresh
            }
        };
        Ok(status)
    }

    pub fn latest_commit(&self, format: impl AsRef<str>) -> Result<String, Error> {
        self.git()
            .command_parse(format!("log -1 --pretty={}", format.as_ref()))
            .run_and_wait_for_str(|s| s.trim().to_owned())
            .map_err(Error::LogFailed)
    }

    pub fn latest_subject(&self) -> Result<String, Error> {
        self.latest_commit("%s")
    }

    pub fn latest_body(&self) -> Result<String, Error> {
        self.latest_commit("%b")
    }

    pub fn update(&self, url: impl AsRef<std::ffi::OsStr>) -> Result<(), Error> {
        let path = self.path();
        if !path.is_dir() {
            let parent = self
                .path()
                .parent()
                .expect("developer error: `Repo` path was at root");
            if !parent.is_dir() {
                std::fs::create_dir_all(parent).map_err(|source| {
                    Error::ParentDirCreationFailed {
                        path: parent.to_owned(),
                        source,
                    }
                })?;
            }
            Git::new(parent)
                .command_parse("clone --depth 1 --single-branch")
                .with_arg(url)
                .with_arg(path)
                .run_and_wait()
                .map_err(Error::CloneFailed)?;
        } else {
            println!(
                "Updating `{}` repo...",
                Path::new(
                    self.path()
                        .file_name()
                        .expect("developer error: `Repo` path had no file name")
                )
                .display()
            );
            self.git()
                .command_parse("fetch --depth 1")
                .run_and_wait()
                .map_err(Error::FetchFailed)?;
            self.git()
                .command_parse("reset --hard origin/master")
                .run_and_wait()
                .map_err(Error::ResetFailed)?;
            self.git()
                .command_parse("clean -dfx --exclude /target")
                .run_and_wait()
                .map_err(Error::CleanFailed)?;
        }
        Ok(())
    }
}
