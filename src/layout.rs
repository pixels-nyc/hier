#![allow(dead_code)]

use crate::spring::Spring;

/// Unique identifier for a Window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct WindowId(pub u32);

/// Represents a single client application window.
#[derive(Debug, Clone)]
pub struct Window {
    pub id: WindowId,
    pub title: String,
}

/// A container in the horizontal layout.
/// Can act as a single window container, or a tabbed group stack (Z-axis).
#[derive(Debug, Clone)]
pub struct Column {
    pub windows: Vec<Window>,
    pub focused_window_idx: usize,
    pub width: f32,
}

impl Column {
    pub fn new(window: Window, width: f32) -> Self {
        Self {
            windows: vec![window],
            focused_window_idx: 0,
            width,
        }
    }

    /// Gets the currently focused window in this column.
    pub fn focused_window(&self) -> Option<&Window> {
        self.windows.get(self.focused_window_idx)
    }

    /// Returns true if this column has multiple windows stacked as tabs.
    pub fn is_tabbed(&self) -> bool {
        self.windows.len() > 1
    }
}

/// A Workspace represented as an infinite horizontal strip of columns.
/// Workspaces themselves are stacked vertically.
#[derive(Debug, Clone)]
pub struct Workspace {
    pub columns: Vec<Column>,
    pub focused_column_idx: usize,
}

impl Workspace {
    pub fn new() -> Self {
        Self {
            columns: Vec::new(),
            focused_column_idx: 0,
        }
    }

    /// Gets the currently focused column.
    pub fn focused_column(&self) -> Option<&Column> {
        self.columns.get(self.focused_column_idx)
    }

    /// Gets the currently focused column mutably.
    pub fn focused_column_mut(&mut self) -> Option<&mut Column> {
        let idx = self.focused_column_idx;
        self.columns.get_mut(idx)
    }

    /// Helper to find a window by ID in this workspace, returning (col_idx, win_idx).
    pub fn find_window(&self, id: WindowId) -> Option<(usize, usize)> {
        for (col_idx, col) in self.columns.iter().enumerate() {
            for (win_idx, win) in col.windows.iter().enumerate() {
                if win.id == id {
                    return Some((col_idx, win_idx));
                }
            }
        }
        None
    }
}

/// The physical viewport (monitor bounds) that acts as a camera.
/// Smoothly animates towards target coordinates using spring physics.
#[derive(Debug, Clone)]
pub struct Viewport {
    /// Current position of the camera (top-left).
    pub x: f32,
    pub y: f32,
    /// Target position the camera is moving to.
    pub target_x: f32,
    pub target_y: f32,
    /// Current scrolling velocities.
    pub velocity_x: f32,
    pub velocity_y: f32,
    /// Viewport physical dimensions.
    pub width: f32,
    pub height: f32,
}

impl Viewport {
    pub fn new(width: f32, height: f32) -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            target_x: 0.0,
            target_y: 0.0,
            velocity_x: 0.0,
            velocity_y: 0.0,
            width,
            height,
        }
    }
}

/// The core layout engine state.
#[derive(Debug, Clone)]
pub struct LayoutEngine {
    pub workspaces: Vec<Workspace>,
    pub active_workspace_idx: usize,
    pub viewport: Viewport,
    pub spring: Spring,
    /// Spacing between adjacent columns.
    pub gap: f32,
    /// Margin between windows and the edge of the physical screen.
    pub outer_margin: f32,
    /// Default width fraction of the viewport for new windows (e.g. 0.5 for 50%).
    pub default_width_fraction: f32,
}

impl LayoutEngine {
    pub fn new(
        viewport_width: f32,
        viewport_height: f32,
        gap: f32,
        outer_margin: f32,
        num_workspaces: usize,
    ) -> Self {
        let mut workspaces = Vec::with_capacity(num_workspaces);
        for _ in 0..num_workspaces {
            workspaces.push(Workspace::new());
        }

        Self {
            workspaces,
            active_workspace_idx: 0,
            viewport: Viewport::new(viewport_width, viewport_height),
            spring: Spring::default(),
            gap,
            outer_margin,
            default_width_fraction: 0.5,
        }
    }

