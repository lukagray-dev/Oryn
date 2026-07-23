// =============================================================================
// Workspace & Document Management Backend (`src/workspace/mod.rs`)
// =============================================================================
// This module implements all Rust backend business logic for document handling,
// filesystem scanning, project folder management, real-time filesystem watching,
// search query filtering, item renaming, file moving, item pinning, and configuration persistence.
//
// Key Responsibilities:
// 1. General Documents Storage: Default directory at `~/.oryn/documents/`.
// 2. Open Projects Persistence: Stores open project folder paths & pinned states in `~/.oryn/projects.json`.
// 3. Real-time Auto Reloading: Watches watched folders via `notify` crate for external filesystem events.
// 4. Document Operations: Renaming, moving via `rfd` dialogs, pinning, and deleting.
// 5. Slint UI Synchronization: Populates `sidebar_chats` and `sidebar_projects` models.

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use rfd::{MessageButtons, MessageDialog, MessageDialogResult, MessageLevel};
use serde::{Deserialize, Serialize};
use slint::{ComponentHandle, Model, ModelRc, VecModel};
use std::cell::RefCell;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc::channel;
use std::sync::Mutex;
use std::thread;

use crate::AppWindow;
use crate::{SidebarConversation, SidebarProject};

// Global handle for real-time filesystem watcher
struct GlobalWatcher {
    _watcher: RecommendedWatcher,
}
static WATCHER: Mutex<Option<GlobalWatcher>> = Mutex::new(None);

// Initial sample markdown content for first-time users
const SAMPLE_MARKDOWN_CONTENT: &str = r#"# Heading Level 1

This is a demonstration paragraph under Heading 1 rendered using the Literata serif font family with regular weight.

## Heading Level 2

Oryn parses CommonMark and GitHub-Flavored Markdown AST tokens natively in Rust.

### Heading Level 3

Heading 3 renders at 20px font size with comfortable vertical paragraph padding.

#### Heading Level 4

Heading 4 renders at 16px font size.

##### Heading Level 5

Heading 5 renders at 14px font size.

<h6>Heading Level 6</h6>

Heading 6 renders at 13.6px with muted text color matching GitHub's specification.
"#;

// =============================================================================
// Data Structures for Persistence
// =============================================================================
/// Persistent application configuration stored at `~/.oryn/projects.json`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WorkspaceConfig {
    /// List of absolute directory paths for open user projects
    pub open_projects: Vec<String>,
    /// List of absolute directory paths for pinned user projects
    #[serde(default)]
    pub pinned_projects: Vec<String>,
    /// List of absolute file paths for pinned documents
    #[serde(default)]
    pub pinned_documents: Vec<String>,
}

// =============================================================================
// Workspace State Manager
// =============================================================================
/// Main workspace state tracker held in thread-local context.
pub struct WorkspaceManager {
    /// Currently opened file path (if any)
    pub active_file_path: Option<PathBuf>,
    /// Active search filter query
    pub search_query: String,
    /// Saved workspace config
    pub config: WorkspaceConfig,
}

impl WorkspaceManager {
    /// Initializes workspace state and loads saved project configuration from disk.
    pub fn new() -> Self {
        let config = Self::load_config().unwrap_or_default();
        Self {
            active_file_path: None,
            search_query: String::new(),
            config,
        }
    }

    /// Returns default general documents directory path: `~/.oryn/documents/`.
    pub fn general_documents_dir() -> PathBuf {
        if let Some(home) = dirs::home_dir() {
            home.join(".oryn").join("documents")
        } else {
            PathBuf::from(".oryn_documents")
        }
    }

    /// Returns path to persistent configuration file: `~/.oryn/projects.json`.
    pub fn config_path() -> PathBuf {
        if let Some(home) = dirs::home_dir() {
            home.join(".oryn").join("projects.json")
        } else {
            PathBuf::from(".oryn_projects.json")
        }
    }

    /// Loads configuration file from disk.
    pub fn load_config() -> Option<WorkspaceConfig> {
        let path = Self::config_path();
        if path.exists() {
            let data = fs::read_to_string(path).ok()?;
            serde_json::from_str(&data).ok()
        } else {
            None
        }
    }

    /// Saves configuration file to disk.
    pub fn save_config(&self) {
        let path = Self::config_path();
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Ok(data) = serde_json::to_string_pretty(&self.config) {
            let _ = fs::write(path, data);
        }
    }

