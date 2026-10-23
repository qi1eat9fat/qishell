//! 用户命令。按钮、菜单和快捷键都派发这里的动作。

gpui_kit::actions!(
    qishell,
    [
        /// 打开新建主机对话框。
        NewHost,
        /// 新建一个终端标签。
        NewTab,
        /// 打开设置标签（单例）。
        OpenSettings,
        /// 关闭当前标签。
        CloseActiveTab,
        /// 连接当前选中的主机。
        ConnectSelected,
        /// 切换到下一个标签。
        NextTab,
        /// 切换到上一个标签。
        PreviousTab,
        /// 关闭右键菜单或当前弹窗。
        DismissOverlay,
        /// 焦点移到下一个可聚焦控件。
        FocusNext,
        /// 焦点移到上一个可聚焦控件。
        FocusPrev,
        /// 复制终端选中的文字。
        CopyTerminal,
        /// 把剪贴板内容粘贴到终端。
        PasteTerminal,
        /// 选中终端里的全部内容。
        SelectAllTerminal,
        /// 清空终端屏幕。
        ClearTerminal,
        /// 在终端里查找。
        FindInTerminal,
        /// 查找下一个匹配。
        FindNextInTerminal,
        /// 查找上一个匹配。
        FindPreviousInTerminal,
        /// 关闭终端查找条。
        DismissTerminalFind,
    ]
);
