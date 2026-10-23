//! 主机存储背后的 SQLite 文件。
//!
//! 这里全是同步的，跑在界面线程上：写入是用户刚确认的单个表单，不是数据流。
//! `HostStore` 以内存为准，把每次改动镜像到这里，所以写失败只是少了持久化，
//! 用户的编辑不会丢。

use std::path::Path;

use rusqlite::{Connection, params};

use super::{AuthKind, Host, HostId, PublicId};

/// SQLite 只有 `i64`，所以 `u64` 的 id 过界时按 `i64` 转。它们是从 1 起的
/// 计数器，离会丢信息的范围很远。
fn to_sql(id: u64) -> i64 {
    id as i64
}

fn from_sql(id: i64) -> u64 {
    id as u64
}

/// schema 的版本，存在 `PRAGMA user_version` 里。这是新应用，只有第一个
/// 版本，没有迁移。
const SCHEMA_VERSION: i64 = 1;

/// 新库拿到的一整份 schema。表都是 `STRICT`：类型不对的值会被拒绝，而不是
/// 存进去。
///
/// - **hosts** 记怎么登录、连到哪里：`auth` 是 `password` 或 `key`；密钥
///   登录必须给出 `key_path`，密码登录则不给（表上的 CHECK 守着这条）。
/// - 秘密永远不在这里：密码在应用目录的加密秘密库，私钥只记路径。
/// - `public_id` 可能缺失（手改出来的行），下次打开时补上。
/// - `last_connected_at` 由连接写入，不由主机表单写。
const SCHEMA: &str = "\
CREATE TABLE hosts (
    id                INTEGER PRIMARY KEY,
    public_id         TEXT UNIQUE,
    sort_order        INTEGER NOT NULL DEFAULT 0,
    name              TEXT NOT NULL CHECK (name <> ''),
    address           TEXT NOT NULL CHECK (address <> ''),
    port              INTEGER NOT NULL CHECK (port BETWEEN 1 AND 65535),
    auth              TEXT NOT NULL CHECK (auth IN ('password', 'key')),
    username          TEXT NOT NULL,
    key_path          TEXT,
    notes             TEXT NOT NULL DEFAULT '',
    last_connected_at INTEGER,
    CHECK ((auth = 'key') = (key_path IS NOT NULL))
) STRICT;
CREATE INDEX hosts_sort_order ON hosts(sort_order);
";

/// 从库里读出来的全部内容。
#[derive(Debug, Default)]
pub struct StoredData {
    pub hosts: Vec<Host>,
}

/// 主机的 SQLite 文件。
pub struct HostDatabase {
    /// 只给测试直接查库。
    #[doc(hidden)]
    pub connection: Connection,
}

