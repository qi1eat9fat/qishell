//! 主机的内存存储：**内存是唯一事实来源**，每次改动写穿到 SQLite。
//!
//! 写库失败时内存照样改，store 发 `PersistFailed`，工作区弹一条提示；
//! 绝不静默吞掉。消费方 `cx.observe(&store, ..)`。

use std::sync::Arc;

use gpui_kit::*;

use crate::secrets::{SecretRef, SharedSecretStore};

use super::{
    Host, HostConnection, HostDatabase, HostDraft, HostId, HostLogin, PublicId,
    database::now_seconds,
};

#[derive(Clone, Debug)]
pub enum HostStoreEvent {
    /// 写库失败：改动还在内存里，界面要说一声。
    PersistFailed(SharedString),
}

impl EventEmitter<HostStoreEvent> for HostStore {}

/// 主机列表。分组的支持留到以后，现在是一个平铺列表。
pub struct HostStore {
    hosts: Vec<Host>,
    next_host_id: u64,
    database: Option<HostDatabase>,
    secrets: SharedSecretStore,
}

impl HostStore {
    /// 空库，什么都不写。
    pub fn empty() -> Self {
        Self {
            hosts: Vec::new(),
            next_host_id: 1,
            database: None,
            secrets: Arc::new(crate::secrets::NoSecretStore),
        }
    }

    /// 从数据库读出全部主机。
    ///
    /// 顺便把从别处拷来的库里那些旧机器留下的私钥绝对路径换成相对路径：
    /// 文件夹搬过来了，前缀变了，尾巴还能对上就改过来，下次拷走也还是对的。
    pub fn load(database: HostDatabase) -> rusqlite::Result<Self> {
        let mut data = database.load()?;
        let adopted = adopt_key_paths(&mut data.hosts);
        for host in &adopted {
            // 写不回去也不影响这次使用，下次打开再试一次。
            let _ = database.update_host(host);
        }
        let next_host_id = data
            .hosts
            .iter()
            .map(|host| host.id.0 + 1)
            .max()
            .unwrap_or(1);
        Ok(Self {
            hosts: data.hosts,
            next_host_id,
            database: Some(database),
            secrets: Arc::new(crate::secrets::NoSecretStore),
        })
    }

    /// 装一个秘密库。没装之前密码不会存。
    pub fn with_secrets(mut self, secrets: SharedSecretStore) -> Self {
        self.secrets = secrets;
        self
    }

    pub fn secrets(&self) -> SharedSecretStore {
        self.secrets.clone()
    }

    // —— 查询 ——

    pub fn hosts(&self) -> impl Iterator<Item = &Host> {
        self.hosts.iter()
    }

    pub fn host(&self, id: HostId) -> Option<&Host> {
        self.hosts.iter().find(|host| host.id == id)
    }

    /// 对外的 `PublicId` 有谁在用（生成新的时避开）。
    fn public_ids(&self) -> Vec<PublicId> {
        self.hosts
            .iter()
            .map(|host| host.public_id.clone())
            .collect()
    }

    /// 这台主机打开连接时的快照。
    pub fn login(&self, id: HostId) -> Option<HostLogin> {
        self.host(id).map(HostLogin::of)
    }

    pub fn connection(&self, id: HostId) -> Option<HostConnection> {
        self.login(id).map(HostConnection::Ssh)
    }

    // —— 写入 ——

    /// 新建一台主机，返回它的 id。秘密单独经 `save_secret` 写。
    pub fn insert(&mut self, draft: HostDraft, cx: &mut Context<Self>) -> HostId {
        let id = self.insert_unnotified(draft);
        self.persist(Change::NewHost, cx, |database| {
            let host = self.host(id).expect("刚插入");
            database.insert_host(host)
        });
        cx.notify();
        id
    }

    /// 只改内存的一半，给测试与加载用。
    pub fn insert_unnotified(&mut self, draft: HostDraft) -> HostId {
        let id = HostId(self.next_host_id);
        self.next_host_id += 1;
        let mut host = Host::new(id, draft);
        host.public_id = PublicId::generate_unused(&self.public_ids());
        // 新主机排在最前：`sort_order` 越小越靠前，取当前最小值再减一。
        // 这样老库里的存量顺序不用改，新加的插到最上面。
        host.sort_order = self
            .hosts
            .iter()
            .map(|host| host.sort_order)
            .min()
            .unwrap_or(1)
            - 1;
        self.hosts.insert(0, host);
        id
    }

