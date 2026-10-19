//! Recursive directory listing (the part of a `walkdir`-style API InstallScope
//! uses), iterative with an explicit stack so deep trees can't overflow.
//!
//! The root is reported at depth 0. Links and junctions are reported but
//! not entered unless `follow_links(true)`. With `contents_first(true)` a
//! folder comes after everything inside it (for deleting).

use std::fs::{self, FileType, Metadata, ReadDir};
use std::io;
use std::path::{Path, PathBuf};

pub struct WalkDir {
    root: PathBuf,
    min: usize,
    max: usize,
    follow: bool,
    contents_first: bool,
}

impl WalkDir {
    pub fn new(root: impl AsRef<Path>) -> Self {
        WalkDir {
            root: root.as_ref().to_path_buf(),
            min: 0,
            max: usize::MAX,
            follow: false,
            contents_first: false,
        }
    }
    pub fn min_depth(mut self, n: usize) -> Self {
        self.min = n;
        self
    }
    pub fn max_depth(mut self, n: usize) -> Self {
        self.max = n;
        self
    }
    pub fn follow_links(mut self, yes: bool) -> Self {
        self.follow = yes;
        self
    }
    pub fn contents_first(mut self, yes: bool) -> Self {
        self.contents_first = yes;
        self
    }
}

#[derive(Clone, Debug)]
pub struct DirEntry {
    path: PathBuf,
    ft: FileType,
    depth: usize,
    follow: bool,
}

impl DirEntry {
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn into_path(self) -> PathBuf {
        self.path
    }
    pub fn file_type(&self) -> FileType {
        self.ft
    }
    pub fn depth(&self) -> usize {
        self.depth
    }
    pub fn path_is_symlink(&self) -> bool {
        fs::symlink_metadata(&self.path)
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false)
    }
    pub fn metadata(&self) -> io::Result<Metadata> {
        if self.follow {
            fs::metadata(&self.path)
        } else {
            fs::symlink_metadata(&self.path)
        }
    }
}

struct Frame {
    rd: ReadDir,
    /// Depth of the entries inside this folder.
    depth: usize,
    /// The folder itself, reported once its contents are done.
    after: Option<DirEntry>,
}

pub struct IntoIter {
    opts: WalkDir,
    start: bool,
    stack: Vec<Frame>,
}

impl IntoIterator for WalkDir {
    type Item = io::Result<DirEntry>;
    type IntoIter = IntoIter;
    fn into_iter(self) -> IntoIter {
        IntoIter {
            opts: self,
            start: true,
            stack: Vec::new(),
        }
    }
}

impl IntoIter {
    fn kind_of(&self, path: &Path, ft: FileType) -> io::Result<FileType> {
        if self.opts.follow && ft.is_symlink() {
            Ok(fs::metadata(path)?.file_type())
        } else {
            Ok(ft)
        }
    }

    /// Enter `e` if it is a folder within the depth limit. Returns the
    /// entry to report now, if any.
    fn visit(&mut self, e: DirEntry) -> Option<io::Result<DirEntry>> {
        let report = e.depth >= self.opts.min;
        if e.ft.is_dir() && e.depth < self.opts.max {
            match fs::read_dir(&e.path) {
                Ok(rd) => {
                    let after = self.opts.contents_first.then(|| e.clone());
                    self.stack.push(Frame {
                        rd,
                        depth: e.depth + 1,
                        after,
                    });
                    if self.opts.contents_first || !report {
                        return None;
                    }
                    return Some(Ok(e));
                }
                // Unreadable folder: still report it (it exists).
                Err(_) => return report.then_some(Ok(e)),
            }
        }
        report.then_some(Ok(e))
    }
}

