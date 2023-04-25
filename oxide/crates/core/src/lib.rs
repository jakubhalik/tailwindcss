use crate::parser::Extractor;
use ignore::WalkBuilder;
use rayon::prelude::*;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::path::PathBuf;
use tracing::event;

pub mod candidate;
pub mod glob;
pub mod location;
pub mod modifier;
pub mod parser;
pub mod utility;
pub mod variant;

#[derive(Debug, Clone)]
pub struct ChangedContent {
    pub file: Option<PathBuf>,
    pub content: Option<String>,
    pub extension: String,
}

pub fn parse_candidate_strings_from_files(changed_content: Vec<ChangedContent>) -> Vec<String> {
    if matches!(std::env::var("DEBUG"), Ok(value) if value.eq("*") || value.eq("1") || value.eq("true") || value.contains("tailwind"))
    {
        tracing_subscriber::fmt()
            .with_max_level(tracing::Level::INFO)
            .with_span_events(tracing_subscriber::fmt::format::FmtSpan::ACTIVE)
            .compact()
            .init();
    }

    parse_all_blobs(read_all_files(changed_content))
}

#[derive(Debug, Clone)]
pub struct ContentPathInfo {
    pub base: String,
}

pub fn resolve_content_paths(args: ContentPathInfo) -> Vec<String> {
    let root = args.base;
    let paths: Vec<_> = WalkBuilder::new(&root)
        .hidden(false)
        .filter_entry(move |entry| {
            // Skip known ignored folders
            if entry.file_type().unwrap().is_dir() {
                return entry
                    .file_name()
                    .to_str()
                    .map(|s| s != ".git")
                    .unwrap_or(false);
            }

            is_allowed_content_path(entry.path())
        })
        .build()
        .filter_map(Result::ok)
        .filter(|e| e.path().is_file())
        .collect();

    // Group paths by parent path (folder) and collect all the extensions
    let mut groups: BTreeMap<PathBuf, BTreeSet<String>> = Default::default();
    for path in &paths {
        if let Some(parent) = path.path().parent() {
            let extension = path
                .path()
                .extension()
                .map(|s| s.to_str().unwrap_or_default().to_string())
                .unwrap_or_default();

            groups
                .entry(parent.to_path_buf())
                .or_insert_with(Default::default)
                .insert(extension);
        }
    }

    let root = Path::new(&root);

    // Convert the groups into glob patterns
    groups
        .iter()
        .flat_map(|(path, extensions)| match extensions.len() {
            0 => None, // This should never happen
            1 => Some(format!(
                "{}/{}.{}",
                path.display(),
                if path == root { "*" } else { "**/*" },
                extensions.iter().next().unwrap()
            )),
            _ => Some(format!(
                "{}/{}.{{{}}}",
                path.display(),
                if path == root { "*" } else { "**/*" },
                extensions
                    .iter()
                    .map(|x| x.to_string())
                    .collect::<Vec<_>>()
                    .join(",")
            )),
        })
        .collect()
}

pub fn is_git_ignored_content_path(base: &Path, path: &Path) -> bool {
    !WalkBuilder::new(base)
        .hidden(false)
        .build()
        .filter_map(Result::ok)
        .any(|e| e.path() == path)
}

pub fn is_allowed_content_path(path: &Path) -> bool {
    let binary_extensions = include_str!("fixtures/binary-extensions.txt")
        .trim()
        .lines()
        .collect::<Vec<_>>();
    let ignored_extensions = include_str!("fixtures/ignored-extensions.txt")
        .trim()
        .lines()
        .collect::<Vec<_>>();
    let ignored_files = include_str!("fixtures/ignored-files.txt")
        .trim()
        .lines()
        .collect::<Vec<_>>();

    let path = PathBuf::from(path);

    // Skip known ignored files
    if path
        .file_name()
        .unwrap()
        .to_str()
        .map(|s| ignored_files.contains(&s))
        .unwrap_or(false)
    {
        return false;
    }

    // Skip known ignored extensions
    return path
        .extension()
        .map(|s| s.to_str().unwrap_or_default())
        .map(|ext| !ignored_extensions.contains(&ext) && !binary_extensions.contains(&ext))
        .unwrap_or(false);
}

#[tracing::instrument(skip(changed_content))]
fn read_all_files(changed_content: Vec<ChangedContent>) -> Vec<Vec<u8>> {
    event!(
        tracing::Level::INFO,
        "Reading {:?} file(s)",
        changed_content.len()
    );

    changed_content
        .into_par_iter()
        .map(|c| match (c.file, c.content) {
            (Some(file), None) => match std::fs::read(file) {
                Ok(content) => content,
                Err(e) => {
                    event!(tracing::Level::ERROR, "Failed to read file: {:?}", e);
                    Default::default()
                }
            },
            (None, Some(content)) => content.into_bytes(),
            _ => Default::default(),
        })
        .collect()
}

#[tracing::instrument(skip(blobs))]
fn parse_all_blobs(blobs: Vec<Vec<u8>>) -> Vec<String> {
    let input: Vec<_> = blobs.iter().map(|blob| &blob[..]).collect();
    let input = &input[..];

    let mut result: Vec<String> = input
        .par_iter()
        .map(|input| Extractor::unique(input, Default::default()))
        .reduce(Default::default, |mut a, b| {
            a.extend(b);
            a
        })
        .into_iter()
        .map(|s| {
            // SAFETY: When we parsed the candidates, we already guaranteed that the byte slices
            // are valid, therefore we don't have to re-check here when we want to convert it back
            // to a string.
            unsafe { String::from_utf8_unchecked(s.to_vec()) }
        })
        .collect();
    result.sort();
    result
}
