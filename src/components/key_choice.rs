//! 私钥选择：已导入的私钥列表 + 「浏览文件…」。
//!
//! 新建/编辑主机时既要能挑 `data/keys/` 里已经导入的私钥，也要能临时选一个
//! 别处的文件（`~/.ssh` 之类）。这里把它们放进同一个下拉框：已导入的按文件
//! 名排在前，最后一项固定是「浏览文件…」，选中它就去开系统文件对话框。

use gpui_kit::component::searchable_list::{SearchableGroup, SearchableListItem, SearchableVec};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::*;

use crate::ssh::vault::StoredKey;

/// 下拉里的一项。
#[derive(Clone, Debug, PartialEq)]
pub enum KeyChoice {
    /// `data/keys/` 里的一把私钥。
    Stored(StoredKey),
    /// 用户从别处选中的文件（`~/.ssh/id_ed25519` 之类）。保存时才收进
    /// `keys/`，在那之前它只存在于表单里，所以单独一项。
    File(std::path::PathBuf),
    /// 打开系统文件对话框另选一个文件。
    Browse,
}

impl KeyChoice {
    /// 选中项代表的私钥路径。`Browse` 没有路径——它只是个动作。
    pub fn path(&self) -> Option<&std::path::Path> {
        match self {
            Self::Stored(key) => Some(&key.path),
            Self::File(path) => Some(path),
            Self::Browse => None,
        }
    }

    /// 与表单里存的路径是否指同一个文件。
    pub fn matches_path(&self, path: &std::path::Path) -> bool {
        self.path().is_some_and(|own| own == path)
    }
}

impl SearchableListItem for KeyChoice {
    type Value = Self;

    fn title(&self) -> SharedString {
        match self {
            // 只显示文件名：完整路径太长，下拉里放不下，而且 `keys/` 是固定的。
            Self::Stored(key) => SharedString::from(key.name.clone()),
            Self::File(path) => SharedString::from(file_name(path)),
            Self::Browse => SharedString::from("浏览文件…"),
        }
    }

    fn render(&self, _: &mut Window, _: &mut App) -> impl IntoElement {
        match self {
            Self::Stored(key) => {
                let mut row = v_flex().gap(px(2.)).child(div().child(key.name.clone()));
                // 算法与指纹是辨认两把同名密钥的唯一依据，放在第二行。
                row = row.child(
                    div()
                        .text_size(crate::theme::TYPE_META)
                        .text_color(crate::theme::solid(crate::theme::SUBTLE))
                        .child(format!(
                            "{} · {}",
                            key.algorithm,
                            short_fingerprint(&key.fingerprint)
                        )),
                );
                row
            }
            Self::File(path) => v_flex()
                .gap(px(2.))
                .child(div().child(file_name(path)))
                .child(
                    div()
                        .text_size(crate::theme::TYPE_META)
                        .text_color(crate::theme::solid(crate::theme::SUBTLE))
                        .truncate()
                        .child(path.to_string_lossy().into_owned()),
                ),
            Self::Browse => h_flex()
                .gap(px(6.))
                .child(gpui_kit::component::Icon::new(
                    gpui_kit::assets::IconName::FolderOpen,
                ))
                .child("浏览文件…"),
        }
    }

    fn value(&self) -> &Self::Value {
        self
    }

    fn matches(&self, query: &str) -> bool {
        let query = query.to_lowercase();
        match self {
            Self::Stored(key) => {
                key.name.to_lowercase().contains(&query)
                    || key.algorithm.to_lowercase().contains(&query)
                    || key.fingerprint.to_lowercase().contains(&query)
            }
            Self::File(path) => path.to_string_lossy().to_lowercase().contains(&query),
            // 「浏览文件…」永远留在列表里，搜索时也找得到。
            Self::Browse => "浏览文件".contains(&query),
        }
    }
}

