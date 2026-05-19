use crate::process_ext::HideConsole;
use base64::{engine::general_purpose, Engine as _};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceTool {
    pub id: String,
    pub name: String,
    pub available: bool,
    pub source: Option<String>,
    pub icon_data_url: Option<String>,
}

type ToolCandidate = (
    &'static str,
    &'static str,
    &'static [&'static str],
    &'static [&'static str],
    bool,
);

#[tauri::command]
pub async fn open_workspace_tool(
    kind: String,
    cwd: String,
    source: Option<String>,
) -> Result<(), String> {
    let path = PathBuf::from(cwd.trim());
    if !path.exists() {
        return Err("Workspace path does not exist".to_string());
    }

    if !matches!(
        kind.as_str(),
        "terminal" | "windows-terminal" | "powershell" | "finder" | "explorer" | "editor"
    ) {
        if let Some(source) = source.as_deref().filter(|s| !s.trim().is_empty()) {
            if open_source_launcher(source, &path).is_ok() {
                return Ok(());
            }
        }
    }

    match kind.as_str() {
        "vscode" => open_vscode(&path),
        "cursor" => open_macos_or_cli(
            &path,
            &["com.todesktop.230313mzl4w4u92"],
            &["Cursor"],
            &["cursor"],
        ),
        "windsurf" => open_macos_or_cli(
            &path,
            &["com.exafunction.windsurf"],
            &["Windsurf"],
            &["windsurf"],
        ),
        "zed" => open_macos_or_cli(&path, &["dev.zed.Zed"], &["Zed"], &["zed"]),
        "sublime" => open_macos_or_cli(
            &path,
            &["com.sublimetext.4", "com.sublimetext.3"],
            &["Sublime Text"],
            &["subl", "sublime_text"],
        ),
        "trae" => open_macos_or_cli(&path, &["com.trae.app"], &["Trae"], &["trae"]),
        "xcode" => open_macos_or_cli(&path, &["com.apple.dt.Xcode"], &["Xcode"], &[]),
        "intellij" => open_macos_or_cli(
            &path,
            &["com.jetbrains.intellij", "com.jetbrains.intellij.ce"],
            &[
                "IntelliJ IDEA",
                "IntelliJ IDEA Ultimate",
                "IntelliJ IDEA CE",
            ],
            &["idea"],
        ),
        "webstorm" => open_macos_or_cli(
            &path,
            &["com.jetbrains.WebStorm"],
            &["WebStorm"],
            &["webstorm"],
        ),
        "pycharm" => open_macos_or_cli(
            &path,
            &["com.jetbrains.pycharm", "com.jetbrains.pycharm.ce"],
            &["PyCharm", "PyCharm Professional", "PyCharm CE"],
            &["pycharm"],
        ),
        "goland" => open_macos_or_cli(&path, &["com.jetbrains.goland"], &["GoLand"], &["goland"]),
        "rustrover" => open_macos_or_cli(
            &path,
            &["com.jetbrains.rustrover"],
            &["RustRover"],
            &["rustrover"],
        ),
        "clion" => open_macos_or_cli(&path, &["com.jetbrains.CLion"], &["CLion"], &["clion"]),
        "rider" => open_macos_or_cli(&path, &["com.jetbrains.rider"], &["Rider"], &["rider"]),
        "phpstorm" => open_macos_or_cli(
            &path,
            &["com.jetbrains.PhpStorm"],
            &["PhpStorm"],
            &["phpstorm"],
        ),
        "rubymine" => open_macos_or_cli(
            &path,
            &["com.jetbrains.rubymine"],
            &["RubyMine"],
            &["rubymine"],
        ),
        "datagrip" => open_macos_or_cli(
            &path,
            &["com.jetbrains.datagrip"],
            &["DataGrip"],
            &["datagrip"],
        ),
        "dataspell" => open_macos_or_cli(
            &path,
            &["com.jetbrains.dataspell"],
            &["DataSpell"],
            &["dataspell"],
        ),
        "android-studio" => open_macos_or_cli(
            &path,
            &["com.google.android.studio"],
            &["Android Studio"],
            &["studio"],
        ),
        "visual-studio" => open_macos_or_cli(&path, &[], &[], &["devenv"]),
        "notepad-plus-plus" => open_macos_or_cli(&path, &[], &[], &["notepad++"]),
        "iterm" => open_macos_or_cli(&path, &["com.googlecode.iterm2"], &["iTerm", "iTerm2"], &[]),
        "warp" => open_macos_or_cli(&path, &["dev.warp.Warp-Stable"], &["Warp"], &["warp"]),
        "wezterm" => open_macos_or_cli(
            &path,
            &["com.github.wez.wezterm"],
            &["WezTerm"],
            &["wezterm"],
        ),
        "alacritty" => open_macos_or_cli(&path, &["org.alacritty"], &["Alacritty"], &["alacritty"]),
        "windows-terminal" => open_windows_terminal(&path),
        "powershell" => open_powershell(&path),
        "finder" | "explorer" => open_default(&path),
        "terminal" => open_terminal(&path),
        "editor" => open_default(&path),
        other => Err(format!("Unsupported workspace tool: {}", other)),
    }
}

