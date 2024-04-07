use std::{
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

use chrono::{DateTime, Utc};
use clap::Parser;
use color_eyre::eyre::{eyre, Result};
use git2::{Commit, DiffOptions, Repository};

/// Simple program to greet a person
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// Lock file to analyze.
    ///
    /// Supports: `composer.lock`
    #[arg(short, long)]
    file: Option<String>,

    /// Name of the library to generate a timeline for
    #[arg(short, long)]
    library: String,
    // TODO option to select the repo
}

fn main() -> Result<()> {
    color_eyre::install()?;

    let args = Args::parse();

    let repo = Repository::open_from_env()?;

    let file_path = if let Some(file) = &args.file {
        PathBuf::from(file)
    } else {
        detect_file()?
    };

    let results = get_commits_for_file(&repo, &file_path)?
        .into_iter()
        // TODO we are ignoring some errors due to the flat_map
        .flat_map(|commit| search_in_file(&repo, commit, &file_path, &args.library))
        .collect::<Vec<_>>();

    let mut previous_version = None;
    let iter = results.into_iter();
    for result in iter.rev() {
        if result.version == previous_version {
            continue;
        }
        previous_version = result.version.clone();

        let date = DateTime::<Utc>::from(result.date);
        println!(
            "Version: {}, Date: {}",
            result.version.unwrap_or_else(|| "None".to_string()),
            date
        );
    }

    Ok(())
}

fn detect_file() -> Result<PathBuf> {
    let paths = ["Cargo.lock", "composer.lock", "package-lock.json"];

    for path in paths {
        let file = PathBuf::from(path);
        if file.exists() {
            return Ok(file);
        }
    }

    Err(eyre!(
        "Couldn't automatically detect file, please specify one with the --file flag"
    ))
}

fn get_commits_for_file<'a>(repo: &'a Repository, file_path: &Path) -> Result<Vec<Commit<'a>>> {
    let mut revwalk = repo.revwalk()?;
    revwalk.push_head()?;
    revwalk.set_sorting(git2::Sort::TIME)?;

    let mut commits = Vec::new();

    for rev in revwalk {
        let commit = repo.find_commit(rev?)?;
        let tree = commit.tree()?;
        let parent_tree = if let Ok(parent) = commit.parent(0) {
            parent.tree()?
        } else {
            commits.push(commit);
            continue;
        };

        let mut diff_opts = DiffOptions::new();
        diff_opts.pathspec(file_path);
        // we dont add `library` to the diff_opts, cause a version upgrade might not change a line that contains `library`
        let diff = repo.diff_tree_to_tree(Some(&parent_tree), Some(&tree), Some(&mut diff_opts))?;

        if diff.deltas().len() > 0 {
            commits.push(commit);
        }
    }

    Ok(commits)
}

#[derive(Debug)]
struct SearchResult {
    version: Option<String>,
    date: SystemTime,
}

fn search_in_file(
    repo: &Repository,
    commit: Commit<'_>,
    file_path: &Path,
    library: &str,
) -> Result<SearchResult> {
    let Ok(blob) = commit
        .tree()?
        .get_path(Path::new(file_path))?
        .to_object(repo)?
        .into_blob()
    else {
        return Err(eyre!("Not a blob"));
    };

    let content = String::from_utf8(blob.content().into())?;

    let file_type = file_path
        .file_name()
        .and_then(|s| s.to_str())
        .and_then(PackageManager::guess_from_file_name);

    let version = if let Some(file_type) = file_type {
        file_type.get_library_version(&content, library)?
    } else {
        todo!("Unimplemented: Can't automatically detect file type from file contents");
    };

    Ok(SearchResult {
        version,
        date: date_from_commit(commit),
    })
}

fn date_from_commit(commit: Commit<'_>) -> SystemTime {
    let time = commit.time();

    SystemTime::UNIX_EPOCH + Duration::from_secs(time.seconds() as u64)
}

enum PackageManager {
    Composer,
    Cargo,
    Npm,
}

impl PackageManager {
    fn guess_from_file_name(file_name: &str) -> Option<Self> {
        match file_name {
            "composer.lock" => Some(PackageManager::Composer),
            "Cargo.lock" => Some(PackageManager::Cargo),
            "package-lock.json" => Some(PackageManager::Npm),
            _ => None,
        }
    }

    fn get_library_version(&self, content: &str, library: &str) -> Result<Option<String>> {
        match self {
            PackageManager::Composer => composer::get_library_version(content, library),
            PackageManager::Cargo => cargo::get_library_version(content, library),
            PackageManager::Npm => npm::get_library_version(content, library),
        }
    }
}

mod composer {
    use color_eyre::eyre::Result;

    #[derive(serde::Deserialize)]
    struct ComposerLock {
        packages: Vec<ComposerPackage>,
    }
    #[derive(serde::Deserialize)]
    struct ComposerPackage {
        /// Package name
        name: String,
        /// Package version
        version: String,
    }

    pub fn get_library_version(content: &str, library: &str) -> Result<Option<String>> {
        let lock: ComposerLock = serde_json::from_str(content)?;

        let package = lock.packages.iter().find(|package| package.name == library);

        if let Some(package) = package {
            Ok(Some(package.version.clone()))
        } else {
            Ok(None)
        }
    }
}

mod cargo {
    use color_eyre::eyre::Result;

    #[derive(serde::Deserialize, Debug)]
    struct CargoLock {
        package: Vec<CargoPackage>,
    }
    #[derive(serde::Deserialize, Debug)]
    struct CargoPackage {
        /// Package name
        name: String,
        /// Package version
        version: String,
    }
    pub fn get_library_version(content: &str, library: &str) -> Result<Option<String>> {
        let lock: CargoLock = toml::from_str(content)?;

        let package = lock.package.iter().find(|package| package.name == library);

        if let Some(package) = package {
            Ok(Some(package.version.clone()))
        } else {
            Ok(None)
        }
    }
}

mod npm {
    use std::collections::HashMap;

    use color_eyre::eyre::Result;

    #[derive(serde::Deserialize, Debug)]
    struct NpmLock {
        packages: HashMap<String, NpmPackage>,
    }
    #[derive(serde::Deserialize, Debug)]
    struct NpmPackage {
        /// Package version
        version: Option<String>,
    }

    pub fn get_library_version(content: &str, library: &str) -> Result<Option<String>> {
        let lock: NpmLock = serde_json::from_str(content)?;

        if let Some(NpmPackage {
            version: Some(version),
        }) = lock.packages.get(&format!("node_modules/{library}"))
        {
            Ok(Some(version.clone()))
        } else {
            Ok(None)
        }
    }
}
