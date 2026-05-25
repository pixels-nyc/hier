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
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TilingMode {
    Diagonal,
    Grid,
    Float,
    Depth,
    Overview,
}

// Transform data for Depth mode card stack
#[derive(Default, Debug, Clone, Copy)]
pub struct DepthTransform {
    pub scale: f32,
    pub opacity: f32,
    pub y_offset: i32,
    pub is_active: bool,
}

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
    /// Tiling mode (Diagonal, Grid, Float, Depth).
    pub tiling_mode: TilingMode,
    /// List of window identifiers for Depth mode ordering.
    pub windows: Vec<WindowId>,
    /// Continuous scroll progress for Depth mode (0.0 = first window centered).
    pub depth_scroll_progress: f32,
    /// Z-scroll sensitivity (determines how fast scroll_z progresses)
    pub scroll_sensitivity: f32,
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

        let scroll_sensitivity = std::env::var("HIER_Z_SENSITIVITY")
            .ok()
            .and_then(|val| val.parse::<f32>().ok())
            .unwrap_or(0.1_f32);

        Self {
            workspaces,
            active_workspace_idx: 0,
            viewport: Viewport::new(viewport_width, viewport_height),
            spring: Spring::default(),
            gap,
            outer_margin,
            default_width_fraction: 0.5,
            tiling_mode: TilingMode::Grid,
            windows: Vec::new(),
            depth_scroll_progress: 0.0,
            scroll_sensitivity,
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
        let old_width = self.viewport.width;
        self.viewport.width = width;
        self.viewport.height = height;

        if old_width > 0.0 && (width - old_width).abs() > 1e-3 {
            let scale_factor = width / old_width;
            for ws in &mut self.workspaces {
                for col in &mut ws.columns {
                    col.width *= scale_factor;
                }
            }
        }

        self.recenter_camera(false);
    }

    /// Computes the X coordinate of each column in a given workspace.
    /// The first column starts at `outer_margin`.
    /// Subsequent columns are placed dynamically based on the width of previous columns and gaps.
    pub fn column_positions(&self, workspace_idx: usize) -> Vec<f32> {
        let mut positions = Vec::new();
        if let Some(workspace) = self.workspaces.get(workspace_idx) {
            match self.tiling_mode {
                TilingMode::Diagonal => {
                    let mut current_x = self.outer_margin;
                    for (i, col) in workspace.columns.iter().enumerate() {
                        // Diagonal offset based on column index
                        let diag_offset = i as f32 * self.gap;
                        positions.push(current_x + diag_offset);
                        current_x += col.width + self.gap;
                    }
                }
                TilingMode::Grid | TilingMode::Overview => {
                    // Simple left‑to‑right layout (current behavior)
                    let mut current_x = self.outer_margin;
                    for col in &workspace.columns {
                        positions.push(current_x);
                        current_x += col.width + self.gap;
                    }
                }
                TilingMode::Float => {
                    // Floating mode: each column retains its own absolute X (no auto‑arrange)
                    for col in &workspace.columns {
                        // Assume the column already stores its absolute X in width field for simplicity
                        positions.push(col.width);
                    }
                }
                TilingMode::Depth => {
                    // All columns stacked at the same X position (card carousel)
                    for _ in &workspace.columns {
                        positions.push(self.outer_margin);
                    }
                }
            }
        }
        positions
    }

    /// Updates the camera's target positions to center on the focused column
    
    /// Scrolls the depth carousel forward or backward.
    /// Positive `delta` moves the front column to the back (next item),
    /// negative `delta` moves the back column to the front (previous item).
    pub fn scroll_z(&mut self, delta: f32) {
        // Only applicable in Depth mode
        if self.tiling_mode != TilingMode::Depth {
            return;
        }
        self.depth_scroll_progress += delta * self.scroll_sensitivity;
        let max_progress = (self.windows.len().saturating_sub(1)) as f32;
        self.depth_scroll_progress = self.depth_scroll_progress.clamp(0.0, max_progress);
    }

    /// Computes spatial transforms for windows in Depth mode, sorted back-to-front
    pub fn depth_transforms(&self) -> Vec<(WindowId, DepthTransform)> {
        let mut transforms = Vec::new();
        for (i, &w_id) in self.windows.iter().enumerate() {
            let dist = (i as f32) - self.depth_scroll_progress;
            
            let (scale, opacity, y_offset) = if dist >= 0.0 {
                let scale = 1.0 / (1.0 + 0.25 * dist);
                let opacity = (1.0 - 0.4 * dist).max(0.0);
                let y_offset = (30.0 * dist) as i32;
                (scale, opacity, y_offset)
            } else {
                let scale = 1.0 - 0.5 * dist;
                let opacity = (1.0 + dist).max(0.0);
                let y_offset = (30.0 * dist) as i32;
                (scale, opacity, y_offset)
            };
            
            let is_active = dist.abs() < 0.5;
            
            transforms.push((
                w_id,
                DepthTransform {
                    scale,
                    opacity,
                    y_offset,
                    is_active,
                },
            ));
        }

        // Sort back-to-front (furthest first, i.e. larger dist first)
        transforms.sort_by(|a, b| {
            let idx_a = self.windows.iter().position(|&x| x == a.0).unwrap_or(0);
            let idx_b = self.windows.iter().position(|&x| x == b.0).unwrap_or(0);
            let dist_a = (idx_a as f32) - self.depth_scroll_progress;
            let dist_b = (idx_b as f32) - self.depth_scroll_progress;
            dist_b.partial_cmp(&dist_a).unwrap_or(std::cmp::Ordering::Equal)
        });

        transforms
    }

    /// of the active workspace and the active workspace's vertical offset.
    pub fn recenter_camera(&mut self, immediate: bool) {
        let active_idx = self.active_workspace_idx;
        
        if self.tiling_mode == TilingMode::Overview {
            let scale = 0.45_f32;
            let spacing = 40.0_f32;
            
            // Vertical target centered on active workspace stack
            let target_y = (active_idx as f32 * (self.viewport.height * scale + spacing))
                + (self.viewport.height * scale / 2.0) - (self.viewport.height / 2.0);
            self.viewport.target_y = target_y;
            if immediate {
                self.viewport.y = target_y;
                self.viewport.velocity_y = 0.0;
            }

            // Horizontal target centered on active workspace's focused column
            let positions = self.column_positions(active_idx);
            let workspace = &self.workspaces[active_idx];
            
            let target_x = if workspace.columns.is_empty() {
                - (self.viewport.width / 2.0)
            } else if workspace.columns.len() == 1 {
                (self.viewport.width / 2.0) * scale - (self.viewport.width / 2.0)
            } else {
                let col_idx = workspace.focused_column_idx;
                let col_x = positions[col_idx];
                let col_w = workspace.columns[col_idx].width;
                (col_x + col_w / 2.0) * scale - (self.viewport.width / 2.0)
            };
            
            self.viewport.target_x = target_x;
            if immediate {
                self.viewport.x = target_x;
                self.viewport.velocity_x = 0.0;
            }
            return;
        }

        // Vertical workspace target offset
        let target_y = active_idx as f32 * self.viewport.height;
        self.viewport.target_y = target_y;
        if immediate {
            self.viewport.y = target_y;
            self.viewport.velocity_y = 0.0;
        }

        if self.tiling_mode == TilingMode::Depth {
            self.viewport.target_x = 0.0;
            if immediate {
                self.viewport.x = 0.0;
                self.viewport.velocity_x = 0.0;
            }
            return;
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

        if workspace.columns.len() == 1 {
            self.viewport.target_x = 0.0;
            if immediate {
                self.viewport.x = 0.0;
                self.viewport.velocity_x = 0.0;
            }
        } else {
            let last_idx = workspace.columns.len() - 1;
            let last_col_x = positions[last_idx];
            let last_col_w = workspace.columns[last_idx].width;
            let total_width = last_col_x + last_col_w + self.outer_margin;

            if total_width <= self.viewport.width {
                self.viewport.target_x = 0.0;
                if immediate {
                    self.viewport.x = 0.0;
                    self.viewport.velocity_x = 0.0;
                }
            } else {
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
        let win_width = self.default_width_fraction * (self.viewport.width - 2.0 * self.outer_margin - self.gap);
        let window = Window { id: window_id, title };
        let column = Column::new(window, win_width);
        
        // Track window ordering for Depth mode
        self.windows.push(window_id);
        
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

            // Remove from Depth ordering if present
            self.windows.retain(|&w_id| w_id != id);

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

    pub fn get_window_rect(&self, id: WindowId) -> Option<(f32, f32, f32, f32)> {
        for (ws_idx, ws) in self.workspaces.iter().enumerate() {
            if let Some((col_idx, _win_idx)) = ws.find_window(id) {
                if self.tiling_mode == TilingMode::Overview {
                    let scale = 0.45_f32;
                    let spacing = 40.0_f32;
                    
                    let (base_x, base_y, base_w, base_h) = if ws.columns.len() == 1 {
                        (
                            self.outer_margin,
                            self.outer_margin,
                            self.viewport.width - 2.0 * self.outer_margin,
                            self.viewport.height - 2.0 * self.outer_margin,
                        )
                    } else {
                        let positions = self.column_positions(ws_idx);
                        let col_x = positions[col_idx];
                        let col = &ws.columns[col_idx];
                        let col_width = col.width;
                        (
                            col_x,
                            self.outer_margin,
                            col_width,
                            self.viewport.height - 2.0 * self.outer_margin,
                        )
                    };
                    
                    let x = base_x;
                    let y = (ws_idx as f32 * (self.viewport.height + spacing / scale)) + base_y;
                    let w = base_w;
                    let h = base_h;
                    return Some((x, y, w, h));
                }

                if self.tiling_mode == TilingMode::Depth {
                    let ws_y = ws_idx as f32 * self.viewport.height;
                    let x = self.outer_margin;
                    let y = ws_y + self.outer_margin;
                    let w = self.viewport.width - 2.0 * self.outer_margin;
                    let h = self.viewport.height - 2.0 * self.outer_margin;
                    return Some((x, y, w, h));
                }

                let ws_y = ws_idx as f32 * self.viewport.height;

                if ws.columns.len() == 1 {
                    let x = self.outer_margin;
                    let y = ws_y + self.outer_margin;
                    let w = self.viewport.width - 2.0 * self.outer_margin;
                    let h = self.viewport.height - 2.0 * self.outer_margin;
                    return Some((x, y, w, h));
                }

                let positions = self.column_positions(ws_idx);
                let col_x = positions[col_idx];
                let col = &ws.columns[col_idx];
                let col_width = col.width;
                
                let x = col_x;
                let y = ws_y + self.outer_margin;
                let w = col_width;
                let h = self.viewport.height - 2.0 * self.outer_margin;
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
        assert_eq!(positions[1], 505.0);
        assert_eq!(positions[2], 990.0);

        // Center on Col 2: target_x = 990.0 + 237.5 - 500.0 = 727.5
        assert_eq!(engine.viewport.target_x, 727.5);

        // Move left (to W2, index 1)
        engine.focus_left();
        assert_eq!(engine.active_workspace().focused_column_idx, 1);
        // Center on Col 1: target_x = 505.0 + 237.5 - 500.0 = 242.5
        assert_eq!(engine.viewport.target_x, 242.5);
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

        // 9. Verify camera target position centered on Column 0
        // Because the columns fit within the viewport (total width <= 1000), target_x is locked to 0.0.
        assert_eq!(engine.viewport.target_x, 0.0);
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

    #[test]
    fn test_depth_scrolling() {
        let mut engine = LayoutEngine::new(1000.0, 600.0, 10.0, 20.0, 5);
        engine.tiling_mode = TilingMode::Depth;
        engine.scroll_sensitivity = 0.5; // override sensitivity

        let w1 = WindowId(1);
        let w2 = WindowId(2);
        let w3 = WindowId(3);

        engine.spawn_window(w1, "W1".to_string());
        engine.spawn_window(w2, "W2".to_string());
        engine.spawn_window(w3, "W3".to_string());

        // We have 3 windows. Maximum progress is saturating_sub(1) = 2.0.
        // Scroll forward: delta = 1.0. With sensitivity 0.5, progress should change by 0.5.
        engine.scroll_z(1.0);
        assert_eq!(engine.depth_scroll_progress, 0.5);

        // Scroll again by 4.0. Clamped to 2.0.
        engine.scroll_z(4.0);
        assert_eq!(engine.depth_scroll_progress, 2.0);

        // Check depth transforms when progress is 1.0.
        engine.depth_scroll_progress = 1.0;
        let transforms = engine.depth_transforms();

        // 3 windows, so 3 transforms.
        assert_eq!(transforms.len(), 3);

        // Under progress 1.0:
        // Distances:
        // W1 (idx 0): dist = 0 - 1 = -1.0. (zoomed past, scale = 1.5, opacity = 0.0)
        // W2 (idx 1): dist = 1 - 1 = 0.0. (active foreground, scale = 1.0, opacity = 1.0)
        // W3 (idx 2): dist = 2 - 1 = 1.0. (background, scale = 0.8, opacity = 0.6)
        //
        // Distance sorting descending order:
        // dist_b > dist_a =>
        // dist_a:
        // W3: 1.0 (furthest back, rendered first)
        // W2: 0.0 (active, rendered second)
        // W1: -1.0 (closest/zoomed past, rendered third)
        //
        // Let's verify the order.
        assert_eq!(transforms[0].0, w3); // W3 first
        assert_eq!(transforms[1].0, w2); // W2 second
        assert_eq!(transforms[2].0, w1); // W1 third

        // Verify active flag
        assert_eq!(transforms[0].1.is_active, false); // W3
        assert_eq!(transforms[1].1.is_active, true);  // W2
        assert_eq!(transforms[2].1.is_active, false); // W1

        // Verify transforms
        assert!((transforms[0].1.scale - 0.8).abs() < 1e-5);
        assert!((transforms[1].1.scale - 1.0).abs() < 1e-5);
        assert!((transforms[2].1.scale - 1.5).abs() < 1e-5);

        assert!((transforms[0].1.opacity - 0.6).abs() < 1e-5);
        assert!((transforms[1].1.opacity - 1.0).abs() < 1e-5);
        assert!((transforms[2].1.opacity - 0.0).abs() < 1e-5);
    }

    #[test]
    fn test_overview_mode() {
        let mut engine = LayoutEngine::new(1000.0, 600.0, 10.0, 20.0, 5);
        engine.tiling_mode = TilingMode::Overview;

        let w1 = WindowId(1);
        let w2 = WindowId(2);
        let w3 = WindowId(3);

        // Spawn w1 on active workspace 0
        // It is alone in the viewport/workspace, so it fullscreens.
        engine.spawn_window(w1, "W1".to_string());
        
        // Switch active workspace to 1 and spawn w2 and w3
        // Workspace 1 has 2 windows (occupied), so they tile next to each other at half-size.
        engine.active_workspace_idx = 1;
        engine.spawn_window(w2, "W2".to_string());
        engine.spawn_window(w3, "W3".to_string());

        // Verify window 1 rect (workspace 0) - should be fullscreen
        let rect1 = engine.get_window_rect(w1).unwrap();
        assert!((rect1.0 - 20.0).abs() < 1e-5);
        assert!((rect1.1 - 20.0).abs() < 1e-5);
        assert!((rect1.2 - 960.0).abs() < 1e-5);
        assert!((rect1.3 - 560.0).abs() < 1e-5);

        // Verify window 2 rect (workspace 1) - should be half size next
        let rect2 = engine.get_window_rect(w2).unwrap();
        assert!((rect2.0 - 20.0).abs() < 1e-5);
        assert!((rect2.1 - 708.88889).abs() < 1e-5);
        assert!((rect2.2 - 475.0).abs() < 1e-5);
        assert!((rect2.3 - 560.0).abs() < 1e-5);

        // Verify window 3 rect (workspace 1) - should be half size next
        let rect3 = engine.get_window_rect(w3).unwrap();
        assert!((rect3.0 - 505.0).abs() < 1e-5);
        assert!((rect3.1 - 708.88889).abs() < 1e-5);
        assert!((rect3.2 - 475.0).abs() < 1e-5);
        assert!((rect3.3 - 560.0).abs() < 1e-5);

        // Verify camera recentering for active workspace 1
        engine.recenter_camera(true);
        assert!((engine.viewport.target_y - 145.0).abs() < 1e-5);
        assert!((engine.viewport.target_x - (-165.875)).abs() < 1e-5);

        // Verify camera recentering for active workspace 0
        engine.active_workspace_idx = 0;
        engine.recenter_camera(true);
        assert!((engine.viewport.target_y - (-165.0)).abs() < 1e-5);
        assert!((engine.viewport.target_x - (-275.0)).abs() < 1e-5);
    }
}