    /// Ensures general documents directory exists on filesystem and populates sample note if empty.
    pub fn ensure_general_docs_dir() -> PathBuf {
        let dir = Self::general_documents_dir();
        if !dir.exists() {
            let _ = fs::create_dir_all(&dir);
        }

        // If documents directory is empty, create an initial Welcome sample document
        if let Ok(mut entries) = fs::read_dir(&dir) {
            if entries.next().is_none() {
                let sample_file = dir.join("Welcome.md");
                let _ = fs::write(sample_file, SAMPLE_MARKDOWN_CONTENT);
            }
        }
        dir
    }

    /// Scans a directory for all `.md` markdown files and flags pinned items.
    pub fn scan_markdown_files(dir: &Path, pinned_docs: &[String]) -> Vec<SidebarConversation> {
        let mut items = Vec::new();
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() {
                    if let Some(ext) = path.extension() {
                        if ext.eq_ignore_ascii_case("md") {
                            let title = path
                                .file_stem()
                                .and_then(|s| s.to_str())
                                .unwrap_or("Untitled")
                                .to_string();
                            let full_path = path.to_string_lossy().to_string();
                            let is_pinned = pinned_docs.contains(&full_path);

                            items.push(SidebarConversation {
                                id: full_path.into(),
                                title: title.into(),
                                is_pinned,
                            });
                        }
                    }
                }
            }
        }

        // Sort items: pinned documents first, then alphabetically by title
        items.sort_by(|a, b| match (a.is_pinned, b.is_pinned) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.title.to_lowercase().cmp(&b.title.to_lowercase()),
        });
        items
    }
}

// Global thread-local reference for workspace state
thread_local! {
    static WORKSPACE: RefCell<WorkspaceManager> = RefCell::new(WorkspaceManager::new());
}

// =============================================================================
// Helper Utility Functions for UI Integration
// =============================================================================

/// Refreshes the Slint UI left-sidebar models for general documents and open projects.
pub fn refresh_sidebar_ui(ui: &AppWindow) {
    let (search_query, pinned_projects, pinned_documents) = WORKSPACE.with(|mgr| {
        let mgr = mgr.borrow();
        (
            mgr.search_query.to_lowercase(),
            mgr.config.pinned_projects.clone(),
            mgr.config.pinned_documents.clone(),
        )
    });

    let general_dir = WorkspaceManager::ensure_general_docs_dir();
    let mut general_docs = WorkspaceManager::scan_markdown_files(&general_dir, &pinned_documents);

    // Apply live search filter to general documents if query is active
    if !search_query.is_empty() {
        general_docs.retain(|doc| doc.title.to_lowercase().contains(&search_query));
    }

    // Populate general documents model
    let general_model = Rc::new(VecModel::from(general_docs));
    ui.set_sidebar_chats(ModelRc::from(general_model));

    // Populate projects model
    let mut projects_list: Vec<SidebarProject> = WORKSPACE.with(|mgr| {
        let mgr = mgr.borrow();
        mgr.config
            .open_projects
            .iter()
            .map(|proj_path_str| {
                let proj_path = Path::new(proj_path_str);
                let proj_name = proj_path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(proj_path_str)
                    .to_string();

                let is_pinned = pinned_projects.contains(proj_path_str);
                let mut conversations =
                    WorkspaceManager::scan_markdown_files(proj_path, &pinned_documents);

                if !search_query.is_empty() {
                    conversations.retain(|doc| doc.title.to_lowercase().contains(&search_query));
                }

                let conv_model = Rc::new(VecModel::from(conversations));

                SidebarProject {
                    name: proj_name.into(),
                    workspace: proj_path_str.as_str().into(),
                    is_pinned,
                    conversations: ModelRc::from(conv_model),
                }
            })
            .collect()
    });

    // Apply live search filter to projects: keep project if project name matches query OR if it has matching documents
    if !search_query.is_empty() {
        projects_list.retain(|proj| {
            proj.name.to_lowercase().contains(&search_query) || proj.conversations.row_count() > 0
        });
    }

    // Sort projects: pinned projects first, then alphabetically by name
    projects_list.sort_by(|a, b| match (a.is_pinned, b.is_pinned) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
    });

    let projects_model = Rc::new(VecModel::from(projects_list));
    ui.set_sidebar_projects(ModelRc::from(projects_model));
}