    /// 改一台主机。地址、端口或用户名变了，旧的密码条目会作废（新端点要
    /// 重新输入）。
    pub fn update(&mut self, id: HostId, draft: HostDraft, cx: &mut Context<Self>) -> bool {
        let Some(previous) = self.host(id).cloned() else {
            return false;
        };
        if !self.update_unnotified(id, draft) {
            return false;
        }
        // 端点变了，旧条目的密码不再属于这台主机：清掉，免得越攒越多。
        let updated = self.host(id).cloned();
        if let Some(updated) = updated
            && endpoint_changed(&previous, &updated)
        {
            let stale = HostLogin::of(&previous).password_secret();
            self.forget_secret(stale, cx);
        }
        self.persist(Change::EditedHost, cx, |database| {
            let host = self.host(id).expect("刚改过");
            database.update_host(host)
        });
        cx.notify();
        true
    }

    /// 只改内存的一半。
    pub fn update_unnotified(&mut self, id: HostId, draft: HostDraft) -> bool {
        let Some(host) = self.hosts.iter_mut().find(|host| host.id == id) else {
            return false;
        };
        host.name = draft.name;
        host.address = draft.address;
        host.port = draft.port;
        host.username = draft.username;
        host.auth = draft.auth;
        host.key_path = draft.key_path;
        host.notes = draft.notes;
        true
    }

    /// 删除一台主机。已经打开的相关标签不会被关掉。
    pub fn remove(&mut self, id: HostId, cx: &mut Context<Self>) -> bool {
        let Some(index) = self.hosts.iter().position(|host| host.id == id) else {
            return false;
        };
        let removed = self.hosts.remove(index);
        self.forget_secret(HostLogin::of(&removed).password_secret(), cx);
        self.persist(Change::RemovedHost, cx, |database| database.remove_host(id));
        cx.notify();
        true
    }

    /// 把 `id` 挪到列表第 `index` 位（拖动排序用），返回是否真的动了。
    ///
    /// 拖完立刻重排 `sort_order` 并整份写库：只改内存不落盘，重启后顺序会
    /// 退回旧值。
    pub fn move_host(&mut self, id: HostId, index: usize, cx: &mut Context<Self>) -> bool {
        let moved = self.move_host_unnotified(id, index);
        if moved {
            self.persist(Change::Order, cx, |database| {
                database.save_order(&self.hosts)
            });
            cx.notify();
        }
        moved
    }

    /// 只改内存的一半，给测试用。
    pub fn move_host_unnotified(&mut self, id: HostId, index: usize) -> bool {
        let Some(from) = self.hosts.iter().position(|host| host.id == id) else {
            return false;
        };
        let index = index.min(self.hosts.len().saturating_sub(1));
        if from == index {
            return false;
        }
        let host = self.hosts.remove(from);
        self.hosts.insert(index, host);
        self.renumber();
        true
    }

    /// 按当前位置重写 `sort_order`，保持「越小越靠前」。
    fn renumber(&mut self) {
        for (index, host) in self.hosts.iter_mut().enumerate() {
            host.sort_order = index as i64;
        }
    }

    /// 把顺序整份存下。
    pub fn save_order(&mut self, cx: &mut Context<Self>) {
        self.renumber();
        self.persist(Change::Order, cx, |database| {
            database.save_order(&self.hosts)
        });
    }

    /// 记下这台主机刚连上，「最近连接」据此排序。
    pub fn record_connection(&mut self, id: HostId, cx: &mut Context<Self>) {
        self.record_connection_unnotified(id);
        self.persist(Change::Connected, cx, |database| {
            database.touch_connected(id, now_seconds())
        });
        cx.notify();
    }

    /// 只改内存的一半。
    pub fn record_connection_unnotified(&mut self, id: HostId) {
        if let Some(host) = self.hosts.iter_mut().find(|host| host.id == id) {
            host.last_connected_at = Some(now_seconds());
        }
    }

    // —— 秘密 ——

    /// 把一条秘密写进秘密库。不等待结果，失败只记日志。
    pub fn save_secret(
        &self,
        reference: SecretRef,
        secret: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let secrets = self.secrets.clone();
        cx.background_spawn(async move {
            if let Err(error) = write_secret(&*secrets, reference, secret) {
                eprintln!("qishell: 无法保存秘密: {error}");
            }
        })
        .detach();
    }

    /// 同步写一条秘密并等结果。**阻塞**：只在后台线程上调用。
    pub fn write_secret(&self, reference: SecretRef, secret: Option<String>) -> Result<(), String> {
        write_secret(&*self.secrets, reference, secret)
    }

    /// 删掉一条密码。
    fn forget_secret(&self, reference: SecretRef, cx: &mut Context<Self>) {
        let secrets = self.secrets.clone();
        cx.background_spawn(async move {
            if let Err(error) = secrets.delete(&reference) {
                eprintln!("qishell: 无法从秘密库删除条目: {error}");
            }
        })
        .detach();
    }

