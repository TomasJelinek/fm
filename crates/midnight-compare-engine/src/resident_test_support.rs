use midnight_work_budget::resident::{Owner, Spawner};

pub(crate) fn workers() -> (Owner, Spawner) {
    Owner::start_elastic("compare-fixture", 2).unwrap()
}