/// Reads the document at `file_path` from disk and loads it into the editor canvas.
pub fn load_document_into_editor(ui: &AppWindow, file_path_str: &str) {
    let path = Path::new(file_path_str);
    if path.exists() && path.is_file() {
        if let Ok(content) = fs::read_to_string(path) {
            ui.set_editor_text(content.as_str().into());

            // Parse Markdown AST elements and update Slint renderer model
            let parsed_elements = crate::markdown::parse_markdown(&content);
            let model = Rc::new(VecModel::from(parsed_elements));
            ui.set_markdown_elements(ModelRc::from(model));

            WORKSPACE.with(|mgr| {
                mgr.borrow_mut().active_file_path = Some(path.to_path_buf());
            });
        }
    }
}

/// Saves current editor content to the active document on disk.
pub fn save_active_document(ui: &AppWindow) {
    let active_path = WORKSPACE.with(|mgr| mgr.borrow().active_file_path.clone());
    if let Some(path) = active_path {
        let content = ui.get_editor_text().to_string();
        let _ = fs::write(path, content);
    }
}

/// Generates a unique document filename (e.g., `Untitled Document.md`, `Untitled Document 1.md`) in `dir`.
fn generate_unique_doc_path(dir: &Path, prefix: &str) -> PathBuf {
    let mut candidate = dir.join(format!("{}.md", prefix));
    let mut counter = 1;
    while candidate.exists() {
        candidate = dir.join(format!("{} {}.md", prefix, counter));
        counter += 1;
    }
    candidate
}

/// Creates a new general document inside `~/.oryn/documents/`, refreshes UI, and opens it.
pub fn create_new_general_document(ui: &AppWindow) {
    let dir = WorkspaceManager::ensure_general_docs_dir();
    let file_path = generate_unique_doc_path(&dir, "Untitled Document");

    let _ = fs::write(&file_path, "");
    refresh_sidebar_ui(ui);

    let path_str = file_path.to_string_lossy().to_string();
    load_document_into_editor(ui, &path_str);

    ui.set_active_project_index(-1);
    ui.set_active_conversation_index(-1);

    let general_docs =
        WorkspaceManager::scan_markdown_files(&dir, &WORKSPACE.with(|m| m.borrow().config.pinned_documents.clone()));
    if let Some(idx) = general_docs.iter().position(|doc| doc.id == path_str) {
        ui.set_active_chat_index(idx as i32);
    }
}

/// Creates a new project-specific document inside `proj_path`, refreshes UI, and opens it.
pub fn create_new_project_document(ui: &AppWindow, proj_path_str: &str, proj_idx: i32) {
    let dir = Path::new(proj_path_str);
    if !dir.exists() {
        return;
    }

    let file_path = generate_unique_doc_path(dir, "Untitled Document");
    let _ = fs::write(&file_path, "");

    refresh_sidebar_ui(ui);

    let path_str = file_path.to_string_lossy().to_string();
    load_document_into_editor(ui, &path_str);

    ui.set_active_chat_index(-1);
    ui.set_active_project_index(proj_idx);

    let conversations = WorkspaceManager::scan_markdown_files(
        dir,
        &WORKSPACE.with(|m| m.borrow().config.pinned_documents.clone()),
    );
    if let Some(conv_idx) = conversations.iter().position(|doc| doc.id == path_str) {
        ui.set_active_conversation_index(conv_idx as i32);
    }
}

/// Launches native folder picker to select a project directory and adds it to open projects.
pub fn pick_and_open_project(ui: &AppWindow) {
    if let Some(path_buf) = rfd::FileDialog::new().pick_folder() {
        let path_str = path_buf.to_string_lossy().to_string();

        WORKSPACE.with(|mgr| {
            let mut mgr = mgr.borrow_mut();
            if !mgr.config.open_projects.contains(&path_str) {
                mgr.config.open_projects.push(path_str.clone());
                mgr.save_config();
            }
        });

        start_filesystem_watcher(ui);
        refresh_sidebar_ui(ui);

        WORKSPACE.with(|mgr| {
            let mgr = mgr.borrow();
            if let Some(idx) = mgr.config.open_projects.iter().position(|p| p == &path_str) {
                ui.set_active_project_index(idx as i32);
                ui.set_active_conversation_index(-1);
                ui.set_active_chat_index(-1);
            }
        });
    }
}

