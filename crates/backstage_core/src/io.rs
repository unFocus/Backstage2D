//! Project directories (`name.bs2d/`):
//!
//! ```text
//! project.ron               format version, settings, editor prefs, root,
//!                           composition list, assets
//! compositions/<id>.ron     one composition each
//! assets/                   bitmaps, fonts, audio (referenced by path)
//! ```
//!
//! Output is canonical (sorted maps, fixed formatting), so saving the same
//! project always produces the same bytes and diffs stay minimal.

use crate::id::{AssetId, CompId};
use crate::project::{Asset, Composition, EditorPrefs, FORMAT_VERSION, Project, ProjectSettings};
use crate::validate::ValidationError;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

const PROJECT_FILE: &str = "project.ron";
const COMPOSITIONS_DIR: &str = "compositions";

#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error("{path}: {source}")]
    Io { path: PathBuf, source: io::Error },
    #[error("{path}: {message}")]
    Parse { path: PathBuf, message: String },
    #[error("{path}: unsupported format version {found} (this build reads {FORMAT_VERSION})")]
    UnsupportedVersion { path: PathBuf, found: u32 },
    #[error("{path}: file holds composition {found}, expected {expected}")]
    IdMismatch { path: PathBuf, expected: CompId, found: CompId },
    #[error("invalid project:\n{}", .0.iter().map(|e| format!("  - {e}")).collect::<Vec<_>>().join("\n"))]
    Invalid(Vec<ValidationError>),
}

#[derive(Debug, thiserror::Error)]
pub enum SaveError {
    #[error("{path}: {source}")]
    Io { path: PathBuf, source: io::Error },
    #[error("serializing {what}: {message}")]
    Serialize { what: String, message: String },
}

/// `project.ron` contents.
#[derive(Serialize, Deserialize)]
struct ProjectFile {
    format_version: u32,
    settings: ProjectSettings,
    #[serde(default)]
    editor: EditorPrefs,
    root: CompId,
    compositions: Vec<CompId>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    assets: BTreeMap<AssetId, Asset>,
}

fn pretty() -> ron::ser::PrettyConfig {
    ron::ser::PrettyConfig::new()
        .indentor("    ")
        .new_line("\n")
        .struct_names(false)
        // Drops `Some(..)` wrappers and the doubled parentheses around
        // newtype variants holding structs. Written as a `#![enable(..)]`
        // header, so files stay self-describing.
        .extensions(
            ron::extensions::Extensions::IMPLICIT_SOME | ron::extensions::Extensions::UNWRAP_VARIANT_NEWTYPES,
        )
}

fn to_ron<T: Serialize>(value: &T, what: &str) -> Result<String, SaveError> {
    let mut text = ron::ser::to_string_pretty(value, pretty())
        .map_err(|e| SaveError::Serialize { what: what.to_owned(), message: e.to_string() })?;
    text.push('\n');
    Ok(text)
}

fn composition_file(dir: &Path, id: CompId) -> PathBuf {
    dir.join(COMPOSITIONS_DIR).join(format!("{id}.ron"))
}

/// Writes via a temporary sibling file and a rename, so readers never see a
/// half-written file.
fn write_atomic(path: &Path, contents: &str) -> Result<(), SaveError> {
    let io_err = |source| SaveError::Io { path: path.to_owned(), source };
    let tmp = path.with_extension("ron.tmp");
    fs::write(&tmp, contents).map_err(io_err)?;
    fs::rename(&tmp, path).map_err(io_err)
}

/// The project's files as `(path relative to the project directory,
/// contents)`, in a fixed order with `project.ron` last. The bytes are
/// canonical: equal projects give equal files.
pub fn to_files(project: &Project) -> Result<Vec<(PathBuf, String)>, SaveError> {
    let mut files = Vec::with_capacity(project.compositions.len() + 1);
    for comp in project.compositions.values() {
        files.push((composition_file(Path::new(""), comp.id), to_ron(comp, &comp.id.to_string())?));
    }
    let file = ProjectFile {
        format_version: FORMAT_VERSION,
        settings: project.settings,
        editor: project.editor,
        root: project.root,
        compositions: project.compositions.keys().copied().collect(),
        assets: project.assets.clone(),
    };
    files.push((PathBuf::from(PROJECT_FILE), to_ron(&file, PROJECT_FILE)?));
    Ok(files)
}