fn file_name(path: &std::path::Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

/// `SHA256:abcd…wxyz`：指纹太长，列表里只留头尾。
fn short_fingerprint(fingerprint: &str) -> String {
    let body = fingerprint.strip_prefix("SHA256:").unwrap_or(fingerprint);
    let characters: Vec<char> = body.chars().collect();
    if characters.len() <= 16 {
        return fingerprint.to_string();
    }
    let head: String = characters[..8].iter().collect();
    let tail: String = characters[characters.len() - 6..].iter().collect();
    format!("SHA256:{head}…{tail}")
}

/// 下拉的数据源：分组标题是「已导入的私钥」，末尾附「浏览文件…」。
///
/// 没有已导入的私钥时不摆空分组，只留「浏览文件…」。
pub fn key_choices(keys: Vec<StoredKey>) -> SearchableVec<SearchableGroup<KeyChoice>> {
    let mut stored = SearchableGroup::new("已导入的私钥");
    stored = stored.items(keys.into_iter().map(KeyChoice::Stored));
    if stored.items.is_empty() {
        return SearchableVec::new(vec![SearchableGroup::new("").item(KeyChoice::Browse)]);
    }
    SearchableVec::new(vec![
        stored,
        SearchableGroup::new("").item(KeyChoice::Browse),
    ])
}

/// 读 `data/keys/` 里已有的私钥。目录不存在时返回空列表。
pub fn stored_keys() -> Vec<StoredKey> {
    crate::ssh::vault::list_keys(&crate::app::paths::keys_dir())
}

/// 表单里存的路径对应哪一项：在 `keys/` 里就是已导入的密钥，否则算「别处
/// 的文件」。这样编辑一台用着 `~/.ssh/id_ed25519` 的主机时，下拉不会显示成
/// 「浏览文件…」而看不出当前选了什么。
pub fn choice_for_path(keys: &[StoredKey], path: &std::path::Path) -> KeyChoice {
    match keys.iter().find(|key| key.path == path) {
        Some(key) => KeyChoice::Stored(key.clone()),
        None => KeyChoice::File(path.to_path_buf()),
    }
}

#[cfg(test)]
mod tests {
    use super::{KeyChoice, short_fingerprint};
    use gpui_kit::component::searchable_list::{
        SearchableListDelegate as _, SearchableListItem as _,
    };

    #[test]
    fn a_long_fingerprint_is_shortened_in_the_middle() {
        let long = "SHA256:abcdefghijklmnopqrstuvwxyz0123456789";
        let short = short_fingerprint(long);
        assert!(short.starts_with("SHA256:abcdefgh"));
        assert!(short.ends_with("56789"));
        assert!(short.len() < long.len());
    }

    #[test]
    fn a_short_fingerprint_is_left_alone() {
        assert_eq!(short_fingerprint("SHA256:abc"), "SHA256:abc");
    }

    #[test]
    fn browse_is_found_by_its_own_words() {
        // 「浏览文件…」不能被搜索藏起来，否则用户搜什么都找不到它。
        assert!(KeyChoice::Browse.matches("浏览"));
        assert!(KeyChoice::Browse.matches("文件"));
        assert!(!KeyChoice::Browse.matches("id_ed25519"));
    }

    #[test]
    fn a_path_outside_the_key_dir_is_shown_by_its_file_name() {
        let choice = KeyChoice::File(std::path::PathBuf::from("/home/me/.ssh/id_ed25519"));
        assert_eq!(choice.title(), "id_ed25519");
        assert!(choice.matches("id_ed25519"));
        // 完整路径也参与搜索：用户记得的是路径，不是文件名。
        assert!(choice.matches(".ssh"));
    }

    #[test]
    fn the_key_dir_always_ends_with_browse() {
        // 没有已导入的密钥时也得有「浏览文件…」，否则新建主机没法选私钥。
        let empty = super::key_choices(Vec::new());
        assert_eq!(empty.position(&KeyChoice::Browse).map(|ix| ix.row), Some(0));

        let keys = vec![super::StoredKey {
            name: "id_ed25519".into(),
            path: std::path::PathBuf::from("/data/keys/id_ed25519"),
            algorithm: "ssh-ed25519".into(),
            fingerprint: "SHA256:abc".into(),
        }];
        let choices = super::key_choices(keys.clone());
        // 已导入的排在「已导入的私钥」组里，浏览文件在最后一组。
        let stored = choices
            .position(&KeyChoice::Stored(keys[0].clone()))
            .expect("the imported key is listed");
        let browse = choices
            .position(&KeyChoice::Browse)
            .expect("browse is always listed");
        assert_eq!(stored.section, 0);
        assert_eq!(browse.section, 1);
    }
}