/// Renames a document or project item.
pub fn rename_item(ui: &AppWindow, target_path_str: &str, new_name: &str, target_type: &str) {
    let clean_name = new_name.trim();
    if clean_name.is_empty() {
        return;
    }

    let old_path = Path::new(target_path_str);
    if !old_path.exists() {
        return;
    }

    if target_type == "document" {
        let parent = old_path.parent().unwrap_or_else(|| Path::new("."));
        let mut new_filename = clean_name.to_string();
        if !new_filename.to_lowercase().ends_with(".md") {
            new_filename.push_str(".md");
        }
        let new_path = parent.join(new_filename);

        if old_path != new_path {
            if fs::rename(old_path, &new_path).is_ok() {
                let old_str = old_path.to_string_lossy().to_string();
                let new_str = new_path.to_string_lossy().to_string();

                WORKSPACE.with(|mgr| {
                    let mut mgr = mgr.borrow_mut();
                    if mgr.active_file_path.as_ref() == Some(&old_path.to_path_buf()) {
                        mgr.active_file_path = Some(new_path.clone());
                    }
                    if let Some(idx) = mgr.config.pinned_documents.iter().position(|p| p == &old_str) {
                        mgr.config.pinned_documents[idx] = new_str;
                        mgr.save_config();
                    }
                });
            }
        }
    } else if target_type == "project" {
        let parent = old_path.parent().unwrap_or_else(|| Path::new("."));
        let new_path = parent.join(clean_name);

        if old_path != new_path {
            if fs::rename(old_path, &new_path).is_ok() {
                let old_str = old_path.to_string_lossy().to_string();
                let new_str = new_path.to_string_lossy().to_string();

                WORKSPACE.with(|mgr| {
                    let mut mgr = mgr.borrow_mut();
                    if let Some(idx) = mgr.config.open_projects.iter().position(|p| p == &old_str) {
                        mgr.config.open_projects[idx] = new_str.clone();
                    }
                    if let Some(idx) = mgr.config.pinned_projects.iter().position(|p| p == &old_str) {
                        mgr.config.pinned_projects[idx] = new_str;
                    }
                    mgr.save_config();
                });
            }
        }
    }

    start_filesystem_watcher(ui);
    refresh_sidebar_ui(ui);
}

/// Moves a document to a different location selected by the user via native file dialog.
pub fn move_to_document(ui: &AppWindow, file_path_str: &str) {
    let old_path = Path::new(file_path_str);
    if !old_path.exists() {
        return;
    }

    let default_name = old_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("Document.md");

    let picked_dest = rfd::FileDialog::new()
        .set_file_name(default_name)
        .add_filter("Markdown Document (*.md)", &["md"])
        .save_file();

    if let Some(new_path) = picked_dest {
        if old_path != new_path {
            if fs::rename(old_path, &new_path).is_ok() {
                let old_str = old_path.to_string_lossy().to_string();
                let new_str = new_path.to_string_lossy().to_string();

                WORKSPACE.with(|mgr| {
                    let mut mgr = mgr.borrow_mut();
                    if mgr.active_file_path.as_ref() == Some(&old_path.to_path_buf()) {
                        mgr.active_file_path = Some(new_path.clone());
                    }
                    if let Some(idx) = mgr.config.pinned_documents.iter().position(|p| p == &old_str) {
                        mgr.config.pinned_documents[idx] = new_str;
                        mgr.save_config();
                    }
                });

                refresh_sidebar_ui(ui);
            }
        }
    }
}

/// Toggles pinned status for a document.
pub fn toggle_pin_document(ui: &AppWindow, file_path_str: &str) {
    WORKSPACE.with(|mgr| {
        let mut mgr = mgr.borrow_mut();
        if let Some(idx) = mgr.config.pinned_documents.iter().position(|p| p == file_path_str) {
            mgr.config.pinned_documents.remove(idx);
        } else {
            mgr.config.pinned_documents.push(file_path_str.to_string());
        }
        mgr.save_config();
    });
    refresh_sidebar_ui(ui);
}

/// Toggles pinned status for a project workspace.
pub fn toggle_pin_project(ui: &AppWindow, proj_path_str: &str) {
    WORKSPACE.with(|mgr| {
        let mut mgr = mgr.borrow_mut();
        if let Some(idx) = mgr.config.pinned_projects.iter().position(|p| p == proj_path_str) {
            mgr.config.pinned_projects.remove(idx);
        } else {
            mgr.config.pinned_projects.push(proj_path_str.to_string());
        }
        mgr.save_config();
    });
    refresh_sidebar_ui(ui);
}