/// Saves `project` into `dir` (created if needed). Composition files that
/// are no longer in the project are removed. Does not validate; callers
/// should only save valid projects.
pub fn save(project: &Project, dir: &Path) -> Result<(), SaveError> {
    let files = to_files(project)?;
    let comps_dir = dir.join(COMPOSITIONS_DIR);
    fs::create_dir_all(&comps_dir).map_err(|source| SaveError::Io { path: comps_dir.clone(), source })?;

    // `project.ron` comes last, so a crash mid-save leaves the old one
    // pointing at complete composition files.
    let (project_file, comp_files) = files.split_last().expect("project.ron is always there");
    for (path, text) in comp_files {
        write_atomic(&dir.join(path), text)?;
    }
    let entries =
        fs::read_dir(&comps_dir).map_err(|source| SaveError::Io { path: comps_dir.clone(), source })?;
    for entry in entries.flatten() {
        let path = entry.path();
        let stale = path.extension().is_some_and(|e| e == "ron")
            && path
                .file_stem()
                .and_then(|s| s.to_str())
                .and_then(|s| s.parse::<CompId>().ok())
                .is_some_and(|id| !project.compositions.contains_key(&id));
        if stale {
            fs::remove_file(&path).map_err(|source| SaveError::Io { path, source })?;
        }
    }
    write_atomic(&dir.join(&project_file.0), &project_file.1)
}

/// One value as a single line of RON, in the same dialect as project files
/// (implicit `Some`, unwrapped newtype variants) but without the header.
/// Used for the command log, one entry per line.
pub fn to_ron_line<T: Serialize>(value: &T) -> Result<String, SaveError> {
    ron_options().to_string(value).map_err(|e| SaveError::Serialize {
        what: std::any::type_name::<T>().to_owned(),
        message: e.to_string(),
    })
}

/// Parses a line written by [`to_ron_line`].
pub fn from_ron_line<T: for<'de> Deserialize<'de>>(line: &str) -> Result<T, ron::error::SpannedError> {
    ron_options().from_str(line)
}

fn ron_options() -> ron::Options {
    ron::Options::default().with_default_extension(
        ron::extensions::Extensions::IMPLICIT_SOME | ron::extensions::Extensions::UNWRAP_VARIANT_NEWTYPES,
    )
}

fn read(path: &Path) -> Result<String, LoadError> {
    fs::read_to_string(path).map_err(|source| LoadError::Io { path: path.to_owned(), source })
}

fn parse<T: for<'de> Deserialize<'de>>(path: &Path, text: &str) -> Result<T, LoadError> {
    ron::from_str(text).map_err(|e| LoadError::Parse { path: path.to_owned(), message: e.to_string() })
}

