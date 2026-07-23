// =============================================================================
// Workspace & Document Management Backend (`src/workspace/mod.rs`)
// =============================================================================
// This module implements all Rust backend business logic for document handling,
// filesystem scanning, project folder management, and configuration persistence.
//
// Key Responsibilities:
// 1. General Documents Storage: Default directory at `~/.oryn/documents/`.
// 2. Open Projects Persistence: Stores open project folder paths in `~/.oryn/projects.json`.
// 3. Native Dialog Interop: Folder picker for projects and confirmation dialogs for deletion.
// 4. Slint UI Synchronization: Populates `sidebar_chats` and `sidebar_projects` models.

use rfd::{MessageButtons, MessageDialog, MessageDialogResult, MessageLevel};
use serde::{Deserialize, Serialize};
use slint::{ComponentHandle, ModelRc, VecModel};
use std::cell::RefCell;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use crate::{SidebarConversation, SidebarProject};
use crate::AppWindow;

// Initial sample markdown content for first-time users
const SAMPLE_MARKDOWN_CONTENT: &str = r#"# Welcome to Oryn

A minimalist, high-performance WYSIWYG markdown editor built with Rust and Slint.

## Quick Start Guide

- **New General Document:** Click the **New document** button at the top of the sidebar or the `+` button in the **Documents** section header. Documents are stored in `~/.oryn/documents/`.
- **Open Project:** Click **File -> Open Project...** in the titlebar or the `+` button in the **Projects** section header.
- **Project Documents:** Click the `+` button on any opened project folder row to create a new markdown file inside that project.

---

### Sample Code Block

```rust
fn main() {
    println!("Hello from Oryn core engine!");
}
```
"#;

// =============================================================================
// Data Structures for Persistence
// =============================================================================
/// Persistent application configuration stored at `~/.oryn/projects.json`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WorkspaceConfig {
    /// List of absolute directory paths for open user projects
    pub open_projects: Vec<String>,
}

// =============================================================================
// Workspace State Manager
// =============================================================================
/// Main workspace state tracker held in thread-local context.
pub struct WorkspaceManager {
    /// Currently opened file path (if any)
    pub active_file_path: Option<PathBuf>,
    /// Saved workspace config
    pub config: WorkspaceConfig,
}

impl WorkspaceManager {
    /// Initializes workspace state and loads saved project configuration from disk.
    pub fn new() -> Self {
        let config = Self::load_config().unwrap_or_default();
        Self {
            active_file_path: None,
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

        // If documents directory is empty, create a initial Welcome sample document
        if let Ok(mut entries) = fs::read_dir(&dir) {
            if entries.next().is_none() {
                let sample_file = dir.join("Welcome.md");
                let _ = fs::write(sample_file, SAMPLE_MARKDOWN_CONTENT);
            }
        }
        dir
    }

    /// Scans a directory for all `.md` markdown files.
    pub fn scan_markdown_files(dir: &Path) -> Vec<SidebarConversation> {
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
                            items.push(SidebarConversation {
                                id: full_path.into(),
                                title: title.into(),
                            });
                        }
                    }
                }
            }
        }
        // Sort items alphabetically by title
        items.sort_by(|a, b| a.title.to_lowercase().cmp(&b.title.to_lowercase()));
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
    let general_dir = WorkspaceManager::ensure_general_docs_dir();
    let general_docs = WorkspaceManager::scan_markdown_files(&general_dir);

    // Populate general documents model
    let general_model = Rc::new(VecModel::from(general_docs));
    ui.set_sidebar_chats(ModelRc::from(general_model));

    // Populate projects model
    let projects_list: Vec<SidebarProject> = WORKSPACE.with(|mgr| {
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

                let conversations = WorkspaceManager::scan_markdown_files(proj_path);
                let conv_model = Rc::new(VecModel::from(conversations));

                SidebarProject {
                    name: proj_name.into(),
                    workspace: proj_path_str.as_str().into(),
                    conversations: ModelRc::from(conv_model),
                }
            })
            .collect()
    });

    let projects_model = Rc::new(VecModel::from(projects_list));
    ui.set_sidebar_projects(ModelRc::from(projects_model));
}

/// Reads the document at `file_path` from disk and loads it into the editor canvas.
pub fn load_document_into_editor(ui: &AppWindow, file_path_str: &str) {
    let path = Path::new(file_path_str);
    if path.exists() && path.is_file() {
        if let Ok(content) = fs::read_to_string(path) {
            ui.set_editor_text(content.into());
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

    // Write initial empty markdown file
    let _ = fs::write(&file_path, "");

    // Refresh UI sidebar
    refresh_sidebar_ui(ui);

    // Find new document index and select it
    let path_str = file_path.to_string_lossy().to_string();
    load_document_into_editor(ui, &path_str);

    // Update active selection indexes in Slint
    ui.set_active_project_index(-1);
    ui.set_active_conversation_index(-1);

    // Find chat index
    let general_dir = WorkspaceManager::general_documents_dir();
    let general_docs = WorkspaceManager::scan_markdown_files(&general_dir);
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

    let conversations = WorkspaceManager::scan_markdown_files(dir);
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

        refresh_sidebar_ui(ui);

        // Expand newly opened project
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

        // Clear editor if deleted document was active
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

/// Deletes a project document after confirmation.
pub fn delete_project_document(ui: &AppWindow, file_path_str: &str) {
    delete_general_document(ui, file_path_str);
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
            mgr.save_config();
        });

        ui.set_active_project_index(-1);
        ui.set_active_conversation_index(-1);

        refresh_sidebar_ui(ui);
    }
}

// =============================================================================
// Callback Wiring Registration
// =============================================================================
/// Wires all document, project, and sidebar callbacks for the given AppWindow instance.
pub fn setup_workspace_callbacks(ui: &AppWindow) {
    // Initial refresh of workspace UI models on app launch
    refresh_sidebar_ui(ui);

    // Auto-open first available document if present
    let general_dir = WorkspaceManager::general_documents_dir();
    let general_docs = WorkspaceManager::scan_markdown_files(&general_dir);
    if let Some(first_doc) = general_docs.first() {
        load_document_into_editor(ui, &first_doc.id);
        ui.set_active_chat_index(0);
    }

    // Wire Open Project callback (Titlebar File menu & Projects header '+' button)
    let ui_handle = ui.as_weak();
    ui.on_open_project_requested(move || {
        if let Some(ui) = ui_handle.upgrade() {
            pick_and_open_project(&ui);
        }
    });

    // Wire New General Document callback (Top 'New document' button & Documents header '+' button)
    let ui_handle = ui.as_weak();
    ui.on_new_document_requested(move || {
        if let Some(ui) = ui_handle.upgrade() {
            create_new_general_document(&ui);
        }
    });

    // Wire New Project Document callback (Project row '+' button)
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
            delete_project_document(&ui, &file_path);
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
}
