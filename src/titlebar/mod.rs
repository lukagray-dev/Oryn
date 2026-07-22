// =============================================================================
// Titlebar Rust Callback Handlers (`src/titlebar/mod.rs`)
// =============================================================================
// This module owns all Rust-side logic and event bindings for the custom Slint
// titlebar component (`ui/titlebar/titlebar.slint`).
//
// Separating titlebar event handlers into this module keeps `main.rs` clean and
// provides a modular, maintainable architectural boundary for window management.

use slint::winit_030::{winit, WinitWindowAccessor};
use slint::ComponentHandle;

// Import the generated Slint UI module components
use crate::AppWindow;

/// Wires all titlebar callbacks (drag, minimize, maximize, close, exit, undo, redo) for the given AppWindow instance.
pub fn setup_titlebar_callbacks(ui: &AppWindow) {
    // -------------------------------------------------------------------------
    // 1. Wire Window Drag Callback
    // -------------------------------------------------------------------------
    // Fires when the user clicks and drags on empty titlebar space. Delegates
    // window dragging to the underlying native winit window handle.
    let ui_handle = ui.as_weak();
    ui.on_drag_window_requested(move || {
        if let Some(ui) = ui_handle.upgrade() {
            ui.window().with_winit_window(|winit_window: &winit::window::Window| {
                let _ = winit_window.drag_window();
            });
        }
    });

    // -------------------------------------------------------------------------
    // 2. Wire Undo & Redo Action Callbacks
    // -------------------------------------------------------------------------
    ui.on_undo_requested(move || {
        // Document undo history handler placeholder
    });

    ui.on_redo_requested(move || {
        // Document redo history handler placeholder
    });

    // -------------------------------------------------------------------------
    // 3. Wire Minimize Window Callback
    // -------------------------------------------------------------------------
    // Minimizes the window to the OS taskbar when the minimize button is clicked.
    let ui_handle = ui.as_weak();
    ui.on_minimize_requested(move || {
        if let Some(ui) = ui_handle.upgrade() {
            ui.window().set_minimized(true);
        }
    });

    // -------------------------------------------------------------------------
    // 4. Wire Maximize / Restore Toggle Callback
    // -------------------------------------------------------------------------
    // Toggles window between maximized and restored states, updating `window_maximized`
    // so `resize-border-width` automatically adapts between 6px (normal) and 0px (maximized).
    let ui_handle = ui.as_weak();
    ui.on_maximize_requested(move || {
        if let Some(ui) = ui_handle.upgrade() {
            let is_maximized = ui.get_window_maximized();
            ui.set_window_maximized(!is_maximized);
            ui.window().set_maximized(!is_maximized);
        }
    });

    // -------------------------------------------------------------------------
    // 5. Wire Close Window Callback
    // -------------------------------------------------------------------------
    // Hides the active application window when the titlebar close button is clicked.
    let ui_handle = ui.as_weak();
    ui.on_close_window_requested(move || {
        if let Some(ui) = ui_handle.upgrade() {
            let _ = ui.hide();
        }
    });

    // -------------------------------------------------------------------------
    // 6. Wire Exit Application Callback
    // -------------------------------------------------------------------------
    // Terminates the Slint event loop cleanly when "Exit" is selected from File/Window menus.
    ui.on_exit_requested(move || {
        let _ = slint::quit_event_loop();
    });
}
