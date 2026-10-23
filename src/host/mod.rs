//! 主机：模型、SQLite、内存存储，以及登录快照。

mod database;
mod login;
mod model;
mod store;

pub use database::{HostDatabase, StoredData, now_seconds};
pub use login::{HostConnection, HostLogin, LoginMethod};
pub use model::{
    AuthKind, DEFAULT_USER, Host, HostDraft, HostDraftError, HostId, PublicId, SSH_PORT,
};
pub use store::{HostStore, HostStoreEvent};

pub use store::seed;
