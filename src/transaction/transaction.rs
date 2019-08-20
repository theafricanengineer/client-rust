// Copyright 2019 TiKV Project Authors. Licensed under Apache-2.0.

use crate::transaction::{Mutation, MutationValue, Timestamp};
use crate::{Key, KvPair, Result, Value};

use derive_new::new;
use futures::stream::BoxStream;
use std::{collections::BTreeMap, ops::RangeBounds};

/// A undo-able set of actions on the dataset.
///
/// Using a transaction you can prepare a set of actions (such as `get`, or `set`) on data at a
/// particular timestamp obtained from the placement driver.
///
/// Once a transaction is commited, a new commit timestamp is obtained from the placement driver.
///
/// Create a new transaction from a timestamp using `new`.
///
/// ```rust,no_run
/// # #![feature(async_await)]
/// use tikv_client::{Config, TransactionClient};
/// use futures::prelude::*;
/// # futures::executor::block_on(async {
/// let connect = TransactionClient::connect(Config::default());
/// let client = connect.await.unwrap();
/// let txn = client.begin().await.unwrap();
/// # });
/// ```
#[derive(new)]
pub struct Transaction {
    pub timestamp: Timestamp,
    #[new(default)]
    mutations: BTreeMap<Key, Mutation>,
}

impl Transaction {
    /// Gets the value associated with the given key.
    ///
    /// ```rust,no_run
    /// # #![feature(async_await)]
    /// # use tikv_client::{Value, Config, transaction::Client};
    /// # use futures::prelude::*;
    /// # futures::executor::block_on(async {
    /// # let connecting_client = Client::connect(Config::new(vec!["192.168.0.100", "192.168.0.101"]));
    /// # let connected_client = connecting_client.await.unwrap();
    /// let mut txn = connected_client.begin().await.unwrap();
    /// let key = "TiKV".to_owned();
    /// let result: Option<Value> = txn.get(key).await.unwrap();
    /// // Finish the transaction...
    /// txn.commit().await.unwrap();
    /// # });
    /// ```
    pub async fn get(&self, key: impl Into<Key>) -> Result<Option<Value>> {
        let key = key.into();
        match self.get_from_mutations(&key) {
            MutationValue::Determined(value) => Ok(value),
            MutationValue::Undetermined => self.get_snap(key).await,
        }
    }
    async fn get_snap(&self, _key: impl Into<Key>) -> Result<Option<Value>> {
        unimplemented!()
    }

    /// Gets the values associated with the given keys. The returned iterator is in the same order
    /// as the given keys.
    ///
    /// ```rust,no_run
    /// # #![feature(async_await)]
    /// # use tikv_client::{Key, Value, Config, transaction::Client};
    /// # use futures::prelude::*;
    /// # use std::collections::HashMap;
    /// # futures::executor::block_on(async {
    /// # let connecting_client = Client::connect(Config::new(vec!["192.168.0.100", "192.168.0.101"]));
    /// # let connected_client = connecting_client.await.unwrap();
    /// let mut txn = connected_client.begin().await.unwrap();
    /// let keys = vec!["TiKV".to_owned(), "TiDB".to_owned()];
    /// let result: HashMap<Key, Value> = txn
    ///     .batch_get(keys)
    ///     .await
    ///     .unwrap()
    ///     .filter_map(|(k, v)| v.map(move |v| (k, v))).collect();
    /// // Finish the transaction...
    /// txn.commit().await.unwrap();
    /// # });
    /// ```
    pub async fn batch_get(
        &self,
        keys: impl IntoIterator<Item = impl Into<Key>>,
    ) -> Result<impl Iterator<Item = (Key, Option<Value>)>> {
        // Partition the keys into those we have buffered and those we have to
        // get from the store.
        let (undetermined_keys, cached_results): (Vec<(Key, MutationValue)>, _) = keys
            .into_iter()
            .map(|k| {
                let key = k.into();
                let value = self.get_from_mutations(&key);
                (key, value)
            })
            .partition(|(_, v)| *v == MutationValue::Undetermined);

        let cached_results = cached_results.into_iter().map(|(k, v)| (k, v.unwrap()));
        let undetermined_keys = undetermined_keys.into_iter().map(|(k, _)| k);
        let fetched_results = self.batch_get_snap(undetermined_keys).await?;
        let results = cached_results.chain(fetched_results);
        Ok(results)
    }
    async fn batch_get_snap(
        &self,
        _keys: impl IntoIterator<Item = impl Into<Key>>,
    ) -> Result<impl Iterator<Item = (Key, Option<Value>)>> {
        Ok(::std::iter::empty())
    }

    pub fn scan(&self, _range: impl RangeBounds<Key>) -> BoxStream<Result<KvPair>> {
        unimplemented!()
    }

    pub fn scan_reverse(&self, _range: impl RangeBounds<Key>) -> BoxStream<Result<KvPair>> {
        unimplemented!()
    }

