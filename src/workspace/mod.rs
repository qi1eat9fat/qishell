//! 主窗口壳：顶栏、主机侧栏、标签栏与终端区。

mod connection_bar;
mod host_panel;
mod snippets_panel;
mod tab_bar;
mod title_bar;
mod wallpaper;
mod workspace_view;

pub use workspace_view::{Toast, Workspace, WorkspaceTestState};
