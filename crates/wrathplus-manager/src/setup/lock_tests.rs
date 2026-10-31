//! Synthetic state and target fencing regressions.
use super::*;
use crate::setup::tests::Fixture;

#[test]
fn shared_state_is_fenced_across_different_target_directories() {
    let f = Fixture::new();
    let mut other = f.config.clone();
    other.target = f.config.repo.join("other-target");
    let lock = operation_lock(&f.config).unwrap();
    assert!(operation_lock(&other).is_err());
    drop(lock);
    let lock = operation_lock(&other).unwrap();
    assert!(operation_lock(&f.config).is_err());
    drop(lock);
    fs::write(running_path(&f.config), b"fixture").unwrap();
    assert!(require_stopped(&other).is_err());
    assert!(with_stopped_state(&other, |_| Ok(())).is_err());
    fs::remove_file(running_path(&f.config)).unwrap();
    assert!(with_stopped_state(&other, |_| Ok(())).is_ok());
    // Older Managers only wrote the target marker. Continue honoring it.
    fs::write(f.config.target.join("manager/running.lock"), b"legacy").unwrap();
    assert!(require_stopped(&f.config).is_err());
}

#[test]
fn shared_target_remains_fenced_and_partial_lock_acquisition_is_released() {
    let first = Fixture::new();
    let second = Fixture::new();
    let mut other = second.config.clone();
    other.target = first.config.target.clone();
    let lock = operation_lock(&first.config).unwrap();
    assert!(operation_lock(&other).is_err());
    assert!(!other.state_root().join("manager/operation.lock").exists());
    drop(lock);
    assert!(operation_lock(&other).is_ok());
    fs::write(first.config.target.join("manager/running.lock"), b"fixture").unwrap();
    assert!(require_stopped(&other).is_err());
}

#[cfg(unix)]
#[test]
fn distinct_checkouts_resolving_to_one_state_home_share_the_fence() {
    let first = Fixture::new();
    let second = Fixture::new();
    let lock = operation_lock(&first.config).unwrap();
    std::os::unix::fs::symlink(first.config.state_root(), second.config.state_root()).unwrap();
    assert!(operation_lock(&second.config).is_err());
    drop(lock);
    fs::write(running_path(&first.config), b"fixture").unwrap();
    assert!(require_stopped(&second.config).is_err());
}