#[tauri::command]
pub async fn detect_workspace_tools() -> Result<Vec<WorkspaceTool>, String> {
    Ok(detect_known_tools())
}

fn run(mut command: Command) -> Result<(), String> {
    let status = command
        .hide_console()
        .status()
        .map_err(|e| format!("Failed to launch tool: {}", e))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("Tool exited with status: {}", status))
    }
}

fn available_tool(
    id: &str,
    name: &str,
    app_names: &[&str],
    cli_bins: &[&str],
    builtin: bool,
) -> Option<WorkspaceTool> {
    if builtin {
        return Some(make_tool(id, name, None));
    }

    for app_name in app_names {
        if let Some(path) = find_macos_app(app_name) {
            return Some(make_tool(id, name, Some(path)));
        }
    }

    for bin in cli_bins {
        if let Some(path) = which_binary(bin) {
            return Some(make_tool(id, name, Some(PathBuf::from(path))));
        }
    }

    None
}

fn detect_known_tools() -> Vec<WorkspaceTool> {
    #[cfg(target_os = "windows")]
    {
        dedupe_tools(detect_windows_tools())
    }

    #[cfg(not(target_os = "windows"))]
    {
        let candidates: &[ToolCandidate] = &[
            (
                "vscode",
                "VS Code",
                &["Visual Studio Code.app"],
                &["code"],
                false,
            ),
            ("cursor", "Cursor", &["Cursor.app"], &["cursor"], false),
            (
                "windsurf",
                "Windsurf",
                &["Windsurf.app"],
                &["windsurf"],
                false,
            ),
            ("zed", "Zed", &["Zed.app"], &["zed"], false),
            (
                "sublime",
                "Sublime Text",
                &["Sublime Text.app"],
                &["subl"],
                false,
            ),
            ("trae", "Trae", &["Trae.app"], &["trae"], false),
            ("xcode", "Xcode", &["Xcode.app"], &[], false),
            (
                "intellij",
                "IntelliJ IDEA",
                &[
                    "IntelliJ IDEA.app",
                    "IntelliJ IDEA Ultimate.app",
                    "IntelliJ IDEA CE.app",
                ],
                &["idea"],
                false,
            ),
            (
                "webstorm",
                "WebStorm",
                &["WebStorm.app"],
                &["webstorm"],
                false,
            ),
            (
                "pycharm",
                "PyCharm",
                &["PyCharm.app", "PyCharm Professional.app", "PyCharm CE.app"],
                &["pycharm"],
                false,
            ),
            ("goland", "GoLand", &["GoLand.app"], &["goland"], false),
            (
                "rustrover",
                "RustRover",
                &["RustRover.app"],
                &["rustrover"],
                false,
            ),
            ("clion", "CLion", &["CLion.app"], &["clion"], false),
            ("rider", "Rider", &["Rider.app"], &["rider"], false),
            (
                "phpstorm",
                "PhpStorm",
                &["PhpStorm.app"],
                &["phpstorm"],
                false,
            ),
            (
                "rubymine",
                "RubyMine",
                &["RubyMine.app"],
                &["rubymine"],
                false,
            ),
            (
                "datagrip",
                "DataGrip",
                &["DataGrip.app"],
                &["datagrip"],
                false,
            ),
            (
                "dataspell",
                "DataSpell",
                &["DataSpell.app"],
                &["dataspell"],
                false,
            ),
            (
                "android-studio",
                "Android Studio",
                &["Android Studio.app"],
                &["studio"],
                false,
            ),
            ("iterm", "iTerm", &["iTerm.app", "iTerm2.app"], &[], false),
            ("warp", "Warp", &["Warp.app"], &["warp"], false),
            ("wezterm", "WezTerm", &["WezTerm.app"], &["wezterm"], false),
            (
                "alacritty",
                "Alacritty",
                &["Alacritty.app"],
                &["alacritty"],
                false,
            ),
            ("finder", "Finder", &["Finder.app"], &[], false),
            ("editor", "Default Editor", &[], &[], true),
            ("terminal", "Terminal", &[], &[], true),
        ];

        dedupe_tools(
            candidates
                .iter()
                .filter_map(|(id, name, app_names, cli_bins, builtin)| {
                    available_tool(id, name, app_names, cli_bins, *builtin)
                })
                .collect(),
        )
    }
}