/// Deletes a general document after confirmation.
pub fn delete_general_document(ui: &AppWindow, file_path_str: &str) {
    let confirmed = MessageDialog::new()
        .set_title("Delete Document")
        .set_description("Are you sure you want to delete this document? This action cannot be undone.")
        .set_level(MessageLevel::Warning)
        .set_buttons(MessageButtons::OkCancel)
        .show();

    if confirmed == MessageDialogResult::Ok {
        let path = Path::new(file_path_str);
        if path.exists() {
            let _ = fs::remove_file(path);
        }

        let is_active = WORKSPACE.with(|mgr| {
            mgr.borrow()
                .active_file_path
                .as_ref()
                .map_or(false, |p| p == path)
        });
        if is_active {
            ui.set_editor_text("".into());
            WORKSPACE.with(|mgr| mgr.borrow_mut().active_file_path = None);
        }

        refresh_sidebar_ui(ui);
    }
}

/// Removes a project from open projects after confirmation.
pub fn remove_project(ui: &AppWindow, proj_path_str: &str) {
    let proj_name = Path::new(proj_path_str)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(proj_path_str);

    let confirmed = MessageDialog::new()
        .set_title("Remove Project")
        .set_description(&format!("Are you sure you want to remove project \"{}\" from Oryn? (Files on disk will not be deleted).", proj_name))
        .set_level(MessageLevel::Warning)
        .set_buttons(MessageButtons::OkCancel)
        .show();

    if confirmed == MessageDialogResult::Ok {
        WORKSPACE.with(|mgr| {
            let mut mgr = mgr.borrow_mut();
            mgr.config.open_projects.retain(|p| p != proj_path_str);
            mgr.config.pinned_projects.retain(|p| p != proj_path_str);
            mgr.save_config();
        });

        ui.set_active_project_index(-1);
        ui.set_active_conversation_index(-1);

        start_filesystem_watcher(ui);
        refresh_sidebar_ui(ui);
    }
}

// =============================================================================
// Real-Time Filesystem Auto Reloading (notify crate integration)
// =============================================================================

/// Starts or updates the real-time background filesystem watcher monitoring document folders.
pub fn start_filesystem_watcher(ui: &AppWindow) {
    let ui_weak = ui.as_weak();

    let (tx, rx) = channel();

    let mut watcher = match RecommendedWatcher::new(
        move |res: Result<Event, notify::Error>| {
            if let Ok(event) = res {
                match event.kind {
                    EventKind::Create(_) | EventKind::Remove(_) | EventKind::Modify(_) => {
                        let _ = tx.send(());
                    }
                    _ => {}
                }
            }
        },
        notify::Config::default(),
    ) {
        Ok(w) => w,
        Err(_) => return,
    };

    // Watch general documents folder
    let general_dir = WorkspaceManager::ensure_general_docs_dir();
    let _ = watcher.watch(&general_dir, RecursiveMode::NonRecursive);

    // Watch all open project folders
    WORKSPACE.with(|mgr| {
        let mgr = mgr.borrow();
        for proj in &mgr.config.open_projects {
            let p = Path::new(proj);
            if p.exists() {
                let _ = watcher.watch(p, RecursiveMode::NonRecursive);
            }
        }
    });

    // Store active watcher handle in global state
    if let Ok(mut guard) = WATCHER.lock() {
        *guard = Some(GlobalWatcher { _watcher: watcher });
    }

    // Spawn background thread to handle filesystem event triggers cleanly on Slint UI loop
    thread::spawn(move || {
        while rx.recv().is_ok() {
            // Debounce rapid event streams slightly (100ms)
            thread::sleep(std::time::Duration::from_millis(100));
            while rx.try_recv().is_ok() {}

            let ui_weak_clone = ui_weak.clone();
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(ui) = ui_weak_clone.upgrade() {
                    refresh_sidebar_ui(&ui);
                }
            });
        }
    });
}