/// Loads and validates the project in `dir`.
pub fn load(dir: &Path) -> Result<Project, LoadError> {
    let project_path = dir.join(PROJECT_FILE);
    let text = read(&project_path)?;
    // Check the version before the full parse, so a newer format gets a
    // clear error instead of a confusing parse failure.
    #[derive(Deserialize)]
    struct VersionOnly {
        format_version: u32,
    }
    let version: VersionOnly = parse(&project_path, &text)?;
    if version.format_version != FORMAT_VERSION {
        return Err(LoadError::UnsupportedVersion { path: project_path, found: version.format_version });
    }
    let file: ProjectFile = parse(&project_path, &text)?;

    let mut compositions = BTreeMap::new();
    for id in file.compositions {
        let path = composition_file(dir, id);
        let comp: Composition = parse(&path, &read(&path)?)?;
        if comp.id != id {
            return Err(LoadError::IdMismatch { path, expected: id, found: comp.id });
        }
        compositions.insert(id, comp);
    }
    let project = Project {
        settings: file.settings,
        editor: file.editor,
        root: file.root,
        compositions,
        assets: file.assets,
    };
    project.validate().map_err(LoadError::Invalid)?;
    Ok(project)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sample::{self, ids};
    use crate::test_util::tempdir;

    fn files(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
        let mut out = Vec::new();
        for sub in [dir.to_owned(), dir.join(COMPOSITIONS_DIR)] {
            for entry in fs::read_dir(sub).unwrap().flatten() {
                if entry.path().is_file() {
                    out.push((entry.path(), fs::read(entry.path()).unwrap()));
                }
            }
        }
        out.sort();
        out
    }

    #[test]
    fn save_then_load_is_identity() {
        let dir = tempdir("roundtrip");
        let project = sample::bounce();
        save(&project, &dir).unwrap();
        assert_eq!(load(&dir).unwrap(), project);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn output_is_canonical() {
        let dir = tempdir("canonical");
        save(&sample::bounce(), &dir).unwrap();
        let first = files(&dir);
        save(&load(&dir).unwrap(), &dir).unwrap();
        assert_eq!(files(&dir), first, "load + save must reproduce the same bytes");
        assert!(
            first.iter().all(|(p, _)| p.extension().is_some_and(|e| e == "ron")),
            "temp files left: {first:?}"
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn to_files_is_what_save_writes() {
        let dir = tempdir("to-files");
        let project = sample::bounce();
        save(&project, &dir).unwrap();
        let mut expected: Vec<_> = to_files(&project)
            .unwrap()
            .into_iter()
            .map(|(p, text)| (dir.join(p), text.into_bytes()))
            .collect();
        expected.sort();
        assert_eq!(files(&dir), expected);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn stale_compositions_are_removed() {
        let dir = tempdir("stale");
        let mut project = sample::bounce();
        save(&project, &dir).unwrap();
        // Drop the blinker (and the stage's instance of it).
        project.compositions.remove(&ids::BLINKER);
        let stage = project.compositions.get_mut(&ids::STAGE).unwrap();
        stage.nodes.remove(&ids::EYES);
        stage.nodes.get_mut(&ids::STAGE_ROOT).unwrap().children.retain(|&n| n != ids::EYES);
        save(&project, &dir).unwrap();
        assert!(!composition_file(&dir, ids::BLINKER).exists());
        assert_eq!(load(&dir).unwrap(), project);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn clear_errors() {
        let dir = tempdir("errors");
        assert!(matches!(load(&dir), Err(LoadError::Io { .. })), "missing project.ron");

        save(&sample::bounce(), &dir).unwrap();
        let project_ron = dir.join(PROJECT_FILE);
        let text = fs::read_to_string(&project_ron).unwrap();
        fs::write(&project_ron, text.replace("format_version: 1", "format_version: 99")).unwrap();
        assert!(matches!(load(&dir), Err(LoadError::UnsupportedVersion { found: 99, .. })));

        fs::write(&project_ron, &text).unwrap();
        let comp = composition_file(&dir, ids::BALL);
        fs::write(&comp, "(this is not valid").unwrap();
        let err = load(&dir).unwrap_err();
        assert!(matches!(err, LoadError::Parse { .. }), "{err}");
        assert!(err.to_string().contains(&ids::BALL.to_string()), "error names the file: {err}");

        fs::write(&comp, fs::read_to_string(composition_file(&dir, ids::BLINKER)).unwrap()).unwrap();
        assert!(matches!(load(&dir), Err(LoadError::IdMismatch { .. })));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn invalid_projects_are_rejected_on_load() {
        let dir = tempdir("invalid");
        let mut project = sample::bounce();
        project.compositions.get_mut(&ids::BALL).unwrap().default_animation =
            Some(crate::AnimId::from_raw(1));
        save(&project, &dir).unwrap();
        let err = load(&dir).unwrap_err();
        assert!(matches!(err, LoadError::Invalid(_)), "{err}");
        assert!(err.to_string().contains("default_animation"), "{err}");
        fs::remove_dir_all(dir).unwrap();
    }
}