fn dedupe_tools(tools: Vec<WorkspaceTool>) -> Vec<WorkspaceTool> {
    let mut seen = HashSet::new();
    tools
        .into_iter()
        .filter(|tool| seen.insert(tool.id.clone()))
        .collect()
}

fn which_binary(bin: &str) -> Option<String> {
    #[cfg(target_os = "windows")]
    {
        return find_binary_on_windows_path(bin);
    }

    #[cfg(not(target_os = "windows"))]
    let output = Command::new("which").arg(bin).output().ok()?;

    if !output.status.success() {
        return None;
    }
    let path = String::from_utf8_lossy(&output.stdout)
        .lines()
        .next()
        .unwrap_or("")
        .trim()
        .to_string();
    if path.is_empty() {
        None
    } else {
        Some(path)
    }
}

#[cfg(target_os = "windows")]
fn find_binary_on_windows_path(bin: &str) -> Option<String> {
    let bin_path = Path::new(bin);
    if bin_path.is_absolute() || bin.contains('\\') || bin.contains('/') {
        return bin_path
            .is_file()
            .then(|| bin_path.to_string_lossy().to_string());
    }

    let has_extension = bin_path.extension().is_some();
    let extensions: Vec<String> = if has_extension {
        vec![String::new()]
    } else {
        std::env::var_os("PATHEXT")
            .map(|value| {
                value
                    .to_string_lossy()
                    .split(';')
                    .filter_map(|ext| {
                        let ext = ext.trim();
                        if ext.is_empty() {
                            None
                        } else if ext.starts_with('.') {
                            Some(ext.to_string())
                        } else {
                            Some(format!(".{ext}"))
                        }
                    })
                    .collect()
            })
            .filter(|exts: &Vec<String>| !exts.is_empty())
            .unwrap_or_else(|| {
                vec![
                    ".COM".to_string(),
                    ".EXE".to_string(),
                    ".BAT".to_string(),
                    ".CMD".to_string(),
                ]
            })
    };

    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        if has_extension {
            let candidate = dir.join(bin);
            if candidate.is_file() {
                return Some(candidate.to_string_lossy().to_string());
            }
            continue;
        }

        for ext in &extensions {
            let candidate = dir.join(format!("{bin}{ext}"));
            if candidate.is_file() {
                return Some(candidate.to_string_lossy().to_string());
            }
        }
    }

    None
}

