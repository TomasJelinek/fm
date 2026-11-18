//! Test-owned resident execution; the original runtime and stop owner stay outside services.
use midnight_work_budget::resident::{Owner, Spawner};
use std::{ops::Deref, time::Duration};

pub(crate) struct TestRuntime {
    runtime: tokio::runtime::Runtime,
    owner: Option<Owner>,
    spawner: Spawner,
}

impl TestRuntime {
    pub(crate) fn new(runtime: tokio::runtime::Runtime) -> Self {
        // Compare's approved domain is two workers, started before any Service.
        let (owner, spawner) = Owner::start("compare-ui-test", 2).unwrap();
        Self {
            runtime,
            owner: Some(owner),
            spawner,
        }
    }

    pub(crate) fn spawner(&self) -> Spawner {
        self.spawner.clone()
    }
}

impl Deref for TestRuntime {
    type Target = tokio::runtime::Runtime;

    fn deref(&self) -> &Self::Target {
        &self.runtime
    }
}

impl Drop for TestRuntime {
    fn drop(&mut self) {
        let Some(owner) = self.owner.take() else {
            return;
        };
        // Callers close UI/sessions first. Drive the ORIGINAL time-enabled runtime
        // through worker exit and finite joins before its own fields are destroyed.
        let shutdown = owner.shutdown(self.runtime.handle());
        let result = self
            .runtime
            .block_on(async { tokio::time::timeout(Duration::from_secs(15), shutdown).await });
        if std::thread::panicking() {
            if !result.as_ref().is_ok_and(|result| result.is_ok()) {
                eprintln!(
                    "Compare test resident shutdown unconfirmed during unwinding: {result:?}"
                );
            }
        } else {
            result
                .expect("Compare resident workers did not exit within the test grace period")
                .expect("Compare resident worker cleanup is unconfirmed");
        }
    }
}