    /// Ticks the spring physics for the camera positioning.
    pub fn tick(&mut self, dt: f32) {
        let (nx, vx) = self.spring.update(
            self.viewport.x,
            self.viewport.velocity_x,
            self.viewport.target_x,
            dt,
        );
        self.viewport.x = nx;
        self.viewport.velocity_x = vx;

        let (ny, vy) = self.spring.update(
            self.viewport.y,
            self.viewport.velocity_y,
            self.viewport.target_y,
            dt,
        );
        self.viewport.y = ny;
        self.viewport.velocity_y = vy;
    }

    /// Resize the physical output dimensions.
    pub fn resize_viewport(&mut self, width: f32, height: f32) {
        self.viewport.width = width;
        self.viewport.height = height;
        self.recenter_camera(false);
    }

    /// Computes the X coordinate of each column in a given workspace.
    /// The first column starts at `outer_margin`.
    /// Subsequent columns are placed dynamically based on the width of previous columns and gaps.
    pub fn column_positions(&self, workspace_idx: usize) -> Vec<f32> {
        let mut positions = Vec::new();
        if let Some(workspace) = self.workspaces.get(workspace_idx) {
            let mut current_x = self.outer_margin;
            for col in &workspace.columns {
                positions.push(current_x);
                current_x += col.width + self.gap;
            }
        }
        positions
    }

    /// Updates the camera's target positions to center on the focused column
    /// of the active workspace and the active workspace's vertical offset.
    pub fn recenter_camera(&mut self, immediate: bool) {
        let active_idx = self.active_workspace_idx;
        
        // Vertical workspace target offset
        let target_y = active_idx as f32 * self.viewport.height;
        self.viewport.target_y = target_y;
        if immediate {
            self.viewport.y = target_y;
            self.viewport.velocity_y = 0.0;
        }

        // Horizontal camera target offset (centered on the active column)
        let positions = self.column_positions(active_idx);
        let workspace = &self.workspaces[active_idx];
        
        if workspace.columns.is_empty() {
            self.viewport.target_x = 0.0;
            if immediate {
                self.viewport.x = 0.0;
                self.viewport.velocity_x = 0.0;
            }
            return;
        }

        let col_idx = workspace.focused_column_idx;
        let col_x = positions[col_idx];
        let col_w = workspace.columns[col_idx].width;

        // Center the active column in the viewport
        let target_x = col_x + (col_w / 2.0) - (self.viewport.width / 2.0);
        self.viewport.target_x = target_x;

        if immediate {
            self.viewport.x = target_x;
            self.viewport.velocity_x = 0.0;
        }
    }

    /// Gets the current active workspace.
    pub fn active_workspace(&self) -> &Workspace {
        &self.workspaces[self.active_workspace_idx]
    }

    /// Gets the current active workspace mutably.
    pub fn active_workspace_mut(&mut self) -> &mut Workspace {
        &mut self.workspaces[self.active_workspace_idx]
    }

    /// Spawns a new window. It will be added to the right of the currently
    /// focused column. If no columns exist, it initializes the first column.
    pub fn spawn_window(&mut self, window_id: WindowId, title: String) {
        let win_width = self.viewport.width * self.default_width_fraction;
        let window = Window { id: window_id, title };
        let column = Column::new(window, win_width);
        
        let workspace = self.active_workspace_mut();
        if workspace.columns.is_empty() {
            workspace.columns.push(column);
            workspace.focused_column_idx = 0;
        } else {
            let insert_idx = workspace.focused_column_idx + 1;
            workspace.columns.insert(insert_idx, column);
            workspace.focused_column_idx = insert_idx;
        }

        self.recenter_camera(false);
    }

    /// Closes a window by ID. Searches all workspaces.
    /// If the column containing the window becomes empty, the column is removed.
    pub fn close_window(&mut self, id: WindowId) {
        let mut found = None;

        for (ws_idx, ws) in self.workspaces.iter().enumerate() {
            if let Some((col_idx, win_idx)) = ws.find_window(id) {
                found = Some((ws_idx, col_idx, win_idx));
                break;
            }
        }

        if let Some((ws_idx, col_idx, win_idx)) = found {
            let ws = &mut self.workspaces[ws_idx];
            let col = &mut ws.columns[col_idx];
            col.windows.remove(win_idx);

            if col.windows.is_empty() {
                // Remove empty column
                ws.columns.remove(col_idx);
                // Adjust workspace focus index
                if ws.focused_column_idx >= ws.columns.len() && !ws.columns.is_empty() {
                    ws.focused_column_idx = ws.columns.len() - 1;
                }
            } else {
                // Adjust column inner window focus
                if col.focused_window_idx >= col.windows.len() {
                    col.focused_window_idx = col.windows.len() - 1;
                }
            }

            self.recenter_camera(false);
        }
    }