#[cfg(target_os = "macos")]
fn find_macos_app(app_name: &str) -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let mut dirs = vec![
        PathBuf::from("/Applications"),
        PathBuf::from("/Applications/Utilities"),
        PathBuf::from("/System/Applications"),
        PathBuf::from("/System/Applications/Utilities"),
        PathBuf::from("/System/Library/CoreServices"),
        PathBuf::from("/Applications/JetBrains Toolbox"),
    ];
    if let Some(home) = home {
        dirs.push(home.join("Applications"));
        dirs.push(home.join("Applications/JetBrains Toolbox"));
    }

    for dir in dirs {
        let candidate = dir.join(app_name);
        if candidate.exists() {
            return Some(candidate);
        }
    }
    None
}

#[cfg(not(target_os = "macos"))]
fn find_macos_app(_app_name: &str) -> Option<PathBuf> {
    None
}

fn make_tool(id: &str, name: &str, source: Option<PathBuf>) -> WorkspaceTool {
    let icon_data_url = source.as_deref().and_then(icon_data_url_for_source);
    WorkspaceTool {
        id: id.to_string(),
        name: name.to_string(),
        available: true,
        source: source.map(|p| p.to_string_lossy().to_string()),
        icon_data_url,
    }
}

fn icon_data_url_for_source(source: &Path) -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        if source.extension().and_then(|s| s.to_str()) == Some("app") {
            return icon_data_url_for_macos_app(source);
        }
    }

    #[cfg(target_os = "windows")]
    {
        if source
            .extension()
            .and_then(|s| s.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("ico"))
        {
            return icon_data_url_for_windows_source(source);
        }
    }

    None
}

fn cached_icon_data_url(key: &str, render: impl FnOnce(&Path) -> bool) -> Option<String> {
    let mut hasher = Sha256::new();
    hasher.update(key.as_bytes());
    let hash = format!("{:x}", hasher.finalize());
    let cache_dir = std::env::temp_dir().join("helioncoder-workspace-icons");
    fs::create_dir_all(&cache_dir).ok()?;
    let out = cache_dir.join(format!("{}.png", &hash[..16]));

    if !out.exists() && !render(&out) {
        return None;
    }

    let bytes = fs::read(out).ok()?;
    Some(format!(
        "data:image/png;base64,{}",
        general_purpose::STANDARD.encode(bytes)
    ))
}

#[cfg(target_os = "macos")]
fn icon_data_url_for_macos_app(app_path: &Path) -> Option<String> {
    let icon_path = find_macos_app_icon(app_path)?;
    cached_icon_data_url(&icon_path.to_string_lossy(), |out| {
        Command::new("sips")
            .args(["-Z", "64", "-s", "format", "png"])
            .arg(&icon_path)
            .arg("--out")
            .arg(out)
            .status()
            .ok()
            .is_some_and(|status| status.success())
    })
}

#[cfg(not(target_os = "macos"))]
fn icon_data_url_for_macos_app(_app_path: &Path) -> Option<String> {
    None
}

#[cfg(target_os = "macos")]
fn find_macos_app_icon(app_path: &Path) -> Option<PathBuf> {
    let resources = app_path.join("Contents").join("Resources");
    if let Some(icon_name) = macos_bundle_icon_name(app_path) {
        let icon_path = if Path::new(&icon_name).extension().is_some() {
            resources.join(icon_name)
        } else {
            resources.join(format!("{}.icns", icon_name))
        };
        if icon_path.exists() {
            return Some(icon_path);
        }
    }

    let entries = fs::read_dir(&resources).ok()?;
    let app_stem = app_path.file_stem().and_then(|s| s.to_str());
    let mut icons: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|s| s.to_str()) == Some("icns"))
        .collect();
    icons.sort();

    if let Some(app_stem) = app_stem {
        if let Some(exact) = icons.iter().find(|path| {
            path.file_stem()
                .and_then(|s| s.to_str())
                .is_some_and(|stem| stem.eq_ignore_ascii_case(app_stem))
        }) {
            return Some(exact.clone());
        }
    }

    icons.into_iter().next()
}

#[cfg(target_os = "macos")]
fn macos_bundle_icon_name(app_path: &Path) -> Option<String> {
    let plist = app_path.join("Contents").join("Info.plist");
    let output = Command::new("plutil")
        .args(["-extract", "CFBundleIconFile", "raw", "-o", "-"])
        .arg(plist)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let icon_name = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if icon_name.is_empty() {
        None
    } else {
        Some(icon_name)
    }
}

