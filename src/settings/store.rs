//! 设置的存储：内存说了算，改动写穿到 `settings.json`（整文件替换）。

use std::path::PathBuf;

use gpui_kit::*;

use crate::model::AppSettings;

use super::model::SettingsFile;

#[derive(Clone, Debug)]
pub enum SettingsStoreEvent {
    /// 写不进去：内存里的改动照样保留，界面弹一条提示。
    PersistFailed(SharedString),
}

impl EventEmitter<SettingsStoreEvent> for SettingsStore {}

pub struct SettingsStore {
    settings: AppSettings,
    file: SettingsFile,
    path: Option<PathBuf>,
}

impl SettingsStore {
    /// 不落盘的存储，给测试与无数据目录时用。
    pub fn in_memory() -> Self {
        let file = SettingsFile::default();
        Self {
            settings: file.to_app(),
            file,
            path: None,
        }
    }

    /// 从 `path` 读设置；文件缺失或读不出就用默认值，读不出时**保持文件
    /// 原样**，不覆盖用户的内容。
    pub fn load(path: PathBuf) -> (Self, Option<SharedString>) {
        let (file, problem) = match std::fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice::<SettingsFile>(&bytes) {
                Ok(file) => (file.normalized(), None),
                Err(error) => (
                    SettingsFile::default(),
                    Some(SharedString::from(format!(
                        "设置文件读不出，已用默认值：{error}"
                    ))),
                ),
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                (SettingsFile::default(), None)
            }
            Err(error) => {
                eprintln!("qishell: 无法读取设置: {error}");
                (SettingsFile::default(), None)
            }
        };
        let settings = file.to_app();
        let mut store = Self {
            settings,
            file,
            path: Some(path),
        };
        // 从别处拷来的设置里，壁纸可能是旧机器留下的绝对路径。
        store.adopt_portable_paths();
        (store, problem)
    }

    /// 把设置里那些指向数据目录如今位置的路径改成相对路径。
    ///
    /// 只在加载时做一次，改了就落盘：下次拷走文件夹，里面的路径已经是对的了。
    fn adopt_portable_paths(&mut self) {
        let dir = crate::app::paths::data_dir();
        let before = self.settings.appearance.wallpaper.path.clone();
        let adopted = before.as_deref().and_then(|path| {
            let path = path.to_string_lossy();
            crate::app::paths::adopt(&dir, &path).map(PathBuf::from)
        });
        if let Some(adopted) = adopted {
            self.settings.appearance.wallpaper.path = Some(adopted);
            self.file = SettingsFile::from_app(&self.settings);
            if let Some(path) = self.path.clone()
                && let Err(error) = self.write(&path)
            {
                eprintln!("qishell: 无法改写壁纸路径: {error}");
            }
        }
    }

    pub fn settings(&self) -> &AppSettings {
        &self.settings
    }

    pub fn appearance(&self) -> &crate::model::Appearance {
        &self.settings.appearance
    }

    pub fn terminal(&self) -> &crate::model::TerminalSettings {
        &self.settings.terminal
    }

    /// 改设置并通知观察者：先改内存，再写文件，写失败发事件。
    pub fn update(&mut self, cx: &mut Context<Self>, change: impl FnOnce(&mut AppSettings)) {
        change(&mut self.settings);
        self.file = SettingsFile::from_app(&self.settings);
        if let Some(path) = &self.path
            && let Err(error) = self.write(path)
        {
            let message = SharedString::from(format!("设置没能保存：{error}"));
            cx.emit(SettingsStoreEvent::PersistFailed(message));
        }
        cx.notify();
    }

    /// 改了但没通知（测试与加载用）。没变化时返回 `None`。
    pub fn update_unnotified(
        &mut self,
        change: impl FnOnce(&mut AppSettings),
    ) -> Option<std::io::Result<()>> {
        let before = self.settings.clone();
        change(&mut self.settings);
        if self.settings == before {
            return None;
        }
        self.file = SettingsFile::from_app(&self.settings);
        self.path.as_ref().map(|path| self.write(path))
    }

    /// 整文件替换：写临时文件再改名，坏在半路也不会留下半份设置。
    fn write(&self, path: &std::path::Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let temporary = path.with_extension("json.tmp");
        std::fs::write(&temporary, serde_json::to_vec_pretty(&self.file)?)?;
        std::fs::rename(temporary, path)
    }
}
