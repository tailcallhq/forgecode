use handlebars::Handlebars;
use include_dir::{Dir, DirEntry, File};

/// Returns an iterator over all files embedded in `dir`, recursively
/// descending into subdirectories.
pub fn files(dir: &'static Dir<'static>) -> impl Iterator<Item = &'static File<'static>> {
    dir.entries().iter().flat_map(walk_entry)
}

fn walk_entry(
    entry: &'static DirEntry<'static>,
) -> Box<dyn Iterator<Item = &'static File<'static>>> {
    match entry {
        DirEntry::File(f) => Box::new(std::iter::once(f)),
        DirEntry::Dir(d) => Box::new(d.entries().iter().flat_map(walk_entry)),
    }
}

/// Registers all files in `dir` (recursively) as Handlebars templates.
///
/// Template names match the relative file path as returned by
/// [`File::path`] (e.g. `forge-system-prompt.md`). Panics if any file path
/// is not valid UTF-8, if any file content is not valid UTF-8, or if template
/// parsing fails.
pub fn register_templates(hb: &mut Handlebars<'_>, dir: &'static Dir<'static>) {
    for file in files(dir) {
        let name = file.path().to_str().unwrap_or_else(|| {
            panic!(
                "embedded template path '{:?}' is not valid UTF-8",
                file.path()
            )
        });
        let content = file
            .contents_utf8()
            .unwrap_or_else(|| panic!("embedded template '{}' is not valid UTF-8", name))
            // git stores templates with LF endings; on Windows checkouts
            // (autocrlf) the embedded bytes carry CRLF, which leaks into
            // rendered output and breaks snapshot determinism. Normalize to
            // the authored LF bytes so rendering is identical on all platforms.
            .replace("\r\n", "\n");
        hb.register_template_string(name, content)
            .unwrap_or_else(|e| panic!("failed to register template '{}': {}", name, e));
    }
}
