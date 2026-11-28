//! Test-only external resident ownership; callers supply the original runtime and domain size.
use midnight_work_budget::resident::{Owner, Spawner};
use std::{ops::Deref, time::Duration};

pub(crate) struct TestRuntime {
    name: &'static str,
    owner: Option<Owner>,
    spawner: Spawner,
    spares: Vec<midnight_viewer_engine::ImageSpareRetirement>,
    runtime: Option<tokio::runtime::Runtime>,
}

impl TestRuntime {
    pub(crate) fn new(runtime: tokio::runtime::Runtime, name: &'static str, count: usize) -> Self {
        let (owner, spawner) = Owner::start(name, count).unwrap();
        Self {
            name,
            owner: Some(owner),
            spawner,
            spares: Vec::new(),
            runtime: Some(runtime),
        }
    }

    pub(crate) fn spawner(&self) -> Spawner {
        self.spawner.clone()
    }

    pub(crate) fn track_spare(&mut self, spare: midnight_viewer_engine::ImageSpareRetirement) {
        self.spares.push(spare);
    }

    fn retire(&mut self) -> Result<(), String> {
        let Some(owner) = self.owner.take() else {
            return Ok(());
        };
        let runtime = self.runtime.as_ref().unwrap();
        for spare in &self.spares {
            spare.stop_spare();
        }
        runtime.block_on(async {
            let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
            let mut errors = Vec::new();
            for spare in &self.spares {
                match tokio::time::timeout_at(deadline, spare.wait()).await {
                    Ok(Ok(())) => {}
                    result => errors.push(format!("image spare cleanup unconfirmed: {result:?}")),
                }
            }
            match tokio::time::timeout_at(deadline, owner.shutdown(runtime.handle())).await {
                Ok(Ok(())) => {}
                result => errors.push(format!("resident cleanup unconfirmed: {result:?}")),
            }
            if errors.is_empty() {
                Ok(())
            } else {
                Err(errors.join("\n"))
            }
        })
    }

    fn finish(&mut self, timeout: Duration) {
        let result = self.retire();
        if let Some(runtime) = self.runtime.take() {
            // Keep spare tickets and the original time driver through checked retirement,
            // then consume Tokio through bounded cleanup, including assertion unwinding.
            runtime.shutdown_timeout(timeout);
        }
        if std::thread::panicking() {
            if let Err(error) = result {
                eprintln!("{}: {error}", self.name);
            }
        } else {
            result.unwrap_or_else(|error| panic!("{}: {error}", self.name));
        }
    }

    pub(crate) fn shutdown_timeout(mut self, timeout: Duration) {
        self.finish(timeout);
    }
}

impl Deref for TestRuntime {
    type Target = tokio::runtime::Runtime;

    fn deref(&self) -> &Self::Target {
        self.runtime.as_ref().unwrap()
    }
}

impl Drop for TestRuntime {
    fn drop(&mut self) {
        if self.runtime.is_some() {
            self.finish(Duration::from_secs(1));
        }
    }
}