#[cfg(target_os = "windows")]
fn icon_data_url_for_windows_source(source: &Path) -> Option<String> {
    let bytes = fs::read(source).ok()?;
    Some(format!(
        "data:image/x-icon;base64,{}",
        general_purpose::STANDARD.encode(bytes)
    ))
}

fn open_source_launcher(source: &str, path: &Path) -> Result<(), String> {
    let source_path = PathBuf::from(source);

    #[cfg(target_os = "macos")]
    {
        if source_path.extension().and_then(|s| s.to_str()) == Some("app") {
            let mut cmd = Command::new("open");
            cmd.arg("-a").arg(&source_path).arg(path);
            return run(cmd);
        }
    }

    #[cfg(target_os = "windows")]
    {
        if source_path
            .extension()
            .and_then(|s| s.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("bat") || ext.eq_ignore_ascii_case("cmd"))
        {
            let mut cmd = Command::new("cmd");
            cmd.arg("/C").arg(source_path).arg(path);
            return run(cmd);
        }
    }

    let mut cmd = Command::new(source);
    cmd.arg(path);
    run(cmd)
}

#[cfg(target_os = "windows")]
struct WindowsToolCandidate {
    id: &'static str,
    name: &'static str,
    exe_names: &'static [&'static str],
    relative_paths: &'static [(&'static str, &'static str)],
    scan_roots: &'static [(&'static str, &'static str, u8)],
}

