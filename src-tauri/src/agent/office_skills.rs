use include_dir::{include_dir, Dir};
use std::path::{Path, PathBuf};

static DOCS_SKILLS: Dir<'static> = include_dir!("$CARGO_MANIFEST_DIR/../DOCS_SKILLS");

pub fn is_cowork_mode(mode: Option<&str>) -> bool {
    matches!(mode, Some("cowork") | Some("office"))
}

pub fn ensure_docs_skills() -> Result<PathBuf, String> {
    let target = crate::storage::data_dir().join("DOCS_SKILLS");
    write_embedded_dir(&DOCS_SKILLS, &target)?;
    Ok(target)
}

pub fn office_prompt(skills_root: &Path) -> String {
    let root = skills_root.to_string_lossy();
    format!(
        r#"HelionCoder Office/Cowork mode is enabled.

The local DOCS_SKILLS repository has been mounted at:
{}

Treat these as available local skills and use them as the source of truth:
- docx: {}/public/docx/SKILL.md
- pptx: {}/public/pptx/SKILL.md
- xlsx: {}/public/xlsx/SKILL.md
- pdf: {}/public/pdf/SKILL.md

Before creating, editing, analyzing, or converting any Office/PDF file, read the relevant SKILL.md completely and follow its workflow. If that SKILL.md points to supporting files such as ooxml.md, docx-js.md, html2pptx.md, REFERENCE.md, FORMS.md, or recalc.py, read the needed supporting file before implementing.

When you create a final .docx, .pptx, .xlsx, or .pdf output, save it in the current working directory unless the user specified another path. Mention each generated file as an exact absolute path wrapped in backticks so the desktop app can open the right-side preview automatically.
"#,
        root, root, root, root, root
    )
}

fn write_embedded_dir(dir: &Dir<'_>, root: &Path) -> Result<(), String> {
    std::fs::create_dir_all(root)
        .map_err(|e| format!("Failed to create DOCS_SKILLS directory: {}", e))?;

    for file in dir.files() {
        let rel = file
            .path()
            .strip_prefix(DOCS_SKILLS.path())
            .unwrap_or_else(|_| file.path());
        if should_skip(rel) {
            continue;
        }
        let out = root.join(rel);
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create {}: {}", parent.display(), e))?;
        }

        let contents = file.contents();
        let unchanged = std::fs::read(&out)
            .map(|existing| existing == contents)
            .unwrap_or(false);
        if !unchanged {
            std::fs::write(&out, contents)
                .map_err(|e| format!("Failed to write {}: {}", out.display(), e))?;
        }
    }

    for child in dir.dirs() {
        write_embedded_dir(child, root)?;
    }

    Ok(())
}

fn should_skip(path: &Path) -> bool {
    path.components().any(|part| {
        let name = part.as_os_str().to_string_lossy();
        name == ".DS_Store" || name == "node_modules" || name == ".git"
    })
}
