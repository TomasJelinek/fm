//! One metadata refresh is permitted only before a comparison Session exists.
use midnight_compare_engine::{OpenError, Service, Session};
use midnight_core::{FileCompareLaunch, ViewerAppearance, ViewerGeometry};

pub(super) fn open(
    service: &Service,
    launch: FileCompareLaunch,
    appearance: ViewerAppearance,
    geometry: ViewerGeometry,
) -> Result<Session, OpenError> {
    open_impl(service, launch, appearance, geometry, |_| {})
}

#[cfg(test)]
pub(super) fn open_observed(
    service: &Service,
    launch: FileCompareLaunch,
    appearance: ViewerAppearance,
    geometry: ViewerGeometry,
    observe: impl FnMut(&FileCompareLaunch),
) -> Result<Session, OpenError> {
    open_impl(service, launch, appearance, geometry, observe)
}

fn open_impl(
    service: &Service,
    mut launch: FileCompareLaunch,
    appearance: ViewerAppearance,
    geometry: ViewerGeometry,
    mut observe: impl FnMut(&FileCompareLaunch),
) -> Result<Session, OpenError> {
    observe(&launch);
    match service.open(launch.clone(), appearance.clone(), geometry) {
        Err(OpenError::PolicyChanged) => {
            let current = service.comparator_snapshot();
            launch.comparators = current.preferences;
            launch.comparator_policy = current.id;
            observe(&launch);
            service.open(launch, appearance, geometry)
        }
        result => result,
    }
}
