//! 命令片段：顺序与持久化（应用数据目录下的 `snippets.json`）。

use std::path::PathBuf;

use gpui_kit::SharedString;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Snippet {
    pub id: u64,
    pub name: String,
    pub command: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct SnippetFile {
    #[serde(default)]
    snippets: Vec<Snippet>,
}

pub struct SnippetStore {
    snippets: Vec<Snippet>,
    next_id: u64,
    path: Option<PathBuf>,
}

impl SnippetStore {
    /// 测试与预览用：不落盘，带几条示例片段。
    pub fn in_memory() -> Self {
        Self {
            snippets: seeded(),
            next_id: 4,
            path: None,
        }
    }

    /// 从 `path` 读取；文件不存在时写入默认片段，损坏时保留原文件并提示。
    pub fn load(path: PathBuf) -> (Self, Option<SharedString>) {
        match std::fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice::<SnippetFile>(&bytes) {
                Ok(file) => {
                    let next_id = file
                        .snippets
                        .iter()
                        .map(|snippet| snippet.id)
                        .max()
                        .unwrap_or(0)
                        + 1;
                    (
                        Self {
                            snippets: file.snippets,
                            next_id,
                            path: Some(path),
                        },
                        None,
                    )
                }
                Err(error) => (
                    Self {
                        snippets: seeded(),
                        next_id: 4,
                        path: Some(path),
                    },
                    Some(SharedString::from(format!(
                        "代码片段文件读不出，已用默认内容：{error}"
                    ))),
                ),
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let store = Self {
                    snippets: seeded(),
                    next_id: 4,
                    path: Some(path),
                };
                let _ = store.save();
                (store, None)
            }
            Err(error) => (
                Self::in_memory(),
                Some(SharedString::from(format!("无法读取代码片段：{error}"))),
            ),
        }
    }

    pub fn snippets(&self) -> &[Snippet] {
        &self.snippets
    }

    pub fn get(&self, id: u64) -> Option<&Snippet> {
        self.snippets.iter().find(|snippet| snippet.id == id)
    }

    /// 新建或更新一条片段；返回落盘失败的原因。
    pub fn upsert(
        &mut self,
        id: Option<u64>,
        name: String,
        command: String,
    ) -> Result<(), SharedString> {
        match id {
            Some(id) => {
                if let Some(snippet) = self.snippets.iter_mut().find(|snippet| snippet.id == id) {
                    snippet.name = name;
                    snippet.command = command;
                }
            }
            None => {
                let id = self.next_id;
                self.next_id += 1;
                self.snippets.push(Snippet { id, name, command });
            }
        }
        self.save()
    }

    pub fn remove(&mut self, id: u64) -> Result<(), SharedString> {
        self.snippets.retain(|snippet| snippet.id != id);
        self.save()
    }

    fn save(&self) -> Result<(), SharedString> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        let file = SnippetFile {
            snippets: self.snippets.clone(),
        };
        let json = serde_json::to_vec_pretty(&file)
            .map_err(|error| SharedString::from(format!("代码片段无法序列化：{error}")))?;
        if let Some(parent) = path.parent()
            && let Err(error) = std::fs::create_dir_all(parent)
        {
            return Err(SharedString::from(format!("无法创建数据目录：{error}")));
        }
        std::fs::write(path, json)
            .map_err(|error| SharedString::from(format!("代码片段没能保存：{error}")))
    }
}

fn seeded() -> Vec<Snippet> {
    vec![
        Snippet {
            id: 1,
            name: "查看磁盘占用".into(),
            command: "df -h".into(),
        },
        Snippet {
            id: 2,
            name: "查看内存".into(),
            command: "free -h".into(),
        },
        Snippet {
            id: 3,
            name: "查看监听端口".into(),
            command: "ss -tulnp".into(),
        },
    ]
}
