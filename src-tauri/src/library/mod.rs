//! What the four services of the Library (skills, agents, memory, instructions) share: where
//! their folders are ([`roots`]), which folders the app knows of ([`known_folders`]) and how a
//! file or folder goes into the bin ([`trash`]).

pub mod known_folders;
pub mod roots;
pub mod trash;

#[cfg(test)]
pub(crate) mod test_support {
    use std::path::Path;

    /// A link to a folder: a junction on Windows, as the app makes them, a symlink elsewhere.
    pub(crate) fn link(target: &Path, at: &Path) {
        #[cfg(windows)]
        junction::create(target, at).unwrap();
        #[cfg(not(windows))]
        std::os::unix::fs::symlink(target, at).unwrap();
    }
}