#[cfg(target_os = "windows")]
fn detect_windows_tools() -> Vec<WorkspaceTool> {
    let candidates = [
        WindowsToolCandidate {
            id: "vscode",
            name: "VS Code",
            exe_names: &["Code.exe"],
            relative_paths: &[
                ("LOCALAPPDATA", r"Programs\Microsoft VS Code\Code.exe"),
                ("ProgramFiles", r"Microsoft VS Code\Code.exe"),
                ("ProgramFiles(x86)", r"Microsoft VS Code\Code.exe"),
            ],
            scan_roots: &[],
        },
        WindowsToolCandidate {
            id: "vscode-insiders",
            name: "VS Code Insiders",
            exe_names: &["Code - Insiders.exe"],
            relative_paths: &[
                (
                    "LOCALAPPDATA",
                    r"Programs\Microsoft VS Code Insiders\Code - Insiders.exe",
                ),
                (
                    "ProgramFiles",
                    r"Microsoft VS Code Insiders\Code - Insiders.exe",
                ),
            ],
            scan_roots: &[],
        },
        WindowsToolCandidate {
            id: "cursor",
            name: "Cursor",
            exe_names: &["Cursor.exe"],
            relative_paths: &[
                ("LOCALAPPDATA", r"Programs\Cursor\Cursor.exe"),
                ("ProgramFiles", r"Cursor\Cursor.exe"),
            ],
            scan_roots: &[],
        },
        WindowsToolCandidate {
            id: "windsurf",
            name: "Windsurf",
            exe_names: &["Windsurf.exe"],
            relative_paths: &[
                ("LOCALAPPDATA", r"Programs\Windsurf\Windsurf.exe"),
                ("ProgramFiles", r"Windsurf\Windsurf.exe"),
            ],
            scan_roots: &[],
        },
        WindowsToolCandidate {
            id: "zed",
            name: "Zed",
            exe_names: &["Zed.exe"],
            relative_paths: &[
                ("LOCALAPPDATA", r"Programs\Zed\Zed.exe"),
                ("ProgramFiles", r"Zed\Zed.exe"),
            ],
            scan_roots: &[],
        },
        WindowsToolCandidate {
            id: "sublime",
            name: "Sublime Text",
            exe_names: &["sublime_text.exe"],
            relative_paths: &[
                ("ProgramFiles", r"Sublime Text\sublime_text.exe"),
                ("ProgramFiles", r"Sublime Text 4\sublime_text.exe"),
                ("ProgramFiles", r"Sublime Text 3\sublime_text.exe"),
                ("ProgramFiles(x86)", r"Sublime Text\sublime_text.exe"),
            ],
            scan_roots: &[],
        },
        WindowsToolCandidate {
            id: "trae",
            name: "Trae",
            exe_names: &["Trae.exe"],
            relative_paths: &[("LOCALAPPDATA", r"Programs\Trae\Trae.exe")],
            scan_roots: &[],
        },
        WindowsToolCandidate {
            id: "visual-studio",
            name: "Visual Studio",
            exe_names: &["devenv.exe"],
            relative_paths: &[],
            scan_roots: &[("ProgramFiles", r"Microsoft Visual Studio", 6)],
        },
        WindowsToolCandidate {
            id: "intellij",
            name: "IntelliJ IDEA",
            exe_names: &["idea64.exe", "idea.exe"],
            relative_paths: &[],
            scan_roots: &[
                ("ProgramFiles", r"JetBrains", 4),
                ("LOCALAPPDATA", r"JetBrains\Toolbox\apps", 8),
            ],
        },
        WindowsToolCandidate {
            id: "webstorm",
            name: "WebStorm",
            exe_names: &["webstorm64.exe", "webstorm.exe"],
            relative_paths: &[],
            scan_roots: &[
                ("ProgramFiles", r"JetBrains", 4),
                ("LOCALAPPDATA", r"JetBrains\Toolbox\apps", 8),
            ],
        },
        WindowsToolCandidate {
            id: "pycharm",
            name: "PyCharm",
            exe_names: &["pycharm64.exe", "pycharm.exe"],
            relative_paths: &[],
            scan_roots: &[
                ("ProgramFiles", r"JetBrains", 4),
                ("LOCALAPPDATA", r"JetBrains\Toolbox\apps", 8),
            ],
        },
        WindowsToolCandidate {
            id: "goland",
            name: "GoLand",
            exe_names: &["goland64.exe", "goland.exe"],
            relative_paths: &[],
            scan_roots: &[
                ("ProgramFiles", r"JetBrains", 4),
                ("LOCALAPPDATA", r"JetBrains\Toolbox\apps", 8),
            ],
        },
        WindowsToolCandidate {
            id: "rustrover",
            name: "RustRover",
            exe_names: &["rustrover64.exe", "rustrover.exe"],
            relative_paths: &[],
            scan_roots: &[
                ("ProgramFiles", r"JetBrains", 4),
                ("LOCALAPPDATA", r"JetBrains\Toolbox\apps", 8),
            ],
        },
        WindowsToolCandidate {
            id: "clion",
            name: "CLion",
            exe_names: &["clion64.exe", "clion.exe"],
            relative_paths: &[],
            scan_roots: &[
                ("ProgramFiles", r"JetBrains", 4),
                ("LOCALAPPDATA", r"JetBrains\Toolbox\apps", 8),
            ],
        },
        WindowsToolCandidate {
            id: "rider",
            name: "Rider",
            exe_names: &["rider64.exe", "rider.exe"],
            relative_paths: &[],
            scan_roots: &[
                ("ProgramFiles", r"JetBrains", 4),
                ("LOCALAPPDATA", r"JetBrains\Toolbox\apps", 8),
            ],
        },
        WindowsToolCandidate {
            id: "phpstorm",
            name: "PhpStorm",
            exe_names: &["phpstorm64.exe", "phpstorm.exe"],
            relative_paths: &[],
            scan_roots: &[
                ("ProgramFiles", r"JetBrains", 4),
                ("LOCALAPPDATA", r"JetBrains\Toolbox\apps", 8),
            ],
        },
        WindowsToolCandidate {
            id: "rubymine",
            name: "RubyMine",
            exe_names: &["rubymine64.exe", "rubymine.exe"],
            relative_paths: &[],
            scan_roots: &[
                ("ProgramFiles", r"JetBrains", 4),
                ("LOCALAPPDATA", r"JetBrains\Toolbox\apps", 8),
            ],
        },
        WindowsToolCandidate {
            id: "datagrip",
            name: "DataGrip",
            exe_names: &["datagrip64.exe", "datagrip.exe"],
            relative_paths: &[],
            scan_roots: &[
                ("ProgramFiles", r"JetBrains", 4),
                ("LOCALAPPDATA", r"JetBrains\Toolbox\apps", 8),
            ],
        },
        WindowsToolCandidate {
            id: "dataspell",
            name: "DataSpell",
            exe_names: &["dataspell64.exe", "dataspell.exe"],
            relative_paths: &[],
            scan_roots: &[
                ("ProgramFiles", r"JetBrains", 4),
                ("LOCALAPPDATA", r"JetBrains\Toolbox\apps", 8),
            ],
        },
        WindowsToolCandidate {
            id: "android-studio",
            name: "Android Studio",
            exe_names: &["studio64.exe", "studio.exe"],
            relative_paths: &[("ProgramFiles", r"Android\Android Studio\bin\studio64.exe")],
            scan_roots: &[],
        },
        WindowsToolCandidate {
            id: "notepad-plus-plus",
            name: "Notepad++",
            exe_names: &["notepad++.exe"],
            relative_paths: &[
                ("ProgramFiles", r"Notepad++\notepad++.exe"),
                ("ProgramFiles(x86)", r"Notepad++\notepad++.exe"),
            ],
            scan_roots: &[],
        },
    ];

    let mut tools: Vec<WorkspaceTool> = candidates
        .iter()
        .filter_map(|candidate| detect_windows_tool(candidate))
        .collect();

    if let Some(path) = which_binary("wt.exe").map(PathBuf::from) {
        tools.push(make_tool(
            "windows-terminal",
            "Windows Terminal",
            Some(path),
        ));
    }
    if let Some(path) = which_binary("pwsh.exe")
        .or_else(|| which_binary("powershell.exe"))
        .map(PathBuf::from)
    {
        tools.push(make_tool("powershell", "PowerShell", Some(path)));
    }
    tools.push(make_tool("explorer", "Explorer", None));
    tools.push(make_tool("terminal", "Terminal", None));
    tools.push(make_tool("editor", "Default Editor", None));
    tools
}