impl HostDatabase {
    /// 打开（必要时新建）`path`，建好 schema 并补齐缺失的对外 id。
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let connection = Connection::open(path)?;
        Self::prepare(connection)
    }

    #[doc(hidden)]
    pub fn in_memory() -> rusqlite::Result<Self> {
        Self::prepare(Connection::open_in_memory()?)
    }

    #[doc(hidden)]
    pub fn prepare(connection: Connection) -> rusqlite::Result<Self> {
        // 外键要显式打开。
        connection.pragma_update(None, "foreign_keys", "ON")?;
        // WAL：读写不互相挡，崩溃后也更容易恢复。
        connection.pragma_update(None, "journal_mode", "WAL")?;
        let database = Self { connection };
        database.migrate()?;
        database.fill_missing_public_ids()?;
        Ok(database)
    }

    /// 建库或升版本。这是新应用，第一次打开就是整段 schema。
    fn migrate(&self) -> rusqlite::Result<()> {
        let version: i64 = self
            .connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))?;
        if version == SCHEMA_VERSION {
            return Ok(());
        }
        if version > SCHEMA_VERSION {
            // 库比程序新：不动文件，直接报错，免得旧版本写坏新结构。
            return Err(rusqlite::Error::InvalidQuery);
        }
        self.connection.execute_batch(SCHEMA)?;
        self.connection
            .pragma_update(None, "user_version", SCHEMA_VERSION)?;
        Ok(())
    }

    /// 手改出来的行可能没有 `public_id`，补上随机的。
    #[doc(hidden)]
    pub fn fill_missing_public_ids(&self) -> rusqlite::Result<()> {
        let mut statement = self
            .connection
            .prepare("SELECT id FROM hosts WHERE public_id IS NULL OR public_id = ''")?;
        let ids: Vec<i64> = statement
            .query_map([], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        drop(statement);
        for id in ids {
            self.connection.execute(
                "UPDATE hosts SET public_id = ?1 WHERE id = ?2",
                params![PublicId::generate().as_str(), id],
            )?;
        }
        Ok(())
    }

    /// 读出全部主机。
    pub fn load(&self) -> rusqlite::Result<StoredData> {
        let mut statement = self.connection.prepare(
            "SELECT id, public_id, sort_order, name, address, port, auth, username, key_path,
                    notes, last_connected_at
             FROM hosts ORDER BY sort_order, id",
        )?;
        let rows = statement.query_map([], |row| {
            let auth: String = row.get(6)?;
            let public_id: Option<String> = row.get(1)?;
            Ok(Host {
                id: HostId(from_sql(row.get(0)?)),
                public_id: public_id
                    .as_deref()
                    .and_then(PublicId::from_stored)
                    .unwrap_or_else(PublicId::generate),
                sort_order: row.get(2)?,
                name: row.get(3)?,
                address: row.get(4)?,
                port: row.get(5)?,
                auth: AuthKind::from_stored(&auth).unwrap_or_default(),
                username: row.get(7)?,
                key_path: row.get(8)?,
                notes: row.get(9)?,
                last_connected_at: row.get::<_, Option<i64>>(10)?.map(from_sql),
            })
        })?;
        Ok(StoredData {
            hosts: rows.collect::<rusqlite::Result<_>>()?,
        })
    }

    pub fn insert_host(&self, host: &Host) -> rusqlite::Result<()> {
        self.connection.execute(
            "INSERT INTO hosts (id, public_id, sort_order, name, address, port, auth, username,
                                key_path, notes, last_connected_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                to_sql(host.id.0),
                host.public_id.as_str(),
                host.sort_order,
                host.name,
                host.address,
                host.port,
                host.auth.as_str(),
                host.username,
                host.key_path,
                host.notes,
                host.last_connected_at.map(to_sql),
            ],
        )?;
        Ok(())
    }

    pub fn update_host(&self, host: &Host) -> rusqlite::Result<()> {
        self.connection.execute(
            "UPDATE hosts SET sort_order = ?2, name = ?3, address = ?4, port = ?5, auth = ?6,
                              username = ?7, key_path = ?8, notes = ?9, last_connected_at = ?10
             WHERE id = ?1",
            params![
                to_sql(host.id.0),
                host.sort_order,
                host.name,
                host.address,
                host.port,
                host.auth.as_str(),
                host.username,
                host.key_path,
                host.notes,
                host.last_connected_at.map(to_sql),
            ],
        )?;
        Ok(())
    }

    pub fn remove_host(&self, id: HostId) -> rusqlite::Result<()> {
        self.connection
            .execute("DELETE FROM hosts WHERE id = ?1", params![to_sql(id.0)])?;
        Ok(())
    }

    /// 一次写下整个列表的顺序。
    pub fn save_order(&self, hosts: &[Host]) -> rusqlite::Result<()> {
        for host in hosts {
            self.connection.execute(
                "UPDATE hosts SET sort_order = ?2 WHERE id = ?1",
                params![to_sql(host.id.0), host.sort_order],
            )?;
        }
        Ok(())
    }

    /// 记下这台主机刚连上。
    pub fn touch_connected(&self, id: HostId, at: u64) -> rusqlite::Result<()> {
        self.connection.execute(
            "UPDATE hosts SET last_connected_at = ?2 WHERE id = ?1",
            params![to_sql(id.0), to_sql(at)],
        )?;
        Ok(())
    }
}

/// 现在的时间（秒），`last_connected_at` 用。
pub fn now_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::HostDraft;

    #[test]
    fn schema_never_contains_secret_columns() {
        let lowered = SCHEMA.to_lowercase();
        assert!(!lowered.contains("secret"));
        assert!(!lowered.contains("passphrase"));
        assert!(!lowered.contains("password text"));
    }

    #[test]
    fn a_host_survives_a_database_round_trip() {
        let database = HostDatabase::in_memory().unwrap();
        let mut draft = HostDraft::new("atlas", "10.0.0.1", 2202, "ops", AuthKind::Password);
        draft.notes = "生产".into();
        let mut host = Host::new(HostId(7), draft.validated().unwrap());
        host.public_id = PublicId::generate();
        host.sort_order = 3;
        database.insert_host(&host).unwrap();

        let loaded = database.load().unwrap().hosts;
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0], host);
    }

    #[test]
    fn deleting_a_host_removes_it() {
        let database = HostDatabase::in_memory().unwrap();
        let draft = HostDraft::new("atlas", "10.0.0.1", 22, "ops", AuthKind::Password);
        let host = Host::new(HostId(1), draft.validated().unwrap());
        database.insert_host(&host).unwrap();
        database.remove_host(HostId(1)).unwrap();
        assert!(database.load().unwrap().hosts.is_empty());
    }
}