    // --- NAVIGATION ---

    /// Move focus to the column on the left.
    pub fn focus_left(&mut self) {
        let ws = self.active_workspace_mut();
        if ws.focused_column_idx > 0 {
            ws.focused_column_idx -= 1;
            self.recenter_camera(false);
        }
    }

    /// Move focus to the column on the right.
    pub fn focus_right(&mut self) {
        let ws = self.active_workspace_mut();
        if ws.focused_column_idx + 1 < ws.columns.len() {
            ws.focused_column_idx += 1;
            self.recenter_camera(false);
        }
    }

    /// Focus the tab above in the current column (Z-axis).
    pub fn focus_tab_up(&mut self) {
        let ws = self.active_workspace_mut();
        if let Some(col) = ws.focused_column_mut() {
            if !col.windows.is_empty() {
                if col.focused_window_idx > 0 {
                    col.focused_window_idx -= 1;
                } else {
                    col.focused_window_idx = col.windows.len() - 1;
                }
            }
        }
    }

    /// Focus the tab below in the current column (Z-axis).
    pub fn focus_tab_down(&mut self) {
        let ws = self.active_workspace_mut();
        if let Some(col) = ws.focused_column_mut() {
            if !col.windows.is_empty() {
                col.focused_window_idx = (col.focused_window_idx + 1) % col.windows.len();
            }
        }
    }

    /// Focus the workspace above (Y-axis decrement).
    pub fn focus_workspace_up(&mut self) {
        if self.active_workspace_idx > 0 {
            self.active_workspace_idx -= 1;
            self.recenter_camera(false);
        }
    }

    /// Focus the workspace below (Y-axis increment).
    pub fn focus_workspace_down(&mut self) {
        if self.active_workspace_idx + 1 < self.workspaces.len() {
            self.active_workspace_idx += 1;
            self.recenter_camera(false);
        }
    }

    // --- MANIPULATION ---

    /// Swaps the focused column with the one to its left.
    pub fn move_column_left(&mut self) {
        let ws = self.active_workspace_mut();
        let idx = ws.focused_column_idx;
        if idx > 0 {
            ws.columns.swap(idx, idx - 1);
            ws.focused_column_idx = idx - 1;
            self.recenter_camera(false);
        }
    }

    /// Swaps the focused column with the one to its right.
    pub fn move_column_right(&mut self) {
        let ws = self.active_workspace_mut();
        let idx = ws.focused_column_idx;
        if idx + 1 < ws.columns.len() {
            ws.columns.swap(idx, idx + 1);
            ws.focused_column_idx = idx + 1;
            self.recenter_camera(false);
        }
    }

    /// Moves the active window to the workspace above.
    pub fn move_window_workspace_up(&mut self) {
        if self.active_workspace_idx > 0 {
            let target_ws_idx = self.active_workspace_idx - 1;
            self.move_focused_window_to_workspace(target_ws_idx);
        }
    }

    /// Moves the active window to the workspace below.
    pub fn move_window_workspace_down(&mut self) {
        if self.active_workspace_idx + 1 < self.workspaces.len() {
            let target_ws_idx = self.active_workspace_idx + 1;
            self.move_focused_window_to_workspace(target_ws_idx);
        }
    }