    /// Sets the value associated with the given key.
    ///
    /// ```rust,no_run
    /// # #![feature(async_await)]
    /// # use tikv_client::{Key, Value, Config, transaction::Client};
    /// # use futures::prelude::*;
    /// # futures::executor::block_on(async {
    /// # let connecting_client = Client::connect(Config::new(vec!["192.168.0.100", "192.168.0.101"]));
    /// # let connected_client = connecting_client.await.unwrap();
    /// let mut txn = connected_client.begin().await.unwrap();
    /// let key = "TiKV".to_owned();
    /// let val = "TiKV".to_owned();
    /// txn.set(key, val);
    /// // Finish the transaction...
    /// txn.commit().await.unwrap();
    /// # });
    /// ```
    pub fn set(&mut self, key: impl Into<Key>, value: impl Into<Value>) {
        self.mutations
            .insert(key.into(), Mutation::Put(value.into()));
    }

    /// Deletes the given key.
    ///
    /// ```rust,no_run
    /// # #![feature(async_await)]
    /// # use tikv_client::{Key, Config, transaction::Client};
    /// # use futures::prelude::*;
    /// # futures::executor::block_on(async {
    /// # let connecting_client = Client::connect(Config::new(vec!["192.168.0.100", "192.168.0.101"]));
    /// # let connected_client = connecting_client.await.unwrap();
    /// let mut txn = connected_client.begin().await.unwrap();
    /// let key = "TiKV".to_owned();
    /// txn.delete(key);
    /// // Finish the transaction...
    /// txn.commit().await.unwrap();
    /// # });
    /// ```
    pub fn delete(&mut self, key: impl Into<Key>) {
        self.mutations.insert(key.into(), Mutation::Del);
    }

    /// Locks the given keys.
    ///
    /// ```rust,no_run
    /// # #![feature(async_await)]
    /// # use tikv_client::{Config, transaction::Client};
    /// # use futures::prelude::*;
    /// # futures::executor::block_on(async {
    /// # let connect = Client::connect(Config::default());
    /// # let connected_client = connect.await.unwrap();
    /// let mut txn = connected_client.begin().await.unwrap();
    /// txn.lock_keys(vec!["TiKV".to_owned(), "Rust".to_owned()]);
    /// // ... Do some actions.
    /// txn.commit().await.unwrap();
    /// # });
    /// ```
    pub fn lock_keys(&mut self, keys: impl IntoIterator<Item = impl Into<Key>>) {
        for key in keys {
            let key = key.into();
            // Mutated keys don't need a lock.
            self.mutations.entry(key).or_insert(Mutation::Lock);
        }
    }

    /// Commits the actions of the transaction.
    ///
    /// ```rust,no_run
    /// # #![feature(async_await)]
    /// # use tikv_client::{Config, transaction::Client};
    /// # use futures::prelude::*;
    /// # futures::executor::block_on(async {
    /// # let connect = Client::connect(Config::default());
    /// # let connected_client = connect.await.unwrap();
    /// let mut txn = connected_client.begin().await.unwrap();
    /// // ... Do some actions.
    /// let req = txn.commit();
    /// let result: () = req.await.unwrap();
    /// # });
    /// ```
    pub async fn commit(&mut self) -> Result<()> {
        self.prewrite().await?;
        self.commit_primary().await?;
        // FIXME: return from this method once the primary key is committed
        let _ = self.commit_secondary().await;
        Ok(())
    }

    async fn prewrite(&mut self) -> Result<()> {
        // TODO: Too many clones. Consider using bytes::Byte.
        let _rpc_mutations: Vec<_> = self
            .mutations
            .iter()
            .map(|(k, v)| v.clone().into_proto_with_key(k.clone()))
            .collect();
        unimplemented!()
    }

    async fn commit_primary(&mut self) -> Result<()> {
        unimplemented!()
    }

    async fn commit_secondary(&mut self) -> Result<()> {
        unimplemented!()
    }

    fn get_from_mutations(&self, key: &Key) -> MutationValue {
        self.mutations
            .get(key)
            .map(Mutation::get_value)
            .unwrap_or(MutationValue::Undetermined)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::executor::block_on;

    #[test]
    fn set_and_get_from_buffer() {
        let mut txn = mock_txn();
        txn.set(b"key1".to_vec(), b"value1".to_vec());
        txn.set(b"key2".to_vec(), b"value2".to_vec());
        assert_eq!(
            block_on(txn.get(b"key1".to_vec())).unwrap().unwrap(),
            b"value1".to_vec().into()
        );

        txn.delete(b"key2".to_vec());
        txn.set(b"key1".to_vec(), b"value".to_vec());
        assert_eq!(
            block_on(txn.batch_get(vec![b"key2".to_vec(), b"key1".to_vec()]))
                .unwrap()
                .collect::<Vec<_>>(),
            vec![
                (Key::from(b"key2".to_vec()), None),
                (
                    Key::from(b"key1".to_vec()),
                    Some(Value::from(b"value".to_vec()))
                ),
            ]
        );
    }

    fn mock_txn() -> Transaction {
        let timestamp = Timestamp {
            physical: 0,
            logical: 0,
        };
        Transaction {
            timestamp,
            mutations: Default::default(),
        }
    }
}
