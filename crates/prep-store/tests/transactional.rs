use prep_store::{PublishOutcome, Store, StoreResultId};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Barrier};
use std::thread;

static TEST_COUNTER: AtomicU64 = AtomicU64::new(1);

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "prep-store-integration-{label}-{}-{:x}",
            std::process::id(),
            TEST_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).expect("create test directory");
        Self(fs::canonicalize(path).expect("canonicalize test directory"))
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn result_id(byte: &str) -> StoreResultId {
    StoreResultId::parse(format!("sha256:{}", byte.repeat(64))).expect("valid result id")
}

#[test]
fn concurrent_same_result_publication_has_one_authoritative_result() {
    let temp = TestDirectory::new("concurrent");
    let store = Arc::new(Store::open(&temp.0.join("store")).expect("open store"));
    let barrier = Arc::new(Barrier::new(2));
    let id = result_id("7");

    let handles = [b"first".as_slice(), b"second".as_slice()].map(|contents| {
        let store = Arc::clone(&store);
        let barrier = Arc::clone(&barrier);
        let id = id.clone();
        thread::spawn(move || {
            let transaction = store.begin().expect("begin transaction");
            fs::write(transaction.prefix().join("tool"), contents).expect("write staged output");
            barrier.wait();
            transaction.commit(&id).expect("commit result")
        })
    });

    let outcomes = handles.map(|handle| handle.join().expect("join publisher"));
    let published = outcomes
        .iter()
        .filter(|outcome| matches!(outcome, PublishOutcome::Published(_)))
        .count();
    let existing = outcomes
        .iter()
        .filter(|outcome| matches!(outcome, PublishOutcome::Existing(_)))
        .count();

    assert_eq!(published, 1);
    assert_eq!(existing, 1);
    assert!(store.get(&id).expect("read final result").is_some());
}

#[test]
fn metadata_creation_failure_never_publishes_a_result() {
    let temp = TestDirectory::new("metadata-failure");
    let store = Store::open(&temp.0.join("store")).expect("open store");
    let id = result_id("8");
    let transaction = store.begin().expect("begin transaction");
    fs::write(transaction.prefix().join("tool"), b"tool").expect("write staged output");
    fs::create_dir(transaction.staging_path().join("result.toml"))
        .expect("block result metadata creation");

    assert!(transaction.commit(&id).is_err());
    assert!(store.get(&id).expect("read result after failure").is_none());
}

#[cfg(unix)]
#[test]
fn post_rename_lease_failure_rolls_back_published_result() {
    let temp = TestDirectory::new("post-rename-failure");
    let store_root = temp.0.join("store");
    let store = Store::open(&store_root).expect("open store");
    let id = result_id("9");
    let transaction = store.begin().expect("begin transaction");
    fs::write(transaction.prefix().join("tool"), b"tool").expect("write staged output");

    let transaction_name = transaction
        .staging_path()
        .file_name()
        .expect("transaction name")
        .to_owned();
    let lock_path = store_root
        .join(".locks")
        .join(PathBuf::from(transaction_name))
        .with_extension("lock");

    fs::remove_file(&lock_path).expect("unlink live lease path");
    fs::write(&lock_path, b"hostile replacement").expect("replace live lease path");

    assert!(transaction.commit(&id).is_err());
    assert!(
        store.get(&id).expect("read result after failed commit").is_none(),
        "a commit that reports failure must not leave a valid published result"
    );
}