    /// Internal helper to move the currently focused window of the active workspace
    /// to another workspace.
    fn move_focused_window_to_workspace(&mut self, target_ws_idx: usize) {
        let current_ws_idx = self.active_workspace_idx;
        let ws = &mut self.workspaces[current_ws_idx];
        if ws.columns.is_empty() {
            return;
        }

        let col_idx = ws.focused_column_idx;
        let col = &mut ws.columns[col_idx];
        let win_idx = col.focused_window_idx;
        
        let window = col.windows.remove(win_idx);
        let col_width = col.width;

        // Clean up empty column
        if col.windows.is_empty() {
            ws.columns.remove(col_idx);
            if ws.focused_column_idx >= ws.columns.len() && !ws.columns.is_empty() {
                ws.focused_column_idx = ws.columns.len() - 1;
            }
        } else if col.focused_window_idx >= col.windows.len() {
            col.focused_window_idx = col.windows.len() - 1;
        }

        // Add to target workspace (appended to the right of its focused column, or at the end)
        let target_ws = &mut self.workspaces[target_ws_idx];
        let target_col = Column::new(window, col_width);
        if target_ws.columns.is_empty() {
            target_ws.columns.push(target_col);
            target_ws.focused_column_idx = 0;
        } else {
            let insert_idx = target_ws.focused_column_idx + 1;
            target_ws.columns.insert(insert_idx, target_col);
            target_ws.focused_column_idx = insert_idx;
        }

        self.recenter_camera(false);
    }

    /// Toggle tabbing behavior on the active column.
    /// - If the current column has multiple windows (tabs), this expels the active tab
    ///   into its own standalone column to the right.
    /// - If the current column has only one window, it consumes the column to its right
    ///   as a tab.
    pub fn toggle_tab_group(&mut self) {
        let ws = self.active_workspace_mut();
        if ws.columns.is_empty() {
            return;
        }

        let idx = ws.focused_column_idx;
        let is_tabbed = ws.columns[idx].is_tabbed();

        if is_tabbed {
            // Expel active tab into a new column to the right
            let col = &mut ws.columns[idx];
            let active_win_idx = col.focused_window_idx;
            let window = col.windows.remove(active_win_idx);
            
            // Adjust current column indices
            if col.focused_window_idx >= col.windows.len() {
                col.focused_window_idx = col.windows.len() - 1;
            }

            let new_col = Column::new(window, col.width);
            ws.columns.insert(idx + 1, new_col);
            ws.focused_column_idx = idx + 1;
        } else if idx + 1 < ws.columns.len() {
            // Consume the column to the right as a tab
            let mut right_col = ws.columns.remove(idx + 1);
            let col = &mut ws.columns[idx];
            let old_len = col.windows.len();
            col.windows.append(&mut right_col.windows);
            // Focus the first of the newly added tabs
            col.focused_window_idx = old_len;
        }

        self.recenter_camera(false);
    }

    /// Sets the width of the currently focused column.
    pub fn set_focused_column_width(&mut self, width: f32) {
        let ws = self.active_workspace_mut();
        if let Some(col) = ws.focused_column_mut() {
            col.width = width.max(50.0); // Maintain a sensible minimum width
            self.recenter_camera(false);
        }
    }