#[cfg(target_os = "windows")]
fn detect_windows_tool(candidate: &WindowsToolCandidate) -> Option<WorkspaceTool> {
    for exe in candidate.exe_names {
        if let Some(path) = query_windows_app_path(exe) {
            return Some(make_tool(candidate.id, candidate.name, Some(path)));
        }
    }

    for (env_key, relative) in candidate.relative_paths {
        if let Some(root) = std::env::var_os(env_key) {
            let path = PathBuf::from(root).join(relative);
            if path.exists() {
                return Some(make_tool(candidate.id, candidate.name, Some(path)));
            }
        }
    }

    for exe in candidate.exe_names {
        if let Some(path) = which_binary(exe).map(PathBuf::from) {
            return Some(make_tool(candidate.id, candidate.name, Some(path)));
        }
    }

    for (env_key, relative, depth) in candidate.scan_roots {
        if let Some(root) = std::env::var_os(env_key) {
            let root = PathBuf::from(root).join(relative);
            if let Some(path) = find_named_file(&root, candidate.exe_names, *depth) {
                return Some(make_tool(candidate.id, candidate.name, Some(path)));
            }
        }
    }

    None
}

#[cfg(target_os = "windows")]
fn query_windows_app_path(_exe_name: &str) -> Option<PathBuf> {
    // Avoid spawning reg.exe during startup. Some Windows installs show a
    // blocking application-error dialog when reg.exe is launched from a GUI app.
    // The remaining PATH, Program Files, and Toolbox scans cover the launchers
    // we expose without creating external helper processes.
    None
}

