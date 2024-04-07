use color_eyre::eyre::Result;
use git2::{Commit, DiffOptions, Repository};
use std::path::Path;

fn main() -> Result<()> {
    color_eyre::install()?;

    let repo = Repository::open_from_env()?;

    let commits = get_commits_for_file(&repo, Path::new("composer.lock"))?;

    dbg!(commits);

    Ok(())
}

fn get_commits_for_file<'a>(repo: &'a Repository, file_path: &'_ Path) -> Result<Vec<Commit<'a>>> {
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
            continue;
        };

        let mut diff_opts = DiffOptions::new();
        diff_opts.pathspec(file_path);
        let diff = repo.diff_tree_to_tree(Some(&parent_tree), Some(&tree), Some(&mut diff_opts))?;

        if diff.deltas().len() > 0 {
            commits.push(commit);
        }
    }

    Ok(commits)
}