    /// Computes the visual bounding box of a window by its ID.
    /// Returns `Option<(x, y, width, height)>` in global coordinate system.
    pub fn get_window_rect(&self, id: WindowId) -> Option<(f32, f32, f32, f32)> {
        for (ws_idx, ws) in self.workspaces.iter().enumerate() {
            if let Some((col_idx, _win_idx)) = ws.find_window(id) {
                let positions = self.column_positions(ws_idx);
                let col_x = positions[col_idx];
                let col = &ws.columns[col_idx];
                
                // If it's a tabbed group, only the active window gets rendered fully.
                // Non-active windows are stacked (we can still report their dimensions,
                // but let's assume standard window geometry matches the column's).
                let col_width = col.width;
                
                let ws_y = ws_idx as f32 * self.viewport.height;
                
                // Visual window geometry within its grid slot (applying outer margins)
                let x = col_x + self.outer_margin / 2.0;
                let y = ws_y + self.outer_margin;
                let w = col_width - self.outer_margin;
                let h = self.viewport.height - 2.0 * self.outer_margin;

                // Adjust for tabs: if it's tabbed, we only render the active tab,
                // or we could report special geometry. For now, they occupy the same space.
                return Some((x, y, w, h));
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_spawn_and_close() {
        let mut engine = LayoutEngine::new(800.0, 600.0, 10.0, 20.0, 5);
        let win1 = WindowId(1);
        let win2 = WindowId(2);

        assert_eq!(engine.active_workspace().columns.len(), 0);

        engine.spawn_window(win1, "Terminal".to_string());
        assert_eq!(engine.active_workspace().columns.len(), 1);
        assert_eq!(engine.active_workspace().focused_column_idx, 0);

        engine.spawn_window(win2, "Browser".to_string());
        assert_eq!(engine.active_workspace().columns.len(), 2);
        assert_eq!(engine.active_workspace().focused_column_idx, 1);

        engine.close_window(win2);
        assert_eq!(engine.active_workspace().columns.len(), 1);
        assert_eq!(engine.active_workspace().focused_column_idx, 0);

        engine.close_window(win1);
        assert_eq!(engine.active_workspace().columns.len(), 0);
    }

    #[test]
    fn test_navigation_and_recenter() {
        let mut engine = LayoutEngine::new(1000.0, 600.0, 10.0, 20.0, 5);
        let w1 = WindowId(1);
        let w2 = WindowId(2);
        let w3 = WindowId(3);

        // Columns width will be 50% of viewport width = 500.0
        engine.spawn_window(w1, "W1".to_string());
        engine.spawn_window(w2, "W2".to_string());
        engine.spawn_window(w3, "W3".to_string());

        // Focused column is index 2 (W3)
        assert_eq!(engine.active_workspace().focused_column_idx, 2);

        // Positions:
        // Col 0: outer_margin = 20.0, width = 500.0
        // Col 1: 20.0 + 500.0 + 10.0 = 530.0, width = 500.0
        // Col 2: 530.0 + 500.0 + 10.0 = 1040.0, width = 500.0
        let positions = engine.column_positions(0);
        assert_eq!(positions[0], 20.0);
        assert_eq!(positions[1], 530.0);
        assert_eq!(positions[2], 1040.0);

        // Center on Col 2: target_x = 1040.0 + 250.0 - 500.0 = 790.0
        assert_eq!(engine.viewport.target_x, 790.0);

        // Move left (to W2, index 1)
        engine.focus_left();
        assert_eq!(engine.active_workspace().focused_column_idx, 1);
        // Center on Col 1: target_x = 530.0 + 250.0 - 500.0 = 280.0
        assert_eq!(engine.viewport.target_x, 280.0);
    }

    #[test]
    fn test_tabbed_group() {
        let mut engine = LayoutEngine::new(1000.0, 600.0, 10.0, 20.0, 5);
        let w1 = WindowId(1);
        let w2 = WindowId(2);

        engine.spawn_window(w1, "W1".to_string());
        engine.spawn_window(w2, "W2".to_string());

        // Two columns. Left column W1, right column W2 (focused).
        assert_eq!(engine.active_workspace().columns.len(), 2);
        engine.focus_left(); // focus W1 at index 0

        // Consume right column as tab
        engine.toggle_tab_group();
        assert_eq!(engine.active_workspace().columns.len(), 1);
        let col = &engine.active_workspace().columns[0];
        assert_eq!(col.windows.len(), 2);
        assert_eq!(col.focused_window_idx, 1); // W2 is focused as the new tab

        // Focus up to switch back to W1 tab
        engine.focus_tab_up();
        assert_eq!(engine.active_workspace().columns[0].focused_window_idx, 0);

        // Expel W1 into its own column
        engine.toggle_tab_group();
        assert_eq!(engine.active_workspace().columns.len(), 2);
        assert_eq!(engine.active_workspace().columns[0].windows.len(), 1);
        assert_eq!(engine.active_workspace().columns[1].windows.len(), 1);
    }

    #[test]
    fn test_workspace_movement() {
        let mut engine = LayoutEngine::new(1000.0, 600.0, 10.0, 20.0, 5);
        let w1 = WindowId(1);

        engine.spawn_window(w1, "W1".to_string());
        assert_eq!(engine.active_workspace_idx, 0);
        assert_eq!(engine.workspaces[0].columns.len(), 1);

        // Move window to workspace below (ws index 1)
        engine.move_window_workspace_down();
        assert_eq!(engine.workspaces[0].columns.len(), 0);
        assert_eq!(engine.workspaces[1].columns.len(), 1);

        // Active workspace index remains 0, but we can navigate down
        engine.focus_workspace_down();
        assert_eq!(engine.active_workspace_idx, 1);
        assert_eq!(engine.viewport.target_y, 600.0);
    }

    #[test]
    fn test_complex_multitasking_flow() {
        let mut engine = LayoutEngine::new(1000.0, 600.0, 10.0, 20.0, 5);
        let w1 = WindowId(1);
        let w2 = WindowId(2);
        let w3 = WindowId(3);

        // 1. Spawn Window 1
        engine.spawn_window(w1, "Terminal".to_string());
        assert_eq!(engine.active_workspace().columns.len(), 1);

        // 2. Spawn Window 2
        engine.spawn_window(w2, "Editor".to_string());
        assert_eq!(engine.active_workspace().columns.len(), 2);
        assert_eq!(engine.active_workspace().focused_column_idx, 1);

        // 3. Spawn Window 3
        engine.spawn_window(w3, "Browser".to_string());
        assert_eq!(engine.active_workspace().columns.len(), 3);
        assert_eq!(engine.active_workspace().focused_column_idx, 2);

        // 4. Focus left to Window 2 (Editor)
        engine.focus_left();
        assert_eq!(engine.active_workspace().focused_column_idx, 1);

        // 5. Toggle tab group: consume Column 2 (Browser) as a tab into Column 1
        engine.toggle_tab_group();
        assert_eq!(engine.active_workspace().columns.len(), 2);
        let col = &engine.active_workspace().columns[1];
        assert_eq!(col.windows.len(), 2);
        assert_eq!(col.focused_window_idx, 1); // Browser (w3) is focused

        // 6. Focus tab up to switch back to Editor (w2)
        engine.focus_tab_up();
        assert_eq!(engine.active_workspace().columns[1].focused_window_idx, 0); // Editor (w2) is focused

        // 7. Move column left: swap Column 1 (tabbed Editor+Browser) with Column 0 (Terminal)
        engine.move_column_left();
        assert_eq!(engine.active_workspace().focused_column_idx, 0);

        // 8. Verify the resulting layout
        let active_ws = engine.active_workspace();
        assert_eq!(active_ws.columns[0].windows[0].id, w2); // Editor
        assert_eq!(active_ws.columns[0].windows[1].id, w3); // Browser
        assert_eq!(active_ws.columns[1].windows[0].id, w1); // Terminal

        // 9. Verify camera target position centered on Column 0 (width = 500)
        // Col 0: x = 20.0, w = 500.0. Center of Col 0 = 20.0 + 250.0 = 270.0.
        // Viewport width = 1000. target_x = 270.0 - 500.0 = -230.0.
        assert_eq!(engine.viewport.target_x, -230.0);
    }

    #[test]
    fn test_perpetual_tab_wrapping() {
        let mut engine = LayoutEngine::new(1000.0, 600.0, 10.0, 20.0, 5);
        let w1 = WindowId(1);
        let w2 = WindowId(2);
        let w3 = WindowId(3);

        engine.spawn_window(w1, "W1".to_string());
        engine.spawn_window(w2, "W2".to_string());
        engine.spawn_window(w3, "W3".to_string());

        // Focus W2 (index 1) and consume W3 (index 2)
        engine.focus_left();
        engine.toggle_tab_group();

        // Focus W1 (index 0) and consume Column 1 (which now has W2 and W3)
        engine.focus_left();
        engine.toggle_tab_group();

        let col_len = engine.active_workspace().columns.len();
        assert_eq!(col_len, 1);
        assert_eq!(engine.active_workspace().columns[0].windows.len(), 3);

        // Make sure the focused window index is at some initial state
        engine.active_workspace_mut().columns[0].focused_window_idx = 2; // W3 focused

        // Tab down wraps to index 0 (W1)
        engine.focus_tab_down();
        assert_eq!(engine.active_workspace().columns[0].focused_window_idx, 0);

        // Tab up wraps back to index 2 (W3)
        engine.focus_tab_up();
        assert_eq!(engine.active_workspace().columns[0].focused_window_idx, 2);

        // Tab up switches to index 1 (W2)
        engine.focus_tab_up();
        assert_eq!(engine.active_workspace().columns[0].focused_window_idx, 1);
    }
}