    // —— 写库 ——

    fn persist(
        &self,
        change: Change,
        cx: &mut Context<Self>,
        write: impl FnOnce(&HostDatabase) -> rusqlite::Result<()>,
    ) {
        let Some(database) = &self.database else {
            return;
        };
        if let Err(error) = write(database) {
            let message = change.not_saved(&error);
            cx.emit(HostStoreEvent::PersistFailed(message));
        }
    }
}

/// 把旧记录里写死的私钥路径换成当前数据目录下的相对路径，返回改过的那些。
fn adopt_key_paths(hosts: &mut [Host]) -> Vec<Host> {
    let dir = crate::app::paths::data_dir();
    let mut changed = Vec::new();
    for host in hosts.iter_mut() {
        let Some(path) = host.key_path.as_deref() else {
            continue;
        };
        let Some(adopted) = crate::app::paths::adopt(&dir, path) else {
            continue;
        };
        host.key_path = Some(adopted);
        changed.push(host.clone());
    }
    changed
}

/// 端点的三个部分有没有变（变了密码条目的归属就不同了）。
fn endpoint_changed(before: &Host, after: &Host) -> bool {
    before.address != after.address
        || before.port != after.port
        || before.username != after.username
}

/// 同步写一条秘密。调用方负责放到后台线程上。
fn write_secret(
    secrets: &dyn crate::secrets::SecretStore,
    reference: SecretRef,
    secret: Option<String>,
) -> Result<(), String> {
    let result = match secret {
        Some(secret) => secrets.set(&reference, &secret),
        None => secrets.delete(&reference),
    };
    result.map_err(|error| format!("{error}"))
}

/// 每次写库动作一个变体，只为了说清楚是哪一步没保存。
#[derive(Clone, Copy, Debug)]
enum Change {
    NewHost,
    EditedHost,
    RemovedHost,
    Connected,
    Order,
}

impl Change {
    fn not_saved(self, error: &rusqlite::Error) -> SharedString {
        let what = match self {
            Change::NewHost => "新建的主机",
            Change::EditedHost => "主机的改动",
            Change::RemovedHost => "删除的主机",
            Change::Connected => "连接记录",
            Change::Order => "主机顺序",
        };
        SharedString::from(format!("{what}没能保存：{error}"))
    }
}

/// 几台主机的最小集合，给测试与界面预览用。
#[doc(hidden)]
pub fn seed() -> HostStore {
    let mut store = HostStore::empty();
    for (name, address, port) in [
        ("Atlas production", "10.24.0.18", 22),
        ("Aurora staging", "10.24.1.42", 22),
        ("Edge relay", "172.16.4.7", 2202),
    ] {
        store.insert_unnotified(HostDraft::new(
            name,
            address,
            port,
            "ops",
            super::AuthKind::Password,
        ));
    }
    // 新插入的排在最前，这里倒回插入顺序，预览与测试里仍是 Atlas 在最上；
    // id 不跟着倒，序号与顺序各自独立。
    store.hosts.reverse();
    store.renumber();
    store
}

#[cfg(test)]
mod tests {
    // 只导入被测类型：`use super::*` 会把 `gpui_kit::*` 再展开一遍，
    // 编译器的 glob 展开在递归上限下会爆栈。
    use super::{HostDraft, HostStore, seed};
    use crate::host::AuthKind;

    fn names(store: &HostStore) -> Vec<String> {
        store.hosts().map(|host| host.name.clone()).collect()
    }

    #[test]
    fn a_new_host_goes_to_the_top() {
        let mut store = seed();
        store.insert_unnotified(HostDraft::new(
            "Fresh",
            "10.0.0.9",
            22,
            "ops",
            AuthKind::Password,
        ));
        assert_eq!(
            names(&store),
            ["Fresh", "Atlas production", "Aurora staging", "Edge relay"]
        );
        // 排在最前的主机不一定是序号最小的那个：序号只保证大小关系。
        let orders: Vec<i64> = store.hosts().map(|host| host.sort_order).collect();
        assert!(orders[0] < orders[1] && orders[1] <= orders[2] && orders[2] <= orders[3]);
    }

    #[test]
    fn moving_a_host_reorders_and_renumbers() {
        let mut store = seed();
        let edge = store
            .hosts()
            .find(|host| host.name == "Edge relay")
            .map(|host| host.id)
            .unwrap();
        assert!(store.move_host_unnotified(edge, 0));

        assert_eq!(
            names(&store),
            ["Edge relay", "Atlas production", "Aurora staging"]
        );
        let orders: Vec<i64> = store.hosts().map(|host| host.sort_order).collect();
        assert_eq!(orders, [0, 1, 2]);
    }
}