impl Iterator for IntoIter {
    type Item = io::Result<DirEntry>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.start {
            self.start = false;
            let root = self.opts.root.clone();
            let ft = match fs::symlink_metadata(&root)
                .and_then(|m| self.kind_of(&root, m.file_type()))
            {
                Ok(ft) => ft,
                Err(e) => return Some(Err(e)),
            };
            let e = DirEntry {
                path: root,
                ft,
                depth: 0,
                follow: self.opts.follow,
            };
            if let Some(r) = self.visit(e) {
                return Some(r);
            }
        }
        loop {
            let top = self.stack.last_mut()?;
            match top.rd.next() {
                None => {
                    let frame = self.stack.pop().unwrap();
                    if let Some(e) = frame.after {
                        if e.depth >= self.opts.min {
                            return Some(Ok(e));
                        }
                    }
                }
                Some(Err(e)) => return Some(Err(e)),
                Some(Ok(de)) => {
                    let depth = top.depth;
                    let path = de.path();
                    let ft = match de.file_type().and_then(|ft| self.kind_of(&path, ft)) {
                        Ok(ft) => ft,
                        Err(e) => return Some(Err(e)),
                    };
                    let e = DirEntry {
                        path,
                        ft,
                        depth,
                        follow: self.opts.follow,
                    };
                    if let Some(r) = self.visit(e) {
                        return Some(r);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree() -> crate::util::TestDir {
        let t = crate::util::tempdir().unwrap();
        let r = t.path();
        fs::create_dir_all(r.join("a/b/c")).unwrap();
        fs::write(r.join("top.txt"), b"x").unwrap();
        fs::write(r.join("a/one.txt"), b"x").unwrap();
        fs::write(r.join("a/b/c/deep.txt"), b"xyz").unwrap();
        t
    }

    fn rel(root: &Path, it: impl Iterator<Item = io::Result<DirEntry>>) -> Vec<(String, usize)> {
        it.map(|e| {
            let e = e.unwrap();
            (
                e.path()
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/"),
                e.depth(),
            )
        })
        .collect()
    }

    #[test]
    fn walks_everything_with_depths() {
        let t = tree();
        let mut got = rel(t.path(), WalkDir::new(t.path()).into_iter());
        got.sort();
        assert_eq!(
            got,
            [
                ("".to_string(), 0),
                ("a".into(), 1),
                ("a/b".into(), 2),
                ("a/b/c".into(), 3),
                ("a/b/c/deep.txt".into(), 4),
                ("a/one.txt".into(), 2),
                ("top.txt".into(), 1),
            ]
        );
    }

    #[test]
    fn depth_limits() {
        let t = tree();
        let mut got = rel(
            t.path(),
            WalkDir::new(t.path()).min_depth(1).max_depth(2).into_iter(),
        );
        got.sort();
        assert_eq!(
            got.iter().map(|(p, _)| p.as_str()).collect::<Vec<_>>(),
            ["a", "a/b", "a/one.txt", "top.txt"]
        );
        let only_root = rel(t.path(), WalkDir::new(t.path()).max_depth(0).into_iter());
        assert_eq!(only_root, [("".to_string(), 0)]);
    }

    #[test]
    fn contents_first_lists_folders_after_what_is_inside() {
        let t = tree();
        let got = rel(
            t.path(),
            WalkDir::new(t.path()).contents_first(true).into_iter(),
        );
        let pos = |p: &str| got.iter().position(|(x, _)| x == p).unwrap();
        assert!(pos("a/b/c/deep.txt") < pos("a/b/c"));
        assert!(pos("a/b/c") < pos("a/b"));
        assert!(pos("a/one.txt") < pos("a"));
        assert_eq!(got.last().unwrap().0, "", "the root comes last");
        // Deleting in this order empties every folder before removing it.
        for e in WalkDir::new(t.path())
            .contents_first(true)
            .min_depth(1)
            .into_iter()
            .flatten()
        {
            if e.file_type().is_dir() {
                fs::remove_dir(e.path()).unwrap();
            } else {
                fs::remove_file(e.path()).unwrap();
            }
        }
        assert_eq!(fs::read_dir(t.path()).unwrap().count(), 0);
    }

    #[test]
    fn a_file_root_is_itself_and_a_missing_root_is_an_error() {
        let t = tree();
        let f = t.path().join("top.txt");
        let got: Vec<_> = WalkDir::new(&f)
            .into_iter()
            .map(|e| e.unwrap().into_path())
            .collect();
        assert_eq!(got, [f]);
        let mut it = WalkDir::new(t.path().join("missing")).into_iter();
        assert!(it.next().unwrap().is_err());
        assert!(it.next().is_none());
    }

    #[cfg(unix)]
    #[test]
    fn links_are_listed_but_not_entered() {
        let t = tree();
        std::os::unix::fs::symlink(t.path().join("a"), t.path().join("link")).unwrap();
        let got = rel(t.path(), WalkDir::new(t.path()).into_iter());
        assert!(got.iter().any(|(p, _)| p == "link"));
        assert!(!got.iter().any(|(p, _)| p.starts_with("link/")));
    }
}