// =============================================================================
// Callback Wiring Registration
// =============================================================================
/// Wires all document, project, search, rename, move, pin, and sidebar callbacks for the given AppWindow instance.
pub fn setup_workspace_callbacks(ui: &AppWindow) {
    // Initial refresh of workspace UI models on app launch
    refresh_sidebar_ui(ui);

    // Start real-time filesystem watcher for automatic external file reloading
    start_filesystem_watcher(ui);

    // Auto-open first available document if present
    let general_dir = WorkspaceManager::ensure_general_docs_dir();
    let general_docs = WorkspaceManager::scan_markdown_files(
        &general_dir,
        &WORKSPACE.with(|m| m.borrow().config.pinned_documents.clone()),
    );
    if let Some(first_doc) = general_docs.first() {
        load_document_into_editor(ui, &first_doc.id);
        ui.set_active_chat_index(0);
    }

    // Wire Open Project callback
    let ui_handle = ui.as_weak();
    ui.on_open_project_requested(move || {
        if let Some(ui) = ui_handle.upgrade() {
            pick_and_open_project(&ui);
        }
    });

    // Wire New General Document callback
    let ui_handle = ui.as_weak();
    ui.on_new_document_requested(move || {
        if let Some(ui) = ui_handle.upgrade() {
            create_new_general_document(&ui);
        }
    });

    // Wire New Project Document callback
    let ui_handle = ui.as_weak();
    ui.on_new_project_document_requested(move |proj_path, proj_idx| {
        if let Some(ui) = ui_handle.upgrade() {
            create_new_project_document(&ui, &proj_path, proj_idx);
        }
    });

    // Wire Click General Document callback
    let ui_handle = ui.as_weak();
    ui.on_general_document_clicked(move |file_path, _chat_idx| {
        if let Some(ui) = ui_handle.upgrade() {
            load_document_into_editor(&ui, &file_path);
        }
    });

    // Wire Click Project Document callback
    let ui_handle = ui.as_weak();
    ui.on_project_document_clicked(move |file_path, _proj_idx, _conv_idx| {
        if let Some(ui) = ui_handle.upgrade() {
            load_document_into_editor(&ui, &file_path);
        }
    });

    // Wire Delete General Document callback
    let ui_handle = ui.as_weak();
    ui.on_delete_general_document_requested(move |file_path, _chat_idx| {
        if let Some(ui) = ui_handle.upgrade() {
            delete_general_document(&ui, &file_path);
        }
    });

    // Wire Delete Project Document callback
    let ui_handle = ui.as_weak();
    ui.on_delete_project_document_requested(move |file_path, _proj_idx, _conv_idx| {
        if let Some(ui) = ui_handle.upgrade() {
            delete_general_document(&ui, &file_path);
        }
    });

    // Wire Remove Project callback
    let ui_handle = ui.as_weak();
    ui.on_delete_project_requested(move |proj_path, _proj_idx| {
        if let Some(ui) = ui_handle.upgrade() {
            remove_project(&ui, &proj_path);
        }
    });

    // Wire Save Document callback
    let ui_handle = ui.as_weak();
    ui.on_save_file_requested(move || {
        if let Some(ui) = ui_handle.upgrade() {
            save_active_document(&ui);
        }
    });

    // Wire Live Search Query callback
    let ui_handle = ui.as_weak();
    ui.on_search_query_changed(move |query| {
        if let Some(ui) = ui_handle.upgrade() {
            WORKSPACE.with(|mgr| {
                mgr.borrow_mut().search_query = query.to_string();
            });
            refresh_sidebar_ui(&ui);
        }
    });

    // Wire Item Rename callback
    let ui_handle = ui.as_weak();
    ui.on_rename_requested(move |target_path, new_name, target_type| {
        if let Some(ui) = ui_handle.upgrade() {
            rename_item(&ui, &target_path, &new_name, &target_type);
        }
    });

    // Wire Move To Document callback
    let ui_handle = ui.as_weak();
    ui.on_move_to_requested(move |file_path| {
        if let Some(ui) = ui_handle.upgrade() {
            move_to_document(&ui, &file_path);
        }
    });

    // Wire Pin Document callback
    let ui_handle = ui.as_weak();
    ui.on_pin_document_requested(move |file_path| {
        if let Some(ui) = ui_handle.upgrade() {
            toggle_pin_document(&ui, &file_path);
        }
    });

    // Wire Pin Project callback
    let ui_handle = ui.as_weak();
    ui.on_pin_project_requested(move |proj_path| {
        if let Some(ui) = ui_handle.upgrade() {
            toggle_pin_project(&ui, &proj_path);
        }
    });
}
