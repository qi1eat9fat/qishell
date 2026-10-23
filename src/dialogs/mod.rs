//! 对话框：新建/编辑主机、连接问答与 SFTP 冲突。
//!
//! 表面、标题、按钮与输入框的统一实现放在 `crate::components`；这里只保留
//! 各对话框自己的内容与交互。

pub mod host;
pub mod prompt;
pub mod sftp_conflict;