#[cfg(target_os = "windows")]
fn find_named_file(root: &Path, file_names: &[&str], max_depth: u8) -> Option<PathBuf> {
    if max_depth == 0 || !root.exists() {
        return None;
    }
    let entries = fs::read_dir(root).ok()?;
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if path.is_file() {
            if let Some(name) = path.file_name().and_then(|s| s.to_str()) {
                if file_names
                    .iter()
                    .any(|candidate| name.eq_ignore_ascii_case(candidate))
                {
                    return Some(path);
                }
            }
        } else if path.is_dir() {
            if let Some(found) = find_named_file(&path, file_names, max_depth - 1) {
                return Some(found);
            }
        }
    }
    None
}

fn open_macos_or_cli(
    path: &Path,
    bundle_ids: &[&str],
    app_names: &[&str],
    cli_bins: &[&str],
) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        for bundle_id in bundle_ids {
            let mut cmd = Command::new("open");
            cmd.args(["-b", bundle_id]).arg(path);
            if run(cmd).is_ok() {
                return Ok(());
            }
        }
        for app_name in app_names {
            let mut cmd = Command::new("open");
            cmd.args(["-a", app_name]).arg(path);
            if run(cmd).is_ok() {
                return Ok(());
            }
        }
    }

    for bin in cli_bins {
        let mut cmd = Command::new(bin);
        cmd.arg(path);
        if run(cmd).is_ok() {
            return Ok(());
        }
    }

    Err("No supported launcher found for this workspace tool".to_string())
}

fn open_vscode(path: &PathBuf) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let mut cmd = Command::new("open");
        cmd.args(["-b", "com.microsoft.VSCode"]).arg(path);
        if run(cmd).is_ok() {
            return Ok(());
        }
        let mut fallback = Command::new("open");
        fallback.args(["-a", "Visual Studio Code"]).arg(path);
        if run(fallback).is_ok() {
            return Ok(());
        }
        let mut cli = Command::new("code");
        cli.arg(path);
        run(cli)
    }

    #[cfg(target_os = "windows")]
    {
        let mut cmd = Command::new("code");
        cmd.arg(path);
        run(cmd)
    }

    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    {
        let mut cmd = Command::new("code");
        cmd.arg(path);
        run(cmd)
    }
}

fn open_windows_terminal(path: &PathBuf) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        let mut cmd = Command::new("wt");
        cmd.args(["-d"]).arg(path);
        run(cmd)
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = path;
        Err("Windows Terminal is only available on Windows".to_string())
    }
}

fn open_powershell(path: &PathBuf) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        let escaped = path.to_string_lossy().replace('\'', "''");
        for shell in ["pwsh", "powershell"] {
            let mut cmd = Command::new(shell);
            cmd.args([
                "-NoExit",
                "-Command",
                &format!("Set-Location -LiteralPath '{}'", escaped),
            ]);
            if run(cmd).is_ok() {
                return Ok(());
            }
        }
        Err("No supported PowerShell launcher found".to_string())
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = path;
        Err("PowerShell workspace launcher is only available on Windows".to_string())
    }
}

fn open_terminal(path: &PathBuf) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let mut cmd = Command::new("open");
        cmd.args(["-a", "Terminal"]).arg(path);
        run(cmd)
    }

    #[cfg(target_os = "windows")]
    {
        let mut cmd = Command::new("cmd");
        cmd.args(["/C", "start", "cmd", "/K", "cd", "/d"]).arg(path);
        run(cmd)
    }

    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    {
        let candidates = [
            "x-terminal-emulator",
            "gnome-terminal",
            "konsole",
            "xfce4-terminal",
        ];
        for candidate in candidates {
            let mut cmd = Command::new(candidate);
            cmd.current_dir(path);
            if run(cmd).is_ok() {
                return Ok(());
            }
        }
        Err("No supported terminal launcher found".to_string())
    }
}

fn open_default(path: &PathBuf) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let mut cmd = Command::new("open");
        cmd.arg(path);
        run(cmd)
    }

    #[cfg(target_os = "windows")]
    {
        let mut cmd = Command::new("cmd");
        cmd.args(["/C", "start", ""]).arg(path);
        run(cmd)
    }

    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    {
        let mut cmd = Command::new("xdg-open");
        cmd.arg(path);
        run(cmd)
    }
}
