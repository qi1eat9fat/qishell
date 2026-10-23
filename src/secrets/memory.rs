//! 不落盘的秘密实现：测试用，也用于秘密库打不开时的降级。

use std::collections::HashMap;
use std::sync::Mutex;

use anyhow::Result;
use zeroize::Zeroizing;

use super::{SecretRef, SecretStore};

/// 存在内存里的秘密。UI 测试用它，绝不碰磁盘。
#[derive(Default)]
pub struct InMemorySecretStore {
    secrets: Mutex<HashMap<SecretRef, Zeroizing<String>>>,
}

impl InMemorySecretStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// 测试断言用：现在存着哪几条。
    pub fn account_names(&self) -> Vec<String> {
        let secrets = self
            .secrets
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let mut names: Vec<String> = secrets.keys().map(SecretRef::account).collect();
        names.sort();
        names
    }
}

impl SecretStore for InMemorySecretStore {
    fn get(&self, reference: &SecretRef) -> Result<Option<Zeroizing<String>>> {
        let secrets = self
            .secrets
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        Ok(secrets.get(reference).cloned())
    }

    fn set(&self, reference: &SecretRef, secret: &str) -> Result<()> {
        let mut secrets = self
            .secrets
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        secrets.insert(reference.clone(), Zeroizing::new(secret.to_string()));
        Ok(())
    }

    fn delete(&self, reference: &SecretRef) -> Result<()> {
        let mut secrets = self
            .secrets
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        secrets.remove(reference);
        Ok(())
    }

    fn is_available(&self) -> bool {
        true
    }
}

/// 什么都不存：秘密库打不开时的降级。密码每次都会问用户。
pub struct NoSecretStore;

impl SecretStore for NoSecretStore {
    fn get(&self, _: &SecretRef) -> Result<Option<Zeroizing<String>>> {
        Ok(None)
    }

    fn set(&self, _: &SecretRef, _: &str) -> Result<()> {
        anyhow::bail!("秘密库不可用")
    }

    fn delete(&self, _: &SecretRef) -> Result<()> {
        Ok(())
    }

    fn is_available(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_secret_comes_back_and_goes_away() {
        let store = InMemorySecretStore::new();
        let reference = SecretRef::password("deploy", "10.0.0.1", 22);
        assert!(store.get(&reference).unwrap().is_none());
        store.set(&reference, "hunter2").unwrap();
        assert_eq!(
            store
                .get(&reference)
                .unwrap()
                .as_deref()
                .map(String::as_str),
            Some("hunter2")
        );
        store.delete(&reference).unwrap();
        assert!(store.get(&reference).unwrap().is_none());
    }

    #[test]
    fn the_plain_store_keeps_nothing() {
        let store = NoSecretStore;
        assert!(!store.is_available());
        let reference = SecretRef::password("deploy", "10.0.0.1", 22);
        assert!(store.set(&reference, "hunter2").is_err());
        assert!(store.get(&reference).unwrap().is_none());
    }
}
